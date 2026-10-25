import type { ReactNode } from 'react';
import { CircleCheck, Goal as GoalIcon, Pause, Play, Trash2, TriangleAlert } from 'lucide-react';
import type { GoalAction, GoalBlock, GoalEvent, GoalStatus, SessionGoal } from '@acpira/shared/transcript';
import type { MsgKey } from '@acpira/shared/i18n';
import { t, useLocale } from '../i18n';
import { IconButton } from '../ui/Button';
import { Row, RowLabel, RowTarget } from '../ui/Row';
import { Shimmer } from '../ui/Shimmer';
import { cn } from '../ui/cn';
import { goalDuration, goalSpend } from './goalText';
import { SentTime } from './TurnClock';

// The session goal (codex-acp / claude-agent-acp goal extension, `SessionView.goal`): the head of the floating to-do
// card for the present, a quiet row in the transcript for each milestone, and "Set as goal" under the last prompt.
// See docs/dev/transcript-pipeline.md (goal) and docs/dev/ui-conventions.md (goal head).

const STATUS_KEY: Record<GoalStatus, MsgKey> = {
  active: 'goal.status.active',
  paused: 'goal.status.paused',
  blocked: 'goal.status.blocked',
  limited: 'goal.status.limited',
  complete: 'goal.status.complete',
};

const EVENT_KEY: Record<GoalEvent, MsgKey> = {
  set: 'goal.event.set',
  paused: 'goal.event.paused',
  resumed: 'goal.event.resumed',
  blocked: 'goal.event.blocked',
  limited: 'goal.event.limited',
  complete: 'goal.event.complete',
  cleared: 'goal.event.cleared',
};

// The status a milestone row borrows its glyph from
const EVENT_STATUS: Record<GoalEvent, GoalStatus> = {
  set: 'active',
  resumed: 'active',
  paused: 'paused',
  cleared: 'paused',
  blocked: 'blocked',
  limited: 'limited',
  complete: 'complete',
};

// Status glyph: the goal mark (accent while it runs), warn for blocked / limited, a check once met. `live` (the head
// while the goal is active and a turn runs) waves the mark's flag around its pole (`.goal-live` in motion.css)
function GoalGlyph({ status, quiet, live }: { status: GoalStatus; quiet?: boolean; live?: boolean }) {
  if (status === 'complete') return <CircleCheck className="size-icon text-ok" strokeWidth={1.5} />;
  if (status === 'blocked' || status === 'limited') return <TriangleAlert className="size-icon text-warn" strokeWidth={1.5} />;
  return <GoalIcon className={cn('size-icon', status === 'active' && !quiet && 'text-accent', live && 'goal-live')} strokeWidth={1.5} />;
}

// One short fact for the head: elapsed time (Codex) or stop-hook rounds (Claude); the spend goes to the hover title
function headMeta(goal: SessionGoal): string | undefined {
  if (goal.timeUsedSeconds !== undefined) return goalDuration(goal.timeUsedSeconds);
  if (goal.iterations !== undefined) return t('goal.rounds', { n: goal.iterations });
  return undefined;
}

export interface GoalHeadProps {
  goal: SessionGoal;
  actions?: GoalAction[];
  running: boolean;
  onAction?: (action: GoalAction) => void;
}

