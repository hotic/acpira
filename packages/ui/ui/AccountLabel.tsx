import { cn } from './cn';
import { t } from '../i18n';

// Keep the account identity and plan on one line; only the identity may truncate.
// detailClassName: the account menu sets its plans a step quieter than settings rows do
export function AccountLabel({ label, detail, className, detailClassName }: { label: string; detail?: string; className?: string; detailClassName?: string }) {
  return <span className={cn('flex min-w-0 items-baseline gap-2', className)} title={[label, detail].filter(Boolean).join(t('common.metaSep'))}>
    <span className="min-w-0 truncate">{label}</span>
    {detail && <span className={cn('shrink-0 text-3 font-normal text-fg-2', detailClassName)}>{detail}</span>}
  </span>;
}
