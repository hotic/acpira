import { Check, FileText, Globe, Square, X } from 'lucide-react';
import { memo, useContext, type ReactNode } from 'react';
import type { AsyncTaskInfo, AsyncTaskState, ToolCallBlock } from '@shared/transcript';
import type { MsgKey } from '@shared/i18n';
import { toolTodoEntries } from '@shared/todoTools';
import { isImageGenTool, toolTexts } from '@shared/imageTools';
import { useAppearance } from '../appearance';
import { Disclosure } from '../ui/Disclosure';
import { EntranceOnce, Row, RowLabel, RowTarget } from '../ui/Row';
import { cn } from '../ui/cn';
import { ConnectedRail } from '../ui/ConnectedRail';
import { IconButton } from '../ui/Button';
import { t } from '../i18n';
import { toolIcon } from './icons';
import { PlanDetails } from './Plan';
import { CodeSurface, DiffBlock } from './CodeBlock';
import { AgentImage } from './AgentImage';
import { GeneratedImages } from './GeneratedImage';
import { TerminalOutput } from './Terminal';
import { toolVerb } from './folding';
import { AsyncTaskStopContext, OpenToolFileContext } from './fileLinks';
import { fileReference, toolFiles, visibleToolContents } from './toolDetails';
import { useToolSeconds } from './useToolSeconds';

export { OpenToolFileContext } from './fileLinks';

// One tool call = one expandable row, command execution included (Codex-style: the command sits on the row, the output is a card below).
// Three modes: text only / with icon / icon + meta. No Orb while running: icon mode uses the same static icon as the completed state, with the verb shimmering.
// Bodies (diff / output / list) are not indented — they align with the row's left edge, like Codex
// Memoized on the block reference: a live turn re-renders on every chunk, and only the tool that changed should pay for it.
// The rows enter once per tool id: the branch below changes shape as the call progresses, and a remount must not replay it
export const ToolCall = memo(function ToolCall({ block, grouped = false }: { block: ToolCallBlock; grouped?: boolean }) {
  return <EntranceOnce id={`${block.id}:tool`}><ToolCallRows block={block} grouped={grouped} /></EntranceOnce>;
});

function ToolCallRows({ block, grouped }: { block: ToolCallBlock; grouped: boolean }) {
  const { toolLine } = useAppearance();
  // Announced calls can wait behind another tool; only execution shimmers.
  const running = block.status === 'in_progress' && block.observation !== 'unknown';
  const execute = block.kind === 'execute';
  const seconds = useToolSeconds(block);
  const files = toolFiles(block);
  const visibleContent = visibleToolContents(block);
  const Icon = toolIcon(block);
  const todos = toolTodoEntries(block);

  const lead = toolLine === 'text' ? undefined : <Icon className="size-icon" strokeWidth={1.5} />;

  // An AIR async task owns this row while it runs: the state tag replaces the generic status icon, and
  // a stoppable task gets a stop button that posts _session/async_task/stop through the host
  const task = block.asyncTask;
  const stopTask = useContext(AsyncTaskStopContext);
  const stoppable = task !== undefined && task.canStop && task.stopRequested !== true
    && (task.state === 'running' || task.state === 'paused') && stopTask !== undefined;
  const taskTag = task && (
    <span className={cn('inline-flex shrink-0 items-center gap-1 whitespace-nowrap',
      task.state === 'failed' ? 'text-danger' : task.state === 'completed' ? 'text-ok' : 'text-fg-3')}>
      {task.stopRequested ? t('asyncTask.stopping') : t(TASK_STATE_KEY[task.state])}
    </span>
  );
  const taskStop = stoppable && (
    <IconButton
      aria-label={t('asyncTask.stop')}
      title={t('asyncTask.stop')}
      className="-my-1 -mr-1"
      onClick={e => { e.stopPropagation(); stopTask(task.id); }}
    >
      <Square className="size-3" strokeWidth={1.5} />
    </IconButton>
  );

  // The diff stat is not decoration — every harness shows it — so it escapes the toolLine axis; 'rich' adds the rest of the meta
  const stat = block.diffStat && <span><span className="text-ok">+{block.diffStat.add}</span> <span className="text-danger">−{block.diffStat.del}</span></span>;
  const trailing = toolLine === 'rich'
    ? <>
        {stat || (block.meta && <span>{block.meta}</span>)}
        {taskTag}
        {taskStop}
        {block.status === 'completed' && <Check className="size-3 text-ok" strokeWidth={2} />}
        {block.status === 'failed' && <X className="size-3 text-danger" strokeWidth={2} />}
      </>
    : (stat || taskTag || taskStop ? <>{stat}{taskTag}{taskStop}</> : undefined);

  const label = <>
    <RowLabel className="tabular-nums" shimmer={running}>
      {seconds !== undefined && block.status === 'in_progress' ? t('tool.runningSeconds', { s: seconds })
        : seconds !== undefined && seconds > 0 && block.status === 'completed' ? t('tool.completedSeconds', { s: seconds })
          : toolVerb(block)}
    </RowLabel>
    {block.target && !(block.kind === 'read' && files.length) && <RowTarget mono={block.targetMono}>{block.target}</RowTarget>}
  </>;
  if (todos !== undefined) return <PlanDetails entries={todos} label={label} trailing={trailing} />;
  // Image generation: the row names the call and its text (codex-acp's revised prompt) opens on demand. In the process fold
  // the images render outside it (CodexMessage); elsewhere they sit right below the row
  if (isImageGenTool(block)) {
    const text = toolTexts(block);
    const row = text
      ? <Disclosure className="action-details" tone="action" lead={lead} trailing={trailing} indent={false} rail={false}
          body={<CodeSurface className="text-fg-2 whitespace-pre-wrap">{text}</CodeSurface>}>{label}</Disclosure>
      : <Row tone="action" lead={lead} trailing={trailing}>{label}</Row>;
    return grouped ? row : <div className="flex flex-col gap-gap">{row}<GeneratedImages block={block} /></div>;
  }
  // Search hits open on demand; read references remain visible inside the process.
  if (files.length && block.kind === 'search') return (
    <Disclosure className="action-details" tone="action" lead={lead} trailing={trailing} indent={false} rail="rows"
      body={<ResultList items={files} kind={block.kind} />}>
      {label}
    </Disclosure>
  );
  // Read and search responses expose references only, including failures and empty results.
  if (files.length) return (
    <ConnectedRail enabled={toolLine !== 'text'} endAtLastRow className="action-details flex flex-col">
      <Row tone="action" lead={lead} trailing={trailing}>{label}</Row>
      <EntranceOnce id={`${block.id}:files`}><ResultList items={files} kind={block.kind} /></EntranceOnce>
    </ConnectedRail>
  );
  // File-less responses must not fall through to the generic raw-output disclosure.
  if (block.kind === 'read' || block.kind === 'search' || (block.kind === 'edit' && !visibleContent.length && !task)
    || (grouped && !block.content)) return <Row tone="action" lead={lead} trailing={trailing}>{label}</Row>;

  // Opening a process fold reveals action rows; outputs only expand on an explicit click.
  return (
    <Disclosure className="action-details" tone="action" lead={lead} trailing={trailing} indent={false} rail={block.content?.type === 'list' ? 'rows' : false} defaultOpen={!grouped && execute && running} body={<><TaskMeta task={block.asyncTask} /><ToolBody block={block} items={visibleContent} /></>}>
      {label}
    </Disclosure>
  );
}

