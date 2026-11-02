import { useSyncExternalStore } from 'react';
import type { AgentId, AgentInfo } from '@acpira/shared/transcript';
import { MarkSvg } from './marks';

// The marks agents bring themselves (AgentInfo.mark: an agents.json entry or a plugin's agent), kept outside React so
// every AgentMark finds its agent's without the agent list being passed down; replaced, never mutated, so a change
// re-renders the marks through useSyncExternalStore whatever memo sits above them
let own: ReadonlyMap<AgentId, string[]> = new Map();
const listeners = new Set<() => void>();

// The host's agent list arrived: keep what it says about marks, notifying only when that changed
export function setAgentMarks(agents: AgentInfo[]) {
  const next = new Map(agents.filter(a => a.mark?.length).map(a => [a.id, a.mark!] as const));
  const same = next.size === own.size && [...next].every(([id, m]) => own.get(id)?.join('\n') === m.join('\n'));
  if (same) return;
  own = next;
  for (const l of listeners) l();
}

const subscribe = (l: () => void) => {
  listeners.add(l);
  return () => { listeners.delete(l); };
};
const snapshot = () => own;

// Vendor logo for an agent; used in the session list / agent Chip to tell agents apart
export function AgentMark({ id, name, className }: { id: AgentId; name?: string; className?: string }) {
  const marks = useSyncExternalStore(subscribe, snapshot, snapshot);
  return <MarkSvg id={id} name={name} className={className} own={marks.get(id)} />;
}
