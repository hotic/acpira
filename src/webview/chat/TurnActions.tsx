import { createContext, Fragment, useContext, useState, type ReactNode } from 'react';
import { ChartNoAxesColumn, Check, Copy, GitBranch } from 'lucide-react';
import type { AgentTurn, SessionControls, TurnSettings } from '@shared/transcript';
import { t, useLocale } from '../i18n';
import { cn } from '../ui/cn';
import { IconButton } from '../ui/Button';
import { Popover } from '../ui/Popover';
import { PanelFooter } from '../ui/Panel';
import { useCopied } from '../ui/useCopied';
import { elapsedDuration } from './folding';
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

function StatRow({ label, value }: { label: string; value: ReactNode }) {
  return (
    <div className="flex items-center justify-between gap-3 px-2 py-0.5 text-3">
      <span className="text-fg-1">{label}</span>
      <span className="min-w-0 truncate text-fg-2">{value}</span>
    </div>
  );
}

// Per-turn response statistics: dense label/value rows grouped by a thin divider — response facts, token accounting,
// the post-turn context snapshot. Only the rows the peer actually reported — Kimi reports no tokens at all, so its
// card is just the response group plus the context row
function StatsCard({ turn, model }: { turn: AgentTurn; model?: string }) {
  const usage = turn.usage;
  const locale = useLocale();
  const fmt = new Intl.NumberFormat(locale).format;
  const requestId = usage?.requestId;
  const copied = useCopied(requestId ?? '');

  const responseRows: ReactNode[] = [];
  if (model) responseRows.push(<StatRow key="model" label={t('stats.model')} value={model} />);
  if (turn.startedAt !== undefined && turn.endedAt !== undefined) responseRows.push(<StatRow key="duration" label={t('stats.duration')} value={elapsedDuration(turn)} />);
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
    .map(([key, label]) => <StatRow key={key} label={label} value={fmt(usage![key]!)} />);

  const groups = [
    responseRows,
    tokenRows,
    usage?.context ? [<StatRow key="ctx" label={t('stats.context')} value={`${fmt(usage.context.used)} / ${fmt(usage.context.size)}`} />] : [],
  ].filter(g => g.length);

  return (
    <div className="flex flex-col gap-1 p-1 tabular-nums">
      <div className="flex h-ctl items-center px-2">
        <span className="text-2 font-medium text-fg-1">{t('turn.stats')}</span>
      </div>
      <div className="flex flex-col">
        {groups.map((rows, i) => (
          <Fragment key={i}>
            {i > 0 && <div className="mx-2 my-1 border-t border-line" />}
            {rows}
          </Fragment>
        ))}
      </div>
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