// The goal as the head of the floating to-do card (`PlanCard`; picked as P4-L1-C2-T1 in lab/goal-dock-pick.preview.html,
// 2026-10-26), so the dock keeps one silhouette: status label and objective on one truncating line, the elapsed time,
// and the advertised controls, which unfold beside the time while the line is hovered or holds focus (`.goal-actions`).
// Under a to-do list `toggle` trails the count and caret and stretches a button over the row that folds the list; without
// one the card is this line alone. The hover title carries the full objective and spend. Buttons follow the advertised
// actions, so Claude (set / clear) shows only the bin.
export function GoalHead({ goal, actions, running, onAction, toggle }: GoalHeadProps & {
  toggle?: { open: boolean; onToggle: () => void; trailing: ReactNode };
}) {
  useLocale();
  const can = (a: GoalAction) => !!onAction && !!actions?.includes(a);
  const meta = headMeta(goal);
  const muted = goal.status === 'paused' || goal.status === 'complete';
  const live = goal.status === 'active' && running;
  const title = [goal.objective, goalSpend(goal)].filter(Boolean).join('\n');
  const button = (action: GoalAction, key: MsgKey, icon: ReactNode) => (
    <IconButton key={action} size="sm" title={t(key)} aria-label={t(key)} onClick={() => onAction?.(action)}>{icon}</IconButton>
  );
  const buttons = [
    goal.status === 'active' && can('pause') && button('pause', 'goal.pause', <Pause strokeWidth={1.5} />),
    goal.status === 'paused' && can('resume') && button('resume', 'goal.resume', <Play strokeWidth={1.5} />),
    goal.status !== 'complete' && can('clear') && button('clear', 'goal.clear', <Trash2 strokeWidth={1.5} />),
  ].filter(Boolean);
  return (
    <Row
      dense
      data-goal-head
      title={title}
      className={cn('goal-head relative', toggle && 'goal-head-toggle -mx-hit px-hit rounded-md transition-colors')}
      lead={<GoalGlyph status={goal.status} live={live} />}
      trailing={<span className="flex items-center">
        {meta && <span className="whitespace-nowrap">{meta}</span>}
        {buttons.length > 0 && <span className="goal-actions"><span className="flex items-center gap-0.5">{buttons}</span></span>}
        {toggle && <span className="ml-2 flex items-center gap-2">{toggle.trailing}</span>}
      </span>}
    >
      {toggle && <button type="button" className="goal-toggle" aria-expanded={toggle.open} aria-label={t('plan.todoTitle')} onClick={toggle.onToggle} />}
      {/* Label and objective share one line and one shimmer, so the sweep crosses the whole goal while a turn runs
          instead of two glints at different speeds; the line truncates as a whole */}
      <Shimmer active={live} className="min-w-0 truncate">
        <span className="mr-2 text-fg-1">{t(STATUS_KEY[goal.status])}</span>
        <span className={muted ? 'text-fg-3' : 'text-fg-2'}>{goal.objective.split('\n')[0]}</span>
      </Shimmer>
    </Row>
  );
}

// Transcript trace of one goal milestone: no bubble, a quiet row. Setting and resuming name the objective; the rows
// that end or hold a stretch of work carry its spend (time / rounds / tokens) on the trailing side
export function GoalRow({ block }: { block: GoalBlock }) {
  const status = EVENT_STATUS[block.event];
  const detail = block.event === 'set' || block.event === 'resumed' ? undefined : [
    block.timeUsedSeconds !== undefined && block.timeUsedSeconds > 0 ? goalDuration(block.timeUsedSeconds) : undefined,
    block.iterations !== undefined ? t('goal.rounds', { n: block.iterations }) : undefined,
    goalSpend(block),
  ].filter(Boolean).join(t('common.metaSep'));
  return (
    <Row lead={<GoalGlyph status={status} quiet />} trailing={detail || undefined} title={block.objective}>
      <RowLabel className="text-fg-2">{t(EVENT_KEY[block.event])}</RowLabel>
      {block.objective && <RowTarget>{block.objective.split('\n')[0]}</RowTarget>}
    </Row>
  );
}

// The strip under a sent prompt: when it went out, and on the last typed prompt Codex's "Set as goal" (turns that prompt
// into the session goal in one click; the thread only offers it on agents advertising `set` while no goal is set).
// It takes the place of the message gap by cancelling the gaps on both sides: with the button it is exactly the button's
// height, so the button touches neither the prompt nor the reply (tuned in lab/set-goal-tune, 2026-10-10); with the time
// alone it is one message gap, so the prompt and its reply keep their usual spacing.
// Hidden like a reply's action row: it fades in while the prompt above (`peer/prompt` on HistoryMessage) or the strip
// itself is hovered / focused; touch screens without hover always show it
export function PromptFooter({ sentAt, onSetGoal }: { sentAt?: number; onSetGoal?: () => void }) {
  return (
    <div data-set-goal={onSetGoal ? '' : undefined} className={cn('-my-msg flex items-center justify-end gap-0.5 opacity-0 transition-opacity duration-(--code-copy-duration) ease-out peer-hover/prompt:opacity-100 peer-focus-within/prompt:opacity-100 hover:opacity-100 focus-within:opacity-100 [@media(hover:none)]:opacity-100 motion-reduce:transition-none',
      onSetGoal ? 'h-ctl-sm' : 'h-msg')}>
      {sentAt !== undefined && <SentTime at={sentAt} />}
      {onSetGoal && (
        <IconButton size="sm" title={t('goal.setFromPromptHint')} aria-label={t('goal.setFromPrompt')} onClick={onSetGoal}>
          <GoalIcon strokeWidth={1.5} />
        </IconButton>
      )}
    </div>
  );
}
