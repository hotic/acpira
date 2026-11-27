import { Metrics, WebMetrics, WebType } from '../design/Tokens';
import { Block, PlanEntry, RetryLog, ToolContent } from '../wire/Blocks';
import {
  commandDuration, diffStatOf, editEntries, editSpan, fileName, groupNames, isImageResult, namesLabel, splitHit,
  toolFiles, toolIcon, toolVerb, tp, visibleToolContents
} from '../logic/Folding';
import { t } from '../store/L10n';
import { Lucide } from './Glyphs';
import { AcpRow, Disclosure, RailBody, RowSpec } from './Rows';
import { CodeBody, CodeFrame, DiffBlockView, shellCommand, TerminalOutput, ToolOutput } from './ToolOutput';

// chat/ToolCall.tsx, ToolGroup.tsx, Terminal.tsx, RetryRow.tsx and the DiffBlock of CodeBlock.tsx, as apps/ios
// Transcript/ToolViews.swift and apps/android ui/ToolViews.kt port them: every process row leads with its kind's icon,
// and an open row draws its rail from that icon. Fold choices live in `folds` under `<scope>:<what>:<id>`.

function running(b: Block): boolean {
  return b.status === 'pending' || b.status === 'in_progress';
}

/** An async task's state on its row: done in the ok tone, failed in danger */
function taskKey(state: string): string {
  switch (state) {
    case 'paused':
    case 'completed':
    case 'failed':
    case 'stopped':
      return `asyncTask.${state}`;
    default:
      return 'asyncTask.running';
  }
}

function taskInk(state: string): ResourceColor {
  return state === 'failed' ? $r('app.color.danger') : state === 'completed' ? $r('app.color.ok') : $r('app.color.fg3');
}

/**
 * The tool row (ToolCall.tsx): the kind's icon, the verb (shimmering while it runs), then the command or target with an
 * edit's line span. Commands end with ✓ / ✗ and their run time; other rows show their diff stat; a background task its
 * state
 */
export function toolSpec(b: Block, rowH: number, now: () => number): RowSpec {
  const s = new RowSpec();
  s.icon = toolIcon(b);
  s.action = true;
  s.height = rowH;
  s.label = toolVerb(b);
  s.shimmer = running(b);
  s.now = now;
  const command = shellCommand(b);
  const files = toolFiles(b);
  if (command.length > 0) {
    s.target = command;
    s.targetMono = true;
  } else if (!(b.kind === 'read' && files.length > 0) && (b.target ?? '').length > 0) {
    s.target = b.target ?? '';
    s.targetMono = b.targetMono === true;
    if (b.kind === 'edit') {
      s.aside = editSpan([b]) ?? '';
    }
  }
  const task = b.asyncTask;
  if (b.kind === 'execute' && task !== undefined) {
    s.meta = t(taskKey(task.state));
    s.metaInk = taskInk(task.state);
  } else if (b.kind === 'execute') {
    s.outcomeSlot = true;
    s.outcome = b.status === 'completed' || b.status === 'failed' ? (b.status ?? '') : '';
    if (b.startedAt !== undefined && b.endedAt !== undefined) {
      s.duration = commandDuration(b.endedAt - b.startedAt);
    } else if (b.status === 'in_progress' && b.startedAt !== undefined) {
      s.runSince = b.startedAt;
    }
  } else {
    s.add = b.diffStat?.add ?? 0;
    s.del = b.diffStat?.del ?? 0;
    if (task !== undefined) {
      s.meta = t(taskKey(task.state));
      s.metaInk = taskInk(task.state);
    }
  }
  return s;
}

/** A read or search row's file: its name in mono fg-2 and the line as a faint note */
function fileSpec(hit: string, icon: string, rowH: number, mask: boolean): RowSpec {
  const split = splitHit(hit);
  const s = new RowSpec();
  s.icon = icon;
  s.action = true;
  s.height = rowH;
  s.mask = mask;
  s.target = split[0];
  s.targetMono = true;
  s.targetInk = $r('app.color.fg2');
  s.aside = split.length > 1 ? split[1] : '';
  return s;
}

