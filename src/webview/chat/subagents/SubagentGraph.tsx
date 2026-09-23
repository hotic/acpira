import { useMemo, useRef } from 'react';
import { GitBranch, X } from 'lucide-react';
import type { SubagentSummary } from '@shared/subagents';
import { t } from '../../i18n';
import { IconButton } from '../../ui/Button';
import { Dialog } from '../../ui/Dialog';
import { stateIcon } from './icons';
import { useElapsed } from './useElapsed';
import { isWaiting, secondLine, stateLabel, subagentTitle } from './subagentState';

interface SubagentGraphProps {
  nodes: SubagentSummary[];
  // The main session's title names the root node
  sessionTitle?: string;
  open: boolean;
  onOpenChange: (open: boolean) => void;
  onInspect: (id: string) => void;
  // The child the inspector shows, marked in place
  selectedId?: string;
}

interface Branch {
  node: SubagentSummary;
  children: Branch[];
}

// Preserve parent links and announcement order; missing parents attach to the main session.
// Break one edge per cycle before rendering, including cycles with no natural root.
function graphRoots(nodes: SubagentSummary[]): Branch[] {
  const branches = new Map<string, Branch>();
  for (const node of nodes) {
    if (!branches.has(node.id)) branches.set(node.id, { node, children: [] });
  }
  const parents = new Map<string, string>();
  for (const { node } of branches.values()) {
    if (node.parentId !== undefined && branches.has(node.parentId)) parents.set(node.id, node.parentId);
  }
  const resolved = new Set<string>();
  for (const id of branches.keys()) {
    const path = new Set<string>();
    let current: string | undefined = id;
    while (current !== undefined && !resolved.has(current)) {
      if (path.has(current)) {
        parents.delete(current);
        break;
      }
      path.add(current);
      current = parents.get(current);
    }
    for (const visited of path) resolved.add(visited);
  }
  const roots: Branch[] = [];
  for (const [id, branch] of branches) {
    const parentId = parents.get(id);
    const parent = parentId === undefined ? undefined : branches.get(parentId);
    if (parent) parent.children.push(branch);
    else roots.push(branch);
  }
  return roots;
}

// Title, then role · elapsed (· model when the children differ); a third line only when the state is worth words:
// the live activity or pending decision, or how it ended short. A completed child says so with its icon alone.
function GraphBranch({ branch, onSelect, selectedId, showModel }: { branch: Branch; onSelect: (id: string) => void; selectedId?: string; showModel: boolean }) {
  const { node, children } = branch;
  const elapsed = useElapsed(node);
  const meta = [node.role !== undefined && node.role !== subagentTitle(node, t) ? node.role : undefined, elapsed, showModel ? node.model : undefined].filter(Boolean).join(' · ');
  const status = node.state === 'running' || isWaiting(node) ? secondLine(node, t) : node.state === 'completed' ? undefined : stateLabel(node, t);
  return <li>
    <button type="button" className="subagent-graph-node" data-node-id={node.id}
      aria-current={node.id === selectedId ? 'true' : undefined}
      aria-label={[subagentTitle(node, t), stateLabel(node, t), meta, status].filter(Boolean).join(' · ')}
      title={[node.task, node.model].filter(Boolean).join('\n\n')} onClick={() => onSelect(node.id)}>
      <span className="shrink-0 text-fg-3" aria-hidden="true">{stateIcon(node)}</span>
      <span className="flex min-w-0 flex-1 flex-col gap-1">
        <span className="truncate text-2 font-medium text-fg-1">{subagentTitle(node, t)}</span>
        <span className="truncate text-3 text-fg-3 tabular-nums">{meta}</span>
        {status && <span className="truncate text-3 text-fg-2">{status}</span>}
      </span>
    </button>
    {children.length > 0 && <ul>{children.map(child => <GraphBranch key={child.node.id} branch={child} onSelect={onSelect} selectedId={selectedId} showModel={showModel} />)}</ul>}
  </li>;
}

export function SubagentGraph({ nodes, sessionTitle, open, onOpenChange, onInspect, selectedId }: SubagentGraphProps) {
  const roots = useMemo(() => graphRoots(nodes), [nodes]);
  // A model line repeated on every card says nothing; it earns its place only when the children differ
  const showModel = useMemo(() => new Set(nodes.map(n => n.model)).size > 1, [nodes]);
  const closeRef = useRef<HTMLButtonElement>(null);
  const inspecting = useRef(false);
  const select = (id: string) => {
    inspecting.current = true;
    onOpenChange(false);
    onInspect(id);
  };
  return <Dialog.Root open={open} onOpenChange={onOpenChange} modal={true}>
    <Dialog.Portal>
      <Dialog.Popup initialFocus={() => { inspecting.current = false; return closeRef.current; }}
        // Ordinary dismissal restores the opener; inspecting hands focus to the real child session.
        finalFocus={() => !inspecting.current}
        onClick={event => { if (event.target === event.currentTarget) onOpenChange(false); }}
        className="absolute inset-0 z-40 flex items-center justify-center bg-scrim p-pad">
        <div className="flex max-h-full w-max min-w-0 max-w-full flex-col overflow-hidden rounded-lg border border-line bg-bg-1 shadow-pop">
          <header className="flex shrink-0 items-center gap-gap border-b border-line p-pad">
            <GitBranch className="size-icon shrink-0 text-fg-3" strokeWidth={1.5} aria-hidden="true" />
            <Dialog.Title className="m-0 min-w-0 flex-1 text-2 font-medium text-fg-1">{t('subagents.graph')}</Dialog.Title>
            <IconButton ref={closeRef} size="sm" title={t('common.close')} aria-label={t('common.close')}
              onClick={() => onOpenChange(false)}><X strokeWidth={1.5} /></IconButton>
          </header>
          <div className="subagent-graph-scroll scroll-thin" tabIndex={0} role="region" aria-label={t('subagents.graph')}>
            <ul className="subagent-graph-tree"><li>
              <div className="subagent-graph-node">
                <GitBranch className="size-icon shrink-0 text-fg-3" strokeWidth={1.5} aria-hidden="true" />
                <span className="flex min-w-0 flex-1 flex-col gap-1">
                  <span className="truncate text-2 font-medium text-fg-1" title={sessionTitle}>{sessionTitle || t('subagents.root')}</span>
                  <span className="text-3 text-fg-3">{t('subagents.entry', { n: nodes.length })}</span>
                </span>
              </div>
              {roots.length > 0 && <ul>{roots.map(branch => <GraphBranch key={branch.node.id} branch={branch} onSelect={select} selectedId={selectedId} showModel={showModel} />)}</ul>}
            </li></ul>
          </div>
        </div>
      </Dialog.Popup>
    </Dialog.Portal>
  </Dialog.Root>;
}
