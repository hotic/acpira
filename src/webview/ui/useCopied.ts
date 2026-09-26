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

export function useCopied(text: string): { state: CopyState; copy: () => Promise<void> } {
  const write = useCallback(() => navigator.clipboard.writeText(text), [text]);
  return useCopyAction(text, write);
}
