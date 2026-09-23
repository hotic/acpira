import type { AgentId, AgentInfo } from './transcript';

// acpira.agentOrder / acpira.disabledAgents: the order every agent list follows and the agents kept out of the new-session entry points
export interface AgentPrefs {
  order: AgentId[];
  disabled: AgentId[];
}

// Listed ids first in their saved order, the rest (new built-ins, fresh custom agents) keep registry order after them.
// External entries stay last. Disabled agents are flagged, never dropped: history, settings and running sessions still need them
export function arrangeAgents<A extends AgentInfo>(list: A[], prefs: AgentPrefs): A[] {
  const rank = new Map(prefs.order.map((id, i) => [id, i]));
  const off = new Set(prefs.disabled);
  const key = (a: A, i: number) => [a.external ? 1 : 0, rank.get(a.id) ?? prefs.order.length, i] as const;
  return list
    .map((a, i) => ({ a: off.has(a.id) && !a.external ? { ...a, disabled: true } : a, k: key(a, i) }))
    .sort((x, y) => x.k[0] - y.k[0] || x.k[1] - y.k[1] || x.k[2] - y.k[2])
    .map(x => x.a);
}

// Agents a new-session entry point offers (the plus menu, the default agent picker)
export const launchable = (agents: AgentInfo[]) => agents.filter(a => !a.external && !a.disabled);

// The configured default unless it was switched off; then the first enabled agent, preferring an installed one.
// An id the list does not know stays as configured (a custom agent not probed yet)
export function pickDefaultAgent(agents: AgentInfo[], preferred: AgentId): AgentId {
  if (!agents.find(a => a.id === preferred)?.disabled) return preferred;
  const open = launchable(agents);
  return (open.find(a => a.available !== false) ?? open[0])?.id ?? preferred;
}

// Move one id to another position of the full visible list; the result is what agentOrder stores
export function moveAgent(ids: AgentId[], id: AgentId, to: number): AgentId[] {
  const from = ids.indexOf(id);
  if (from < 0) return ids;
  const next = ids.filter(x => x !== id);
  next.splice(Math.max(0, Math.min(next.length, to)), 0, id);
  return next;
}
