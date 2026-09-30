import type { AccountInfo, LocalAccountInfo } from '@shared/transcript';
import { t } from '../i18n';
import { QuotaBars } from './QuotaBars';

export function LocalAccountQuota({ account }: { account: LocalAccountInfo }) {
  return account.quota
    ? <QuotaBars quota={account.quota} />
    : <span className="pt-1 text-3 text-fg-2">{t(`quota.status.${account.status}`)}</span>;
}

// A saved account: its bars, or why the last read failed; nothing while the first read is still out
export function SavedAccountQuota({ account }: { account: AccountInfo }) {
  if (account.quota) return <QuotaBars quota={account.quota} />;
  return account.quotaIssue ? <span className="pt-1 text-3 text-fg-2">{t(`quota.status.${account.quotaIssue}`)}</span> : null;
}
