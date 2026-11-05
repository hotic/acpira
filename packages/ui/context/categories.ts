import type { ContextCategory } from '@acpira/shared/context';
import type { MsgKey } from '@acpira/shared/i18n/core';
import { t } from '../i18n';

// What the ring card and the report call each category, and its tone (tokens.css --context-*)
const LABEL: Record<ContextCategory, MsgKey> = {
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
  system: 'bg-context-system',
  tools: 'bg-context-tools',
  docs: 'bg-context-docs',
  skills: 'bg-context-skills',
  mcp: 'bg-context-mcp',
  chat: 'bg-context-chat',
};
