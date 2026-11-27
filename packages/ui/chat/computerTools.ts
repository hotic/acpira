import { Camera, LayoutGrid, MousePointerClick, ScanEye, type LucideIcon } from 'lucide-react';
import type { MsgKey } from '@acpira/shared/i18n/core';
import type { ToolCallBlock } from '@acpira/shared/transcript';
import { t } from '../i18n';

// Acpira's computer-use rows (engine `computer_tool_target`): the target is the app the call went to, and an act's
// target has a second line saying what its steps did, in the driver's `> ` line format: `click "Select", type "hello"`.
// Each step is a verb, then what it acted on as a JSON string when there is something to name
const ICONS: Record<string, LucideIcon> = {
  'verb.computerApps': LayoutGrid,
  'verb.computerObserve': ScanEye,
  'verb.computerAct': MousePointerClick,
  'verb.computerSee': Camera,
};

export function isComputerTool(block: ToolCallBlock): boolean {
  return block.verbKey !== undefined && block.verbKey in ICONS;
}

export function computerIcon(block: ToolCallBlock): LucideIcon {
  return ICONS[block.verbKey ?? ''] ?? MousePointerClick;
}

// The app a call went to; computer_apps names none
export function computerApp(block: ToolCallBlock): string | undefined {
  return block.target?.split('\n')[0] || undefined;
}

export interface ComputerStep { verb: string; what?: string }

const STEP = /\s*([a-z_]+)(?: ("(?:[^"\\]|\\.)*"))?\s*(?:,|$)/y;

export function computerSteps(block: ToolCallBlock): ComputerStep[] {
  const line = block.target?.split('\n')[1];
  if (!line) return [];
  const steps: ComputerStep[] = [];
  STEP.lastIndex = 0;
  while (STEP.lastIndex < line.length) {
    const at = STEP.lastIndex;
    const m = STEP.exec(line);
    if (!m || STEP.lastIndex === at) break;
    let what: string | undefined;
    try { what = m[2] === undefined ? undefined : JSON.parse(m[2]) as string; } catch { what = m[2]; }
    steps.push({ verb: m[1]!, ...(what !== undefined ? { what } : {}) });
  }
  return steps;
}

const STEP_KEY: Record<string, MsgKey> = {
  click: 'computer.click',
  double_click: 'computer.doubleClick',
  right_click: 'computer.rightClick',
  type: 'computer.type',
  set: 'computer.set',
  key: 'computer.key',
  scroll: 'computer.scroll',
  menu: 'computer.menu',
  wait: 'computer.wait',
};
// Keys as a keyboard prints them: ctrl+shift+a reads Ctrl+Shift+A
function keyName(keys: string): string {
  return keys.split('+').map(k => k.length === 1 ? k.toUpperCase() : k.charAt(0).toUpperCase() + k.slice(1)).join('+');
}

// A step as words: its verb, and what it acted on with no quotes around it (the row sets that part apart by ink)
export function computerStepParts(step: ComputerStep): { verb: string; what?: string } {
  const key = STEP_KEY[step.verb];
  const verb = key ? t(key) : step.verb;
  if (step.what === undefined) return { verb };
  return { verb, what: step.verb === 'key' ? keyName(step.what) : step.what };
}

// An act's steps as plain text, for places without ink: 点击 选择、输入 hello、按 Ctrl+A
export function computerStepsLabel(block: ToolCallBlock): string | undefined {
  const steps = computerSteps(block);
  return steps.length ? steps.map(computerStepParts).map(p => p.what === undefined ? p.verb : `${p.verb} ${p.what}`).join(t('turns.nameSep')) : undefined;
}

// One call inside a run on the same app: what that call did
export function computerEntryLabel(block: ToolCallBlock): string | undefined {
  if (block.verbKey === 'verb.computerObserve') return t('computer.observeEntry');
  if (block.verbKey === 'verb.computerSee') return t('computer.seeEntry');
  return computerSteps(block).length ? undefined : t('verb.computerAct');
}
