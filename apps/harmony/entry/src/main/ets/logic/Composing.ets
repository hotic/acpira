import { util } from '@kit.ArkTS';
import {
  AgentBlock, Attachment, ConfigControl, Draft, EditTurnRequest, PlanEntry, SessionControls, SessionGoal, SessionView,
  Turn, TurnError, TurnSettings, Usage
} from '../wire/Wire';
import { t } from '../store/L10n';

// The composer's and the dock's pure logic: pasted text (apps/ios Chat/PastedText.swift, shared/pastedText.ts), the
// context ring (Logic/UsageBreakdown.swift), the drafts a send hands the engine (AppModel.drafts), rewriting a sent
// prompt (Logic/TurnEdit.swift), what the dock shows (Chat/Dock.swift openQuestion / shortStop / dockPlan, PlanReview)
// and the goal head's words (Logic/GoalText.swift).

// Pasted text

/** shared/pastedText.ts: only a paste is classified; this long or longer it becomes a text attachment */
export class PastedText {
  static readonly characters: number = 2000;
  static readonly lines: number = 20;
  static readonly maxBytes: number = 1024 * 1024;

  /** `inline` (into the field), `attachment` (staged as a text file) or `tooBig` */
  static classify(text: string): string {
    if (text.length < PastedText.characters) {
      // CRLF is one line break, as are a lone CR and LF
      const count = 1 + (text.match(/\r\n|\r|\n/g)?.length ?? 0);
      if (count < PastedText.lines) {
        return 'inline';
      }
    }
    return utf8(text).length > PastedText.maxBytes ? 'tooBig' : 'attachment';
  }

  /** A local text attachment carries base64 until the engine gives it a blob name */
  static attachment(text: string, name: string): Attachment {
    const a: Attachment = { kind: 'text', blob: base64(utf8(text)), name: name };
    return a;
  }

  /** The text of a blob still held as base64 (a blob the engine stored is named `<hash>.<ext>`) */
  static inline(blob: string | undefined): string | undefined {
    if (blob === undefined || blob.includes('.')) {
      return undefined;
    }
    try {
      return new util.TextDecoder('utf-8').decodeToString(new util.Base64Helper().decodeSync(blob));
    } catch (e) {
      return undefined;
    }
  }
}

export function utf8(text: string): Uint8Array {
  return new util.TextEncoder('utf-8').encodeInto(text);
}

export function base64(bytes: Uint8Array): string {
  return new util.Base64Helper().encodeToStringSync(bytes);
}

/** AttachmentFile.messageBytes: one message on the paired link holds at most this much base64 */
export const MESSAGE_BYTES: number = 60 * 1024 * 1024;

/** The base64 a draft carries, to keep one message under MESSAGE_BYTES */
export function draftBytes(text: string, staged: Attachment[]): number {
  let bytes = text.length;
  for (const a of staged) {
    bytes += (a.kind === 'image' || a.kind === 'text') && a.blob !== undefined && !a.blob.includes('.') ? a.blob.length : 0;
    bytes += a.data?.length ?? 0;
  }
  return bytes;
}

/**
 * What the engine takes for the composer's attachments (AppModel.drafts): photos and pasted text as their bytes, a
 * file picked on the phone as its bytes too (the engine stages them and links its copy), any other file (an @ mention,
 * a path on the computer) as the link it is; quotes as they are
 */
export function draftsOf(attachments: Attachment[]): Draft[] {
  const out: Draft[] = [];
  for (const a of attachments) {
    if (a.kind === 'image' && a.blob !== undefined && !a.blob.includes('.')) {
      out.push({ kind: 'image', mimeType: a.mimeType ?? 'image/jpeg', data: a.blob, name: a.name });
    } else if (a.kind === 'text') {
      const text = PastedText.inline(a.blob);
      if (text !== undefined) {
        out.push({ kind: 'text', name: a.name ?? '', text: text, context: a.context });
      }
    } else if (a.kind === 'file' && a.uri !== undefined) {
      out.push({ kind: 'file', uri: a.uri, name: a.name ?? '', data: a.data });
    } else if (a.kind === 'quote') {
      out.push({ kind: 'quote', text: a.text ?? '', comment: a.comment });
    }
  }
  return out;
}

