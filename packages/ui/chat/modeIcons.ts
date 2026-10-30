import { Bug, CircleDashed, Code, Eye, FolderPen, Infinity as InfinityIcon, ListTodo, MessageCircleQuestionMark, ShieldOff, Sparkles, Unplug, Users, type LucideIcon } from 'lucide-react';
import type { MsgKey } from '@acpira/shared/i18n';
import type { SessionOption } from '@acpira/shared/transcript';
import { t } from '../i18n';

// ACP modes carry only id / name / description, so the glyph is inferred from keywords in id + name (Devin: code / smart / ask / plan / bypass;
// Grok synthetic: default / plan / yolo; Kimi and custom agents fall through the same table). First match wins; unknown modes get a dashed circle
const RULES: [RegExp, LucideIcon][] = [
  // The built-in agent's Raw mode: the agent's tools unplugged, the model reached directly
  [/\braw\b/, Unplug],
  [/bypass|yolo|auto[\s-]?accept|full[\s-]?access|unsafe/, ShieldOff],
  [/debug/, Bug],
  [/plan/, ListTodo],
  [/read[\s-]?only/, Eye],
  // Codex `workspace-write`: edits allowed inside the workspace only
  [/workspace/, FolderPen],
  [/\bask\b|chat|question/, MessageCircleQuestionMark],
  [/smart|\bauto\b/, Sparkles],
  [/code|edit|build/, Code],
  // The built-in agent's Team mode (and other agents' multi-agent modes): it leads the summoned agents
  [/\bteam\b|multitask|orchestrat/, Users],
  [/agent|default|normal/, InfinityIcon],
];

export function modeIcon(mode: SessionOption): LucideIcon {
  // The adapter's explicit classification outranks keyword guesses.
  if (mode.kind === 'full_access') return ShieldOff;
  if (mode.kind === 'auto_review') return Sparkles;
  if (mode.id === 'auto_edit' || mode.id === 'acceptEdits') return Code;
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
  // The built-in agent's own mode: named for what it does next to Team mode, apart from other agents' "default"
  agent: 'composer.modeShort.agent',
  build: 'composer.modeShort.build',
  plan: 'composer.modeShort.plan',
  raw: 'composer.modeShort.raw',
  team: 'composer.modeShort.team',
  acceptEdits: 'composer.modeShort.autoEdit',
  auto_edit: 'composer.modeShort.autoEdit',
  auto: 'composer.modeShort.auto',
  smart: 'composer.modeShort.smart',
  bypassPermissions: 'composer.modeShort.full',
  bypass: 'composer.modeShort.full',
  yolo: 'composer.modeShort.full',
  'agent-full-access': 'composer.modeShort.full',
  'read-only': 'composer.modeShort.readOnly',
  'workspace-write': 'composer.modeShort.workspace',
  ask: 'composer.modeShort.ask',
  code: 'composer.modeShort.code',
  // Devin calls this Code; it is not Claude's acceptEdits permission mode.
  'accept-edits': 'composer.modeShort.code',
};

export function modeLabel(mode: SessionOption): string {
  const key = mode.kind === 'full_access' ? 'composer.modeShort.full'
    : mode.kind === 'auto_review' ? 'composer.modeShort.auto'
    : Object.hasOwn(SHORT, mode.id) ? SHORT[mode.id] : undefined;
  return key ? t(key) : mode.name;
}

// An approval level in the mode menu's scale and on the mode chip: the built-in agent's levels (`APPROVAL_LEVELS`) by id,
// short enough for an even third of the menu; any other agent's level keeps its short mode label
const ACCESS: Record<string, MsgKey> = {
  ask: 'composer.access.ask',
  'auto-edit': 'composer.access.autoEdit',
  full: 'composer.access.full',
};

export function accessLabel(level: SessionOption): string {
  return Object.hasOwn(ACCESS, level.id) ? t(ACCESS[level.id]!) : modeLabel(level);
}
