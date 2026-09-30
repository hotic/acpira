import { useMemo, useState } from 'react';
import { Check } from 'lucide-react';
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
    <div className="flex flex-col gap-gap" aria-label={t('subagents.group', { n: nodes.length })}>
      {shown.map(n => <SubagentRow key={n.id} node={n} all={all} onInspect={onInspect} />)}
      {hidden.length > 0 && (
        <Disclosure
          open={open}
          onToggle={next => { if (next) setOpen(true); }}
          lead={<Check className="size-icon" strokeWidth={1.5} />}
          body={<div className="flex flex-col gap-gap">{hidden.map(n => <SubagentRow key={n.id} node={n} all={all} onInspect={onInspect} />)}</div>}
        >
          <span className="text-fg-3">{t('subagents.moreCompleted', { n: hidden.length })}</span>
        </Disclosure>
      )}
    </div>
  );
}

// An unframed row like the process folds ("Done ›"): state mark, title, then the live activity / waiting / result excerpt
// one step fainter. No chevron: the row opens the inspector rather than dropping down, and a chevron read as a fold.
// The activity gives up width first, the role before the title.
// The mark fills the lead slot (--subagent-mark) so the dotted bars read; the ✓ scales back to a plain Check.
function SubagentRow({ node, all, onInspect }: { node: SubagentSummary; all: SubagentSummary[]; onInspect: GroupProps['onInspect'] }) {
  const title = subagentTitle(node, t);
  const descendants = descendantCount(node.id, all);
  const line = secondLine(node, t);
  return (
    <Row
      as="button"
      interactive
      className="subagent-row [&>.row-lead]:size-subagent-mark [&>.row-lead>.subagent-mark]:size-subagent-mark"
      aria-label={`${title} · ${stateLabel(node, t)} · ${line}`}
      title={node.task?.slice(0, 200)}
      onClick={() => onInspect(node.id)}
      lead={stateIcon(node)}
    >
      <span className="min-w-0 truncate">{title}</span>
      {node.role !== undefined && node.role !== title && <span className="min-w-0 max-w-project truncate text-fg-3 [flex-shrink:4]">{node.role}</span>}
      <span className="min-w-0 truncate text-fg-3 [flex-shrink:9]">{line}</span>
      {descendants > 0 && <span className="shrink-0 text-fg-3">· {t('subagents.descendants', { n: descendants })}</span>}
    </Row>
  );
}
