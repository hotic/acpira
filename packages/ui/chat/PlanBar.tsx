import { memo, useLayoutEffect, useMemo, useRef } from 'react';
import type { GoalAction, SessionGoal, Turn } from '@acpira/shared/transcript';
import { PlanCard } from './Plan';
import { dockPlan } from './dockPlan';
import { Collapsible, LazyPanelContext } from '../ui/Collapsible';
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
  const shown = !!entries || !!head;
  // The card as last drawn stays in the panel while it closes: the dock folds away under the thread instead of
  // vanishing in one frame, which dropped the whole conversation by the card's height
  const last = useRef<{ entries: typeof entries; head: typeof head }>(undefined);
  if (shown) last.current = { entries, head };
  if (!last.current) return null;
  return (
    <QuietEntranceContext.Provider value={opening}>
      {/* Lazy: the closed panel lets go of the card once it has folded. The card's own folds keep their bodies */}
      <LazyPanelContext.Provider value>
        <Collapsible.Root open={shown}><Collapsible.Panel className="plan-bar-fold">
          <LazyPanelContext.Provider value={false}>
            <div className="pointer-events-none px-dock pb-(--dock-gap)">
              <div className="pointer-events-auto">
                <PlanCard entries={last.current.entries} live={shown && running} goal={last.current.head} />
              </div>
            </div>
          </LazyPanelContext.Provider>
        </Collapsible.Panel></Collapsible.Root>
      </LazyPanelContext.Provider>
    </QuietEntranceContext.Provider>
  );
});
