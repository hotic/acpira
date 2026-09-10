import DiffWorker from './codeDiff.worker?worker&inline';
import type { DiffLine, DiffSource } from '@shared/transcript';
import type { CodeDiffRow } from './codeDiff';
import type { DiffHighlightRequest, DiffHighlightResponse } from './codeDiff.worker';

let worker: Worker | undefined;
let unavailable: Error | undefined;
let sequence = 0;
const pending = new Map<number, { resolve: (rows: CodeDiffRow[]) => void; reject: (error: Error) => void }>();

// The inline worker is bundled into the webview script, so both IDE hosts can
// start it from a blob without fetching worker modules through a separate origin.
export function requestDiffHighlight(lines: DiffLine[], source: DiffSource | undefined, path: string): Promise<CodeDiffRow[]> {
  if (unavailable) return Promise.reject(unavailable);
  if (!worker) {
    try {
      worker = new DiffWorker({ name: 'acpira-diff-highlight' });
      worker.onmessage = ({ data }: MessageEvent<DiffHighlightResponse>) => {
        const request = pending.get(data.id);
        if (!request) return;
        pending.delete(data.id);
        if (data.error !== undefined) request.reject(new Error(data.error));
        else request.resolve(data.rows);
      };
      worker.onerror = event => {
        event.preventDefault();
        unavailable = new Error(event.message || 'Diff highlighting worker failed');
        worker?.terminate();
        worker = undefined;
        for (const request of pending.values()) request.reject(unavailable);
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
    try { worker!.postMessage({ id, lines, source, path } satisfies DiffHighlightRequest); }
    catch (error) { pending.delete(id); reject(error); }
  });
}
