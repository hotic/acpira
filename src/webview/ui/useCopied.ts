import { useCallback, useEffect, useState } from 'react';

// One clipboard state machine: idle → copied / failed → idle after 1.8 s, and a changed source text resets it immediately
export function useCopied(text: string): { state: 'idle' | 'copied' | 'failed'; copy: () => Promise<void> } {
  const [state, setState] = useState<'idle' | 'copied' | 'failed'>('idle');
  useEffect(() => { setState('idle'); }, [text]);
  useEffect(() => {
    if (state === 'idle') return;
    const timer = window.setTimeout(() => setState('idle'), 1800);
    return () => window.clearTimeout(timer);
  }, [state]);
  const copy = useCallback(async () => {
    try { await navigator.clipboard.writeText(text); setState('copied'); } catch { setState('failed'); }
  }, [text]);
  return { state, copy };
}
