import { Check, FileText, Globe, Square, X } from 'lucide-react';
import { memo, useContext, type ReactNode } from 'react';
import type { AsyncTaskInfo, AsyncTaskState, ToolCallBlock } from '@shared/transcript';
import type { MsgKey } from '@shared/i18n';
import { toolTodoEntries } from '@shared/todoTools';
import { isImageGenTool, isShowImageTool, toolTexts } from '@shared/imageTools';
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
import { TerminalOutput, ToolOutput } from './Terminal';
import { toolTarget, toolVerb } from './folding';
import { AsyncTaskStopContext, OpenToolFileContext } from './fileLinks';
import { fileReference, toolFiles, visibleToolContents } from './toolDetails';
import { useToolSeconds } from './useToolSeconds';
import { commandDuration } from './commandDuration';
import { useAutoFold } from './autoFold';
import { editSpan } from './processGroups';

export { OpenToolFileContext } from './fileLinks';

// One tool call = one expandable row, command execution included (the row names the program, the card below holds the full command and its output).
// Three modes: text only / with icon / icon + meta. No Orb while running: icon mode uses the same static icon as the completed state, with the verb shimmering.
// Bodies (diff / list) are not indented — they align with the row's left edge, like Codex; a command card is indented to the label
// column and hangs from the row icon on a connected rail
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
  // A shell command the agent handed over verbatim: the row shows it on one line (truncated to the row), the card wraps it whole.
  // Title-derived targets may be prose, and background wait / kill rows name another command, so both stay on the row
  const command = execute && block.targetMono && !block.verbKey && block.target?.trim() ? block.target : undefined;
  const files = toolFiles(block);
  const visibleContent = visibleToolContents(block);
  const Icon = toolIcon(block);
  const todos = toolTodoEntries(block);
  const fold = useAutoFold();

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
  const stat = block.diffStat && <DiffStat {...block.diffStat} />;
  const glyph = <>
    {block.status === 'completed' && <Check className="size-3 text-ok" strokeWidth={2} aria-hidden="true" />}
    {block.status === 'failed' && <X className="size-3 text-danger" strokeWidth={2} aria-hidden="true" />}
  </>;
  // Commands always end their row with the outcome and run time; an async task's state tag stands in for the glyph.
  // A live command ticks in whole seconds, a finished one reads its host timestamps, and no timer means no duration
  const duration = seconds === undefined ? undefined
    : block.startedAt !== undefined && block.endedAt !== undefined ? commandDuration(block.endedAt - block.startedAt)
      : commandDuration(seconds * 1000, true);
  const trailing = execute
    ? <>
        {toolLine === 'rich' && block.meta && <span>{block.meta}</span>}
        {taskTag}
        {taskStop}
        {!task && glyph}
        {duration && <span>{duration}</span>}
      </>
    : toolLine === 'rich'
      ? <>
          {stat || (block.meta && <span>{block.meta}</span>)}
          {taskTag}
          {taskStop}
          {glyph}
        </>
      : (stat || taskTag || taskStop ? <>{stat}{taskTag}{taskStop}</> : undefined);

  // An edit names where it landed, first to last changed line, the way a read names its range
  const span = block.kind === 'edit' ? editSpan([block]) : undefined;
  const label = <>
    <RowLabel shimmer={running}>{toolVerb(block)}</RowLabel>
    {command
      ? <RowTarget mono><span title={command}>{command}</span></RowTarget>
      : block.target && !(block.kind === 'read' && files.length) && (span
        ? <span className="flex min-w-0 items-baseline gap-1"><RowTarget mono={block.targetMono}>{toolTarget(block)}</RowTarget><Aside>{span}</Aside></span>
        : <RowTarget mono={block.targetMono}>{toolTarget(block)}</RowTarget>)}
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
  // Shown images (Acpira's show_image tool): a plain row naming the caption or files; the images sit outside the process fold
  // (CodexMessage) or right below the row, and the receipt text is for the model only
  if (isShowImageTool(block)) {
    const row = <Row tone="action" lead={lead} trailing={trailing}>{label}</Row>;
    return grouped ? row : <div className="flex flex-col gap-gap">{row}<GeneratedImages block={block} /></div>;
  }
  // Search hits open on demand; read references remain visible inside the process.
  if (files.length && block.kind === 'search') return (
    <Disclosure className="action-details" tone="action" lead={lead} trailing={trailing} indent={false} rail="rows" open={fold?.open} onToggle={fold?.onToggle}
      body={<ResultList items={files} kind={block.kind} />}>
      {label}
    </Disclosure>
  );
  // A read of one file is one row: the verb, then the file reference with its range
  if (files.length === 1 && block.kind === 'read') {
    const { main, aside } = splitHit(files[0]!);
    return <Row tone="action" lead={lead} trailing={trailing} title={files[0]}>
      {label}<FileRef hit={files[0]!} aside={aside}><RowTarget mono className="text-fg-2">{main}</RowTarget></FileRef>
    </Row>;
  }
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
  // A command card sits in the label column on the row's rail; the space after it stays outside the rail so the end dot meets the card.
  // A tool that returned only text gets the same card; diffs, lists and images stay full width
  if (execute || (visibleContent.length && visibleContent.every(item => item.type === 'text'))) return (
    <Disclosure className="action-details data-open:mb-command-after" bodyClassName="pt-gap-half" tone="action" lead={lead} trailing={trailing} defaultOpen={execute && !grouped && running}
      open={fold?.open} onToggle={fold?.onToggle}
      body={<><TaskMeta task={block.asyncTask} /><ToolBody block={block} items={visibleContent} command={command} /></>}>
      {label}
    </Disclosure>
  );
  return (
    <Disclosure className="action-details" tone="action" lead={lead} trailing={trailing} indent={false} rail={block.content?.type === 'list' ? 'rows' : false} open={fold?.open} onToggle={fold?.onToggle} body={<><TaskMeta task={block.asyncTask} /><ToolBody block={block} items={visibleContent} /></>}>
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

function ToolBody({ block, items, command }: { block: ToolCallBlock; items: ReturnType<typeof visibleToolContents>; command?: string }) {
  // Command output owns the execute body; an image it produced (screenshot tools) renders below the text
  if (block.kind === 'execute') {
    const images = block.contents?.filter((i): i is Extract<typeof i, { type: 'image' }> => i.type === 'image') ?? [];
    return <div className="flex flex-col gap-gap"><TerminalOutput block={block} command={command} />{images.map((i, n) => <AgentImage key={n} image={i} />)}</div>;
  }
  // Text-only results read as one output stream, like several text items of a command
  const texts = items.filter((i): i is Extract<typeof i, { type: 'text' }> => i.type === 'text');
  if (items.length && texts.length === items.length) return <ToolOutput block={block} text={texts.map(i => i.text).join('\n')} />;
  // Several content items in one update (e.g. two diffs with a receipt line between them) render stacked in wire order —
  // each diff keeps its own file path, `content` alone would only ever show the first
  if (items.length > 1) {
    return <div className="flex flex-col gap-gap">
      {items.map((item, i) => {
        if (item.type === 'diff') return <DiffBlock key={i} lines={item.lines} source={item.source} path={item.source?.path ?? block.locations?.[0]?.path ?? block.target} />;
        if (item.type === 'list') return <ResultList key={i} items={item.items} kind={block.kind} />;
        if (item.type === 'image') return <AgentImage key={i} image={item} />;
        return <ToolOutput key={i} block={block} text={item.text} />;
      })}
    </div>;
  }
  const c = items[0];
  if (!c || c.type === 'text') return null;
  if (c.type === 'diff') return <DiffBlock lines={c.lines} source={c.source} path={block.locations?.[0]?.path ?? block.target} />;
  if (c.type === 'list') return <ResultList items={c.items} kind={block.kind} />;
  return <AgentImage image={c} />;
}

// Result rows share the parent's connected icon rail, with a faint line/host suffix.
export function ResultList({ items, kind, rail = true }: { items: string[]; kind: ToolCallBlock['kind']; rail?: boolean }) {
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
  return <Row tone="action" dense lead={lead} title={hit}><FileRef hit={hit} aside={aside}>{children}</FileRef></Row>;
}

function FileRef({ hit, aside, children }: { hit: string; aside?: string; children: ReactNode }) {
  const openFile = useContext(OpenToolFileContext);
  const file = fileReference(hit);
  const asideEl = aside && <Aside>{aside}</Aside>;
  return openFile ? <button type="button" title={hit} className="group/ref flex min-w-0 max-w-full items-baseline gap-1 cursor-pointer text-left"
    onClick={() => openFile(file.path, file.line)}>
    {/* Only the file name underlines on hover / focus; the line range beside it stays plain */}
    <span className="flex min-w-0 group-hover/ref:underline group-focus-visible/ref:underline">{children}</span>{asideEl}
  </button>
    : <span className="flex min-w-0 items-baseline gap-1">{children}{asideEl}</span>;
}

// A faint note beside a target: a line range, a hit's line
export function Aside({ children }: { children: ReactNode }) {
  return <span className="shrink-0 whitespace-nowrap text-3 text-fg-3/70 tabular-nums">{children}</span>;
}

export function DiffStat({ add, del }: { add: number; del: number }) {
  return <span><span className="text-ok">+{add}</span> <span className="text-danger">−{del}</span></span>;
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
