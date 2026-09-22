import { useMemo, useState } from 'react';
import { Check, ChevronRight, Network } from 'lucide-react';
import type { SubagentSummary } from '@shared/subagents';
import { useAppearance } from '../../appearance';
import { t } from '../../i18n';
import { cn } from '../../ui/cn';
import { Disclosure } from '../../ui/Disclosure';
import { Row, RowLabel } from '../../ui/Row';
import { Shimmer } from '../../ui/Shimmer';
import { stateIcon } from './icons';
import { countsLabel, descendantCount, isWaiting, rootRows, secondLine, subagentTitle, type SubagentTab } from './subagentState';
import { useElapsed } from './useElapsed';

interface GroupProps {
  // Every node anchored to this turn (nested included — the header counts them all)
  nodes: SubagentSummary[];
  // The whole session's nodes, for descendant counts and breadcrumbs
  all: SubagentSummary[];
  onInspect: (id: string, tab: SubagentTab) => void;
}

// One quiet cluster per turn that delegated: a header tally plus a two-line row per top-level child.
// Rows are hand-shaped buttons — the shared Row's single content line cannot hold the two-line layout.
export function SubagentGroup({ nodes, all, onInspect }: GroupProps) {
  const { toolLine } = useAppearance();
  const rows = useMemo(() => rootRows(nodes), [nodes]);
  // Past five children the group folds its older finished rows; the disclosure stays open once opened
  const [open, setOpen] = useState(false);
  const terminal = rows.filter(n => n.state !== 'running' && !isWaiting(n));
  const fold = rows.length > 5;
  const hidden = fold && !open ? terminal.slice(0, Math.max(0, terminal.length - 3)) : [];
  const shown = rows.filter(n => !hidden.includes(n));
  if (!nodes.length || !rows.length) return null;
  return (
    <div className="flex flex-col gap-0.5">
      <Row
        lead={toolLine === 'text' ? undefined : <Network className="size-icon" strokeWidth={1.5} />}
        trailing={(
          <button type="button" onClick={() => onInspect(rows[0]!.id, 'tree')}
            className="cursor-pointer text-3 text-fg-3 transition-colors hover:text-fg-1 focus-visible:text-fg-1">
            {t('subagents.tree')}
          </button>
        )}
      >
        <RowLabel>{t('subagents.group', { n: nodes.length })}</RowLabel>
        {countsLabel(nodes, t) && <span className="min-w-0 truncate text-3 text-fg-3">{countsLabel(nodes, t)}</span>}
      </Row>
      {shown.map(n => <SubagentRow key={n.id} node={n} all={all} onInspect={onInspect} />)}
      {fold && terminal.length > 3 && (
        <Disclosure
          open={open}
          onToggle={next => { if (next) setOpen(true); }}
          lead={toolLine === 'text' ? undefined : <Check className="size-icon" strokeWidth={1.5} />}
          body={<div className="flex flex-col gap-0.5">{terminal.slice(0, -3).map(n => <SubagentRow key={n.id} node={n} all={all} onInspect={onInspect} />)}</div>}
        >
          <span className="text-fg-3">{t('subagents.moreCompleted', { n: Math.max(0, terminal.length - 3) })}</span>
        </Disclosure>
      )}
    </div>
  );
}

// Title (or role) on the first line, live activity / waiting / result excerpt on the second
function SubagentRow({ node, all, onInspect }: { node: SubagentSummary; all: SubagentSummary[]; onInspect: GroupProps['onInspect'] }) {
  const { toolLine } = useAppearance();
  const title = subagentTitle(node, t);
  const waiting = isWaiting(node);
  const running = node.state === 'running';
  const descendants = descendantCount(node.id, all);
  const elapsed = useElapsed(node);
  return (
    <button
      type="button"
      title={node.task?.slice(0, 200)}
      onClick={() => onInspect(node.id, 'session')}
      className={cn(
        '-mx-hit flex min-h-row w-full min-w-0 items-center gap-gap rounded-md px-hit text-left select-none transition-colors',
        'hover:bg-hover focus-visible:bg-hover',
      )}
    >
      {toolLine !== 'text' && (
        <span className="flex h-[var(--text-2-lh)] w-lead shrink-0 items-center justify-center self-start text-fg-3">{stateIcon(node)}</span>
      )}
      <span className="flex min-w-0 flex-1 flex-col">
        <span className="flex min-w-0 items-baseline gap-2">
          <span className="min-w-0 truncate text-2 text-fg-1">{title}</span>
          {node.role !== undefined && node.role !== title && <span className="min-w-0 max-w-project truncate text-3 text-fg-3 [flex-shrink:9]">{node.role}</span>}
          {descendants > 0 && <span className="shrink-0 text-3 text-fg-3">· {t('subagents.descendants', { n: descendants })}</span>}
        </span>
        <span className="truncate text-3 text-fg-3"><Shimmer active={running && !waiting}>{secondLine(node, t)}</Shimmer></span>
      </span>
      <span className="ml-auto flex shrink-0 items-center gap-2 self-center text-3 text-fg-3 tabular-nums">
        <span>{elapsed}</span>
        <ChevronRight className="size-3" strokeWidth={1.75} />
      </span>
    </button>
  );
}
