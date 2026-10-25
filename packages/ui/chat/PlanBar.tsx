import { memo, useMemo } from 'react';
import type { GoalAction, SessionGoal, Turn } from '@acpira/shared/transcript';
import { PlanCard } from './Plan';
import { dockPlan } from './dockPlan';

// The floating card over the thread's lower edge: the live to-do list and, while one is set, the session goal as its
// head (a goal without a list still shows, as a one-line card)
export const PlanBar = memo(function PlanBar({ turns, running, goal, goalActions, onGoalAction }: {
  turns: Turn[]; running: boolean;
  goal?: SessionGoal; goalActions?: GoalAction[]; onGoalAction?: (action: GoalAction) => void;
}) {
  const entries = dockPlan(turns, running);
  const head = useMemo(() => goal && { goal, actions: goalActions, running, onAction: onGoalAction }, [goal, goalActions, running, onGoalAction]);
  if (!entries && !head) return null;
  return (
    <div className="pointer-events-none px-dock pb-(--dock-gap)">
      <div className="pointer-events-auto">
        <PlanCard entries={entries} live={running} goal={head} />
      </div>
    </div>
  );
});
