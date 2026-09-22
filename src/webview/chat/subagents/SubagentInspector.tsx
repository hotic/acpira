import { Fragment, useEffect, useLayoutEffect, useRef, useState, type ReactNode } from 'react';
import { ArrowLeft, Eye, X } from 'lucide-react';
import type { SubagentSummary } from '@shared/subagents';
import type { MsgKey } from '@shared/i18n';
import type { TextBlock, Turn } from '@shared/transcript';
import { t } from '../../i18n';
import { cn } from '../../ui/cn';
import { Button, IconButton } from '../../ui/Button';
import { Row } from '../../ui/Row';
import { Shimmer } from '../../ui/Shimmer';
import { AgentMessage, UserMessage } from '../Turns';
import { Prose } from '../Prose';
import { HistoryContext } from '../HistoryMessage';
import { TurnActionsContext } from '../TurnActions';
import { scrollerUsable } from '../promptStuck';
import { breadcrumb, elapsedText, stateLabel, subagentTitle, type SubagentTab } from './subagentState';
import { SubagentTreeView } from './SubagentTreeView';
import { useElapsed } from './useElapsed';

interface InspectorProps {
  node: SubagentSummary;
  all: SubagentSummary[];
  sessionTitle: string;
  // The observed child's transcript; absent until the host answers observeSubagent
  transcript?: { turns: Turn[]; running: boolean; rev: number };
  tab: SubagentTab;
  onTab: (tab: SubagentTab) => void;
  onSelect: (id: string) => void;
  onClose: () => void;
  onCancel?: () => void;
  onPermission: (blockId: string, optionId: string) => void;
  mode: 'docked' | 'overlay';
  blobUrl?: (blob: string) => string;
}

const TABS: SubagentTab[] = ['session', 'tree', 'info'];
const TAB_KEY: Record<SubagentTab, 'subagents.tabs.session' | 'subagents.tabs.tree' | 'subagents.tabs.info'> = {
  session: 'subagents.tabs.session',
  tree: 'subagents.tabs.tree',
  info: 'subagents.tabs.info',
};

// Read-only drill-down for one delegated child: the parent's delegation row opens this, and nothing
// inside it controls the session except the declared cancel affordance.
export function SubagentInspector(p: InspectorProps) {
  const { node, mode } = p;
  const root = useRef<HTMLDivElement>(null);
  useEffect(() => { root.current?.focus({ preventScroll: true }); }, [node.id]);
  const crumbs = breadcrumb(node.id, p.all);
  const elapsed = useElapsed(node);
  const meta = [stateLabel(node, t), node.role, elapsed, t('subagents.toolCount', { n: node.toolCount })].filter(Boolean);
  return (
    <div
      ref={root}
      tabIndex={-1}
      className="flex h-full min-h-0 flex-col outline-none"
      onKeyDown={e => {
        if (e.key === 'Escape' && !e.defaultPrevented) { e.stopPropagation(); p.onClose(); }
      }}
    >
      <div className="flex h-hdr shrink-0 items-center gap-gap border-b border-line px-pad">
        <IconButton onClick={p.onClose} aria-label={t('subagents.back')} title={t('subagents.back')}>
          {mode === 'overlay' ? <ArrowLeft strokeWidth={1.5} /> : <X strokeWidth={1.5} />}
        </IconButton>
        <nav aria-label="breadcrumb" className="flex min-w-0 flex-1 items-baseline gap-1 text-3 text-fg-3">
          <button type="button" onClick={p.onClose} className="shrink-0 cursor-pointer transition-colors hover:text-fg-1">{t('subagents.root')}</button>
          {crumbs.map((c, i) => (
            <Fragment key={c.id}>
              <span aria-hidden="true" className="shrink-0">›</span>
              {i === crumbs.length - 1
                ? <span className="min-w-0 truncate text-fg-2">{subagentTitle(c, t)}</span>
                : <button type="button" onClick={() => p.onSelect(c.id)} className="min-w-0 cursor-pointer truncate transition-colors hover:text-fg-1">{subagentTitle(c, t)}</button>}
            </Fragment>
          ))}
        </nav>
        <div role="tablist" className="flex shrink-0 items-center gap-gap">
          {TABS.map(tb => (
            <button
              key={tb}
              type="button"
              role="tab"
              aria-selected={p.tab === tb}
              onClick={() => p.onTab(tb)}
              className={cn('cursor-pointer text-2 transition-colors', p.tab === tb ? 'font-medium text-fg-1' : 'text-fg-3 hover:text-fg-1')}
            >
              {t(TAB_KEY[tb])}
            </button>
          ))}
        </div>
      </div>
      <div className="shrink-0 px-pad pt-pad">
        <h2 className="m-0 truncate text-1 font-medium text-fg-1">{subagentTitle(node, t)}</h2>
        <div className="mt-1 text-3 text-fg-3">{meta.join(' · ')}</div>
      </div>
      {p.tab === 'session' && <SessionTab {...p} />}
      {p.tab === 'tree' && (
        <div className="min-h-0 flex-1 overflow-y-auto scroll-thin">
          <SubagentTreeView all={p.all} sessionTitle={p.sessionTitle} selectedId={node.id}
            onSelect={id => { p.onSelect(id); p.onTab('session'); }} onClose={p.onClose} />
        </div>
      )}
      {p.tab === 'info' && (
        <div className="min-h-0 flex-1 overflow-y-auto scroll-thin">
          <InfoTab node={node} />
        </div>
      )}
    </div>
  );
}

