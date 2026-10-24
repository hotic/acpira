import { useEffect, useRef, type ReactNode } from 'react';
import { Trash2, X } from 'lucide-react';
import { Button, IconButton } from './Button';
import { t } from '../i18n';

export interface ToastProps {
  text: string;
  // Lead icon, --icon sized; deletion's trash can by default
  icon?: ReactNode;
  // An undoable action (session deletion and the like)
  onUndo?: () => void;
  // Any other one-click follow-up (the shell's "Open" for a file it wrote); its label comes localized
  action?: { label: string; onClick: () => void };
  onClose: () => void;
  // Milliseconds before it dismisses itself
  ttl?: number;
}

// A capsule floating above the bottom of the conversation flow: icon, text, undo or another action, close. Dismisses itself
// when time's up
export function Toast({ text, icon, onUndo, action, onClose, ttl = 8000 }: ToastProps) {
  // The timer runs once per mount; a parent re-rendering with a fresh onClose must not restart it
  const close = useRef(onClose);
  close.current = onClose;
  useEffect(() => {
    const t = setTimeout(() => close.current(), ttl);
    return () => clearTimeout(t);
  }, [ttl]);
  return (
    <div role="status" className="enter pointer-events-auto flex max-w-[calc(100%-2*var(--pad))] items-center gap-2 rounded-lg border border-line bg-bg-2 py-1 pr-1 pl-3 text-2 text-fg-1 shadow-pop">
      {icon ?? <Trash2 className="size-icon shrink-0 text-fg-3" strokeWidth={1.5} />}
      <span className="truncate">{text}</span>
      {onUndo && <Button variant="secondary" onClick={onUndo} className="ml-1">{t('common.undo')}</Button>}
      {action && <Button variant="secondary" onClick={action.onClick} className="ml-1">{action.label}</Button>}
      <IconButton aria-label={t('common.close')} onClick={onClose}><X strokeWidth={1.5} /></IconButton>
    </div>
  );
}
