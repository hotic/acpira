import type { ReactNode } from 'react';
import { UserRound } from 'lucide-react';
import { cn } from './cn';
import { t } from '../i18n';

// Keep the account identity and plan on one line; only the identity may truncate.
export function AccountLabel({ label, detail }: { label: string; detail?: string }) {
  return <span className="flex min-w-0 items-baseline gap-2" title={[label, detail].filter(Boolean).join(t('common.metaSep'))}>
    <span className="min-w-0 truncate">{label}</span>
    {detail && <span className="shrink-0 text-3 font-normal text-fg-2">{detail}</span>}
  </span>;
}

// One account identity in every account menu: a round avatar, the name line, and an optional caption under it.
// A caption that explains a state (signed out, how to sign in) wraps; a short tag ("Official account") truncates.
export function AccountIdentity({ children, caption, wrapCaption = false, className }: {
  children: ReactNode; caption?: ReactNode; wrapCaption?: boolean; className?: string;
}) {
  return <span className={cn('flex min-w-0 flex-1 items-center gap-gap', className)}>
    <span className="flex size-ctl shrink-0 items-center justify-center rounded-full bg-hover text-fg-3" aria-hidden>
      <UserRound className="size-icon" strokeWidth={1.75} />
    </span>
    <span className="flex min-w-0 flex-1 flex-col text-left">
      <span className="min-w-0 truncate">{children}</span>
      {caption && <span className={cn('text-3 text-fg-3', wrapCaption ? 'text-pretty' : 'truncate')}>{caption}</span>}
    </span>
  </span>;
}
