import type { DiffLine, DiffSource } from '@acpira/shared/transcript';
import type { CodeDiffRow } from './codeDiff';
import type { CodeTheme, CodeToken, Language } from './codeLanguage';
import type { HighlightRequest, HighlightResponse, HighlightResult } from './codeDiff.worker';

let worker: Promise<Worker> | undefined;
let unavailable: Error | undefined;
let sequence = 0;
// At most MAX_IN_FLIGHT requests are with the worker at a time, `first` ones not counted: those (a file the user just
// opened, its text and its removed hunks as one more) go at once, and the worker gives them turns with the rest instead
// of queueing them behind
const MAX_IN_FLIGHT = 2;
interface Entry { first: boolean; part?: (lines: CodeToken[][]) => void; resolve: (result: HighlightResult) => void; reject: (error: Error) => void }
const pending = new Map<number, Entry>();
const waiting: (Entry & { id: number; payload: Payload; signal?: AbortSignal })[] = [];

type Payload = HighlightRequest extends infer R ? R extends HighlightRequest ? R extends { kind: 'cancel' } ? never : Omit<R, 'id'> : never : never;

// Highlighting stops for the page's lifetime; whatever was waiting gets the error
function fail(error: unknown): Error {
  unavailable = error instanceof Error ? error : new Error(String(error));
  for (const entry of pending.values()) entry.reject(unavailable);
  pending.clear();
  for (const item of waiting.splice(0)) item.reject(unavailable);
  return unavailable;
}

const inFlight = () => {
  let n = 0;
  for (const entry of pending.values()) if (!entry.first) n++;
  return n;
};

// `first` requests stand at the head of the queue (`request`)
function pump(w: Worker) {
  while (waiting.length > 0 && (waiting[0]!.first || inFlight() < MAX_IN_FLIGHT)) {
    const { id, payload, ...entry } = waiting.shift()!;
    pending.set(id, entry);
    w.postMessage({ ...payload, id } satisfies HighlightRequest);
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
      if (data.part) {
        entry?.part?.(data.part);
        return;
      }
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

function request(payload: Payload, signal?: AbortSignal, first = false, part?: Entry['part']): Promise<HighlightResult> {
  if (unavailable) return Promise.reject(unavailable);
  if (signal?.aborted) return Promise.reject(new Error('Aborted'));
  const id = ++sequence;
  return new Promise((resolve, reject) => {
    const item = { id, payload, first, part, resolve, reject, signal };
    // `first` ones go ahead of the rest, in the order they came
    const ahead = first ? waiting.findIndex(w => !w.first) : -1;
    if (ahead >= 0) waiting.splice(ahead, 0, item);
    else waiting.push(item);
    signal?.addEventListener('abort', () => {
      const idx = waiting.indexOf(item);
      if (idx >= 0) {
        waiting.splice(idx, 1);
        reject(new Error('Aborted'));
      } else if (pending.delete(id)) {
        reject(new Error('Aborted'));
        // The worker ends the run at its next slice, and its place goes to the next request
        void worker?.then(w => {
          w.postMessage({ kind: 'cancel', id } satisfies HighlightRequest);
          pump(w);
        }, () => undefined);
      }
    }, { once: true });
    start().then(w => pump(w)).catch(error => {
      const idx = waiting.indexOf(item);
      if (idx >= 0) waiting.splice(idx, 1);
      if (pending.delete(id) || idx >= 0) reject(error instanceof Error ? error : new Error(String(error)));
    });
  });
}

// A waiting request whose code came into sight (a fence the reader scrolled to) goes ahead of the rest, as a `first`
// one: asked out of sight, it stood behind every fence asked before it
export function promoteHighlight(signal: AbortSignal) {
  const at = waiting.findIndex(w => w.signal === signal);
  if (at < 0 || waiting[at]!.first) return;
  const [item] = waiting.splice(at, 1);
  item!.first = true;
  const ahead = waiting.findIndex(w => !w.first);
  waiting.splice(ahead < 0 ? waiting.length : ahead, 0, item!);
  void worker?.then(pump, () => undefined);
}

export function requestDiffHighlight(lines: DiffLine[], source: DiffSource | undefined, path: string, theme: CodeTheme, signal?: AbortSignal): Promise<CodeDiffRow[]> {
  return request({ kind: 'diff', lines, source, path, theme }, signal) as Promise<CodeDiffRow[]>;
}

// One token array per source line, coloured for `theme`. `signal` withdraws the request (and stops it in the worker);
// `first` puts it ahead of every other one; `onPart` gets the lines of a long source in runs from the top as they are
// tokenized, before the whole result
export function requestCodeHighlight(text: string, language: Language, theme: CodeTheme,
  opts: { signal?: AbortSignal; first?: boolean; onPart?: (lines: CodeToken[][]) => void } = {}): Promise<CodeToken[][]> {
  return request({ kind: 'code', text, language, theme, parts: !!opts.onPart }, opts.signal, opts.first, opts.onPart) as Promise<CodeToken[][]>;
}

// Many short sources in one request (a file's removed hunks), each tokenized from the grammar's first state
export function requestCodesHighlight(texts: string[], language: Language, theme: CodeTheme,
  opts: { signal?: AbortSignal; first?: boolean } = {}): Promise<CodeToken[][][]> {
  return request({ kind: 'codes', texts, language, theme }, opts.signal, opts.first) as Promise<CodeToken[][][]>;
}