/** Content identity for a ForEach key: what a rebuilt row would show differently */
export function blockSig(b: Block): string {
  const contents = b.contents ?? (b.content !== undefined ? [b.content] : []);
  let size = 0;
  for (const c of contents) {
    size += (c.text ?? '').length + (c.lines ?? []).length + (c.items ?? []).length;
  }
  return `${b.type}:${b.id ?? ''}:${b.status ?? ''}:${(b.markdown ?? '').length}:${(b.text ?? '').length}:` +
    `${b.streaming === true}:${size}:${b.endedAt ?? ''}:${b.resumedAt ?? ''}:${b.asyncTask?.state ?? ''}:${b.outcome ?? ''}:` +
    `${(b.entries ?? b.todoEntries ?? []).map((e: PlanEntry) => e.status).join('')}:${b.retry?.attempts.length ?? ''}` +
    `${b.retry?.status ?? ''}:${b.diffStat?.add ?? ''}:${b.diffStat?.del ?? ''}`;
}

// Plans

/** The entry list: a status dot in the lead slot, the title dimming once done, a faint priority glyph for high / low */
@ComponentV2
export struct PlanEntries {
  @Param entries: PlanEntry[] = [];

  spec(e: PlanEntry): RowSpec {
    const s = new RowSpec();
    s.lead = 'plan';
    s.planStatus = e.status;
    s.label = e.title;
    s.wrap = true;
    s.height = WebMetrics.rowDense;
    s.labelInk = e.status === 'completed' ? $r('app.color.fg2') : e.status === 'in_progress' ? $r('app.color.fg_strong') :
      $r('app.color.fg1');
    return s;
  }

  build() {
    Column() {
      ForEach(this.entries, (e: PlanEntry) => {
        Row() {
          AcpRow({ spec: this.spec(e) })
            .layoutWeight(1)
          if (e.priority === 'high' || e.priority === 'low') {
            Lucide({
              name: e.priority === 'high' ? 'chevronsUp' : 'chevronsDown', tint: $r('app.color.fg3'), extent: WebMetrics.chevron,
              weight: WebMetrics.strokeChevron
            })
              .margin({ left: Metrics.gap })
          }
        }
        .width('100%')
      }, (e: PlanEntry, i: number) => `${i}:${e.status}:${e.title}`)
    }
    .width('100%')
  }
}

/** Historical plans and to-do tool results: one disclosure row with the done count and a caret, the entries below */
@ComponentV2
export struct PlanDetails {
  @Param entries: PlanEntry[] = [];
  /** The head: its label and trailing parts; the count and caret are set here */
  @Param head: RowSpec = new RowSpec();
  @Param foldKey: string = '';

  spec(): RowSpec {
    const s = this.head.folded(false);
    s.icon = 'listTodo';
    s.lead = 'icon';
    s.action = true;
    s.count = `${this.entries.filter((e: PlanEntry) => e.status === 'completed').length}/${this.entries.length}`;
    s.chevron = 'caret';
    return s;
  }

  build() {
    Disclosure({ spec: this.spec(), foldKey: this.foldKey }) {
      PlanEntries({ entries: this.entries })
    }
  }
}

// One tool call

/** A tool call in the transcript, by what it carries (ToolCall.tsx ToolRows) */
@ComponentV2
export struct ToolCallView {
  @Param block: Block = { type: 'tool_call' };
  @Param grouped: boolean = false;
  @Param scope: string = '';
  @Param rowH: number = WebMetrics.chatRow;
  @Param now: () => number = () => Date.now();

  foldKeyOf(what: string): string {
    return `${this.scope}:${what}:${this.block.id ?? ''}`;
  }

