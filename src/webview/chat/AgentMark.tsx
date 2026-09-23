import type { AgentId } from '@shared/transcript';
import { MarkSvg } from './marks';

// Vendor logo for an agent; used in the session list / agent Chip to tell agents apart
export function AgentMark({ id, name, className }: { id: AgentId; name?: string; className?: string }) {
  return <MarkSvg id={id} name={name} className={className} />;
}
