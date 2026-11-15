import { memo, useLayoutEffect, useMemo, useRef } from 'react';
import type { GoalAction, SessionGoal, Turn } from '@acpira/shared/transcript';
import { PlanCard } from './Plan';
import { dockPlan } from './dockPlan';
import { QuietEntranceContext } from '../ui/Row';

// The floating card over the thread's lower edge: the live to-do list and, while one is set, the session goal as its
// head (a goal without a list still shows, as a one-line card)
export const PlanBar = memo(function PlanBar({ turns, running, goal, goalActions, onGoalAction, catchingUp }: {
  turns: Turn[]; running: boolean;
  goal?: SessionGoal; goalActions?: GoalAction[]; onGoalAction?: (action: GoalAction) => void;
  catchingUp?: boolean;
}) {
  // Like the thread, the first host reply after a cached switch restores history, not live progress.
  // Keep that commit quiet too: closing the old entry slower than the new one opens briefly grows the card.
  const opening = useRef(true);
  useLayoutEffect(() => { opening.current = !!catchingUp; });
  const entries = dockPlan(turns, running);
  const head = useMemo(() => goal && { goal, actions: goalActions, running, onAction: onGoalAction }, [goal, goalActions, running, onGoalAction]);
  if (!entries && !head) return null;
  return (
    <div className="pointer-events-none px-dock pb-(--dock-gap)">
      <div className="pointer-events-auto">
        <QuietEntranceContext.Provider value={opening}>
          <PlanCard entries={entries} live={running} goal={head} />
        </QuietEntranceContext.Provider>
      </div>
    </div>
  );
});
