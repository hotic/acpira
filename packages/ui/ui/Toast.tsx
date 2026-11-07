import { useEffect, useRef, type ReactNode } from 'react';
import { Trash2, Undo2, X } from 'lucide-react';
import { Button, IconButton } from './Button';
import { t } from '../i18n';

export interface ToastProps {
  text: string;
  // Lead icon, --icon sized; deletion's trash can by default
  icon?: ReactNode;
  // An undoable action (session deletion and the like)
  onUndo?: () => void;
  // Any other one-click follow-ups (restart to update; Open and reveal for a file the shell wrote); labels come localized.
  // One with an icon is a bare icon button, its label the tooltip
  actions?: { label: string; icon?: ReactNode; onClick: () => void }[];
  onClose: () => void;
  // Milliseconds before it dismisses itself
  ttl?: number;
}

// A capsule floating above the bottom of the conversation flow: icon, text, undo or other actions, close. Dismisses itself
// when time's up. The lead glyph matches the icon buttons' size, and the icon buttons sit side by side (their own hover
// fills keep them apart)
export function Toast({ text, icon, onUndo, actions, onClose, ttl = 8000 }: ToastProps) {
  // The timer runs once per mount; a parent re-rendering with a fresh onClose must not restart it
  const close = useRef(onClose);
  close.current = onClose;
  useEffect(() => {
    const t = setTimeout(() => close.current(), ttl);
    return () => clearTimeout(t);
  }, [ttl]);
  return (
    <div role="status" className="enter pointer-events-auto flex max-w-[calc(100%-2*var(--pad))] items-center gap-2 rounded-lg border border-line bg-bg-2 py-1 pr-1 pl-3 text-2 text-fg-1 shadow-pop">
      <span className="flex shrink-0 [&_svg]:size-icon-ctl">{icon ?? <Trash2 className="text-fg-3" strokeWidth={1.5} />}</span>
      <span className="truncate">{text}</span>
      <div className="ml-1 flex shrink-0 items-center gap-1">
        {actions?.filter(a => !a.icon).map(a => <Button key={a.label} variant="secondary" onClick={a.onClick}>{a.label}</Button>)}
        <div className="flex items-center">
          {onUndo && <IconButton title={t('common.undo')} aria-label={t('common.undo')} onClick={onUndo}><Undo2 strokeWidth={1.5} /></IconButton>}
          {actions?.filter(a => a.icon).map(a => <IconButton key={a.label} title={a.label} aria-label={a.label} onClick={a.onClick}>{a.icon}</IconButton>)}
          <IconButton aria-label={t('common.close')} onClick={onClose}><X strokeWidth={1.5} /></IconButton>
        </div>
      </div>
    </div>
  );
}
