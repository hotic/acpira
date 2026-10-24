import { useSyncExternalStore } from 'react';
import type { ContextItem, ContextSource, EditorSelection } from '@shared/protocol';
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

// Keys cover the whole text (two selections that only differ past a prefix are different excerpts); a selection object is
// never mutated, so its key is built once rather than on every render that asks for the offered selection
const keys = new WeakMap<EditorSelection, string>();

export function selectionKey(s: EditorSelection): string {
  let key = keys.get(s);
  if (key === undefined) {
    key = `${s.uri}#${s.startLine}:${s.endLine}:${s.text}`;
    keys.set(s, key);
  }
  return key;
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

// Other tools' live selections (`contextSelection`), one slot per source id beside the editor's. Each slot is offered until it
// goes out with a prompt, then stays hidden until that source reports something else; the shell decides which instance owns a slot
export interface LiveContext {
  source: ContextSource;
  items: ContextItem[];
  // Identity of this content, for the sent mark and the composer's exclusion toggle
  key: string;
}

let contexts: LiveContext[] = [];
const contextConsumed = new Map<string, string>();
// Snapshot handed to React: rebuilt only when a slot or a sent mark changes, so useSyncExternalStore sees a stable value
let offeredContexts: LiveContext[] = [];

const rebuildOffered = () => {
  offeredContexts = contexts.filter(c => contextConsumed.get(c.source.id) !== c.key);
  notify();
};

export function setContextSelection(source: ContextSource, items: ContextItem[] | undefined) {
  const rest = contexts.filter(c => c.source.id !== source.id);
  if (!items?.length) {
    if (rest.length === contexts.length) return;
    contexts = rest;
    contextConsumed.delete(source.id);
    rebuildOffered();
    return;
  }
  const key = contextKey(items);
  const cur = contexts.find(c => c.source.id === source.id);
  if (cur?.key === key && cur.source.name === source.name) return;
  // A new slot keeps its source's place in the toolbar, so chips never swap places while one source updates
  const next = { source, items, key };
  contexts = cur ? contexts.map(c => (c === cur ? next : c)) : [...contexts, next];
  rebuildOffered();
}

export function markContextSent(c: LiveContext) {
  contextConsumed.set(c.source.id, c.key);
  rebuildOffered();
}

// Every source's live content, minus what already went out
export function offeredLiveContexts(): LiveContext[] {
  return offeredContexts;
}

export function useLiveContexts(): LiveContext[] {
  return useSyncExternalStore(subscribe, offeredLiveContexts);
}

function contextKey(items: ContextItem[]): string {
  return JSON.stringify(items.map(i => [i.label, i.text, i.uri ?? '', i.comment ?? '']));
}

// The text drafts a source's items go out as: each keeps where it came from, so the engine can head it with its source
export function contextDrafts(source: ContextSource, items: ContextItem[]): Extract<Draft, { kind: 'text' }>[] {
  return items.map(i => ({
    kind: 'text',
    name: i.label,
    text: i.text,
    context: { sourceId: source.id, sourceName: source.name, ...(i.uri ? { uri: i.uri } : {}), ...(i.comment ? { comment: i.comment } : {}) },
  }));
}

// Whether two drafts are the same shared context (pinning it twice only duplicates it)
export function sameContext(a: Draft, b: Draft): boolean {
  return a.kind === 'text' && b.kind === 'text' && !!a.context && !!b.context && a.context.sourceId === b.context.sourceId
    && a.name === b.name && a.text === b.text && a.context.uri === b.context.uri;
}
