import { createContext, useContext, useState, type ReactNode } from 'react';
import { ChartNoAxesColumn, Check, Copy, GitBranch } from 'lucide-react';
import type { AgentTurn, SessionControls, TurnSettings } from '@shared/transcript';
import { t, useLocale } from '../i18n';
import { cn } from '../ui/cn';
import { IconButton } from '../ui/Button';
import { Popover } from '../ui/Popover';
import { PanelFooter } from '../ui/Panel';
import { useCopied } from '../ui/useCopied';
import { elapsedLabel } from './folding';
import { modelLabel, replyMarkdown, toolCallCount } from './turnActionHelpers';

// What the turn action row needs from the shell: the session it belongs to, the control list for resolving the model
// name, and the fork action — absent for external (ChatGPT) sessions and in hosts that do not wire it (LAB)
export const TurnActionsContext = createContext<{ sessionId: string; controls: SessionControls; fork?: (turnIndex: number) => void } | undefined>(undefined);

// The row under a finished agent reply (Cursor's trio): copy the reply · fork the session from this turn · open the
// response statistics card. Hidden until the turn is hovered / focused, except on the last reply where it always shows
export function TurnActions({ turn, turnIndex, last, settings }: { turn: AgentTurn; turnIndex: number; last: boolean; settings?: TurnSettings }) {
  const ctx = useContext(TurnActionsContext);
  const reply = replyMarkdown(turn);
  const copyState = useCopied(reply);
  const [statsOpen, setStatsOpen] = useState(false);
  const copyLabel = copyState.state === 'copied' ? t('code.copied') : copyState.state === 'failed' ? t('code.copyFailed') : t('turn.copy');
  return (
    <div
      data-open={statsOpen || undefined}
      className={cn('flex h-ctl-sm items-center justify-end gap-0.5',
        !last && 'opacity-0 transition-opacity duration-(--code-copy-duration) ease-out group-hover/turn:opacity-100 group-focus-within/turn:opacity-100 [@media(hover:none)]:opacity-100 motion-reduce:transition-none data-[open]:opacity-100')}
    >
      {reply && (
        <>
          <IconButton size="sm" title={copyLabel} aria-label={copyLabel} onClick={copyState.copy}>
            {copyState.state === 'copied' ? <Check strokeWidth={1.5} /> : <Copy strokeWidth={1.5} />}
          </IconButton>
          <span className="sr-only" role="status">{copyState.state === 'idle' ? '' : copyLabel}</span>
        </>
      )}
      {ctx?.fork && (
        <IconButton size="sm" title={t('turn.fork')} aria-label={t('turn.fork')} onClick={() => ctx.fork!(turnIndex)}>
          <GitBranch strokeWidth={1.5} />
        </IconButton>
      )}
      <Popover.Root open={statsOpen} onOpenChange={setStatsOpen}>
        <Popover.Trigger render={
          <IconButton size="sm" data-open={statsOpen || undefined} title={t('turn.stats')} aria-label={t('turn.stats')} aria-expanded={statsOpen}>
            <ChartNoAxesColumn strokeWidth={1.5} />
          </IconButton>
        } />
        <Popover.Portal><Popover.Positioner side="bottom" align="end" width="lg"><Popover.Popup>
          <StatsCard turn={turn} model={modelLabel(turn.usage, settings, ctx?.controls ?? { modes: [], options: [] })} />
        </Popover.Popup></Popover.Positioner></Popover.Portal>
      </Popover.Root>
    </div>
  );
}

// One caption + rows pair; a row renders only when its value exists
function Section({ caption, rows }: { caption: string; rows: ReactNode[] }) {
  if (!rows.length) return null;
  return (
    <>
      <div className="px-2 text-3 text-fg-3">{caption}</div>
      {rows}
    </>
  );
}

function StatRow({ label, value, faint }: { label: string; value: ReactNode; faint?: boolean }) {
  return (
    <div className="flex min-h-row items-center justify-between px-2 text-3">
      <span className={faint ? 'text-fg-3' : 'text-fg-1'}>{label}</span>
      <span className="min-w-0 truncate text-fg-2">{value}</span>
    </div>
  );
}

// Per-turn response statistics (UsagePanel's shape): only the rows the peer actually reported — Kimi shows no tokens at
// all and gets the explanatory line instead of a column of dashes
function StatsCard({ turn, model }: { turn: AgentTurn; model?: string }) {
  const usage = turn.usage;
  const locale = useLocale();
  const fmt = new Intl.NumberFormat(locale).format;
  const requestId = usage?.requestId;
  const copied = useCopied(requestId ?? '');

  const responseRows: ReactNode[] = [];
  if (model) responseRows.push(<StatRow key="model" label={t('stats.model')} value={model} />);
  if (turn.startedAt !== undefined && turn.endedAt !== undefined) responseRows.push(<StatRow key="duration" label={t('stats.duration')} value={elapsedLabel(turn)} />);
  const tools = toolCallCount(turn);
  if (tools > 0) responseRows.push(<StatRow key="tools" label={t('stats.toolCalls')} value={fmt(tools)} />);
  if (usage?.modelCalls !== undefined) responseRows.push(<StatRow key="modelCalls" label={t('stats.modelCalls')} value={fmt(usage.modelCalls)} />);

  const tokenFields: [key: 'input' | 'output' | 'cachedRead' | 'cachedWrite' | 'reasoning', label: string][] = [
    ['input', t('stats.input')], ['output', t('stats.output')], ['cachedRead', t('stats.cachedRead')],
    ['cachedWrite', t('stats.cachedWrite')], ['reasoning', t('stats.reasoning')],
  ];
  // input / output render whenever reported, 0 included; the optional cache / reasoning rows only when they carry a
  // non-zero count — Grok reports cacheCreationTokens: 0 on every turn, and a "0 tokens" row is noise
  const tokenRows = tokenFields.filter(([key]) => { const v = usage?.[key]; return v !== undefined && (key === 'input' || key === 'output' || v > 0); })
    .map(([key, label]) => <StatRow key={key} label={label} value={t('stats.tokensN', { n: fmt(usage![key]!) })} />);

  return (
    <div className="flex flex-col gap-1 p-1 tabular-nums">
      <div className="flex h-ctl items-center px-2">
        <span className="text-2 font-medium text-fg-1">{t('turn.stats')}</span>
      </div>
      <Section caption={t('stats.response')} rows={responseRows} />
      <Section caption={t('stats.tokens')} rows={tokenRows} />
      {usage?.context && <StatRow label={t('stats.context')} value={`${fmt(usage.context.used)} / ${fmt(usage.context.size)}`} />}
      {/* The note explains the missing token section; a context snapshot is session-level and does not replace it */}
      {!tokenRows.length && <StatRow faint label={t('stats.none')} value="" />}
      {requestId && (
        <>
          <PanelFooter action={{ label: copied.state === 'copied' ? t('stats.requestIdCopied') : t('stats.copyRequestId'), icon: copied.state === 'copied' ? <Check strokeWidth={1.5} /> : <Copy strokeWidth={1.5} />, onClick: () => void copied.copy() }}>
            <span className="text-3 text-fg-3">{requestId}</span>
          </PanelFooter>
          <span className="sr-only" role="status">{copied.state === 'idle' ? '' : copied.state === 'copied' ? t('stats.requestIdCopied') : t('code.copyFailed')}</span>
        </>
      )}
    </div>
  );
}
