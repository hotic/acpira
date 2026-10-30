import { createContext, Fragment, useContext, useState, type ReactNode } from 'react';
import { ChartNoAxesColumn, Check, Copy, GitBranch } from 'lucide-react';
import type { AgentId, AgentInfo, AgentTurn, SessionControls, TurnSettings } from '@acpira/shared/transcript';
import { t, useLocale } from '../i18n';
import { IconButton } from '../ui/Button';
import { DropdownMenu } from '../ui/DropdownMenu';
import { Popover } from '../ui/Popover';
import { OptionContent, PanelFooter } from '../ui/Panel';
import { useCopied } from '../ui/useCopied';
import { AgentMark } from './AgentMark';
import { elapsedDuration } from './folding';
import { forkTargets, modelLabel, replyMarkdown, turnOutputTps, turnTtft } from './turnActionHelpers';
import { hasTurnStats } from './turnOutcome';
import { PluginActions } from '../plugins/MessageActions';

// What the turn action row needs from the shell: the session it belongs to, the control list for resolving the model
// name, and the fork action with the agents it can continue on — absent for external (ChatGPT) sessions and in hosts
// that do not wire it (LAB)
export const TurnActionsContext = createContext<{
  sessionId: string; controls: SessionControls;
  fork?: (turnIndex: number, agent: AgentId) => void; agents?: AgentInfo[]; agentId?: AgentId;
} | undefined>(undefined);

// The row under a finished agent reply (Cursor's trio): copy the reply · fork the session from this turn onto an agent
// picked from a menu (the session's own first) · open the response statistics card. Hidden until the turn is hovered / focused (the last reply included), always shown without hover (touch).
// The row keeps its height (hover must not shift the transcript) but stands in for the spacing around it: it hugs the reply
// (`-mt-gap` cancels the turn's row gap, the buttons' own padding is the air) and the exchange below it starts one row gap
// after it instead of a message gap (`Thread` in Shell.tsx keys that off `data-turn-actions`)
export function TurnActions({ turn, turnIndex, settings }: { turn: AgentTurn; turnIndex: number; settings?: TurnSettings }) {
  const ctx = useContext(TurnActionsContext);
  const reply = replyMarkdown(turn);
  const copyState = useCopied(reply);
  const [statsOpen, setStatsOpen] = useState(false);
  const [forkOpen, setForkOpen] = useState(false);
  // A failed reply with nothing in it offers the fork alone: there is nothing to copy or measure. A stopped one keeps
  // its stats even when nothing streamed (`hasTurnStats`)
  const stats = hasTurnStats(turn);
  const copyLabel = copyState.state === 'copied' ? t('code.copied') : copyState.state === 'failed' ? t('code.copyFailed') : t('turn.copy');
  if (!stats && !ctx?.fork) return null;
  return (
    <div
      data-open={statsOpen || forkOpen || undefined}
      data-turn-actions
      className="-mt-gap flex h-ctl-sm items-center justify-end gap-0.5 opacity-0 transition-opacity duration-(--code-copy-duration) ease-out group-hover/turn:opacity-100 group-focus-within/turn:opacity-100 [@media(hover:none)]:opacity-100 motion-reduce:transition-none data-[open]:opacity-100"
    >
      {ctx && reply && <PluginActions session={ctx.sessionId} turn={turnIndex} text={reply} />}
      {reply && (
        <>
          <IconButton size="sm" title={copyLabel} aria-label={copyLabel} onClick={copyState.copy}>
            {copyState.state === 'copied' ? <Check strokeWidth={1.5} /> : <Copy strokeWidth={1.5} />}
          </IconButton>
          <span className="sr-only" role="status">{copyState.state === 'idle' ? '' : copyLabel}</span>
        </>
      )}
      {ctx?.fork && ctx.agentId && (
        <DropdownMenu.Root open={forkOpen} onOpenChange={setForkOpen}>
          <DropdownMenu.Trigger render={
            <IconButton size="sm" data-open={forkOpen || undefined} title={t('turn.fork')} aria-label={t('turn.fork')} aria-expanded={forkOpen}>
              <GitBranch strokeWidth={1.5} />
            </IconButton>
          } />
          <DropdownMenu.Portal><DropdownMenu.Positioner side="bottom" align="end" width="md"><DropdownMenu.Popup>
            {forkOpen && (
              <div className="scroll-thin flex max-h-pop flex-col overflow-y-auto">
                {forkTargets(ctx.agents ?? [], ctx.agentId).map(a => (
                  <DropdownMenu.Item key={a.id} disabled={a.available === false} title={a.available === false ? t('agent.notInstalled') : undefined}
                    onClick={() => ctx.fork!(turnIndex, a.id)}>
                    <OptionContent icon={<AgentMark id={a.id} name={a.name} />}>{a.name}</OptionContent>
                  </DropdownMenu.Item>
                ))}
              </div>
            )}
          </DropdownMenu.Popup></DropdownMenu.Positioner></DropdownMenu.Portal>
        </DropdownMenu.Root>
      )}
      {stats && <Popover.Root open={statsOpen} onOpenChange={setStatsOpen}>
        <Popover.Trigger render={
          <IconButton size="sm" data-open={statsOpen || undefined} title={t('turn.stats')} aria-label={t('turn.stats')} aria-expanded={statsOpen}>
            <ChartNoAxesColumn strokeWidth={1.5} />
          </IconButton>
        } />
        <Popover.Portal><Popover.Positioner side="bottom" align="end" width="lg"><Popover.Popup>
          {statsOpen && <StatsCard turn={turn} model={modelLabel(turn.usage, settings, ctx?.controls ?? { modes: [], options: [] })} />}
        </Popover.Popup></Popover.Positioner></Popover.Portal>
      </Popover.Root>}
    </div>
  );
}