const TASK_STATE_KEY: Record<AsyncTaskState, MsgKey> = {
  running: 'asyncTask.running',
  paused: 'asyncTask.paused',
  completed: 'asyncTask.completed',
  failed: 'asyncTask.failed',
  stopped: 'asyncTask.stopped',
};

// An AIR async task's own report, under its tool row: latest summary, the tool it was last on,
// usage when the adapter reports it, and the output file as an ordinary file link
function TaskMeta({ task }: { task: AsyncTaskInfo | undefined }) {
  const openFile = useContext(OpenToolFileContext);
  if (!task) return null;
  const usage = [
    task.usage?.totalTokens !== undefined ? t('asyncTask.tokens', { n: task.usage.totalTokens }) : undefined,
    task.usage?.toolUses !== undefined ? t('asyncTask.toolUses', { n: task.usage.toolUses }) : undefined,
    task.usage?.durationMs !== undefined ? t('asyncTask.duration', { s: Math.round(task.usage.durationMs / 1000) }) : undefined,
  ].filter(Boolean).join(' · ');
  if (!task.summary && !task.lastToolName && !usage && !task.outputFilePath) return null;
  return (
    <div className="flex min-w-0 flex-col gap-1 text-3 text-fg-3">
      {task.summary && <span className="whitespace-pre-wrap [overflow-wrap:anywhere]">{task.summary}</span>}
      {task.lastToolName && <span>{task.lastToolName}</span>}
      {usage && <span>{usage}</span>}
      {task.outputFilePath && (openFile
        ? <button type="button" title={task.outputFilePath} className="min-w-0 cursor-pointer truncate text-left hover:underline focus-visible:underline"
            onClick={() => openFile(task.outputFilePath!)}>{task.outputFilePath}</button>
        : <span className="truncate" title={task.outputFilePath}>{task.outputFilePath}</span>)}
    </div>
  );
}

