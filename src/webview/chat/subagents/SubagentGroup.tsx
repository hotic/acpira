import { useMemo, useState } from 'react';
import { Check, ChevronRight } from 'lucide-react';
import type { SubagentSummary } from '@shared/subagents';
import { t } from '../../i18n';
import { Disclosure } from '../../ui/Disclosure';
import { Row } from '../../ui/Row';
import { stateIcon } from './icons';
import { descendantCount, partitionRows, rootRows, secondLine, stateLabel, subagentTitle } from './subagentState';

interface GroupProps {
  // Every node anchored to this turn, including descendants counted in the accessible label.
  nodes: SubagentSummary[];
  // The whole session's nodes, for descendant counts and breadcrumbs
  all: SubagentSummary[];
  onInspect: (id: string) => void;
}

// Delegations stay inline; the session-wide graph is reached from the composer.
export function SubagentGroup({ nodes, all, onInspect }: GroupProps) {
  const rows = useMemo(() => rootRows(nodes), [nodes]);
  // Past five children the group folds its older finished rows; the disclosure stays open once opened
  const [open, setOpen] = useState(false);
  const { shown, hidden } = partitionRows(rows);
  if (!nodes.length || !rows.length) return null;
  return (
    <div className="flex flex-col gap-1.5" aria-label={t('subagents.group', { n: nodes.length })}>
      {shown.map(n => <SubagentRow key={n.id} node={n} all={all} onInspect={onInspect} />)}
      {hidden.length > 0 && (
        <Disclosure
          open={open}
          onToggle={next => { if (next) setOpen(true); }}
          lead={<Check className="size-icon" strokeWidth={1.5} />}
          body={<div className="flex flex-col gap-1.5">{hidden.map(n => <SubagentRow key={n.id} node={n} all={all} onInspect={onInspect} />)}</div>}
        >
          <span className="text-fg-3">{t('subagents.moreCompleted', { n: hidden.length })}</span>
        </Disclosure>
      )}
    </div>
  );
}

// Title (or role) on the first line, live activity / waiting / result excerpt on the second. The shared Row
// holds lead / trailing / hover; items-start + lead-top keeps the state icon on the first of the two lines.
function SubagentRow({ node, all, onInspect }: { node: SubagentSummary; all: SubagentSummary[]; onInspect: GroupProps['onInspect'] }) {
  const title = subagentTitle(node, t);
  const descendants = descendantCount(node.id, all);
  return (
    <Row
      as="button"
      interactive
      className="group/subagent w-full items-start lead-top py-[9px]"
      aria-label={`${title} · ${stateLabel(node, t)} · ${secondLine(node, t)}`}
      title={node.task?.slice(0, 200)}
      onClick={() => onInspect(node.id)}
      lead={stateIcon(node)}
      trailing={<ChevronRight className="size-icon opacity-0 transition-opacity group-hover/subagent:opacity-100 group-focus-visible/subagent:opacity-100" strokeWidth={1.5} />}
    >
      <span className="flex min-w-0 flex-1 flex-col gap-1">
        <span className="flex min-w-0 items-baseline gap-2">
          <span className="min-w-0 truncate text-2 font-medium text-fg-1">{title}</span>
          {node.role !== undefined && node.role !== title && <span className="min-w-0 max-w-project truncate text-3 text-fg-3 [flex-shrink:9]">{node.role}</span>}
          {descendants > 0 && <span className="shrink-0 text-3 text-fg-3">· {t('subagents.descendants', { n: descendants })}</span>}
        </span>
        <span className="truncate text-3 text-fg-3">{secondLine(node, t)}</span>
      </span>
    </Row>
  );
}
