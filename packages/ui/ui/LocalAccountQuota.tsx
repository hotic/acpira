import type { AccountInfo, LocalAccountInfo } from '@acpira/shared/transcript';
import { t } from '../i18n';
import { QuotaBars } from './QuotaBars';

export function LocalAccountQuota({ account }: { account: LocalAccountInfo }) {
  return account.quota
    ? <QuotaBars quota={account.quota} stale={account.status !== 'ready' && account.status !== 'loading'} />
    : <span className="pt-1 text-3 text-fg-2">{t(`quota.status.${account.status}`)}</span>;
}

// A saved account: its bars (grey once a later read failed), or why the last read failed; nothing while the first read
// is still out
export function SavedAccountQuota({ account }: { account: AccountInfo }) {
  if (account.quota) return <QuotaBars quota={account.quota} stale={!!account.quotaIssue} />;
  return account.quotaIssue ? <span className="pt-1 text-3 text-fg-2">{t(`quota.status.${account.quotaIssue}`)}</span> : null;
}
