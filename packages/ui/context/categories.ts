import type { ContextCategory, ContextNode } from '@acpira/shared/context';
import type { MsgKey } from '@acpira/shared/i18n/core';
import { t } from '../i18n';

// What the ring card and the report call each category, and its tone (tokens.css --context-*)
const LABEL: Record<ContextCategory, MsgKey> = {
  agent: 'context.cat.agent',
  system: 'context.cat.system',
  tools: 'context.cat.tools',
  docs: 'context.cat.docs',
  skills: 'context.cat.skills',
  mcp: 'context.cat.mcp',
  chat: 'context.cat.chat',
};

export const categoryLabel = (id: ContextCategory) => t(LABEL[id]);

// Fill classes, spelled out so the stylesheet build sees them
export const CATEGORY_BG: Record<ContextCategory, string> = {
  agent: 'bg-context-agent',
  system: 'bg-context-system',
  tools: 'bg-context-tools',
  docs: 'bg-context-docs',
  skills: 'bg-context-skills',
  mcp: 'bg-context-mcp',
  chat: 'bg-context-chat',
};

// The conversation's leaves shaded by what they hold (contextReport.css --shade-*): the user's words, the model's
// thinking and replies, and its calls by tool kind
export type Shade = 'message' | 'thinking' | 'reply' | 'run' | 'read' | 'edit' | 'other';
export const SHADES: Shade[] = ['message', 'thinking', 'reply', 'run', 'read', 'edit', 'other'];

const CALL_SHADE: Record<string, Shade> = { execute: 'run', read: 'read', search: 'read', fetch: 'read', edit: 'edit', delete: 'edit', move: 'edit' };

export function shadeOf(n: ContextNode | undefined): Shade | undefined {
  switch (n?.kind) {
    case 'message': case 'note': case 'summary': return 'message';
    case 'thinking': return 'thinking';
    case 'reply': return 'reply';
    case 'call': return CALL_SHADE[n.toolKind ?? ''] ?? 'other';
    default: return undefined;
  }
}

// The legend names a shade the way the tree names its rows
const SHADE_LABEL: Record<Shade, MsgKey> = {
  message: 'context.node.message', thinking: 'context.node.thinking', reply: 'context.node.reply',
  run: 'verb.execute', read: 'verb.read', edit: 'verb.edit', other: 'verb.other',
};
export const shadeLabel = (s: Shade) => t(SHADE_LABEL[s]);
