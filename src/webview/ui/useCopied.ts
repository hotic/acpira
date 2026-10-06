import { useCallback, useEffect, useState } from 'react';

export type CopyState = 'idle' | 'copied' | 'failed';

// One clipboard state machine: idle → copied / failed → idle after 1.8 s, and a changed source resets it immediately.
// `write` should be memoized on the same inputs as `source` so the returned `copy` stays stable between renders
export function useCopyAction(source: unknown, write: () => Promise<void>): { state: CopyState; copy: () => Promise<void> } {
  const [state, setState] = useState<CopyState>('idle');
  useEffect(() => { setState('idle'); }, [source]);
  useEffect(() => {
    if (state === 'idle') return;
    const timer = window.setTimeout(() => setState('idle'), 1800);
    return () => window.clearTimeout(timer);
  }, [state]);
  const copy = useCallback(async () => {
    try { await write(); setState('copied'); } catch { setState('failed'); }
  }, [write]);
  return { state, copy };
}

// `read`, when given, supplies the text at click time (fetched from the host): the clipboard is handed the pending text at
// once, inside the click's user activation, which a write after the fetch would have outlived
export function useCopied(text: string, read?: () => Promise<string>): { state: CopyState; copy: () => Promise<void> } {
  const write = useCallback(() => read
    ? navigator.clipboard.write([new ClipboardItem({ 'text/plain': read().then(value => new Blob([value], { type: 'text/plain' })) })])
    : navigator.clipboard.writeText(text), [text, read]);
  return useCopyAction(text, write);
}
