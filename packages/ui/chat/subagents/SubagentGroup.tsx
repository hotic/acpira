import { useMemo } from 'react';
import type { SubagentSummary } from '@acpira/shared/subagents';
import { effortLabel } from '@acpira/shared/composerControls';
import { t } from '../../i18n';
import { ChipTag } from '../../ui/Button';
import { Row } from '../../ui/Row';
import { cn } from '../../ui/cn';
import { AgentMark } from '../AgentMark';
import { stateIcon } from './icons';
import { descendantCount, isWaiting, rootRows, secondLine, stateLabel, subagentTitle } from './subagentState';
import { useElapsed } from './useElapsed';
import { SidekickRelay, isSidekick } from './SidekickRelay';

interface GroupProps {
  // Every node anchored to this turn, including descendants counted in the accessible label.
  nodes: SubagentSummary[];
  // The whole session's nodes, for descendant counts and breadcrumbs
  all: SubagentSummary[];
  onInspect: (id: string) => void;
}

// Delegations stay inline; the session-wide graph is reached from the composer.
// Every child keeps its row, finished ones included: a workflow's rows are its progress, so folding the done ones
// away hid results and made the list jump as agents finished
export function SubagentGroup({ nodes, all, onInspect }: GroupProps) {
  const rows = useMemo(() => rootRows(nodes), [nodes]);
  if (!nodes.length || !rows.length) return null;
  return (
    // No gap between rows: each row carries its own vertical padding, so the hover fills abut like a menu list
    // and the text keeps the spacing it had when the gap sat between unpadded rows
    <div className="flex flex-col" aria-label={t('subagents.group', { n: nodes.length })}>
      {/* The lead's sidekick reads as a relay, never as a summoned helper's row */}
      {rows.map(n => isSidekick(n)
        ? <SidekickRelay key={n.id} node={n} onInspect={onInspect} />
        : <SubagentRow key={n.id} node={n} all={all} onInspect={onInspect} />)}
    </div>
  );
}

// An unframed row like the process folds ("Done ›"): state mark, title, then the live activity / waiting / result excerpt
// one step fainter. No chevron: the row opens the inspector rather than dropping down, and a chevron read as a fold.
// The activity gives up width first, the role before the title.
// The mark fills the lead slot (--subagent-mark) so the dotted bars read; the ✓ scales back to a plain Check.
// A summoned child (another CLI, `harness`) reads like any other row: its logo rides on the state mark's corner, the
// task names the row, the model follows faintly with its reasoning effort as the composer chip's badge; the persona name
// and CLI are in the tooltip
function SubagentRow({ node, all, onInspect }: { node: SubagentSummary; all: SubagentSummary[]; onInspect: GroupProps['onInspect'] }) {
  const title = subagentTitle(node, t);
  const descendants = descendantCount(node.id, all);
  const elapsed = useElapsed(node);
  const line = secondLine(node, t);
  const waiting = isWaiting(node);
  const status = waiting
    ? (node.permissions?.length ? t('host.awaitingApproval') : t('host.awaitingAnswers'))
    : stateLabel(node, t);
  const statusTone = waiting ? 'text-warn'
    : node.state === 'completed' ? 'text-ok'
      : node.state === 'failed' ? 'text-danger'
        : 'text-fg-3';
  // Keep the activity as a detail line. The raw command can be long, so it must never participate in the title row's width.
  const detail = line !== status ? line : undefined;
  const h = node.harness;
  const who = h && [node.role, node.model].filter(Boolean).join(' · ');
  return (
    <Row
      as="button"
      interactive
      // Vertical padding matches a single-line row's breathing room (row height minus one line), so the hover fill
      // does not hug the title and the detail line. A row without a detail line centres its mark like other rows
      className={cn('subagent-row py-gap-half [&>.row-lead]:size-subagent-mark [&_.subagent-mark]:size-subagent-mark', detail ? 'subagent-row-two items-start' : 'items-center')}
      aria-label={`${title} · ${who ? `${who} · ` : ''}${status} · ${elapsed}${detail ? ` · ${detail}` : ''}`}
      title={who || undefined}
      onClick={() => onInspect(node.id)}
      lead={h ? <span className="subagent-harness">{stateIcon(node)}<AgentMark id={h.agent} className="subagent-harness-mark" /></span> : stateIcon(node)}
    >
      <span className="flex min-w-0 flex-1 flex-col gap-gap">
        <span className="flex min-w-0 items-baseline gap-gap">
          <span className="min-w-0 truncate text-fg-1">{title}</span>
          {h
            ? <>
              {node.model && <span className="min-w-0 max-w-project truncate text-3 text-fg-3 [flex-shrink:4]">{node.model}</span>}
              {h.effort && <ChipTag small>{effortLabel({ id: h.effort, name: h.effort })}</ChipTag>}
              {h.round > 1 && <span className="shrink-0 text-3 text-fg-3">{t('subagents.round', { n: h.round })}</span>}
            </>
            : node.role !== undefined && node.role !== title && <span className="min-w-0 max-w-project truncate text-3 text-fg-3 [flex-shrink:4]">{node.role}</span>}
          {descendants > 0 && <span className="min-w-0 truncate text-3 text-fg-3 [flex-shrink:4]">{t('subagents.descendants', { n: descendants })}</span>}
          <span className="ml-auto flex shrink-0 items-baseline gap-gap text-3 tabular-nums">
            <span className={cn('whitespace-nowrap', statusTone)}>{status}</span>
            <span className="whitespace-nowrap text-fg-3">{elapsed}</span>
          </span>
        </span>
        {detail && <span className="block min-w-0 truncate text-3 text-fg-3" title={detail}>{detail}</span>}
      </span>
    </Row>
  );
}