// Several ACP read calls form one visible list of file references.
// The grouping array is rebuilt on every render, so compare its members rather than the array itself.
export const ReadGroup = memo(function ReadGroup({ blocks }: { blocks: ToolCallBlock[] }) {
  const { toolLine } = useAppearance();
  const first = blocks[0]!;
  return <ConnectedRail enabled={toolLine !== 'text'} endAtLastRow className="action-details read-group flex flex-col">
    <EntranceOnce id={`${first.id}:tool`}>
      <Row tone="action" lead={toolLine === 'text' ? undefined : <FileText className="size-icon" strokeWidth={1.5} />}>
        <RowLabel>{toolVerb(first)}</RowLabel>
      </Row>
    </EntranceOnce>
    <div className="tool-results flex flex-col">
      {blocks.map(block => <EntranceOnce key={block.id} id={`${block.id}:files`}><ResultList items={toolFiles(block)} kind="read" rail={false} /></EntranceOnce>)}
    </div>
  </ConnectedRail>;
}, (a, b) => a.blocks.length === b.blocks.length && a.blocks.every((block, i) => block === b.blocks[i]));

function ToolBody({ block, items }: { block: ToolCallBlock; items: ReturnType<typeof visibleToolContents> }) {
  // Command output owns the execute body; an image it produced (screenshot tools) renders below the text
  if (block.kind === 'execute') {
    const images = block.contents?.filter((i): i is Extract<typeof i, { type: 'image' }> => i.type === 'image') ?? [];
    return <div className="flex flex-col gap-gap"><TerminalOutput block={block} />{images.map((i, n) => <AgentImage key={n} image={i} />)}</div>;
  }
  // Several content items in one update (e.g. two diffs with a receipt line between them) render stacked in wire order —
  // each diff keeps its own file path, `content` alone would only ever show the first
  if (items.length > 1) {
    return <div className="flex flex-col gap-gap">
      {items.map((item, i) => {
        if (item.type === 'diff') return <DiffBlock key={i} lines={item.lines} source={item.source} path={item.source?.path ?? block.locations?.[0]?.path ?? block.target} />;
        if (item.type === 'list') return <ResultList key={i} items={item.items} kind={block.kind} />;
        if (item.type === 'image') return <AgentImage key={i} image={item} />;
        return <CodeSurface key={i} className="text-fg-2 whitespace-pre">{item.text}</CodeSurface>;
      })}
    </div>;
  }
  const c = items[0];
  if (!c) return null;
  if (c.type === 'diff') return <DiffBlock lines={c.lines} source={c.source} path={block.locations?.[0]?.path ?? block.target} />;
  if (c.type === 'list') return <ResultList items={c.items} kind={block.kind} />;
  if (c.type === 'image') return <AgentImage image={c} />;
  return <CodeSurface className="text-fg-2 whitespace-pre">{c.text}</CodeSurface>;
}

// Result rows share the parent's connected icon rail, with a faint line/host suffix.
function ResultList({ items, kind, rail = true }: { items: string[]; kind: ToolCallBlock['kind']; rail?: boolean }) {
  const { toolLine } = useAppearance();
  const Icon = kind === 'fetch' ? Globe : FileText;
  return (
    <div className={cn('flex flex-col', rail && 'tool-results')}>
      {items.map(it => {
        const { main, aside } = splitHit(it);
        const lead = toolLine === 'text' ? undefined : <Icon className="size-icon" strokeWidth={1.5} />;
        const target = <RowTarget mono={kind !== 'fetch'} className="text-fg-2">{main}</RowTarget>;
        if (kind === 'read' || kind === 'search') {
          return <FileResultRow key={it} hit={it} lead={lead} aside={aside}>{target}</FileResultRow>;
        }
        return (
          <Row tone="action" key={it} dense lead={lead} trailing={aside} title={it}>
            {target}
          </Row>
        );
      })}
    </div>
  );
}

// File rows have one interaction: open the reference in the editor.
function FileResultRow({ hit, lead, aside, children }: { hit: string; lead: ReactNode; aside?: string; children: ReactNode }) {
  const openFile = useContext(OpenToolFileContext);
  const file = fileReference(hit);
  const asideEl = aside && <span className="shrink-0 whitespace-nowrap text-3 text-fg-3/70 tabular-nums">{aside}</span>;
  return <Row tone="action" dense lead={lead} title={hit}>
    {openFile ? <button type="button" title={hit} className="group/ref flex min-w-0 max-w-full items-baseline gap-1 cursor-pointer text-left"
      onClick={() => openFile(file.path, file.line)}>
      {/* Only the file name underlines on hover / focus; the line range beside it stays plain */}
      <span className="flex min-w-0 group-hover/ref:underline group-focus-visible/ref:underline">{children}</span>{asideEl}
    </button>
      : <span className="flex min-w-0 items-baseline gap-1">{children}{asideEl}</span>}
  </Row>;
}

function splitHit(hit: string): { main: string; aside?: string } {
  const line = /^(.+?):(\d+(?:[–-]\d+)?)(?::\d+)?$/.exec(hit);
  if (line) return { main: line[1]!.split(/[\\/]/).pop()!, aside: `L${line[2]}` };
  if (/^(?:\.{0,2}\/|[A-Za-z]:[\\/])/.test(hit)) return { main: hit.split(/[\\/]/).pop()! };
  try {
    const u = new URL(hit);
    return { main: u.pathname === '/' ? u.host : `${u.host}${u.pathname}`, aside: u.host };
  } catch { return { main: hit }; }
}