function SessionTab({ node, transcript, onPermission, onCancel, blobUrl }: InspectorProps) {
  const scroll = useRef<HTMLDivElement>(null);
  const pinned = useRef(true);
  // Stick to the bottom while the child streams, exactly like the main thread; scrolling up releases the follow
  useEffect(() => {
    const el = scroll.current;
    if (!el) return;
    const onScroll = () => { if (scrollerUsable(el)) pinned.current = el.scrollHeight - el.scrollTop - el.clientHeight < 48; };
    el.addEventListener('scroll', onScroll, { passive: true });
    return () => el.removeEventListener('scroll', onScroll);
  }, []);
  useLayoutEffect(() => {
    const el = scroll.current;
    if (el && scrollerUsable(el) && pinned.current) el.scrollTop = el.scrollHeight;
  }, [transcript?.turns, transcript?.running]);
  const lastAgent = transcript ? transcript.turns.reduce((at, turn, i) => (turn.role === 'agent' ? i : at), -1) : -1;
  return (
    <>
      <div ref={scroll} className="scroll-thin min-h-0 min-w-0 flex-1 overflow-y-auto">
        <div className="flex min-w-0 flex-col gap-msg px-pad py-gap">
          {node.task !== undefined && <TaskCard task={node.task} />}
          {node.visibility === 'receipt' ? (
            <>
              <Row className="text-fg-3"><span>{t('subagents.receiptOnly')}</span></Row>
              {node.result !== undefined && (
                <section className="flex min-w-0 flex-col gap-1">
                  <div className="text-3 text-fg-3">{t('subagents.result')}</div>
                  <Prose block={textOf(node.result)} />
                </section>
              )}
            </>
          ) : transcript === undefined ? (
            <Row className="text-fg-3"><Shimmer active>{t('subagents.loading')}</Shimmer></Row>
          ) : (
            <TurnActionsContext.Provider value={undefined}>
              <HistoryContext.Provider value={undefined}>
                <div className="flex min-w-0 flex-col gap-msg">
                  {transcript.turns.map((turn, ti) => turn.role === 'agent'
                    ? (
                      <AgentMessage
                        key={ti}
                        turn={turn}
                        index={ti}
                        running={!!transcript.running && ti === lastAgent}
                        onPermission={onPermission}
                        memoryKey={`sub:${node.id}:${ti}`}
                        turnIndex={ti}
                        last={ti === transcript.turns.length - 1}
                        actions={false}
                        lead="static"
                      />
                    )
                    : <UserMessage key={ti} turn={turn} index={ti} blobUrl={blobUrl} />)}
                  {transcript.turns.length === 0 && <Row className="text-fg-3"><Shimmer active>{t('subagents.loading')}</Shimmer></Row>}
                </div>
              </HistoryContext.Provider>
            </TurnActionsContext.Provider>
          )}
          {node.visibility === 'nested' && node.result !== undefined && (
            <section className="flex min-w-0 flex-col gap-1">
              <div className="text-3 text-fg-3">{t('subagents.result')}</div>
              <Prose block={textOf(node.result)} />
            </section>
          )}
          {node.visibility === 'nested' && node.state === 'running' && node.result === undefined && (
            <Row className="text-fg-3"><span>{t('subagents.nestedNote')}</span></Row>
          )}
        </div>
      </div>
      <footer className="shrink-0 border-t border-line px-pad py-gap text-3 text-fg-3">
        <div className="flex items-center gap-2">
          <Eye className="size-icon shrink-0" strokeWidth={1.5} />
          <span className="min-w-0 flex-1">{t('subagents.observedOnly')}</span>
          {node.state === 'running' && node.controls.cancel && onCancel !== undefined && (
            <Button onClick={onCancel} disabled={node.cancelRequested} className="shrink-0 disabled:cursor-not-allowed disabled:text-fg-3">
              {t(node.cancelRequested ? 'subagents.cancelling' : 'subagents.cancel')}
            </Button>
          )}
        </div>
        {node.state === 'running' && !node.controls.cancel && <div className="mt-1">{t('subagents.noCancel')}</div>}
        {node.state === 'disconnected' && <div className="mt-1">{t('subagents.disconnectedNote')}</div>}
      </footer>
    </>
  );
}

