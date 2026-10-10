import { memo } from 'react';
import { CircleCheck, Goal as GoalIcon, Pause, Play, Trash2, TriangleAlert } from 'lucide-react';
import type { GoalAction, GoalBlock, GoalEvent, GoalStatus, SessionGoal } from '@shared/transcript';
import type { MsgKey } from '@shared/i18n';
import { t } from '../i18n';
import { IconButton } from '../ui/Button';
import { Row, RowLabel, RowTarget } from '../ui/Row';
import { Shimmer } from '../ui/Shimmer';
import { cn } from '../ui/cn';
import { goalDuration, goalSpend } from './goalText';

// The session goal (codex-acp / claude-agent-acp goal extension, `SessionView.goal`): a one-line strip hanging off the
// composer's top edge for the present, a quiet row in the transcript for each milestone, and "Set as goal" under the
// last prompt. See docs/dev/transcript-pipeline.md (goal) and docs/dev/ui-conventions.md (goal strip).

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

// Status glyph: the goal mark (accent while it runs), warn for blocked / limited, a check once met. `live` (the strip
// while the goal is active and a turn runs) waves the mark's flag around its pole (`.goal-live` in motion.css)
function GoalGlyph({ status, quiet, live }: { status: GoalStatus; quiet?: boolean; live?: boolean }) {
  if (status === 'complete') return <CircleCheck className="size-icon text-ok" strokeWidth={1.5} />;
  if (status === 'blocked' || status === 'limited') return <TriangleAlert className="size-icon text-warn" strokeWidth={1.5} />;
  return <GoalIcon className={cn('size-icon', status === 'active' && !quiet && 'text-accent', live && 'goal-live')} strokeWidth={1.5} />;
}

// One short fact for the strip: elapsed time (Codex) or stop-hook rounds (Claude); the spend goes to the hover title
function stripMeta(goal: SessionGoal): string | undefined {
  if (goal.timeUsedSeconds !== undefined) return goalDuration(goal.timeUsedSeconds);
  if (goal.iterations !== undefined) return t('goal.rounds', { n: goal.iterations });
  return undefined;
}

// The goal strip: a tab inset from the composer's sides, top corners rounded, its bottom edge flush with the composer
// (the dock gap is cancelled), hairline on three sides. One fixed line, never expands: the objective truncates and the
// hover title carries the full text and spend. Buttons follow the advertised actions, so Claude (set / clear) shows only the bin.
export const GoalStrip = memo(function GoalStrip({ goal, actions, running, onAction }: {
  goal: SessionGoal; actions?: GoalAction[]; running: boolean; onAction?: (action: GoalAction) => void;
}) {
  const can = (a: GoalAction) => !!onAction && !!actions?.includes(a);
  const meta = stripMeta(goal);
  const muted = goal.status === 'paused' || goal.status === 'complete';
  const live = goal.status === 'active' && running;
  const title = [goal.objective, goalSpend(goal)].filter(Boolean).join('\n');
  const button = (action: GoalAction, key: MsgKey, icon: React.ReactNode) => (
    <IconButton size="sm" title={t(key)} aria-label={t(key)} onClick={() => onAction?.(action)}>{icon}</IconButton>
  );
  return (
    <div data-goal-strip className="-mb-(--dock-gap) px-page">
      <div className="mx-pad flex flex-col rounded-t-lg bg-(--cmp-bg) px-pad pt-0.5 shadow-[inset_0_1px_0_0_var(--conversation-line),inset_1px_0_0_0_var(--conversation-line),inset_-1px_0_0_0_var(--conversation-line)]">
        <Row
          dense
          title={title}
          lead={<GoalGlyph status={goal.status} live={live} />}
          trailing={<>
            {meta && <span className="whitespace-nowrap">{meta}</span>}
            <span className="flex items-center gap-0.5">
              {goal.status === 'active' && can('pause') && button('pause', 'goal.pause', <Pause strokeWidth={1.5} />)}
              {goal.status === 'paused' && can('resume') && button('resume', 'goal.resume', <Play strokeWidth={1.5} />)}
              {goal.status !== 'complete' && can('clear') && button('clear', 'goal.clear', <Trash2 strokeWidth={1.5} />)}
            </span>
          </>}
        >
          {/* Label and objective share one line and one shimmer, so the sweep crosses the whole goal while a turn runs
              (like the to-do title) instead of two glints at different speeds; the line truncates as a whole */}
          <Shimmer active={live} className="min-w-0 truncate">
            <span className="mr-2 text-fg-1">{t(STATUS_KEY[goal.status])}</span>
            <span className={muted ? 'text-fg-3' : 'text-fg-2'}>{goal.objective.split('\n')[0]}</span>
          </Shimmer>
        </Row>
      </div>
    </div>
  );
});

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

// Under the last sent prompt (Codex's "Set as goal"): turns that prompt into the session goal in one click. The thread
// only offers it on agents advertising `set` while no goal is set. The strip takes the place of the message gap: it cancels
// the gaps on both sides and is exactly the button's height, so the button touches neither the prompt nor the reply and
// the reply sits one button height under the prompt instead of one message gap (tuned in lab/set-goal-tune, 2026-10-10)
export function SetGoalAction({ onSet }: { onSet: () => void }) {
  return (
    <div data-set-goal className="-my-msg flex h-ctl-sm items-center justify-end">
      <button type="button" title={t('goal.setFromPromptHint')} onClick={onSet}
        className="flex h-ctl-sm items-center gap-1 rounded-md px-1.5 text-3 text-fg-3 transition-colors hover:bg-hover hover:text-fg-1">
        <GoalIcon className="size-icon" strokeWidth={1.5} />{t('goal.setFromPrompt')}
      </button>
    </div>
  );
}
