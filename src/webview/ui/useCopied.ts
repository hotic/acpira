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

// Copy text the host still has to supply: the clipboard is handed the pending text at once, inside the click's user activation,
// which a write after the fetch would have outlived. A webview whose ClipboardItem takes no promise (or has none) writes the text
// once it arrives instead; a failed read fails the copy either way. Call it synchronously from the click
export async function writeClipboardLater(read: () => Promise<string>): Promise<void> {
  const pending = read();
  try {
    await navigator.clipboard.write([new ClipboardItem({ 'text/plain': pending.then(value => new Blob([value], { type: 'text/plain' })) })]);
  } catch {
    await navigator.clipboard.writeText(await pending);
  }
}

// `read`, when given, supplies the text at click time (fetched from the host, writeClipboardLater)
export function useCopied(text: string, read?: () => Promise<string>): { state: CopyState; copy: () => Promise<void> } {
  const write = useCallback(() => read ? writeClipboardLater(read) : navigator.clipboard.writeText(text), [text, read]);
  return useCopyAction(text, write);
}