// A label / value line; an optional chip (cache share, context share) rides with the value on the right, so the label
// column stays a clean list
function StatRow({ label, value, chip }: { label: string; value: ReactNode; chip?: ReactNode }) {
  return (
    <div className="flex items-center gap-2 px-2 py-0.5 text-3">
      <span className="min-w-0 truncate text-fg-2">{label}</span>
      <span className="ml-auto flex shrink-0 items-center gap-2 text-fg-1">{chip}<span>{value}</span></span>
    </div>
  );
}

function StatChip({ children }: { children: ReactNode }) {
  return <span className="rounded-sm bg-active px-1 text-fg-2">{children}</span>;
}

// Digits large, the unit words between them small ("2 分钟 24 秒", "33 tok/s"); group separators (NBSP included) stay with the number
function Figure({ text }: { text: string }) {
  return <>{text.split(/(\d(?:[\d.,\u00a0\u202f]*\d)?)/).map((part, i) => part && (i % 2
    ? <Fragment key={i}>{part}</Fragment>
    : <span key={i} className="text-4 font-normal text-fg-3/60">{part}</span>))}</>;
}

// Per-turn response statistics, no colour: the timing facts as tiles (duration, first token, output speed), then the
// token lines with the cache share as a chip on the input line, then the post-turn context with its share. Only what the
// peer actually reported: Kimi reports no tokens at all, so its card is the tiles plus the context line
function StatsCard({ turn, model }: { turn: AgentTurn; model?: string }) {
  const usage = turn.usage;
  const locale = useLocale();
  const fmt = new Intl.NumberFormat(locale).format;
  // Floored, so a 99.6% hit never reads as a full 100%
  const pct = (part: number, whole: number) => new Intl.NumberFormat(locale, { style: 'percent', maximumFractionDigits: 0 }).format(Math.floor(part / whole * 100) / 100);
  const requestId = usage?.requestId;
  const copied = useCopied(requestId ?? '');

  const tiles: { label: string; value: string }[] = [];
  if (turn.startedAt !== undefined && turn.endedAt !== undefined) tiles.push({ label: t('stats.duration'), value: elapsedDuration(turn) });
  const ttft = turnTtft(turn);
  if (ttft) tiles.push({ label: t('stats.firstToken'), value: ttft });
  const tps = turnOutputTps(turn);
  if (tps !== undefined) tiles.push({ label: t('stats.outputShort'), value: `${fmt(tps)} tok/s` });

  const tokenFields: [key: 'input' | 'output' | 'cachedRead' | 'cachedWrite' | 'reasoning', label: string][] = [
    ['input', t('stats.input')], ['cachedWrite', t('stats.cachedWrite')], ['output', t('stats.output')], ['reasoning', t('stats.reasoning')],
  ];
  // input / output render whenever reported, 0 included; the optional cache write / reasoning lines only when they carry
  // a non-zero count — Grok reports cacheCreationTokens: 0 on every turn, and a "0 tokens" line is noise. A stopped turn
  // whose counts are all zero reported nothing (claude-agent-acp answers a cancel with zeros), so it shows no token lines
  const unreported = turn.stop === 'cancelled' && !(['input', 'output', 'cachedRead', 'cachedWrite', 'reasoning'] as const).some(key => (usage?.[key] ?? 0) > 0);
  const cached = usage?.cachedRead && usage.input ? <StatChip>{t('stats.cacheHit', { pct: pct(usage.cachedRead, usage.input) })}</StatChip> : undefined;
  const tokenRows = tokenFields.filter(([key]) => { const v = usage?.[key]; return !unreported && v !== undefined && (key === 'input' || key === 'output' || v > 0); })
    .map(([key, label]) => <StatRow key={key} label={label} value={fmt(usage![key]!)} chip={key === 'input' ? cached : undefined} />);
  const context = usage?.context;

  return (
    <div className="flex flex-col gap-1 p-1 tabular-nums">
      <div className="flex h-ctl items-center justify-between gap-3 px-2">
        <span className="shrink-0 text-2 font-medium text-fg-1">{t('turn.stats')}</span>
        {model && <span className="min-w-0 truncate text-3 text-fg-3" title={model}>{model}</span>}
      </div>
      {tiles.length > 0 && (
        <div className="mx-1.5 grid grid-flow-col auto-cols-fr gap-px overflow-hidden rounded-md bg-line">
          {tiles.map(tile => (
            <div key={tile.label} className="flex min-w-0 flex-col gap-0.5 bg-bg-2 px-2 py-1.5">
              <span className="whitespace-nowrap text-2 font-semibold text-fg-1"><Figure text={tile.value} /></span>
              <span className="truncate text-4 text-fg-3/60" title={tile.label}>{tile.label}</span>
            </div>
          ))}
        </div>
      )}
      {(tokenRows.length > 0 || context) && (
        <div className="flex flex-col">
          {tokenRows}
          {context && <>
            {tokenRows.length > 0 && <div className="mx-2 my-1 border-t border-line" />}
            <StatRow label={t('stats.contextShort')} chip={<StatChip>{pct(context.used, context.size)}</StatChip>}
              value={<>{fmt(context.used)}<span className="text-fg-3"> / {fmt(context.size)}</span></>} />
          </>}
        </div>
      )}
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