function textOf(markdown: string): TextBlock {
  return { type: 'text', markdown };
}

// The delegated task in a quiet card; long briefs clamp to six lines with a text toggle
function TaskCard({ task }: { task: string }) {
  const body = useRef<HTMLDivElement>(null);
  const [clamped, setClamped] = useState(false);
  const [open, setOpen] = useState(false);
  useLayoutEffect(() => {
    const el = body.current;
    if (el) setClamped(el.scrollHeight > el.clientHeight + 1);
  }, [task]);
  return (
    <section className="flex min-w-0 flex-col gap-1">
      <div className="text-3 text-fg-3">{t('subagents.task')}</div>
      <div className="rounded-md border border-line bg-bg-1 p-pad">
        <div ref={body} className={cn('text-2 text-fg-2 whitespace-pre-wrap [overflow-wrap:anywhere]', !open && 'line-clamp-6')}>{task}</div>
        {(clamped || open) && (
          <button type="button" onClick={() => setOpen(o => !o)} className="mt-1 cursor-pointer text-3 text-fg-3 transition-colors hover:text-fg-1">
            {t(open ? 'subagents.taskCollapse' : 'subagents.taskExpand')}
          </button>
        )}
      </div>
    </section>
  );
}

// Facts the agent actually reported — absent fields stay absent, nothing is invented
function InfoTab({ node }: { node: SubagentSummary }) {
  const peers = (Object.entries(node.peer) as [string, string][]).filter(([, v]) => v).map(([k, v]) => `${k} ${v}`).join(' · ');
  const facts: ReactNode[] = [];
  const fact = (label: string, value: ReactNode, mono = false) => facts.push(
    <Row key={label} dense>
      <dt className="shrink-0 text-fg-3">{label}</dt>
      <dd className={cn('m-0 min-w-0 truncate text-fg-1', mono && 'font-mono text-mono text-fg-3')} title={typeof value === 'string' ? value : undefined}>{value}</dd>
    </Row>,
  );
  fact(t('subagents.info.state'), stateLabel(node, t));
  if (node.role !== undefined) fact(t('subagents.info.role'), node.role);
  if (node.model !== undefined) fact(t('subagents.info.model'), node.model);
  fact(
    t('subagents.info.visibility'),
    <span className="flex min-w-0 flex-col">
      <span className="truncate text-fg-1">{t(`subagents.visibility.${node.visibility}` as MsgKey)}</span>
      <span className="text-3 text-fg-3">{t(`subagents.visibility.${node.visibility}Hint` as MsgKey)}</span>
    </span>,
  );
  fact(t('subagents.info.elapsed'), elapsedText(node, Date.now(), t));
  fact(t('subagents.info.tools'), node.toolCount);
  if (node.background) fact(t('subagents.info.background'), t('question.yes'));
  fact(t('subagents.info.cancelable'), node.controls.cancel ? t('question.yes') : t('question.no'));
  if (node.usage !== undefined) fact(t('subagents.info.context'), `${node.usage.used} / ${node.usage.size}`);
  if (peers) fact(t('subagents.info.peer'), peers, true);
  return <dl className="m-0 flex flex-col gap-0.5 px-pad py-gap">{facts}</dl>;
}
