import type { MsgKey } from '@acpira/shared/i18n/core';
import type { SessionOption } from '@acpira/shared/transcript';
import { t } from '../i18n';
import {
  AgentGlyph,
  AskShieldGlyph,
  AutoEditShieldGlyph,
  AutoShieldGlyph,
  ChatGlyph,
  CodeGlyph,
  CustomModeGlyph,
  DebugGlyph,
  FullAccessShieldGlyph,
  PlanGlyph,
  RawGlyph,
  ReadOnlyGlyph,
  SmartGlyph,
  TeamGlyph,
  WorkspaceGlyph,
  type ModeIcon,
} from './modeGlyphs';

export type { ModeIcon } from './modeGlyphs';

// ACP modes carry only id / name / description, so the glyph is inferred from keywords in id + name (Devin: code / smart / ask / plan / bypass;
// Grok synthetic: default / plan / yolo; Droid spec; Kimi and custom agents fall through the same table). First match wins; unknown modes get a dashed circle
const RULES: [RegExp, ModeIcon][] = [
  // The built-in agent's Raw mode: the agent's tools unplugged, the model reached directly
  [/\braw\b/, RawGlyph],
  [/bypass|yolo|auto[\s-]?accept|full[\s-]?access|unsafe/, FullAccessShieldGlyph],
  [/debug/, DebugGlyph],
  [/plan|\bspec\b/, PlanGlyph],
  [/read[\s-]?only/, ReadOnlyGlyph],
  // Codex `workspace-write`: edits allowed inside the workspace only
  [/workspace/, WorkspaceGlyph],
  [/\bask\b|chat|question/, ChatGlyph],
  [/smart|\bauto\b/, SmartGlyph],
  [/code|edit|build/, CodeGlyph],
  // The built-in agent's Team mode (and other agents' multi-agent modes): it leads the summoned agents
  [/\bteam\b|multitask|orchestrat/, TeamGlyph],
  [/agent|default|normal/, AgentGlyph],
];

export function modeIcon(mode: SessionOption): ModeIcon {
  // The adapter's explicit classification outranks keyword guesses.
  if (mode.kind === 'full_access') return FullAccessShieldGlyph;
  if (mode.kind === 'auto_review') return SmartGlyph;
  if (mode.id === 'auto_edit' || mode.id === 'acceptEdits') return CodeGlyph;
  return Object.hasOwn(BY_ID, mode.id) ? BY_ID[mode.id]! : byKeyword(mode);
}

// Ids the keywords misread: Droid's `normal` is named "Auto (Off)" (asks before edits), its `auto-low` allows edits and its
// `auto-high` approves everything; Hermes's `accept_edits` allows workspace edits only and `dont_ask` edits anywhere but
// sensitive paths (`acp_adapter/modes.py`), so neither is full access
const BY_ID: Record<string, ModeIcon> = {
  normal: AgentGlyph, 'auto-low': CodeGlyph, 'auto-high': FullAccessShieldGlyph, accept_edits: WorkspaceGlyph, dont_ask: CodeGlyph,
};

function byKeyword(mode: SessionOption): ModeIcon {
  const key = `${mode.id} ${mode.name}`.toLowerCase();
  return RULES.find(([re]) => re.test(key))?.[1] ?? CustomModeGlyph;
}

// The toolbar's short name for a mode the built-in integrations are known to send (Claude, Codex, Devin, Grok, OpenCode, the
// built-in agent), matched on the exact id, never on the display name, so a user's own OpenCode agent keeps its name. As in
// Codex's composer, the labels are one short phrase each, written to the same length in CJK locales (four characters), so a
// switch barely moves the toolbar and every agent reads alike; the agent's own name and description stay in the tooltip.
// Anything unknown shows the agent's own name
const SHORT: Record<string, MsgKey> = {
  // Every known agent's `default` asks before acting (Grok, Hermes); "Default" told nothing of that
  default: 'composer.access.ask',
  // The built-in agent's own mode: named for what it does next to Team mode, apart from other agents' "default"
  agent: 'composer.modeShort.agent',
  build: 'composer.modeShort.build',
  plan: 'composer.modeShort.plan',
  raw: 'composer.modeShort.raw',
  team: 'composer.modeShort.team',
  acceptEdits: 'composer.modeShort.autoEdit',
  // Hermes: workspace edits only; `dont_ask` allows edits anywhere
  accept_edits: 'composer.modeShort.workspace',
  dont_ask: 'composer.modeShort.autoEdit',
  auto_edit: 'composer.modeShort.autoEdit',
  auto: 'composer.modeShort.auto',
  smart: 'composer.modeShort.smart',
  bypassPermissions: 'composer.modeShort.full',
  bypass: 'composer.modeShort.full',
  yolo: 'composer.modeShort.full',
  'agent-full-access': 'composer.modeShort.full',
  // Droid's autonomy ladder: Auto (Off) asks, Spec plans read-only, Low allows edits, Medium also reversible commands, High all
  normal: 'composer.access.ask',
  spec: 'composer.modeShort.plan',
  'auto-low': 'composer.modeShort.autoEdit',
  'auto-medium': 'composer.modeShort.auto',
  'auto-high': 'composer.modeShort.full',
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

// An approval level on the permission chip and in its menu: the built-in agent's levels (`APPROVAL_LEVELS`) by id; any
// other agent's level keeps its short mode label
const ACCESS: Record<string, MsgKey> = {
  ask: 'composer.access.ask',
  'auto-edit': 'composer.access.autoEdit',
  full: 'composer.access.full',
};

export function accessLabel(level: SessionOption): string {
  return Object.hasOwn(ACCESS, level.id) ? t(ACCESS[level.id]!) : modeLabel(level);
}

// The approval levels (the built-in agent's, and `auto` from the engine's split of Claude / Antigravity modes) as one
// shield family, so the chip reads as a scale; other levels keep their mode glyph
const ACCESS_ICON: Record<string, ModeIcon> = {
  ask: AskShieldGlyph,
  'auto-edit': AutoEditShieldGlyph,
  auto: AutoShieldGlyph,
  full: FullAccessShieldGlyph,
};

export function accessIcon(level: SessionOption): ModeIcon {
  return Object.hasOwn(ACCESS_ICON, level.id) ? ACCESS_ICON[level.id]! : modeIcon(level);
}

// A working mode where it is picked by name instead of sitting on a chip (the composer's pill and its row in the / list):
// the bare name, without the chip labels' padding to one length, and one line on what it does. Unknown modes keep the
// agent's own wording
const NAME: Record<string, MsgKey> = { plan: 'composer.modeName.plan', team: 'composer.modeName.team', raw: 'composer.modeName.raw' };
const HINT: Record<string, MsgKey> = { plan: 'composer.modeHint.plan', team: 'composer.modeHint.team', raw: 'composer.modeHint.raw' };

export function modeName(mode: SessionOption): string {
  return Object.hasOwn(NAME, mode.id) ? t(NAME[mode.id]!) : modeLabel(mode);
}

export function modeHint(mode: SessionOption): string | undefined {
  return Object.hasOwn(HINT, mode.id) ? t(HINT[mode.id]!) : mode.description;
}
