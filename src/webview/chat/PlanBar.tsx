import type { Turn } from '@shared/transcript';
import { PlanCard } from './Plan';
import { dockPlan } from './dockPlan';

export function PlanBar({ turns, running }: { turns: Turn[]; running: boolean }) {
  const entries = dockPlan(turns, running);
  if (!entries) return null;
  return (
    <div className="pointer-events-none px-dock pb-(--dock-gap)">
      <div className="pointer-events-auto">
        <PlanCard entries={entries} live={running} />
      </div>
    </div>
  );
}