// Context usage

export interface UsagePanel {
  /** ok / pending / limited / unsupported */
  tone: string;
  note?: string;
}

/** usageBreakdown.ts: the ring's fill and the auto-compact threshold, read only from what the agent reported */
export class UsageBreakdown {
  /** The fill of a snapshot that predates a finished compaction: a tiny sliver, dimmed */
  static readonly staleFill: number = 0.02;

  static fill(usage: Usage): number {
    if (usage.compactedFrom !== undefined) {
      return UsageBreakdown.staleFill;
    }
    return usage.size > 0 ? Math.min(1, usage.used / usage.size) : 0;
  }

  static percent(usage: Usage): number {
    return Math.floor((usage.size > 0 ? Math.min(1, usage.used / usage.size) : 0) * 100);
  }

  /** UsagePanel's status: unsupported without `/compact`, window-limited, pending once past the threshold */
  static panel(usage: Usage, compactAt: number | undefined, canCompact: boolean, running: boolean): UsagePanel {
    const stale = usage.compactedFrom !== undefined;
    const over = !stale && compactAt !== undefined && compactAt > 0 && usage.used >= compactAt;
    if (!canCompact) {
      return { tone: 'unsupported', note: t('usage.unsupported') };
    }
    if (compactAt !== undefined && compactAt >= usage.size) {
      return { tone: 'limited', note: t('usage.budgetExceedsWindow') };
    }
    if (over) {
      return { tone: 'pending', note: t(running ? 'usage.pendingAfterTurn' : 'usage.pendingBeforeSend') };
    }
    return { tone: 'ok' };
  }
}

/** A number in the app's language, as the webview's Intl.NumberFormat */
export function groupedNumber(n: number): string {
  return Math.round(n).toString().replace(/\B(?=(\d{3})+(?!\d))/g, ',');
}

// Prompt edits

/** A sent prompt being rewritten in the composer; what the composer held before comes back afterwards */
export interface TurnEdit {
  turnIndex: number;
  turnCount: number;
  original: Turn;
  controls: SessionControls;
  keptText: string;
  keptStaged: Attachment[];
}

export function isModelControl(c: ConfigControl): boolean {
  return c.category === 'model' || (c.category === undefined && c.id === 'model');
}

/** captureTurnSettings: wire ids, never display names */
export function captureTurnSettings(controls: SessionControls): TurnSettings {
  const config: Record<string, string> = {};
  for (const c of controls.options) {
    if (c.value !== undefined && config[c.id] === undefined) {
      config[c.id] = c.value;
    }
  }
  const out: TurnSettings = { modeId: controls.modeId, config: config };
  return out;
}

function copyControl(c: ConfigControl, value: string | undefined): ConfigControl {
  const out: ConfigControl = { id: c.id, name: c.name, category: c.category, type: c.type, options: c.options, value: value };
  return out;
}

/** controlsForTurn: a selection the controls no longer offer yields to the current one, never to a guess */
export function controlsForTurn(controls: SessionControls, settings: TurnSettings | undefined): SessionControls {
  const mode = settings?.modeId;
  const out: SessionControls = {
    modes: controls.modes, modeConfigId: controls.modeConfigId,
    modeId: mode !== undefined && controls.modes.some((m) => m.id === mode) ? mode : controls.modeId,
    options: controls.options.map((c: ConfigControl) => {
      const value: string | undefined = settings?.config[c.id];
      return copyControl(c, value !== undefined && c.options.some((o) => o.id === value) ? value : c.value);
    })
  };
  return out;
}

/**
 * openTurnControls without the remembered model shapes: the prompt's selections over the live controls; a model
 * other than the live one drops the live model's parameters, which may not belong to it (TurnEdit.swift reshape)
 */