  /** Which body the call draws: todos, skill, image, search, read, files, row, command, list, diff */
  shape(): string {
    const b = this.block;
    const files = toolFiles(b);
    const visible = visibleToolContents(b);
    if (b.todoEntries !== undefined) {
      return 'todos';
    }
    if (b.verbKey === 'verb.skill' && !(b.status === 'failed' && visible.length > 0)) {
      return 'row';
    }
    if (isImageResult(b)) {
      return 'image';
    }
    if (files.length > 0 && b.kind === 'search') {
      return 'search';
    }
    if (files.length === 1 && b.kind === 'read') {
      return 'read';
    }
    if (files.length > 0) {
      return 'files';
    }
    const command = shellCommand(b);
    if (b.kind === 'read' || b.kind === 'search' || (b.kind === 'edit' && visible.length === 0 && b.asyncTask === undefined) ||
      (this.grouped && b.content === undefined && b.contents === undefined && command.length === 0)) {
      return 'row';
    }
    if (b.kind === 'execute' || (visible.length > 0 && visible.every((c: ToolContent) => c.type === 'text'))) {
      return 'command';
    }
    if (b.content?.type === 'list') {
      return 'list';
    }
    return 'diff';
  }

  build() {
    if (this.shape() === 'todos') {
      PlanDetails({ entries: this.block.todoEntries ?? [], head: toolSpec(this.block, this.rowH, this.now), foldKey: this.foldKeyOf('plan') })
    } else if (this.shape() === 'row') {
      AcpRow({ spec: toolSpec(this.block, this.rowH, this.now) })
    } else if (this.shape() === 'image') {
      Column({ space: Metrics.gap }) {
        AcpRow({ spec: toolSpec(this.block, this.rowH, this.now) })
        if (!this.grouped) {
          ImageResults({ block: this.block })
        }
      }
      .width('100%')
    } else if (this.shape() === 'search') {
      Disclosure({ spec: toolSpec(this.block, this.rowH, this.now), foldKey: this.foldKeyOf('tool'), rail: 'rows', bodyTop: 0 }) {
        ResultList({ items: toolFiles(this.block), kind: this.block.kind ?? '', rowH: this.rowH })
      }
    } else if (this.shape() === 'read') {
      AcpRow({ spec: this.readSpec() })
    } else if (this.shape() === 'files') {
      Column() {
        AcpRow({ spec: toolSpec(this.block, this.rowH, this.now) })
        RailBody({ rows: true, top: 0 }) {
          ResultList({ items: toolFiles(this.block), kind: this.block.kind ?? '', rowH: this.rowH })
        }
      }
      .width('100%')
    } else if (this.shape() === 'command') {
      Disclosure({
        spec: toolSpec(this.block, this.rowH, this.now), foldKey: this.foldKeyOf('command'),
        defaultOpen: this.block.kind === 'execute' && !this.grouped && this.block.status === 'in_progress',
        bodyTop: WebMetrics.codeCardGap, after: WebMetrics.codeCardAfter
      }) {
        Column({ space: Metrics.gap }) {
          TaskMeta({ block: this.block })
          if (this.block.kind === 'execute') {
            TerminalOutput({ block: this.block })
          } else {
            ToolOutput({ text: visibleToolContents(this.block).map((c: ToolContent) => c.text ?? '').join('\n') })
          }
        }
        .width('100%')
      }
    } else if (this.shape() === 'list') {
      Disclosure({ spec: toolSpec(this.block, this.rowH, this.now), foldKey: this.foldKeyOf('tool'), rail: 'rows', bodyTop: 0 }) {
        ToolBody({ block: this.block, rowH: this.rowH })
      }
    } else {
      // A diff is a code card too: in the label column on the row's rail, spaced as the command card is
      Disclosure({
        spec: toolSpec(this.block, this.rowH, this.now), foldKey: this.foldKeyOf('tool'), bodyTop: WebMetrics.codeCardGap,
        after: WebMetrics.codeCardAfter
      }) {
        Column({ space: Metrics.gap }) {
          TaskMeta({ block: this.block })
          ToolBody({ block: this.block, rowH: this.rowH })
        }
        .width('100%')
      }
    }
  }

  /** A single read: the verb, then the file's name and line note */
  readSpec(): RowSpec {
    const s = toolSpec(this.block, this.rowH, this.now);
    const split = splitHit(toolFiles(this.block)[0]);
    s.target = split[0];
    s.targetMono = true;
    s.targetInk = $r('app.color.fg2');
    s.aside = split.length > 1 ? split[1] : '';
    return s;
  }
}

