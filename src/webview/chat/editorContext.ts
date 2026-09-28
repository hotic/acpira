import { useSyncExternalStore } from 'react';
import type { EditorSelection } from '@shared/protocol';
import type { Draft } from '@shared/transcript';
import { nameOf } from './drafts';

// IDE editor state the shell pushes straight into the webview (not through the sidecar): the live selection and the last copy.
// Module state, like the composer drafts: one webview shows one composer at a time, and a session switch must not lose either
let selection: EditorSelection | undefined;
let lastCopy: EditorSelection | undefined;
// The live selection that already went out with a prompt; its chip stays hidden until the selection changes or is cleared
let consumed: string | undefined;
const listeners = new Set<() => void>();
const notify = () => { for (const l of listeners) l(); };
const subscribe = (l: () => void) => { listeners.add(l); return () => { listeners.delete(l); }; };

export function setEditorSelection(next: EditorSelection | undefined) {
  if (next && selection && selectionKey(next) === selectionKey(selection)) return;
  selection = next;
  // Collapsing the selection and picking the same range again offers it again
  if (!next) consumed = undefined;
  notify();
}

export function setEditorCopy(next: EditorSelection) {
  lastCopy = next;
}

export function markSelectionSent(s: EditorSelection) {
  consumed = selectionKey(s);
  notify();
}

// The live selection, minus one that was already sent
export function offeredSelection(): EditorSelection | undefined {
  return selection && selectionKey(selection) !== consumed ? selection : undefined;
}

export function useEditorSelection(): EditorSelection | undefined {
  return useSyncExternalStore(subscribe, offeredSelection);
}

export function selectionKey(s: EditorSelection): string {
  return `${s.uri}#${s.startLine}:${s.endLine}:${s.text.length}:${s.text.slice(0, 64)}`;
}

const normalize = (text: string) => text.replace(/\r\n?/g, '\n').replace(/\n+$/, '');

// The range a paste came from, when the pasted text is exactly what was last copied in an editor and spans more than one line
// (a copied identifier stays plain text, the way Cursor treats it)
export function copiedSelection(pasted: string): EditorSelection | undefined {
  if (!lastCopy || lastCopy.endLine <= lastCopy.startLine) return undefined;
  const text = normalize(pasted);
  return text && text === normalize(lastCopy.text) ? lastCopy : undefined;
}

export function selectionDraft(s: EditorSelection, cwd: string): Extract<Draft, { kind: 'selection' }> {
  return { kind: 'selection', uri: s.uri, name: nameOf(s.uri, cwd), startLine: s.startLine, endLine: s.endLine, text: s.text };
}

// Whether two drafts are the same editor excerpt (adding it twice only duplicates context). The text counts too: two different
// expressions on one line are two excerpts
export function sameRange(a: Draft, b: Draft): boolean {
  return a.kind === 'selection' && b.kind === 'selection' && a.uri === b.uri && a.startLine === b.startLine && a.endLine === b.endLine
    && a.text === b.text;
}
