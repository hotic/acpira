import { useEffect, useState } from 'react';
import type { SubagentSummary } from '@shared/subagents';
import { t } from '../../i18n';
import { elapsedText } from './subagentState';

// Host timestamps survive remounts; only a running child needs a local tick, same cadence as useToolSeconds
export function useElapsed(node: SubagentSummary): string {
  const live = node.state === 'running' && node.endedAt === undefined;
  const [now, setNow] = useState(Date.now);
  useEffect(() => {
    if (!live) return;
    setNow(Date.now());
    const timer = setInterval(() => setNow(Date.now()), 1000);
    return () => clearInterval(timer);
  }, [live, node.id, node.announcedAt]);
  return elapsedText(node, now, t);
}