/** TaskMeta: the task's own latest summary under its row */
@ComponentV2
struct TaskMeta {
  @Param block: Block = { type: 'tool_call' };

  build() {
    if ((this.block.asyncTask?.summary ?? '').trim().length > 0) {
      Text(this.block.asyncTask?.summary)
        .fontSize(WebType.text3)
        .lineHeight(WebType.text3Line)
        .fontColor($r('app.color.fg3'))
        .width('100%')
    }
  }
}

/** Diff / list / text / image bodies, stacked in wire order */
@ComponentV2
struct ToolBody {
  @Param block: Block = { type: 'tool_call' };
  @Param rowH: number = WebMetrics.chatRow;

  build() {
    Column({ space: Metrics.gap }) {
      ForEach(visibleToolContents(this.block), (c: ToolContent) => {
        if (c.type === 'diff') {
          DiffBlockView({ content: c, title: this.block.target ?? '' })
        } else if (c.type === 'list') {
          ResultList({ items: c.items ?? [], kind: this.block.kind ?? '', rowH: this.rowH })
        } else if (c.type === 'text') {
          ToolOutput({ text: c.text ?? '' })
        } else if (c.type === 'image') {
          AgentImage({ uri: c.uri ?? '', name: '' })
        }
      }, (c: ToolContent, i: number) => `${i}:${c.type}:${(c.text ?? '').length}:${(c.lines ?? []).length}:${(c.items ?? []).length}`)
    }
    .width('100%')
  }
}

// Result list

/** File / hit rows under a read or search: dense action rows, the name in mono fg-2 and the line as a faint note */
@ComponentV2
export struct ResultList {
  @Param items: string[] = [];
  @Param kind: string = '';
  @Param rowH: number = WebMetrics.chatRow;

  build() {
    Column() {
      ForEach(this.items, (item: string) => {
        AcpRow({ spec: fileSpec(item, this.kind === 'fetch' ? 'globe' : 'fileText', this.rowH, true) })
      }, (item: string, i: number) => `${i}:${item}`)
    }
    .width('100%')
  }
}

// Groups

/** A run of consecutive reads or edits under one head: the verb, the files it touched, an edit run's total change */
@ComponentV2
export struct ToolGroupView {
  @Param kind: string = 'read';
  @Param blocks: Block[] = [];
  @Param scope: string = '';
  @Param rowH: number = WebMetrics.chatRow;

  spec(): RowSpec {
    const current = this.blocks.filter(running).pop() ?? this.blocks[this.blocks.length - 1];
    const s = new RowSpec();
    s.icon = this.kind === 'read' ? 'fileText' : 'pencil';
    s.action = true;
    s.height = this.rowH;
    s.label = toolVerb(current);
    s.shimmer = running(current);
    const names = groupNames(this.blocks);
    if (names.length > 0) {
      s.target = namesLabel(names);
      s.targetMono = true;
    }
    if (this.kind === 'edit') {
      const stat = diffStatOf(this.blocks);
      s.add = stat?.add ?? 0;
      s.del = stat?.del ?? 0;
    }
    return s;
  }

  build() {
    Disclosure({ spec: this.spec(), foldKey: `${this.scope}:group:${this.kind}:${this.blocks[0]?.id ?? ''}`, rail: 'rows', bodyTop: 0 }) {
      Column() {
        if (this.kind === 'read') {
          ForEach(this.blocks, (b: Block) => {
            ReadEntry({ block: b, rowH: this.rowH })
          }, (b: Block) => blockSig(b))
        } else {
          ForEach(editEntries(this.blocks), (entry: Block[]) => {
            EditEntry({ blocks: entry, scope: this.scope, rowH: this.rowH })
          }, (entry: Block[]) => entry.map(blockSig).join('|'))
        }
      }
      .width('100%')
    }
  }
}

@ComponentV2
struct ReadEntry {
  @Param block: Block = { type: 'tool_call' };
  @Param rowH: number = WebMetrics.chatRow;

