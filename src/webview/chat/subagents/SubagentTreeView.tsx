import type { ReactNode } from 'react';
import { Network } from 'lucide-react';
import type { SubagentSummary } from '@shared/subagents';
import { t } from '../../i18n';
import { cn } from '../../ui/cn';
import { Row, RowLabel } from '../../ui/Row';
import { stateIcon } from './icons';
import { flattenTree, subagentTitle } from './subagentState';
import { useElapsed } from './useElapsed';

type Flat = ReturnType<typeof flattenTree>;

// The relations tab: the main session row on top, then the delegation tree as nested lists.
// A row click selects that child and jumps back to its session tab (the inspector wires both).
export function SubagentTreeView({ all, sessionTitle, selectedId, onSelect, onClose }: {
  all: SubagentSummary[];
  sessionTitle: string;
  selectedId: string;
  onSelect: (id: string) => void;
  onClose: () => void;
}) {
  const flat = flattenTree(all);
  return (
    <div className="flex flex-col gap-0.5 px-pad py-gap">
      <Row as="button" interactive onClick={onClose}
        lead={<Network className="size-icon" strokeWidth={1.5} />}>
        <RowLabel>{sessionTitle}</RowLabel>
        <span className="shrink-0 text-3 text-fg-3">{t('subagents.rootRole')}</span>
      </Row>
      <ul className="m-0 flex list-none flex-col gap-0.5 p-0">
        <TreeList flat={flat} depth={0} selectedId={selectedId} onSelect={onSelect} />
      </ul>
    </div>
  );
}

// flattenTree yields depth-first rows; siblings share a depth, deeper rows nest under the previous one
function TreeList({ flat, depth, selectedId, onSelect }: { flat: Flat; depth: number; selectedId: string; onSelect: (id: string) => void }) {
  const rows: ReactNode[] = [];
  for (let i = 0; i < flat.length; i++) {
    const it = flat[i]!;
    if (it.depth < depth) break;
    if (it.depth > depth) continue;
    const kids: Flat = [];
    let j = i + 1;
    while (j < flat.length && flat[j]!.depth > depth) kids.push(flat[j++]!);
    i = j - 1;
    rows.push(
      <li key={it.node.id}>
        <TreeRow node={it.node} selected={it.node.id === selectedId} onSelect={onSelect} />
        {kids.length > 0 && (
          <ul className="m-0 flex list-none flex-col gap-0.5 p-0 pl-indent">
            <TreeList flat={kids} depth={depth + 1} selectedId={selectedId} onSelect={onSelect} />
          </ul>
        )}
      </li>,
    );
  }
  return <>{rows}</>;
}

// State icons stay on here: the tree is a navigation surface, so toolLine does not strip them
function TreeRow({ node, selected, onSelect }: { node: SubagentSummary; selected: boolean; onSelect: (id: string) => void }) {
  const elapsed = useElapsed(node);
  return (
    <Row as="button" interactive
      aria-current={selected ? 'true' : undefined}
      onClick={() => onSelect(node.id)}
      className={cn('w-full', selected && 'bg-hover')}
      lead={stateIcon(node)}
      trailing={<span>{elapsed}</span>}
    >
      <span className="min-w-0 truncate text-fg-1">{subagentTitle(node, t)}</span>
      {node.role !== undefined && <span className="min-w-0 max-w-project truncate text-3 text-fg-3 [flex-shrink:9]">{node.role}</span>}
    </Row>
  );
}
