import type { AgentBlock, AgentTurn, GoalBlock, NoticeBlock, TextBlock, ToolCallBlock, ToolKind } from '@shared/transcript';
import type { MsgKey } from '@shared/i18n';
import { getLocale, t } from '../i18n';
import { planApprovalTitle } from './permissionOptions';

// Thinking and to-do bookkeeping between closing paragraphs do not end the reply; in a foldable turn they join
// the process fold instead, so a summary followed by one more thought and a short coda stays visible as a whole.
const TAIL_PROCESS: ReadonlySet<AgentBlock['type']> = new Set<AgentBlock['type']>(['thought', 'plan']);

// A notice that only informs: nothing to act on and no known transient cause
function isAdvisory(b: NoticeBlock) {
  return b.severity === 'warning' && b.category === 'unknown' && b.actions.length === 0;
}

// ACP has no final/commentary distinction: the text after the last action stays outside the process fold.
// If another action arrives, that text becomes process history on the next render.
// Turns without tool calls render flat and chronological, so there only the trailing text run is the reply.
// The open question card is pinned above the composer; a resolved one stays in the process history
// at the point where it was asked, so the answers read in sequence with the actions around them.
// `live`: the turn is still running. Its blocks then keep arrival order: a thought after the trailing text is the
// agent working again (Claude writes a hand-off, then thinks over a background subagent's report), and joining the
// fold it rendered above the text it follows. The open fold shows that text in place, and once the turn settles a
// thought between closing paragraphs joins the fold again so summary and coda read as one reply
export function splitCodexBlocks(blocks: AgentBlock[], live = false) {
  const permissions = blocks.filter(b => b.type === 'permission');
  // AIR sessionFailure notices are status rows, not process detail: they never fold away under tools. An advisory
  // (warning, category unknown, no remedy) is the exception: codex-acp sends "Heads up: Long threads and multiple
  // compactions …" after every automatic compaction inside a turn, each under a new id, and lifted out they would
  // stack under the live fold for the rest of the turn, so they keep their place after the compaction row instead
  const lifted = (b: AgentBlock): b is NoticeBlock => b.type === 'notice' && !isAdvisory(b);
  const notices = blocks.filter(lifted);
  // Goal milestones are status rows too: the ones before any other content (a `/goal` prompt's own "Goal set") open
  // the turn above the fold, the ones after the last action (met, cleared …) follow the reply. A milestone that more
  // actions follow (a `/goal` steered into a running turn) keeps its place in the process history; lifted out, it would
  // sit under the whole live process until the turn ends
  const isAction = (b: AgentBlock) => b.type !== 'goal' && b.type !== 'text' && b.type !== 'permission' && b.type !== 'notice' && (b.type !== 'question' || !!b.outcome);
  const firstOther = blocks.findIndex(b => b.type !== 'goal');
  let lastAction = blocks.length - 1;
  while (lastAction >= 0 && !isAction(blocks[lastAction]!)) lastAction--;
  const goalsLead = blocks.filter((b, i): b is GoalBlock => b.type === 'goal' && (firstOther < 0 || i < firstOther));
  const goals = blocks.filter((b, i): b is GoalBlock => b.type === 'goal' && firstOther >= 0 && i > firstOther && i > lastAction);
  const content = blocks.filter((b, i) => b.type === 'goal' ? firstOther >= 0 && i > firstOther && i < lastAction
    : b.type !== 'permission' && !lifted(b) && (b.type !== 'question' || !!b.outcome));
  const foldable = content.some(b => b.type === 'tool_call');
  let end = content.length;
  while (end > 0 && (content[end - 1]!.type === 'text' || (foldable && !live && TAIL_PROCESS.has(content[end - 1]!.type)))) end--;
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
  // An MCP call names its server ("Call Blockbench"), its target is the tool's title
  const verb = block.mcpServer && !block.verbKey ? t('verb.mcp', { server: block.mcpServer })
    : t(block.verbKey ?? (isWorkflowRun(block) ? 'verb.workflow' : `verb.${block.kind}`));
  return t(FOLD_KEY[block.status], { verb });
}

// Claude's Workflow tool reaches the transcript as a generic `other` call titled "Workflow"; its async task says what
// it is and names the run. Read here rather than in the engine so transcripts stored before this render the same way
export function isWorkflowRun(block: ToolCallBlock): boolean {
  return block.asyncTask?.taskType === 'workflow';
}

// Only mode-switch headings are adapter UI; commands and file targets stay verbatim.
export function toolTarget(block: ToolCallBlock): string | undefined {
  if (isWorkflowRun(block)) return block.asyncTask!.name ?? block.target;
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

// Where the turn's clock starts: the first streamed output, not the send, so the wait for the agent to answer is not
// counted. A turn that already has blocks but no stamp (older transcripts, mirrors fed outside the ACP stream) falls
// back to the send time; a live turn with no output yet has no clock.
export function clockStart(turn: AgentTurn): number | undefined {
  if (turn.respondedAt !== undefined) return turn.respondedAt;
  return turn.blocks.length > 0 || turn.endedAt !== undefined ? turn.startedAt : undefined;
}

// Thought durations omit tool execution and waiting, so they cannot substitute for turn timing.
// Bare "5m 35s", the same on the turn's fold head, its live counter and the stats card.
export function elapsedDuration(turn: AgentTurn): string {
  const from = clockStart(turn);
  if (from === undefined || turn.endedAt === undefined) return '';
  return durationText(turn.endedAt - from);
}

// "5m 35s" for a span in ms; shared by the finished label and the live counter on a running turn's head
export function durationText(ms: number): string {
  const seconds = Math.max(0, Math.round(ms / 1000));
  const minutes = Math.floor(seconds / 60);
  const rest = seconds % 60;
  return minutes
    ? (rest ? t('turns.elapsed.ms', { m: minutes, s: rest }) : t('turns.elapsed.m', { m: minutes }))
    : t('turns.elapsed.s', { s: rest });
}