  spec(): RowSpec {
    const s = new RowSpec();
    s.icon = 'fileText';
    s.action = true;
    s.mask = true;
    s.height = this.rowH;
    s.target = this.block.target ?? toolVerb(this.block);
    s.targetMono = true;
    s.targetInk = $r('app.color.fg2');
    return s;
  }

  build() {
    if (toolFiles(this.block).length > 0) {
      ResultList({ items: toolFiles(this.block), kind: 'read', rowH: this.rowH })
    } else {
      AcpRow({ spec: this.spec() })
    }
  }
}

/** One file of an edit group: its changed lines and total change; the diffs open on a tap */
@ComponentV2
struct EditEntry {
  @Param blocks: Block[] = [];
  @Param scope: string = '';
  @Param rowH: number = WebMetrics.chatRow;

  diffs(): ToolContent[] {
    const out: ToolContent[] = [];
    for (const b of this.blocks) {
      for (const c of visibleToolContents(b)) {
        if (c.type === 'diff' || c.type === 'text') {
          out.push(c);
        }
      }
    }
    return out;
  }

  spec(): RowSpec {
    const s = new RowSpec();
    s.icon = 'pencil';
    s.action = true;
    s.mask = true;
    s.height = this.rowH;
    s.target = fileName(this.blocks[0]);
    s.targetMono = true;
    s.targetInk = $r('app.color.fg2');
    s.aside = editSpan(this.blocks) ?? '';
    const stat = diffStatOf(this.blocks);
    s.add = stat?.add ?? 0;
    s.del = stat?.del ?? 0;
    return s;
  }

  build() {
    if (this.diffs().length === 0) {
      AcpRow({ spec: this.spec() })
    } else {
      // No rail root of its own: the group's rail runs on through its icon, the diff a code card in the label column
      Disclosure({
        spec: this.spec(), foldKey: `${this.scope}:edit:${this.blocks[0]?.id ?? ''}`, rail: 'off',
        bodyTop: WebMetrics.codeCardGap, after: WebMetrics.codeCardGap
      }) {
        Column({ space: Metrics.gap }) {
          ForEach(this.diffs(), (c: ToolContent) => {
            if (c.type === 'diff') {
              DiffBlockView({ content: c, title: fileName(this.blocks[0]) })
            } else {
              ToolOutput({ text: c.text ?? '' })
            }
          }, (c: ToolContent, i: number) => `${i}:${c.type}:${(c.lines ?? []).length}:${(c.text ?? '').length}`)
        }
        .width('100%')
      }
    }
  }
}

// Retries

/** "Attempt 5/10, in 12s" while the next attempt waits, the bare attempt once it is under way */
function retryMeta(log: RetryLog, now: number): string {
  const last = log.attempts.length > 0 ? log.attempts[log.attempts.length - 1].attempt : undefined;
  const left = log.nextAt === undefined ? 0 : Math.ceil((log.nextAt - now) / 1000);
  if (last !== undefined && log.max !== undefined && left > 0) {
    return tp('retry.attemptIn', 'attempt', `${last}`, 'max', `${log.max}`, 'seconds', `${left}`);
  }
  if (last !== undefined && log.max !== undefined) {
    return tp('retry.attempt', 'attempt', `${last}`, 'max', `${log.max}`);
  }
  return left > 0 ? tp('retry.in', 'seconds', `${left}`) : '';
}

function clock(at: number): string {
  const d = new Date(at);
  const two = (n: number): string => `${n}`.padStart(2, '0');
  return `${two(d.getHours())}:${two(d.getMinutes())}:${two(d.getSeconds())}`;
}

/** Every attempt on its own line: when it failed and which attempt, then the error verbatim */
function retryText(log: RetryLog): string {
  const rows: string[] = [];
  for (const a of log.attempts) {
    const head = [clock(a.at)];
    if (a.attempt !== undefined && log.max !== undefined) {
      head.push(tp('retry.attempt', 'attempt', `${a.attempt}`, 'max', `${log.max}`));
    }
    const parts = a.text.split('\n');
    rows.push(`${head.join('  ')}  ${parts[0]}`);
    for (const p of parts.slice(1)) {
      rows.push(`  ${p}`);
    }
  }
  return rows.join('\n');
}

