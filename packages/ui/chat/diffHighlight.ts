import type { DiffLine, DiffSource } from '@acpira/shared/transcript';
import type { CodeDiffRow } from './codeDiff';
import type { CodeToken, Language } from './codeLanguage';
import type { HighlightRequest, HighlightResponse, HighlightResult } from './codeDiff.worker';

let worker: Promise<Worker> | undefined;
let unavailable: Error | undefined;
let sequence = 0;
const MAX_IN_FLIGHT = 2;
const pending = new Map<number, { resolve: (result: HighlightResult) => void; reject: (error: Error) => void }>();
const waiting: { id: number; payload: Payload; resolve: (result: HighlightResult) => void; reject: (error: Error) => void }[] = [];

type Payload = HighlightRequest extends infer R ? R extends HighlightRequest ? Omit<R, 'id'> : never : never;

// Highlighting stops for the page's lifetime; whatever was waiting gets the error
function fail(error: unknown): Error {
  unavailable = error instanceof Error ? error : new Error(String(error));
  for (const entry of pending.values()) entry.reject(unavailable);
  pending.clear();
  for (const item of waiting.splice(0)) item.reject(unavailable);
  return unavailable;
}

function pump(w: Worker) {
  while (pending.size < MAX_IN_FLIGHT && waiting.length > 0) {
    const next = waiting.shift()!;
    pending.set(next.id, { resolve: next.resolve, reject: next.reject });
    w.postMessage({ ...next.payload, id: next.id } satisfies HighlightRequest);
  }
}

// The inline worker runs from a blob, so both IDE hosts can start it without fetching worker modules through a separate
// origin. Its script (shiki and its grammars, about 1.4 MB) is a chunk of its own, loaded with the first request
// instead of with the page
function start(): Promise<Worker> {
  return worker ??= import('./codeDiff.worker?worker&inline').then(({ default: DiffWorker }) => {
    const w = new DiffWorker({ name: 'acpira-syntax-highlight' });
    w.onmessage = ({ data }: MessageEvent<HighlightResponse>) => {
      const entry = pending.get(data.id);
      if (entry) {
        pending.delete(data.id);
        if (data.error !== undefined) entry.reject(new Error(data.error));
        else entry.resolve(data.result);
      }
      pump(w);
    };
    w.onerror = event => {
      event.preventDefault();
      w.terminate();
      fail(new Error(event.message || 'Syntax highlighting worker failed'));
    };
    return w;
  }).catch(error => { throw fail(error); });
}

function request(payload: Payload, signal?: AbortSignal): Promise<HighlightResult> {
  if (unavailable) return Promise.reject(unavailable);
  if (signal?.aborted) return Promise.reject(new Error('Aborted'));
  const id = ++sequence;
  return new Promise((resolve, reject) => {
    const item = { id, payload, resolve, reject };
    waiting.push(item);
    signal?.addEventListener('abort', () => {
      const idx = waiting.indexOf(item);
      if (idx >= 0) {
        waiting.splice(idx, 1);
        reject(new Error('Aborted'));
      } else if (pending.delete(id)) {
        reject(new Error('Aborted'));
      }
    }, { once: true });
    start().then(w => pump(w)).catch(error => {
      const idx = waiting.indexOf(item);
      if (idx >= 0) waiting.splice(idx, 1);
      if (pending.delete(id) || idx >= 0) reject(error instanceof Error ? error : new Error(String(error)));
    });
  });
}

export function requestDiffHighlight(lines: DiffLine[], source: DiffSource | undefined, path: string, signal?: AbortSignal): Promise<CodeDiffRow[]> {
  return request({ kind: 'diff', lines, source, path }, signal) as Promise<CodeDiffRow[]>;
}

// One token array per source line, in both theme colours
export function requestCodeHighlight(text: string, language: Language): Promise<CodeToken[][]> {
  return request({ kind: 'code', text, language }) as Promise<CodeToken[][]>;
}
