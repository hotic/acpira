import { memo, useContext, useState, type ReactNode } from 'react';
import { MessagesSquare } from 'lucide-react';
import { splitSessionTarget } from '@shared/sessionLinks';
import type { ToolCallBlock } from '@shared/transcript';
import { useAppearance } from '../appearance';
import { Collapsible } from '../ui/Collapsible';
import { Disclosure, DisclosureRow } from '../ui/Disclosure';
import { EntranceOnce, Row, RowLabel, RowTarget } from '../ui/Row';
import { Shimmer } from '../ui/Shimmer';
import { t } from '../i18n';
import { TOOL_ICON } from './icons';
import { DiffBlock } from './CodeBlock';
import { ToolOutput } from './Terminal';
import { Aside, DiffStat, FileRef, ResultList } from './ToolCall';
import { sessionReadLabel, toolVerb } from './folding';
import { SessionTitleContext } from './fileLinks';
import { diffIndex, toolFiles, visibleToolContents } from './toolDetails';
import { useAutoFold } from './autoFold';
import { diffStatOf, editEntries, editReference, editSpan, fileName, filePath, groupNames, itemEntrance, sessionEntries, type GroupKind, type SessionEntry } from './processGroups';

const live = (block: ToolCallBlock) => block.status === 'pending' || block.status === 'in_progress';

// The files a head names: two by name, the rest as a count
function namesLabel(names: string[]): string {
  const sep = t('turns.nameSep');
  return names.length <= 2 ? names.join(sep) : t('turns.groupFiles', { names: names.slice(0, 2).join(sep), n: names.length });
}

// A run of consecutive reads, edits or session reads under one head: the verb, the files (sessions) it touched, an edit
// run's total change. A session group reads one session: the head names it and counts the turns read in the same text a
// read group counts its files with, one entry per turn.
// Entries hang on the head's rail; each takes its first call's entrance identity, so the row that was a single call
// before the group formed keeps still. The grouping array is rebuilt on every render, so compare its members
export const ToolGroup = memo(function ToolGroup({ kind, blocks }: { kind: GroupKind; blocks: ToolCallBlock[] }) {
  const { toolLine } = useAppearance();
  const fold = useAutoFold();
  const current = [...blocks].reverse().find(live) ?? blocks.at(-1)!;
  const lead = useLead(kind);
  const titleOf = useContext(SessionTitleContext);
  const session = kind === 'session' ? splitSessionTarget(blocks[0]!.target).id : undefined;
  const names = kind === 'session' ? (session ? [titleOf?.(session) ?? session] : []) : groupNames(blocks);
  const stat = kind === 'edit' ? diffStatOf(blocks) : undefined;
  const reads = kind === 'session' ? sessionEntries(blocks) : undefined;
  // The count holds only when every turn read is listed; an open or long range leaves it out
  const turns = reads && reads.entries.every(e => e.read.type === 'turns' || e.read.type === 'outline') ? reads.turns : 0;
  const body = <div className="tool-results flex flex-col">{kind === 'read'
    ? blocks.map(block => <EntranceOnce key={block.id} id={itemEntrance(block.id)}><ReadEntry block={block} /></EntranceOnce>)
    : kind === 'session'
      ? reads!.entries.map(entry => <EntranceOnce key={entry.key} id={`${itemEntrance(entry.blocks[0]!.id)}:${entry.key}`}><SessionReadEntry entry={entry} /></EntranceOnce>)
      : editEntries(blocks).map(entry => <EntranceOnce key={entry.key} id={itemEntrance(entry.blocks[0]!.id)}><EditEntry blocks={entry.blocks} /></EntranceOnce>)}
  </div>;
  // Entries follow the head directly, so the head and its first entry keep the entries' own rhythm
  return (
    <Disclosure className="action-details" bodyClassName="pt-0" tone="action" lead={lead} trailing={stat && <DiffStat {...stat} />}
      indent={false} rail={toolLine === 'text' ? false : 'rows'} open={fold?.open} onToggle={fold?.onToggle} body={body}>
      <RowLabel shimmer={live(current)}>{toolVerb(current)}</RowLabel>
      {turns > 0 ? <TurnsTarget title={names[0]!} n={turns} /> : names.length > 0 && <RowTarget mono={kind !== 'session'}>{namesLabel(names)}</RowTarget>}
    </Disclosure>
  );
}, (a, b) => a.kind === b.kind && a.blocks.length === b.blocks.length && a.blocks.every((block, i) => block === b.blocks[i]));