/**
 * RetryRow.tsx: one run of provider retries, drawn like a tool row. The verb says where the run stands (shimmering while
 * live), the trailing text counts the attempt and the wait, then the outcome glyph; the card holds every failed attempt
 */
@ComponentV2
export struct RetryRow {
  @Param noticeId: string = '';
  @Param log: RetryLog = { status: 'in_progress', attempts: [] };
  @Param scope: string = '';
  @Param rowH: number = WebMetrics.chatRow;
  @Param now: () => number = () => Date.now();

  spec(): RowSpec {
    const n = `${this.log.attempts.length}`;
    const live = this.log.status === 'in_progress';
    const s = new RowSpec();
    s.icon = this.log.reconnect === true ? 'unplug' : 'rotateCw';
    s.action = true;
    s.height = this.rowH;
    s.label = live ? t(this.log.reconnect === true ? 'retry.reconnecting' : 'retry.retrying') :
      this.log.status === 'completed' ? tp('retry.recovered', 'n', n) :
        this.log.status === 'failed' ? tp('retry.failed', 'n', n) : t('retry.stopped');
    s.shimmer = live;
    s.meta = live ? retryMeta(this.log, this.now()) : '';
    s.outcomeSlot = true;
    s.outcome = this.log.status === 'completed' || this.log.status === 'failed' ? this.log.status : '';
    return s;
  }

  body(): CodeBody {
    const b = new CodeBody();
    b.kind = 'plain';
    b.text = retryText(this.log);
    b.title = t('retry.log');
    b.copy = b.text;
    b.copyLabel = t('retry.copyLog');
    return b;
  }

  build() {
    Disclosure({ spec: this.spec(), foldKey: `${this.scope}:retry:${this.noticeId}`, bodyTop: WebMetrics.codeCardGap }) {
      CodeFrame({ body: this.body() })
    }
  }
}

// Media

/**
 * An agent image (MediaViews.swift AgentImageView): a web address loads in place; an engine blob or a file on the
 * computer needs the media layer, so its card holds the image glyph and its name until then
 */
@ComponentV2
export struct AgentImage {
  @Param uri: string = '';
  @Param name: string = '';
  @Local failed: boolean = false;

  build() {
    if (/^https?:/i.test(this.uri) && !this.failed) {
      Image(this.uri)
        .objectFit(ImageFit.Contain)
        .constraintSize({ maxHeight: WebMetrics.agentImage, maxWidth: '100%' })
        .borderRadius(Metrics.rLg)
        .onError(() => {
          this.failed = true;
        })
    } else {
      Column({ space: Metrics.s2 }) {
        Lucide({ name: 'image', tint: $r('app.color.fg3'), extent: Metrics.icon })
        if (this.name.length > 0) {
          Text(this.name)
            .fontSize(WebType.text3)
            .lineHeight(WebType.text3Line)
            .fontColor($r('app.color.fg3'))
            .maxLines(1)
            .textOverflow({ overflow: TextOverflow.Ellipsis })
            .ellipsisMode(EllipsisMode.CENTER)
        }
      }
      .justifyContent(FlexAlign.Center)
      .padding(Metrics.pad)
      .height(WebMetrics.agentImage)
      .width('100%')
      .backgroundColor($r('app.color.chip'))
      .borderRadius(Metrics.rLg)
    }
  }
}

/** A generated or shown image's results under its row (ImageResults): every image content, one card each */
@ComponentV2
export struct ImageResults {
  @Param block: Block = { type: 'tool_call' };

  build() {
    Column({ space: Metrics.gap }) {
      ForEach(visibleToolContents(this.block).filter((c: ToolContent) => c.type === 'image'), (c: ToolContent) => {
        AgentImage({ uri: c.uri ?? '', name: (c.uri ?? '').split('/').pop() ?? '' })
      }, (c: ToolContent, i: number) => `${i}:${c.uri ?? c.blob ?? ''}`)
      if (!visibleToolContents(this.block).some((c: ToolContent) => c.type === 'image')) {
        AgentImage({ uri: '', name: this.block.target ?? '' })
      }
    }
    .width('100%')
  }
}
