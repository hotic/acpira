import type { DiffLine, DiffSource } from '@shared/transcript';
import type { CodeDiffRow } from './codeDiff';
import type { CodeToken, Language } from './codeLanguage';
import type { HighlightRequest, HighlightResponse, HighlightResult } from './codeDiff.worker';

let worker: Promise<Worker> | undefined;
let unavailable: Error | undefined;
let sequence = 0;
const pending = new Map<number, { resolve: (result: HighlightResult) => void; reject: (error: Error) => void }>();

type Payload = HighlightRequest extends infer R ? R extends HighlightRequest ? Omit<R, 'id'> : never : never;

// Highlighting stops for the page's lifetime; whatever was waiting gets the error
function fail(error: unknown): Error {
  unavailable = error instanceof Error ? error : new Error(String(error));
  for (const entry of pending.values()) entry.reject(unavailable);
  pending.clear();
  return unavailable;
}

// The inline worker runs from a blob, so both IDE hosts can start it without fetching worker modules through a separate
// origin. Its script (shiki and its grammars, about 1.4 MB) is a chunk of its own, loaded with the first request
// instead of with the page
function start(): Promise<Worker> {
  return worker ??= import('./codeDiff.worker?worker&inline').then(({ default: DiffWorker }) => {
    const w = new DiffWorker({ name: 'acpira-syntax-highlight' });
    w.onmessage = ({ data }: MessageEvent<HighlightResponse>) => {
      const entry = pending.get(data.id);
      if (!entry) return;
      pending.delete(data.id);
      if (data.error !== undefined) entry.reject(new Error(data.error));
      else entry.resolve(data.result);
    };
    w.onerror = event => {
      event.preventDefault();
      w.terminate();
      fail(new Error(event.message || 'Syntax highlighting worker failed'));
    };
    return w;
  }).catch(error => { throw fail(error); });
}

function request(payload: Payload): Promise<HighlightResult> {
  if (unavailable) return Promise.reject(unavailable);
  const id = ++sequence;
  return new Promise((resolve, reject) => {
    pending.set(id, { resolve, reject });
    start().then(w => w.postMessage({ ...payload, id } satisfies HighlightRequest)).catch(error => {
      // A worker that could not start has rejected every waiting request already (`fail`)
      if (pending.delete(id)) reject(error instanceof Error ? error : new Error(String(error)));
    });
  });
}

export function requestDiffHighlight(lines: DiffLine[], source: DiffSource | undefined, path: string): Promise<CodeDiffRow[]> {
  return request({ kind: 'diff', lines, source, path }) as Promise<CodeDiffRow[]>;
}

// One token array per source line, in both theme colours
export function requestCodeHighlight(text: string, language: Language): Promise<CodeToken[][]> {
  return request({ kind: 'code', text, language }) as Promise<CodeToken[][]>;
}
