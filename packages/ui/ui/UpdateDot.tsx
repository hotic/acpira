import type { UpdateState } from '@acpira/shared/protocol';
import type { AgentInfo } from '@acpira/shared/transcript';
import { cn } from './cn';

// Something new to install: the session list's unread dot, breathing (motion.css .unread-halo). Marks the update action on an
// agent tile, the settings rail entry that leads to it and the desktop window's gear
export function UpdateDot({ className }: { className?: string }) {
  return <span aria-hidden className={cn('size-unread-dot shrink-0 rounded-full bg-fg-strong unread-halo', className)} />;
}

// An agent in use (not off, not another extension's) whose adapter is older than the version Acpira pins (AgentInfo.update)
export const agentUpdates = (agents: AgentInfo[]) => agents.some(a => a.update && !a.disabled);

// The desktop app has a newer version to download or one waiting for a restart
export const appUpdatePending = (s: UpdateState | undefined) => s?.phase === 'available' || s?.phase === 'ready';
