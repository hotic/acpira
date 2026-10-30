import { useEffect, useState } from 'react';
import type { Draft } from '@acpira/shared/transcript';

// Unsent drafts by session id. Each session has its own composer state: switching away parks what was typed / pasted here, switching back
// restores it, and a draft never leaks into another session's field. Webview memory only — a window reload starts clean
const DRAFTS = new Map<string, { text: string; drafts: Draft[] }>();

type Update = (drafts: Draft[]) => Draft[];
interface Sink { update: (u: Update) => void; focus: () => void; write?: (text: string, replace: boolean) => void; key?: string }

// The main composer (never an inline history / queue editor) also takes drafts from outside it: the editor's "Add to chat" and quotes
// picked in the transcript. Updates made while no main composer is mounted wait for the next one
let sink: Sink | undefined;
const pending: Update[] = [];

export function updateMainComposer(update: Update, focus = false) {
  if (!sink) { pending.push(update); return; }
  sink.update(update);
  if (focus) sink.focus();
}

// Text for the main composer from a plugin's process (`ui/composer`): at the caret, or in place of the draft. With a
// session, only while the main composer shows it; without, only in the focused window (the push reaches every page).
// Dropped when no main composer takes it: it was meant for the field on screen
export function writeMainComposer(text: string, replace: boolean, session?: string) {
  if (session !== undefined ? sink?.key !== session : !document.hasFocus()) return;
  sink?.write?.(text, replace);
}

// A draft command's text that came back after its composer went (the user switched sessions): parked as that
// session's draft, so it is there on the way back
export function parkDraftText(draftKey: string, text: string) {
  DRAFTS.set(draftKey, { text, drafts: DRAFTS.get(draftKey)?.drafts ?? [] });
}

export function useComposerDraft(draftKey?: string, editText?: string, main?: { focus: () => void; write?: (text: string, replace: boolean) => void }) {
  const parked = draftKey ? DRAFTS.get(draftKey) : undefined;
  // An inline history editor parks under its own key too, so what was typed there outlives a click elsewhere and wins over the
  // original message text on the next open
  const [text, setText] = useState(parked?.text ?? editText ?? '');
  const [drafts, setDrafts] = useState<Draft[]>(parked?.drafts ?? []);
  useEffect(() => {
    if (!draftKey) return;
    if (text || drafts.length) DRAFTS.set(draftKey, { text, drafts }); else DRAFTS.delete(draftKey);
  }, [draftKey, text, drafts]);
  const isMain = !!main;
  const focus = main?.focus;
  const write = main?.write;
  useEffect(() => {
    if (!isMain || !focus) return;
    const own: Sink = { update: u => setDrafts(u), focus, write, key: draftKey };
    sink = own;
    for (const u of pending.splice(0)) setDrafts(u);
    return () => { if (sink === own) sink = undefined; };
  }, [isMain, focus, write, draftKey]);
  return { text, setText, drafts, setDrafts };
}

// A history edit that was sent or cancelled leaves nothing parked behind
export function dropDraft(draftKey: string) {
  DRAFTS.delete(draftKey);
}

// The desktop start page starts its empty session over when the agent or project changes: what was typed there moves to the
// new session, before that session's composer mounts and reads it
export function moveDraft(from: string, to: string) {
  const parked = DRAFTS.get(from);
  if (!parked || DRAFTS.has(to)) return;
  DRAFTS.set(to, parked);
  DRAFTS.delete(from);
}
