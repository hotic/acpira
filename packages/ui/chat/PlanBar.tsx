import { memo } from 'react';
import type { Turn } from '@acpira/shared/transcript';
import { PlanCard } from './Plan';
import { dockPlan } from './dockPlan';

export const PlanBar = memo(function PlanBar({ turns, running }: { turns: Turn[]; running: boolean }) {
  const entries = dockPlan(turns, running);
  if (!entries) return null;
  return (
    <div className="pointer-events-none px-dock pb-(--dock-gap)">
      <div className="pointer-events-auto">
        <PlanCard entries={entries} live={running} />
      </div>
    </div>
  );
});
