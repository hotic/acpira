import DiffWorker from './codeDiff.worker?worker&inline';
import type { DiffLine, DiffSource } from '@shared/transcript';
import type { CodeDiffRow } from './codeDiff';
import type { CodeToken, Language } from './codeSyntax';
import type { HighlightRequest, HighlightResponse, HighlightResult } from './codeDiff.worker';

let worker: Worker | undefined;
let unavailable: Error | undefined;
let sequence = 0;
const pending = new Map<number, { resolve: (result: HighlightResult) => void; reject: (error: Error) => void }>();

type Payload = HighlightRequest extends infer R ? R extends HighlightRequest ? Omit<R, 'id'> : never : never;

// The inline worker is bundled into the webview script, so both IDE hosts can
// start it from a blob without fetching worker modules through a separate origin.
function request(payload: Payload): Promise<HighlightResult> {
  if (unavailable) return Promise.reject(unavailable);
  if (!worker) {
    try {
      worker = new DiffWorker({ name: 'acpira-syntax-highlight' });
      worker.onmessage = ({ data }: MessageEvent<HighlightResponse>) => {
        const entry = pending.get(data.id);
        if (!entry) return;
        pending.delete(data.id);
        if (data.error !== undefined) entry.reject(new Error(data.error));
        else entry.resolve(data.result);
      };
      worker.onerror = event => {
        event.preventDefault();
        unavailable = new Error(event.message || 'Syntax highlighting worker failed');
        worker?.terminate();
        worker = undefined;
        for (const entry of pending.values()) entry.reject(unavailable);
        pending.clear();
      };
    } catch (error) {
      unavailable = error instanceof Error ? error : new Error(String(error));
      return Promise.reject(unavailable);
    }
  }
  const id = ++sequence;
  return new Promise((resolve, reject) => {
    pending.set(id, { resolve, reject });
    try { worker!.postMessage({ ...payload, id } satisfies HighlightRequest); }
    catch (error) { pending.delete(id); reject(error); }
  });
}

export function requestDiffHighlight(lines: DiffLine[], source: DiffSource | undefined, path: string): Promise<CodeDiffRow[]> {
  return request({ kind: 'diff', lines, source, path }) as Promise<CodeDiffRow[]>;
}

// One token array per source line, in both theme colours
export function requestCodeHighlight(text: string, language: Language): Promise<CodeToken[][]> {
  return request({ kind: 'code', text, language }) as Promise<CodeToken[][]>;
}
