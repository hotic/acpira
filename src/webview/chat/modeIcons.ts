import { Bug, CircleDashed, Code, Eye, Infinity as InfinityIcon, ListTodo, MessageCircleQuestionMark, ShieldOff, Sparkles, type LucideIcon } from 'lucide-react';
import type { MsgKey } from '@shared/i18n';
import type { SessionOption } from '@shared/transcript';
import { t } from '../i18n';

// ACP modes carry only id / name / description, so the glyph is inferred from keywords in id + name (Devin: code / smart / ask / plan / bypass;
// Grok synthetic: default / plan / yolo; Kimi and custom agents fall through the same table). First match wins; unknown modes get a dashed circle
const RULES: [RegExp, LucideIcon][] = [
  [/bypass|yolo|auto[\s-]?accept|full[\s-]?access|unsafe/, ShieldOff],
  [/debug/, Bug],
  [/plan/, ListTodo],
  [/read[\s-]?only/, Eye],
  [/\bask\b|chat|question/, MessageCircleQuestionMark],
  [/smart|\bauto\b/, Sparkles],
  [/code|edit|build/, Code],
  [/agent|default|normal/, InfinityIcon],
];

export function modeIcon(mode: SessionOption): LucideIcon {
  const key = `${mode.id} ${mode.name}`.toLowerCase();
  return RULES.find(([re]) => re.test(key))?.[1] ?? CircleDashed;
}

// The toolbar's short name for a mode the built-in integrations are known to send (Claude, Codex, Devin, Grok, OpenCode, the
// built-in agent), matched on the exact id, never on the display name, so a user's own OpenCode agent keeps its name. As in
// Codex's composer, the labels are one short phrase each, written to the same length in CJK locales (four characters), so a
// switch barely moves the toolbar and every agent reads alike; the agent's own name and description stay in the tooltip.
// Anything unknown shows the agent's own name
const SHORT: Record<string, MsgKey> = {
  default: 'composer.modeShort.default',
  agent: 'composer.modeShort.default',
  build: 'composer.modeShort.build',
  plan: 'composer.modeShort.plan',
  acceptEdits: 'composer.modeShort.autoEdit',
  auto: 'composer.modeShort.auto',
  smart: 'composer.modeShort.smart',
  bypassPermissions: 'composer.modeShort.full',
  bypass: 'composer.modeShort.full',
  yolo: 'composer.modeShort.full',
  'agent-full-access': 'composer.modeShort.full',
  'read-only': 'composer.modeShort.readOnly',
  ask: 'composer.modeShort.ask',
  code: 'composer.modeShort.code',
};

export function modeLabel(mode: SessionOption): string {
  const key = Object.hasOwn(SHORT, mode.id) ? SHORT[mode.id] : mode.kind === 'full_access' ? 'composer.modeShort.full' : undefined;
  return key ? t(key) : mode.name;
}