export function openTurnControls(live: SessionControls, settings: TurnSettings | undefined): SessionControls {
  const next = controlsForTurn(live, settings);
  const model = live.options.find(isModelControl);
  const value: string | undefined = model !== undefined ? settings?.config[model.id] : undefined;
  if (model === undefined || value === undefined || value === model.value) {
    return next;
  }
  next.options = next.options.filter((c: ConfigControl) => c.category !== 'thought_level' && c.category !== 'model_config');
  return next;
}

/** editTurnConfig: one pick in the editor's local snapshot of the selections */
export function editTurnConfig(editor: SessionControls, id: string, value: string): SessionControls {
  const out: SessionControls = {
    modes: editor.modes, modeId: editor.modeId, modeConfigId: editor.modeConfigId,
    options: editor.options.map((c: ConfigControl) => c.id === id ? copyControl(c, value) : c)
  };
  return out;
}

/** Prompts can be rewritten while the session is idle and usable, with nothing queued */
export function canEditPrompts(view: SessionView): boolean {
  return !view.running && (view.status === 'ready' || view.status === 'starting') && (view.queued ?? []).length === 0;
}

function sameAttachment(a: Attachment, b: Attachment): boolean {
  return a.kind === b.kind && a.blob === b.blob && a.uri === b.uri && a.name === b.name && a.text === b.text &&
    a.comment === b.comment && a.startLine === b.startLine;
}

/** The prompt's attachments still staged, by their index in the prompt: each staged copy keeps one */
export function retainedAttachments(original: Attachment[], staged: Attachment[]): number[] {
  const left = staged.slice();
  const out: number[] = [];
  original.forEach((a: Attachment, i: number) => {
    const at = left.findIndex((b: Attachment) => sameAttachment(a, b));
    if (at >= 0) {
      left.splice(at, 1);
      out.push(i);
    }
  });
  return out;
}

/** The editTurn request: kept attachments by index, the ones staged during the edit as drafts */
export function editTurnRequest(sessionId: string, edit: TurnEdit, text: string, staged: Attachment[]): EditTurnRequest {
  const original = edit.original.attachments ?? [];
  const retained = retainedAttachments(original, staged);
  const added = staged.slice();
  for (const i of retained) {
    const at = added.findIndex((b: Attachment) => sameAttachment(original[i], b));
    if (at >= 0) {
      added.splice(at, 1);
    }
  }
  const out: EditTurnRequest = {
    sessionId: sessionId, turnIndex: edit.turnIndex, turnCount: edit.turnCount, originalText: edit.original.text ?? '',
    turnId: edit.original.id, text: text.trim(), retainedAttachments: retained, attachments: draftsOf(added),
    settings: captureTurnSettings(edit.controls)
  };
  return out;
}

// What the dock shows

/** The question waiting for an answer in the latest agent turn */
export function openQuestion(view: SessionView): AgentBlock | undefined {
  const last = view.turns[view.turns.length - 1];
  if (last === undefined || last.role !== 'agent') {
    return undefined;
  }
  return (last.blocks ?? []).find((b: AgentBlock) => b.type === 'question' && b.outcome === undefined);
}

/** isShortStop: the last turn ended short (an error, a refusal, a limit) and the session is otherwise usable */
export function shortStop(view: SessionView): Turn | undefined {
  const last = view.turns[view.turns.length - 1];
  if (view.running || view.status !== 'ready' || last === undefined || last.role !== 'agent') {
    return undefined;
  }
  const stop = last.stop;
  return stop === undefined || stop === 'end_turn' || stop === 'cancelled' ? undefined : last;
}

/** The plan approval the newest agent turn is blocked on: a send meanwhile revises the plan */
export function pendingPlanReview(turns: Turn[]): AgentBlock | undefined {
  const last = turns[turns.length - 1];
  if (last === undefined || last.role !== 'agent') {
    return undefined;
  }
  return (last.blocks ?? []).find((b: AgentBlock) => b.type === 'permission' && b.planId !== undefined);
}

const CONTEXT_KINDS: string[] = ['context_length_exceeded', 'context_window_exceeded', 'prompt_too_long'];

