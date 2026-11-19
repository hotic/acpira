import { useEffect, useState } from 'react';
import { Check } from 'lucide-react';
import type { AccountQuota } from '@acpira/shared/transcript';
import { callPluginMethod, usePluginPieces } from '../pluginSlots';
import { QuotaBars } from '../ui/QuotaBars';

export interface PublishedAccountRow { label: string; badge: string; selected: boolean; windows: AccountQuota['windows'] }

// Only display data crosses into the account menu. No plugin row is a stored login or an account-selection action.
export function accountRowsOf(result: unknown): PublishedAccountRow[] {
  if (!Array.isArray(result)) return [];
  return result.slice(0, 8).flatMap(r => {
    if (typeof r?.label !== 'string' || !r.label.trim() || typeof r.badge !== 'string' || !r.badge.trim()) return [];
    const windows: AccountQuota['windows'] = [];
    for (const w of Array.isArray(r.windows) ? r.windows.slice(0, 8) : []) {
      if (typeof w?.id === 'string' && Number.isFinite(w.remaining) && w.remaining >= 0 && w.remaining <= 1) {
        windows.push({ id: w.id.slice(0, 64), remaining: w.remaining, ...(typeof w.resetsAt === 'string' && Number.isFinite(Date.parse(w.resetsAt)) && { resetsAt: w.resetsAt }) });
      }
    }
    return [{ label: r.label.slice(0, 160), badge: r.badge.slice(0, 32), selected: r.selected === true, windows }];
  });
}

export function PublishedAccountRow({ row, source, showCheck }: { row: PublishedAccountRow; source: string; showCheck: boolean }) {
  return <div data-plugin-account className="mx-1 flex min-w-0 flex-col border-t border-line px-1 py-1.5 text-2">
    <div className="flex min-w-0 items-center gap-2">
      <span className="truncate font-medium text-fg-1" title={row.label}>{row.label}</span>
      <span title={source} className="max-w-1/2 shrink-0 truncate rounded-sm border border-(--account-source)/25 bg-(--account-source)/10 px-1 text-3 text-(--account-source)">{row.badge}</span>
      {showCheck && row.selected && <Check aria-label={row.label} className="ml-auto size-icon shrink-0 text-fg-1" />}
    </div>
    {!!row.windows.length && <QuotaBars quota={{ windows: row.windows, fetchedAt: '' }} className="pt-3" />}
  </div>;
}

export function PluginAccountRows({ agent, session, showCheck }: { agent: string; session?: string; showCheck: boolean }) {
  const pieces = usePluginPieces('account.rows');
  return <>{pieces.map(p => <SourceRows key={JSON.stringify([p.plugin, agent, session])} plugin={p.plugin} source={p.title} agent={agent} session={session} showCheck={showCheck} />)}</>;
}

function SourceRows({ plugin, source, agent, session, showCheck }: { plugin: string; source: string; agent: string; session?: string; showCheck: boolean }) {
  const [rows, setRows] = useState<PublishedAccountRow[]>([]);
  useEffect(() => {
    let active = true;
    let busy = false;
    const refresh = async () => {
      if (busy) return;
      busy = true;
      try {
        const result = await callPluginMethod(plugin, 'account.rows', { agent, session });
        if (active) setRows(accountRowsOf(result));
      } catch { if (active) setRows([]); }
      finally { busy = false; }
    };
    void refresh();
    const timer = setInterval(refresh, 60_000);
    return () => { active = false; clearInterval(timer); };
  }, [agent, session, plugin]);
  return <>{rows.map((row, i) => <PublishedAccountRow key={i} row={row} source={source} showCheck={showCheck} />)}</>;
}
