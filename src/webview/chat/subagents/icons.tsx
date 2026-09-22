import { Bot, Check, Hand, MessageCircleQuestion, TriangleAlert, Unplug, X } from 'lucide-react';
import type { SubagentSummary } from '@shared/subagents';
import { cn } from '../../ui/cn';

// Child rows keep static icons; the only Orb belongs to the turn's top-level activity.
// A pending card outranks the state icon — that is what the child is blocked on.
export function stateIcon(node: SubagentSummary) {
  const cls = 'size-icon';
  if (node.permissions?.length) return <Hand className={cn(cls, 'text-warn')} strokeWidth={1.5} />;
  if (node.question) return <MessageCircleQuestion className={cn(cls, 'text-warn')} strokeWidth={1.5} />;
  switch (node.state) {
    case 'running': return <Bot className={cls} strokeWidth={1.5} />;
    case 'completed': return <Check className={cls} strokeWidth={1.5} />;
    case 'failed': return <TriangleAlert className={cn(cls, 'text-danger')} strokeWidth={1.5} />;
    case 'cancelled': return <X className={cls} strokeWidth={1.5} />;
    case 'disconnected': return <Unplug className={cls} strokeWidth={1.5} />;
  }
}
