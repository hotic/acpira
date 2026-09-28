import { useEffect, useState } from 'react';
import type { Draft } from '@shared/transcript';

// Unsent drafts by session id. Each session has its own composer state: switching away parks what was typed / pasted here, switching back
// restores it, and a draft never leaks into another session's field. Webview memory only — a window reload starts clean
const DRAFTS = new Map<string, { text: string; drafts: Draft[] }>();

type Update = (drafts: Draft[]) => Draft[];
interface Sink { update: (u: Update) => void; focus: () => void }

// The main composer (never an inline history / queue editor) also takes drafts from outside it: the editor's "Add to chat" and quotes
// picked in the transcript. Updates made while no main composer is mounted wait for the next one
let sink: Sink | undefined;
const pending: Update[] = [];

export function updateMainComposer(update: Update, focus = false) {
  if (!sink) { pending.push(update); return; }
  sink.update(update);
  if (focus) sink.focus();
}

export function useComposerDraft(draftKey?: string, editText?: string, main?: { focus: () => void }) {
  const parked = draftKey ? DRAFTS.get(draftKey) : undefined;
  const [text, setText] = useState(editText ?? parked?.text ?? '');
  const [drafts, setDrafts] = useState<Draft[]>(parked?.drafts ?? []);
  useEffect(() => {
    if (!draftKey) return;
    if (text || drafts.length) DRAFTS.set(draftKey, { text, drafts }); else DRAFTS.delete(draftKey);
  }, [draftKey, text, drafts]);
  const isMain = !!main;
  const focus = main?.focus;
  useEffect(() => {
    if (!isMain || !focus) return;
    const own: Sink = { update: u => setDrafts(u), focus };
    sink = own;
    for (const u of pending.splice(0)) setDrafts(u);
    return () => { if (sink === own) sink = undefined; };
  }, [isMain, focus]);
  return { text, setText, drafts, setDrafts };
}