// A session group's head text, "<title> 中的 3 段对话" in the read group's colour and wording: only the title truncates,
// so a long title never hides the count
const TITLE_SLOT = '\u0000';

function TurnsTarget({ title, n }: { title: string; n: number }) {
  const [before = '', after = ''] = t('turns.groupTurns', { title: TITLE_SLOT, n }).split(TITLE_SLOT);
  return <span className="flex min-w-0 items-baseline">
    <RowTarget>{before}{title}</RowTarget>
    <RowTarget className="shrink-0 whitespace-pre">{after}</RowTarget>
  </span>;
}

function useLead(kind: GroupKind) {
  const { toolLine } = useAppearance();
  const Icon = kind === 'session' ? MessagesSquare : TOOL_ICON[kind];
  return toolLine === 'text' ? undefined : <Icon className="size-icon" strokeWidth={1.5} />;
}

// A read in a group: its file references, or the target while the call has none yet
function ReadEntry({ block }: { block: ToolCallBlock }) {
  const lead = useLead('read');
  const files = toolFiles(block);
  if (files.length) return <ResultList items={files} kind="read" rail={false} />;
  return <Row tone="action" dense lead={lead}><RowTarget mono className="text-fg-2"><Shimmer active={live(block)}>{block.target ?? toolVerb(block)}</Shimmer></RowTarget></Row>;
}

// One entry of a session group: a turn, the outline, or an open / long range; one format for every entry
function SessionReadEntry({ entry }: { entry: SessionEntry }) {
  const lead = useLead('session');
  return <Row tone="action" dense lead={lead}>
    <RowTarget className="text-fg-2"><Shimmer active={entry.blocks.some(live)}>{sessionReadLabel(entry.read)}</Shimmer></RowTarget>
  </Row>;
}

// One file of an edit group: its first to last changed line and total change; the diffs open only on a click
function EditEntry({ blocks }: { blocks: ToolCallBlock[] }) {
  const lead = useLead('edit');
  const stat = diffStatOf(blocks);
  const span = editSpan(blocks);
  const file = editReference(blocks);
  const items = blocks.flatMap(block => visibleToolContents(block).map((item, i, all) => ({ block, item, nth: diffIndex(all, i) })))
    .filter(({ item }) => item.type === 'diff' || item.type === 'text');
  const target = <RowTarget mono className="text-fg-2"><Shimmer active={blocks.some(live)}>{fileName(blocks[0]!)}</Shimmer></RowTarget>;
  const label = file ? <FileRef hit={file.path} line={file.line} aside={span}>{target}</FileRef> : <span className="flex min-w-0 items-baseline gap-1">
    {target}
    {span && <Aside>{span}</Aside>}
  </span>;
  const trailing = stat && <DiffStat {...stat} />;
  if (!items.length) return <Row tone="action" dense lead={lead} trailing={trailing} title={filePath(blocks[0]!)}>{label}</Row>;
  return <EntryFold independentAction={!!file} lead={lead} trailing={trailing} body={<div className="flex flex-col gap-gap">{items.map(({ block, item, nth }, i) => item.type === 'diff'
    ? <DiffBlock key={i} lines={item.lines} source={item.source} path={item.source?.path ?? filePath(block)} locate={{ toolCallId: block.id, nth }} />
    : item.type === 'text' && <ToolOutput key={i} block={block} text={item.text} />)}</div>}>{label}</EntryFold>;
}

// An expandable entry with no rail root of its own, so the group's rail keeps running through its icon
function EntryFold({ lead, trailing, body, children, independentAction }: { lead?: ReactNode; trailing?: ReactNode; body: ReactNode; children: ReactNode; independentAction?: boolean }) {
  const [open, setOpen] = useState(false);
  return <Collapsible.Root open={open} onOpenChange={setOpen} render={<div className="group flex min-w-0 flex-col" data-open={open || undefined} />}>
    <DisclosureRow independentAction={independentAction} tone="action" dense lead={lead} trailing={trailing}>{children}</DisclosureRow>
    <Collapsible.Panel className="-mx-hit [&>div]:px-hit"><div className="pt-1 pb-1.5">{body}</div></Collapsible.Panel>
  </Collapsible.Root>;
}