export function isContextLengthError(error: TurnError | undefined): boolean {
  if (error === undefined) {
    return false;
  }
  return CONTEXT_KINDS.includes(error.kind ?? '') ||
    /\bprompt (?:to the model )?(?:was|is) too long\b|\bcontext (?:length|window) (?:exceeded|limit exceeded)\b|\bmaximum context length\b/i
      .test(error.message);
}

/**
 * dockPlan: open entries stay docked across turns; a finished list stays only while its own turn still runs. A turn's
 * list is its `plan` block, else its last completed todo tool row
 */
export function dockPlan(turns: Turn[], running: boolean): PlanEntry[] | undefined {
  for (let i = turns.length - 1; i >= 0; i--) {
    const turn = turns[i];
    if (turn.role !== 'agent') {
      continue;
    }
    const blocks = (turn.blocks ?? []).slice().reverse();
    const plan = blocks.find((b: AgentBlock) => b.type === 'plan')?.entries;
    const todos = blocks.find((b: AgentBlock) => b.type === 'tool_call' && b.verbKey === 'verb.todo' &&
      b.status === 'completed' && b.todoEntries !== undefined)?.todoEntries;
    const entries = plan ?? todos;
    if (entries === undefined) {
      continue;
    }
    // A list written while planning is part of the plan: the dock waits until that turn's plan is built
    if (blocks.some((b: AgentBlock) => b.type === 'plan_document' && b.status !== 'approved' && b.status !== 'executing')) {
      return undefined;
    }
    if (entries.some((e: PlanEntry) => e.status !== 'completed')) {
      return entries;
    }
    return running && i === turns.length - 1 ? entries : undefined;
  }
  return undefined;
}

// Goal head

export function goalStatusLabel(status: string): string {
  switch (status) {
    case 'paused':
      return t('goal.status.paused');
    case 'blocked':
      return t('goal.status.blocked');
    case 'limited':
      return t('goal.status.limited');
    case 'complete':
      return t('goal.status.complete');
    default:
      return t('goal.status.active');
  }
}

/** goalDuration: "45s", "6m 12s", "2h 05m" */
export function goalDuration(seconds: number): string {
  const whole = Math.max(0, Math.round(seconds));
  const h = Math.floor(whole / 3600);
  const m = Math.floor(whole / 60) % 60;
  const s = whole % 60;
  if (h > 0) {
    return t('goal.elapsed.hm', new Map<string, string>([['h', `${h}`], ['m', `${m}`.padStart(2, '0')]]));
  }
  if (m > 0) {
    return s > 0 ? t('turns.elapsed.ms', new Map<string, string>([['m', `${m}`], ['s', `${s}`]]))
      : t('turns.elapsed.m', new Map<string, string>([['m', `${m}`]]));
  }
  return t('turns.elapsed.s', new Map<string, string>([['s', `${s}`]]));
}

/** headMeta: the head's one fact, elapsed time (Codex) or stop-hook rounds (Claude) */
export function goalHeadMeta(goal: SessionGoal): string | undefined {
  if (goal.timeUsedSeconds !== undefined) {
    return goalDuration(goal.timeUsedSeconds);
  }
  if (goal.iterations !== undefined) {
    return t('goal.rounds', new Map<string, string>([['n', `${goal.iterations}`]]));
  }
  return undefined;
}

/** The head's controls: those the agent advertised that apply to the goal's status */
export function goalHeadActions(goal: SessionGoal, advertised: string[] | undefined): string[] {
  const can = advertised ?? [];
  const out: string[] = [];
  if (goal.status === 'active' && can.includes('pause')) {
    out.push('pause');
  }
  if (goal.status === 'paused' && can.includes('resume')) {
    out.push('resume');
  }
  if (goal.status !== 'complete' && can.includes('clear')) {
    out.push('clear');
  }
  return out;
}

/** The text's first line, undefined when it has none */
export function firstLine(text: string | undefined): string | undefined {
  const line = (text ?? '').split(/\r?\n/)[0]?.trim() ?? '';
  return line.length > 0 ? line : undefined;
}
