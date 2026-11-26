import { type ReactNode } from 'react';
import { X } from 'lucide-react';
import { t } from '../i18n';
import { cn } from './cn';

// Wraps a chip with a remove button riding its top-right corner — a badge over the edge, so it never lands on the
// label (and keeps the thumbnail's corner visible too). Shown on hover / focus, inert while hidden.
export function Removable({ label, onRemove, disabled, children }: { label: string; onRemove?: () => void; disabled?: boolean; children: ReactNode }) {
  return (
    <span className="group/chip relative inline-flex max-w-full min-w-0">
      {children}
      {onRemove && <button
        type="button"
        disabled={disabled}
        aria-label={label}
        title={t('common.remove')}
        onClick={onRemove}
        className={cn(
          'pointer-events-none absolute -top-2 -right-2 flex size-icon-ctl items-center justify-center rounded-full border border-line-strong bg-bg-2 text-fg-2 opacity-0 transition-opacity',
          'group-hover/chip:pointer-events-auto group-hover/chip:opacity-100 group-focus-within/chip:pointer-events-auto group-focus-within/chip:opacity-100 focus-visible:pointer-events-auto focus-visible:opacity-100',
          'hover:bg-bg-1 hover:text-fg-1',
        )}
      >
        <X className="size-3" strokeWidth={2} />
      </button>}
    </span>
  );
}
