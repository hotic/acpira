import { useEffect, useState } from 'react';
import type { ToolCallBlock } from '@shared/transcript';

// Host timestamps survive remounts; only unfinished commands need a local tick.
export function useToolSeconds(block: ToolCallBlock): number | undefined {
  const live = block.observation !== 'unknown' && block.kind === 'execute' && block.status === 'in_progress' && block.startedAt !== undefined && block.endedAt === undefined;
  const [now, setNow] = useState(Date.now);
  useEffect(() => {
    if (!live) return;
    setNow(Date.now());
    const timer = setInterval(() => setNow(Date.now()), 1000);
    return () => clearInterval(timer);
  }, [live, block.id, block.startedAt]);
  if (block.observation === 'unknown' || block.kind !== 'execute' || block.startedAt === undefined || (!live && block.endedAt === undefined)) return;
  return Math.max(0, Math.floor(((block.endedAt ?? now) - block.startedAt) / 1000));
}
