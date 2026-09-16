import { t } from '../i18n';

// Keep the account identity and plan on one line; only the identity may truncate.
export function AccountLabel({ label, detail }: { label: string; detail?: string }) {
  return <span className="flex min-w-0 items-baseline gap-2" title={[label, detail].filter(Boolean).join(t('common.metaSep'))}>
    <span className="min-w-0 truncate">{label}</span>
    {detail && <span className="shrink-0 text-3 font-normal text-fg-2">{detail}</span>}
  </span>;
}
