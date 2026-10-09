import type { AgentBlock, AgentTurn, GoalBlock, NoticeBlock, TextBlock, ToolCallBlock, ToolKind } from '@shared/transcript';
import type { MsgKey } from '@shared/i18n';
import { getLocale, t } from '../i18n';
import { planApprovalTitle } from './permissionOptions';

// Thinking and to-do bookkeeping between closing paragraphs do not end the reply; in a foldable turn they join
// the process fold instead, so a summary followed by one more thought and a short coda stays visible as a whole.
const TAIL_PROCESS: ReadonlySet<AgentBlock['type']> = new Set<AgentBlock['type']>(['thought', 'plan']);

// ACP has no final/commentary distinction: the text after the last action stays outside the process fold.
// If another action arrives, that text becomes process history on the next render.
// Turns without tool calls render flat and chronological, so there only the trailing text run is the reply.
// The open question card is pinned above the composer; a resolved one stays in the process history
// at the point where it was asked, so the answers read in sequence with the actions around them.
export function splitCodexBlocks(blocks: AgentBlock[]) {
  const permissions = blocks.filter(b => b.type === 'permission');
  // AIR sessionFailure notices are status rows, not process detail: they never fold away under tools
  const notices = blocks.filter((b): b is NoticeBlock => b.type === 'notice');
  // Goal milestones are status rows too: the ones before any other content (a `/goal` prompt's own "Goal set") open
  // the turn above the fold, the rest follow it with the notices
  const firstOther = blocks.findIndex(b => b.type !== 'goal');
  const goalsLead = blocks.filter((b, i): b is GoalBlock => b.type === 'goal' && (firstOther < 0 || i < firstOther));
  const goals = blocks.filter((b, i): b is GoalBlock => b.type === 'goal' && firstOther >= 0 && i > firstOther);
  const content = blocks.filter(b => b.type !== 'permission' && b.type !== 'notice' && b.type !== 'goal' && (b.type !== 'question' || !!b.outcome));
  const foldable = content.some(b => b.type === 'tool_call');
  let end = content.length;
  while (end > 0 && (content[end - 1]!.type === 'text' || (foldable && TAIL_PROCESS.has(content[end - 1]!.type)))) end--;
  const tail = content.slice(end);
  return {
    // Explicit source phases win; unannotated ACP text keeps the trailing-text heuristic.
    process: [...content.slice(0, end).filter(b => b.type !== 'text' || b.phase !== 'final'),
      ...tail.filter(b => b.type !== 'text' || b.phase === 'commentary')],
    reply: content.filter((b, i): b is TextBlock => b.type === 'text' &&
      (b.phase === 'final' || (i >= end && b.phase !== 'commentary'))),
    permissions,
    notices,
    goalsLead,
    goals,
  };
}

const FOLD_KEY: Record<ToolCallBlock['status'], MsgKey> = {
  pending: 'fold.queued',
  in_progress: 'fold.pending',
  completed: 'fold.done',
  failed: 'fold.failed',
  cancelled: 'fold.cancelled',
};

export function toolVerb(block: ToolCallBlock): string {
  if (block.observation === 'unknown') return t('chatgpt.unknownTool');
  // Stored verbs use the host locale at creation time; render from semantic kind, or from verbKey when the verb came from the tool's identity
  return t(FOLD_KEY[block.status], { verb: t(block.verbKey ?? `verb.${block.kind}`) });
}

// Only mode-switch headings are adapter UI; commands and file targets stay verbatim.
export function toolTarget(block: ToolCallBlock): string | undefined {
  return block.kind === 'switch_mode' && block.target && !block.targetMono
    ? planApprovalTitle(block.target, getLocale()) : block.target;
}

export interface FoldActivity {
  kind: ToolKind | 'compaction';
  label: string;
  target?: string;
  mono?: boolean;
  active?: boolean;
}

export function foldActivity(turn: AgentTurn): FoldActivity {
  if (turn.observation === 'unknown') return { kind: 'other', label: t('chatgpt.unknownTurn') };
  if (turn.blocks.some(b => b.type === 'permission')) return { kind: 'other', label: t('host.awaitingApproval') };
  if (turn.blocks.some(b => b.type === 'question' && !b.outcome)) return { kind: 'other', label: t('host.awaitingAnswers') };
  // Concurrent calls can finish out of order; a newer completed call must not hide an active one.
  // A command parked in the background keeps running on its own and is never the agent's current action.
  for (let i = turn.blocks.length - 1; i >= 0; i--) {
    const b = turn.blocks[i]!;
    if (b.type === 'tool_call' && !b.background && (b.status === 'pending' || b.status === 'in_progress')) {
      return { kind: b.kind, label: toolVerb(b), target: toolTarget(b), mono: b.targetMono, active: true };
    }
    if (b.type === 'compaction' && b.status === 'in_progress') return { kind: 'compaction', label: t('turns.compacting') };
  }
  // Read current transcript state before a cached, already-localized activity label.
  // Completed tools stay visible between notifications without claiming they still run.
  for (let i = turn.blocks.length - 1; i >= 0; i--) {
    const b = turn.blocks[i]!;
    if (b.type === 'text' && b.streaming) return { kind: 'other', label: t('host.replying'), active: true };
    // An open thought may already be followed by unreported tool-argument generation.
    if (b.type === 'thought' && b.streaming) return { kind: 'think', label: t('host.working'), active: true };
    if (b.type === 'tool_call' && !b.background) return { kind: b.kind, label: toolVerb(b), target: toolTarget(b), mono: b.targetMono };
  }
  return { kind: 'other', label: t('host.working'), active: true };
}

// Thought durations omit tool execution and waiting, so they cannot substitute for turn timing.
// Bare "5m 35s" for label/value rows; elapsedLabel wraps it in the 用时/Took prefix used by the transcript note line.
export function elapsedDuration(turn: AgentTurn): string {
  if (turn.startedAt === undefined || turn.endedAt === undefined) return '';
  const seconds = Math.max(0, Math.round((turn.endedAt - turn.startedAt) / 1000));
  const minutes = Math.floor(seconds / 60);
  const rest = seconds % 60;
  return minutes
    ? (rest ? t('turns.elapsed.ms', { m: minutes, s: rest }) : t('turns.elapsed.m', { m: minutes }))
    : t('turns.elapsed.s', { s: rest });
}

export function elapsedLabel(turn: AgentTurn): string {
  if (turn.startedAt === undefined || turn.endedAt === undefined) return t('turns.done');
  return t('turns.elapsed', { t: elapsedDuration(turn) });
}
