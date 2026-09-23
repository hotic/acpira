import type { CSSProperties } from 'react';
import { Check, Hand, MessageCircleQuestion, TriangleAlert, Unplug, X } from 'lucide-react';
import type { SubagentSummary } from '@shared/subagents';
import { cn } from '../../ui/cn';

// A small travelling constellation distinguishes child activity from the root Orb.
// A pending card outranks the state icon — that is what the child is blocked on.
export function stateIcon(node: SubagentSummary) {
  const cls = 'size-icon';
  if (node.permissions?.length) return <Hand className={cn(cls, 'text-warn')} strokeWidth={1.5} />;
  if (node.question) return <MessageCircleQuestion className={cn(cls, 'text-warn')} strokeWidth={1.5} />;
  switch (node.state) {
    case 'running': return <span className="subagent-signal" aria-hidden="true">
      {Array.from({ length: 6 }, (_, index) => <span key={index} style={{ '--signal-step': index } as CSSProperties} />)}
    </span>;
    case 'completed': return <Check className={cls} strokeWidth={1.5} />;
    case 'failed': return <TriangleAlert className={cn(cls, 'text-danger')} strokeWidth={1.5} />;
    case 'cancelled': return <X className={cls} strokeWidth={1.5} />;
    case 'disconnected': return <Unplug className={cls} strokeWidth={1.5} />;
  }
}
