import type { ContextItem, ContextSource, HostMsg } from '@shared/protocol';
import { MAX_TEXT_BYTES } from '@shared/attachments';

// The payload other extensions hand `acpira.context.set` / `acpira.context.add` (docs/context-api.md). Validated here, outside
// `vscode`, so the rules are unit-tested: anything another extension sends is untrusted input bound for a prompt
export interface ContextPayload {
  source: ContextSource;
  // Which instance of the source pushed it (a canvas document's URI): only the same owner can clear a live slot
  owner?: string;
  items: ContextItem[];
}

export const CONTEXT_API_VERSION = 1;
export const MAX_CONTEXT_ITEMS = 20;
const MAX_LABEL = 120;
const MAX_NAME = 60;
const MAX_URI = 2048;
const MAX_COMMENT = 4000;
const SOURCE_ID = /^[A-Za-z0-9][\w.-]{0,63}$/;

export class ContextPayloadError extends Error {}

const str = (v: unknown): v is string => typeof v === 'string';
// Labels and names show on one line: control characters and runs of whitespace collapse to one space
const oneLine = (s: string, max: number) => [...s.replace(/[\s\p{Cc}]+/gu, ' ').trim()].slice(0, max).join('');

// Checks and normalizes a payload; throws ContextPayloadError naming the first problem. `items` may be empty (a clear)
export function parseContextPayload(raw: unknown): ContextPayload {
  const fail = (why: string): never => { throw new ContextPayloadError(`acpira.context: ${why}`); };
  if (!raw || typeof raw !== 'object') fail('the payload must be an object');
  const p = raw as Record<string, unknown>;
  if (p.version !== undefined && p.version !== CONTEXT_API_VERSION) fail(`unsupported version ${String(p.version)}, expected ${CONTEXT_API_VERSION}`);
  const src = p.source as Record<string, unknown> | undefined;
  if (!src || typeof src !== 'object' || !str(src.id) || !SOURCE_ID.test(src.id)) fail('source.id must be 1-64 letters, digits, ".", "_" or "-"');
  const name = str(src!.name) ? oneLine(src!.name, MAX_NAME) : '';
  if (!name) fail('source.name must be a non-empty string');
  if (p.owner !== undefined && (!str(p.owner) || p.owner.length > MAX_URI)) fail('owner must be a string');
  const rawItems = p.items ?? [];
  if (!Array.isArray(rawItems)) fail('items must be an array');
  const list = rawItems as unknown[];
  if (list.length > MAX_CONTEXT_ITEMS) fail(`at most ${MAX_CONTEXT_ITEMS} items`);
  let bytes = 0;
  const items = list.map((it, i): ContextItem => {
    const o = it as Record<string, unknown> | undefined;
    if (!o || typeof o !== 'object') return fail(`items[${i}] must be an object`);
    if (!str(o.text) || !o.text.trim()) fail(`items[${i}].text must be a non-empty string`);
    const label = str(o.label) ? oneLine(o.label, MAX_LABEL) : '';
    if (!label) fail(`items[${i}].label must be a non-empty string`);
    if (o.uri !== undefined && (!str(o.uri) || o.uri.length > MAX_URI)) fail(`items[${i}].uri must be a string`);
    if (o.comment !== undefined && (!str(o.comment) || o.comment.length > MAX_COMMENT)) fail(`items[${i}].comment must be a string under ${MAX_COMMENT} characters`);
    const text = o.text as string;
    bytes += Buffer.byteLength(text);
    if (bytes > MAX_TEXT_BYTES) fail(`item texts must stay under ${MAX_TEXT_BYTES >> 10} KB in total`);
    const comment = str(o.comment) ? o.comment.trim() : '';
    return { label, text, ...(str(o.uri) && o.uri ? { uri: o.uri } : {}), ...(comment ? { comment } : {}) };
  });
  return { source: { id: src!.id as string, name }, ...(str(p.owner) ? { owner: p.owner } : {}), items };
}

// A page in Acpira's browser panel (window.acpira.context, docs/context-api.md "Web pages") has its owner filed under the
// panel: a page clears only what a page in the same panel set, never another tool's slot, and the panel can drop them all
export const pageOwner = (panelId: string, owner = '') => `browser:${panelId}:${owner}`;

interface Slot {
  source: ContextSource;
  owner?: string;
  items: ContextItem[];
}

// The live slots, one per source id. Any push with items takes the slot over (the canvas the user just clicked into wins);
// an empty push clears it only when it comes from the owner that filled it, so closing canvas A leaves canvas B's selection
export class ContextSlots {
  private slots = new Map<string, Slot>();

  // The page message for this push, or undefined when it changes nothing (a stale clear)
  set(p: ContextPayload): HostMsg | undefined {
    const cur = this.slots.get(p.source.id);
    if (!p.items.length) {
      if (!cur || cur.owner !== p.owner) return undefined;
      this.slots.delete(p.source.id);
      return { type: 'contextSelection', source: p.source };
    }
    this.slots.set(p.source.id, { source: p.source, owner: p.owner, items: p.items });
    return { type: 'contextSelection', source: p.source, items: p.items };
  }

  // Clears every slot owned by one of these (the owner's tab closed without it clearing first)
  release(owners: ReadonlySet<string>): HostMsg[] {
    const out: HostMsg[] = [];
    for (const [id, slot] of this.slots) {
      if (slot.owner === undefined || !owners.has(slot.owner)) continue;
      this.slots.delete(id);
      out.push({ type: 'contextSelection', source: slot.source });
    }
    return out;
  }

  // Clears every slot a page in this browser panel filled (pageOwner): the page it showed is gone
  releasePage(panelId: string): HostMsg[] {
    const prefix = pageOwner(panelId);
    return this.release(new Set([...this.owners()].filter(o => o.startsWith(prefix))));
  }

  owners(): Set<string> {
    return new Set([...this.slots.values()].flatMap(s => (s.owner === undefined ? [] : [s.owner])));
  }

  // What a page that (re)initializes needs to show the current slots
  replay(): HostMsg[] {
    return [...this.slots.values()].map(s => ({ type: 'contextSelection', source: s.source, items: s.items }));
  }

  // The messages that take every slot off the pages (sharing turned off); the slots themselves are kept for when it comes back
  hidden(): HostMsg[] {
    return [...this.slots.values()].map(s => ({ type: 'contextSelection', source: s.source }));
  }
}
