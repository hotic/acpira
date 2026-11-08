import { AgentBlock } from '../wire/Wire';
import { asBlock, Block, DiffLine, DiffStat, PermissionChoice, ToolContent, TurnX } from '../wire/Blocks';
import { t } from '../store/L10n';

// The webview's pure transcript logic (chat/processGroups.ts, folding.ts, toolDetails.ts, turnOutcome.ts,
// planSections.ts, permissionOptions.ts) as apps/ios AcpiraUI/Logic/Folding.swift and apps/android logic/Folding.kt port
// it one to one, so a behaviour change there maps onto one place here.

/** t() with `{name}` params given as name, value pairs */
export function tp(key: string, ...pairs: string[]): string {
  const params = new Map<string, string>();
  for (let i = 0; i + 1 < pairs.length; i += 2) {
    params.set(pairs[i], pairs[i + 1]);
  }
  return t(key, params);
}

function isBlank(text: string | undefined): boolean {
  return (text ?? '').trim().length === 0;
}

/** A call whose observation was lost must not pass for running work (ToolCall.tsx): it shows settled */
export function observed(b: AgentBlock): Block {
  const block = asBlock(b);
  if (block.type === 'tool_call' && block.observation === 'unknown' && (block.status === 'pending' || block.status === 'in_progress')) {
    const copy: Block = JSON.parse(JSON.stringify(block)) as Block;
    copy.status = 'cancelled';
    return copy;
  }
  return block;
}

// Codex split

/** A turn's blocks split the way CodexMessage renders them */
export class CodexSplit {
  process: Block[] = [];
  reply: Block[] = [];
  permissions: Block[] = [];
  notices: Block[] = [];
  goalsLead: Block[] = [];
  goals: Block[] = [];
}

/** isAdvisory: a notice that only informs, with nothing to act on and no known transient cause */
export function isAdvisory(n: Block): boolean {
  return n.severity === 'warning' && n.category === 'unknown' && (n.actions ?? []).length === 0;
}

function isAction(b: Block): boolean {
  if (b.type === 'goal' || b.type === 'text' || b.type === 'permission' || b.type === 'notice') {
    return false;
  }
  if (b.type === 'question') {
    return b.outcome !== undefined;
  }
  return true;
}

/**
 * splitCodexBlocks: the text after the last action stays outside the process fold; explicit phases win. Goal milestones
 * are status rows: the leading ones open the turn, the ones after the last action follow the reply
 */
export function splitCodexBlocks(blocks: Block[]): CodexSplit {
  const out = new CodexSplit();
  const content: Block[] = [];
  const firstOther = blocks.findIndex((b: Block) => b.type !== 'goal');
  let lastAction = -1;
  blocks.forEach((b: Block, i: number) => {
    if (isAction(b)) {
      lastAction = i;
    }
  });
  blocks.forEach((b: Block, i: number) => {
    if (b.type === 'permission') {
      out.permissions.push(b);
    } else if (b.type === 'notice' && !isAdvisory(b) && b.retry === undefined) {
      // Failure notices are status rows lifted under the fold; an advisory and a retry row keep their place
      out.notices.push(b);
    } else if (b.type === 'question' && b.outcome === undefined) {
      // The open card waits in the dock
    } else if (b.type === 'goal' && (firstOther < 0 || i < firstOther)) {
      out.goalsLead.push(b);
    } else if (b.type === 'goal' && (lastAction < 0 || i > lastAction)) {
      out.goals.push(b);
    } else {
      content.push(b);
    }
  });
  const foldable = content.some((b: Block) => b.type === 'tool_call');
  let end = content.length;
  // Thinking and to-do bookkeeping between closing paragraphs join the fold in a foldable turn
  while (end > 0) {
    const last = content[end - 1];
    if (last.type === 'text' || (foldable && (last.type === 'thought' || last.type === 'plan'))) {
      end -= 1;
    } else {
      break;
    }
  }
  content.forEach((b: Block, i: number) => {
    if (b.type === 'text' && (b.phase === 'final' || (i >= end && b.phase !== 'commentary'))) {
      out.reply.push(b);
    } else {
      out.process.push(b);
    }
  });
  return out;
}

// Plan sections

/** One stretch of a turn: the blocks before a saved plan (the plan's card closes it), or the continuation after it */
export class PlanSection {
  key: string = '';
  blocks: Block[] = [];
  plan: Block | undefined = undefined;
}

/** Work, not the status rows and cards that stand beside a reply */
function startsStretch(b: Block): boolean {
  if (b.type === 'permission' || b.type === 'notice' || b.type === 'goal' || b.type === 'compaction' || b.type === 'plan_document') {
    return false;
  }
  return b.type !== 'question' || b.outcome !== undefined;
}

/**
 * splitPlanSections: a plan is a chronological boundary outside both neighbouring process folds; the trailing section
 * stays even while empty. A question's own tool row is dropped (its card holds the request and the answer) unless it
 * failed, and so are thoughts with no text. A delegation row stays once per node in `placed`, the others drop
 */
export function splitPlanSections(blocks: Block[], placed: Set<string>): PlanSection[] {
  const first = new PlanSection();
  first.key = 'start';
  const sections: PlanSection[] = [first];
  const seen = new Set<string>();
  const questionTools = new Set<string>();
  blocks.forEach((b: Block) => {
    if (b.type === 'question' && b.toolCallId !== undefined) {
      questionTools.add(b.toolCallId);
    }
  });
  let replied = false;
  blocks.forEach((b: Block, i: number) => {
    if (b.type === 'tool_call' && b.status !== 'failed' && questionTools.has(b.id ?? '')) {
      return;
    }
    const delegated = b.type === 'tool_call' ? b.subagentId : undefined;
    if (delegated !== undefined && (!placed.has(delegated) || seen.has(delegated))) {
      return;
    }
    if (b.type === 'thought' && isBlank(b.text)) {
      return;
    }
    if (b.type === 'text' && b.phase === 'final') {
      replied = true;
    } else if (replied && startsStretch(b)) {
      replied = false;
      const next = new PlanSection();
      next.key = `resume:${i}`;
      sections.push(next);
    }
    if (b.type === 'plan_document') {
      replied = false;
      sections[sections.length - 1].plan = b;
      const next = new PlanSection();
      next.key = b.id ?? `plan:${i}`;
      sections.push(next);
    } else {
      if (delegated !== undefined) {
        seen.add(delegated);
      }
      sections[sections.length - 1].blocks.push(b);
    }
  });
  return sections;
}

// Process groups

/** One item of a turn's process list: a block on its own, or a run of consecutive reads / edits under one head */
export class ProcessItem {
  id: string = '';
  /** '' for a single block, else the run's kind (read, edit) */
  group: string = '';
  block: Block | undefined = undefined;
  blocks: Block[] = [];
}

function groupKind(b: Block): string {
  if (b.type !== 'tool_call' || b.status === 'failed' || b.status === 'cancelled' || b.asyncTask !== undefined ||
    b.subagentId !== undefined) {
    return '';
  }
  if (b.kind === 'read') {
    return b.status !== 'completed' || (b.locations ?? []).length > 0 || toolFiles(b).length > 0 ? 'read' : '';
  }
  return b.kind === 'edit' ? 'edit' : '';
}

export function blockItemId(b: Block, at: number): string {
  if (b.type === 'tool_call') {
    return b.id ?? `at:${at}`;
  }
  if (b.type === 'thought' && b.startedAt !== undefined) {
    return `thought:${Math.floor(b.startedAt)}`;
  }
  return b.id ?? `at:${at}`;
}

function single(b: Block, at: number): ProcessItem {
  const item = new ProcessItem();
  item.id = blockItemId(b, at);
  item.block = b;
  return item;
}

/** groupProcess: consecutive reads (with files) or edits share one head */
export function groupProcess(blocks: Block[]): ProcessItem[] {
  const out: ProcessItem[] = [];
  let run: Block[] = [];
  let runKind = '';
  let runAt = 0;
  const flush = () => {
    if (run.length === 1) {
      out.push(single(run[0], runAt));
    } else if (run.length > 1) {
      const item = new ProcessItem();
      item.id = run[0].id ?? `at:${runAt}`;
      item.group = runKind;
      item.blocks = run;
      out.push(item);
    }
    run = [];
  };
  blocks.forEach((b: Block, at: number) => {
    const kind = groupKind(b);
    if (kind.length > 0 && kind === runKind) {
      run.push(b);
      return;
    }
    flush();
    runKind = kind;
    if (kind.length > 0) {
      run.push(b);
      runAt = at;
    } else {
      out.push(single(b, at));
    }
  });
  flush();
  return out;
}

/** imageTools.ts isImageResultBlock: generated and shown images, and widgets, are results for the reader */
export function isImageResult(b: Block): boolean {
  return b.verbKey === 'verb.imagegen' || b.verbKey === 'verb.showImage' || b.verbKey === 'verb.showVideo' ||
    b.verbKey === 'verb.showWidget';
}

/**
 * The turn's process cut where something has to stay in view while the parts around it fold: a steered prompt, the
 * subagent rows of a delegation, an image result. Runs of items fold. A part is keyed by its first item, so content
 * arriving later leaves the earlier parts in place
 */
export class FoldPart {
  id: string = '';
  /** items, steer, media, subagents */
  kind: string = 'items';
  items: ProcessItem[] = [];
  block: Block | undefined = undefined;
  ids: string[] = [];
}

export function processParts(items: ProcessItem[]): FoldPart[] {
  const out: FoldPart[] = [];
  let rows: ProcessItem[] = [];
  let children: string[] = [];
  let runId = '';
  const close = () => {
    if (rows.length > 0) {
      const part = new FoldPart();
      part.id = runId;
      part.items = rows;
      out.push(part);
      rows = [];
    }
    if (children.length > 0) {
      const part = new FoldPart();
      part.id = runId;
      part.kind = 'subagents';
      part.ids = children;
      out.push(part);
      children = [];
    }
  };
  for (const item of items) {
    const b = item.block;
    if (b !== undefined && b.type === 'steer') {
      close();
      const part = new FoldPart();
      part.id = item.id;
      part.kind = 'steer';
      part.block = b;
      out.push(part);
    } else if (b !== undefined && b.type === 'tool_call' && b.subagentId !== undefined) {
      if (rows.length > 0) {
        close();
      }
      if (children.length === 0) {
        runId = item.id;
      }
      children.push(b.subagentId);
    } else {
      if (children.length > 0) {
        close();
      }
      if (rows.length === 0) {
        runId = item.id;
      }
      rows.push(item);
      if (b !== undefined && b.type === 'tool_call' && isImageResult(b)) {
        // The tool's row folds with the process, but its result keeps a stable place and is never clipped
        close();
        const part = new FoldPart();
        part.id = `media:${item.id}`;
        part.kind = 'media';
        part.block = b;
        out.push(part);
      }
    }
  }
  close();
  return out;
}

// Tool details

const editReceipts: string[] = ['Edit applied successfully.', 'Wrote file successfully.'];

/** visibleToolContents: a completed edit's success receipt is dropped, everything else stays */
export function visibleToolContents(b: Block): ToolContent[] {
  const items: ToolContent[] = b.contents ?? (b.content !== undefined ? [b.content] : []);
  if (b.kind !== 'edit' || b.status !== 'completed') {
    return items;
  }
  return items.filter((c: ToolContent) => !(c.type === 'text' && editReceipts.includes((c.text ?? '').trim())));
}

const explicitPath = /^(?:\.{0,2}\/|[A-Za-z]:[\\/])/;
const barePath = /^[^\s:]+[.][\w-]+$/;
const hitLine = /^(.+?):(\d+)(?::\d+)?(?::.*)?$/;

function fileHit(raw: string): string | undefined {
  const text = raw.startsWith('file://') ? raw.slice('file://'.length) : raw;
  const hit = text.match(hitLine);
  const path = hit !== null ? hit[1] : text;
  if (!explicitPath.test(path) && !barePath.test(path)) {
    return undefined;
  }
  return hit !== null ? `${path}:${hit[2]}` : path;
}

/** toolFiles: the file references a read or search names */
export function toolFiles(b: Block): string[] {
  if (b.kind !== 'read' && b.kind !== 'search') {
    return [];
  }
  const files: string[] = [];
  for (const l of b.locations ?? []) {
    const r = b.readRange;
    if (b.kind === 'read' && r !== undefined && r.path === l.path) {
      files.push(`${l.path}:${r.start}` + (r.end !== undefined && r.end !== r.start ? `–${r.end}` : ''));
    } else if (l.line === undefined || (b.kind === 'read' && l.line <= 1)) {
      files.push(l.path);
    } else {
      files.push(`${l.path}:${l.line}`);
    }
  }
  if (b.kind === 'search') {
    const c = b.content;
    const lines: string[] = c === undefined ? [] : c.type === 'list' ? (c.items ?? []) : c.type === 'text' ? (c.text ?? '').split('\n') : [];
    for (const line of lines) {
      const hit = fileHit(line.trim());
      if (hit !== undefined) {
        files.push(hit);
      }
    }
  }
  if (files.length === 0 && b.kind === 'read' && b.target !== undefined) {
    const hit = fileHit(b.target);
    if (hit !== undefined) {
      files.push(hit);
    }
  }
  return files.filter((f: string, i: number) => files.indexOf(f) === i);
}

export function lastComponent(path: string): string {
  const parts = path.split(/[/\\]/).filter((p: string) => p.length > 0);
  return parts.length > 0 ? parts[parts.length - 1] : path;
}

export function filePath(b: Block): string {
  const locations = b.locations ?? [];
  return locations.length > 0 ? locations[0].path : (b.target ?? '');
}

export function fileName(b: Block): string {
  const name = lastComponent(filePath(b));
  return name.length > 0 ? name : (b.target ?? '');
}

/** groupNames: the files a group names in its head, first-seen order */
export function groupNames(blocks: Block[]): string[] {
  const names = blocks.map(fileName).filter((n: string) => n.length > 0);
  return names.filter((n: string, i: number) => names.indexOf(n) === i);
}

/** namesLabel: two names, the rest as a count */
export function namesLabel(names: string[]): string {
  const sep = t('turns.nameSep');
  if (names.length <= 2) {
    return names.join(sep);
  }
  return tp('turns.groupFiles', 'names', names.slice(0, 2).join(sep), 'n', `${names.length}`);
}

/** toolVerb: 已读取 / 正在运行 / 运行失败 …, rendered from the semantic kind */
export function toolVerb(b: Block): string {
  if (b.observation === 'unknown') {
    return t('observation.unknownTool');
  }
  let key = 'fold.done';
  switch (b.status) {
    case 'pending':
      key = 'fold.queued';
      break;
    case 'in_progress':
      key = 'fold.pending';
      break;
    case 'failed':
      key = 'fold.failed';
      break;
    case 'cancelled':
      key = 'fold.cancelled';
      break;
    default:
      key = 'fold.done';
  }
  return tp(key, 'verb', t(b.verbKey ?? `verb.${b.kind ?? 'other'}`));
}

/** chat/icons.ts TOOL_ICON */
export function kindIcon(kind: string | undefined): string {
  switch (kind) {
    case 'read':
      return 'fileText';
    case 'edit':
      return 'pencil';
    case 'delete':
      return 'trash2';
    case 'move':
      return 'moveRight';
    case 'search':
      return 'search';
    case 'execute':
      return 'terminal';
    case 'think':
      return 'brain';
    case 'fetch':
      return 'globe';
    case 'switch_mode':
      return 'repeat';
    default:
      return 'wrench';
  }
}

/** toolIcon: to-do, image generation, shown images and skill loads by their verb, the rest by kind */
export function toolIcon(b: Block): string {
  switch (b.verbKey) {
    case 'verb.todo':
      return 'listTodo';
    case 'verb.imagegen':
      return 'imagePlus';
    case 'verb.showImage':
      return 'image';
    case 'verb.showVideo':
      return 'fileVideo';
    case 'verb.skill':
      return 'bookOpen';
    default:
      return kindIcon(b.kind);
  }
}

const rangeSuffix = /^(.+?):(\d+(?:[–-]\d+)?)(?::\d+)?$/;
const urlParts = /^([a-zA-Z][a-zA-Z0-9+.-]*):\/\/([^/?#]+)([^?#]*)/;

/** splitHit: a reference as the name shown and its faint line note */
export function splitHit(hit: string): string[] {
  const range = hit.match(rangeSuffix);
  if (range !== null && !urlParts.test(hit)) {
    return [lastComponent(range[1]), `L${range[2]}`];
  }
  if (explicitPath.test(hit)) {
    return [lastComponent(hit)];
  }
  const url = hit.match(urlParts);
  if (url !== null) {
    const path = url[3];
    return [path.length === 0 || path === '/' ? url[2] : url[2] + path, url[2]];
  }
  return [hit];
}

/** editEntries: consecutive edits of one file share an entry */
export function editEntries(blocks: Block[]): Block[][] {
  const keys: string[] = [];
  const map = new Map<string, Block[]>();
  for (const b of blocks) {
    const path = filePath(b);
    const key = path.length > 0 ? path : (b.id ?? '');
    if (!map.has(key)) {
      keys.push(key);
      map.set(key, []);
    }
    map.get(key)?.push(b);
  }
  return keys.map((k: string) => map.get(k) ?? []);
}

/** editSpan: first to last changed line, e.g. L535–549 */
export function editSpan(blocks: Block[]): string | undefined {
  let lo = Number.MAX_SAFE_INTEGER;
  let hi = Number.MIN_SAFE_INTEGER;
  for (const b of blocks) {
    for (const c of visibleToolContents(b)) {
      if (c.type !== 'diff') {
        continue;
      }
      const lines: DiffLine[] = c.lines ?? [];
      let ns: number[] = lines.filter((l: DiffLine) => l.kind === 'add' && l.newLine !== undefined).map((l: DiffLine) => l.newLine as number);
      if (ns.length === 0) {
        ns = lines.filter((l: DiffLine) => l.kind === 'del' && l.oldLine !== undefined).map((l: DiffLine) => l.oldLine as number);
      }
      for (const n of ns) {
        lo = Math.min(lo, n);
        hi = Math.max(hi, n);
      }
    }
  }
  if (lo === Number.MAX_SAFE_INTEGER) {
    return undefined;
  }
  return lo === hi ? `L${lo}` : `L${lo}–${hi}`;
}

export function diffStatOf(blocks: Block[]): DiffStat | undefined {
  if (!blocks.some((b: Block) => b.diffStat !== undefined)) {
    return undefined;
  }
  let add = 0;
  let del = 0;
  for (const b of blocks) {
    add += b.diffStat?.add ?? 0;
    del += b.diffStat?.del ?? 0;
  }
  const stat: DiffStat = { add: add, del: del };
  return stat;
}

/** `DiffLine.text` carries its sign column (transcript/diff.rs): the gutter draws the sign, the code drops it */
export function diffCode(line: DiffLine): string {
  return line.kind === 'hunk' ? line.text : line.text.slice(1);
}

/** A command's output: its text contents, else what its one content holds */
export function outputOf(b: Block): string {
  const texts = (b.contents ?? []).filter((c: ToolContent) => c.type === 'text');
  if (texts.length > 0) {
    return texts.map((c: ToolContent) => c.text ?? '').join('\n');
  }
  const c = b.content;
  if (c === undefined) {
    return '';
  }
  if (c.type === 'text') {
    return c.text ?? '';
  }
  if (c.type === 'list') {
    return (c.items ?? []).join('\n');
  }
  if (c.type === 'diff') {
    return (c.lines ?? []).map(diffCode).join('\n');
  }
  return '';
}

/** codeDiff.ts diffCopyText: the new file when the diff carries it, else the visible new code without signs */
export function diffCopyText(c: ToolContent): string {
  const source = c.source;
  if (source !== undefined && source.omitted !== true && source.newText !== undefined) {
    return source.newText;
  }
  return (c.lines ?? []).filter((l: DiffLine) => l.kind === 'add' || l.kind === 'ctx').map(diffCode).join('\n');
}

// Durations and outcome

/** folding.ts clockStart: the turn's first streamed output (the send time on older transcripts) */
export function clockStart(turn: TurnX): number | undefined {
  if (turn.respondedAt !== undefined) {
    return turn.respondedAt;
  }
  return (turn.blocks ?? []).length > 0 || turn.endedAt !== undefined ? turn.startedAt : undefined;
}

/** folding.ts durationText: "5m 35s" for a span in ms */
export function durationText(ms: number): string {
  const seconds = Math.max(0, Math.round(ms / 1000));
  const minutes = Math.floor(seconds / 60);
  const rest = seconds % 60;
  if (minutes > 0) {
    return rest > 0 ? tp('turns.elapsed.ms', 'm', `${minutes}`, 's', `${rest}`) : tp('turns.elapsed.m', 'm', `${minutes}`);
  }
  return tp('turns.elapsed.s', 's', `${rest}`);
}

/** The finished head's time: the bare duration */
export function elapsedLabel(turn: TurnX): string {
  const start = clockStart(turn);
  if (turn.startedAt === undefined || turn.endedAt === undefined || start === undefined) {
    return '';
  }
  return durationText(turn.endedAt - start);
}

/** commandDuration: tenths below ten seconds once finished, one tenth as the floor; `whole` is the live clock */
export function commandDuration(ms: number, whole: boolean = false): string {
  const clamped = Math.max(0, ms);
  // Rounded first, so 9.96 s carries into the whole-second branch as "10s" instead of "10.0s"
  const tenths = Math.round(Math.max(100, clamped) / 100);
  if (!whole && tenths < 100) {
    return tp('turns.elapsed.s', 's', (tenths / 10).toFixed(1));
  }
  const seconds = whole ? Math.floor(clamped / 1000) : Math.round(clamped / 1000);
  const m = Math.floor(seconds / 60);
  const s = seconds % 60;
  if (m > 0) {
    return s > 0 ? tp('turns.elapsed.ms', 'm', `${m}`, 's', `${s}`) : tp('turns.elapsed.m', 'm', `${m}`);
  }
  return tp('turns.elapsed.s', 's', `${s}`);
}

/** turnOutcome: a line for a short stop, or for a normal end with nothing to show */
export function turnOutcome(turn: TurnX): string | undefined {
  if (turn.observation === 'unknown') {
    return t('observation.unknownTurn');
  }
  switch (turn.stop) {
    case 'error':
      return t('turns.stop.error');
    case 'refusal':
      return t('turns.stop.refusal');
    case 'max_tokens':
      return t('turns.stop.maxTokens');
    case 'max_turn_requests':
      return t('turns.stop.maxTurns');
    case 'cancelled':
      return t('turns.stop.cancelled');
    case 'end_turn': {
      const empty = (turn.blocks ?? []).every((b: AgentBlock) => b.type === 'text' && isBlank(b.markdown));
      if (!empty) {
        return undefined;
      }
      const command = asTurnCommand(turn);
      if (command === undefined) {
        return t('turns.stop.empty');
      }
      const said: string[] = [];
      if (command.mode !== undefined) {
        said.push(tp('turns.command.mode', 'mode', command.mode));
      }
      for (const o of command.options ?? []) {
        said.push(tp('turns.command.option', 'name', o.name, 'value', o.value));
      }
      return said.length === 0 ? t('turns.command.empty') : said.join(t('common.listSep'));
    }
    default:
      return undefined;
  }
}

/** An agent turn's `command` (an object), which a user turn's string field of the same name shadows in the mirror */
interface AgentCommandOption {
  name: string;
  value: string;
}

interface AgentCommand {
  name: string;
  mode?: string;
  options?: AgentCommandOption[];
}

interface AgentCommandHolder {
  command?: AgentCommand;
}

function asTurnCommand(turn: TurnX): AgentCommand | undefined {
  const raw = (turn as Object as AgentCommandHolder).command;
  return typeof raw === 'object' ? raw : undefined;
}

/** The live head of a working turn (Turns.tsx liveActivity); lead: orb, hand, question, plan */
export class LiveActivity {
  label: string = '';
  active: boolean = true;
  lead: string = 'orb';
}

function activity(label: string, active: boolean, lead: string): LiveActivity {
  const a = new LiveActivity();
  a.label = label;
  a.active = active;
  a.lead = lead;
  return a;
}

export function liveActivity(turn: TurnX): LiveActivity {
  const blocks = (turn.blocks ?? []).map(asBlock);
  if (blocks.some((b: Block) => b.type === 'permission' && b.planId === undefined)) {
    return activity(t('host.awaitingApproval'), false, 'hand');
  }
  if (blocks.some((b: Block) => b.type === 'permission')) {
    return activity(t('plan.awaitingBuild'), false, 'plan');
  }
  if (blocks.some((b: Block) => b.type === 'question' && b.outcome === undefined)) {
    return activity(t('host.awaitingAnswers'), false, 'question');
  }
  // A provider retry relabels the working row, unless the turn's retry row already says so
  const retryRow = blocks.some((b: Block) => b.type === 'notice' && b.retry?.status === 'in_progress');
  const r = turn.retry;
  if (!retryRow && r !== undefined) {
    const label = r.attempt !== undefined && r.max !== undefined ?
      tp('turns.retryingAttempt', 'attempt', `${r.attempt}`, 'max', `${r.max}`) : t('turns.retrying');
    return activity(label, true, 'orb');
  }
  return activity(t('host.working'), true, 'orb');
}

// Permissions

/** A permission option as the card shows it (permissionOptions.ts): its label localized when it is a known one */
export class DisplayOption {
  id: string = '';
  label: string = '';
  kind: string = '';
  detail: string | undefined = undefined;
  bypass: boolean = false;
}

/** Complete adapter plan headings (PLAN_TITLES); custom mode names stay the agent's text */
const planTitles: Map<string, string> = new Map<string, string>([
  ['Approve Plan', 'permission.plan.approve'], ['Ready to code?', 'permission.plan.ready'],
  ['Implement this plan?', 'permission.plan.implementTitle'], ['Enter plan mode', 'permission.plan.enterTitle'],
  ['EnterPlanMode', 'permission.plan.enterTitle'], ['Exit plan mode', 'permission.plan.exitTitle'],
  ['ExitPlanMode', 'permission.plan.exitTitle'], ['exit_plan_mode', 'permission.plan.exitTitle'],
]);

export function planApprovalTitle(title: string): string {
  const key = planTitles.get(title);
  return key !== undefined ? t(key) : title;
}

class PlanOption {
  kind: string = '';
  labels: string[] = [];
  key: string = '';
  bypass: boolean = false;
}

function planOption(kind: string, labels: string[], key: string, bypass: boolean): PlanOption {
  const o = new PlanOption();
  o.kind = kind;
  o.labels = labels;
  o.key = key;
  o.bypass = bypass;
  return o;
}

/** Known plan approval options (PLAN_OPTIONS): Claude's and Codex's exact labels, matched with their kind */
const planOptions: PlanOption[] = [
  planOption('allow_once', ['Yes, manually approve edits', 'Yes, and manually approve edits'], 'permission.plan.manual', false),
  planOption('reject_once', ['No, keep planning'], 'permission.plan.continue', false),
  planOption('allow_always', ['Yes, and use auto mode', 'Yes, and use "auto" mode'], 'permission.plan.auto', false),
  planOption('allow_always', ['Yes, auto-accept edits', 'Yes, and auto-accept edits'], 'permission.plan.acceptEdits', false),
  planOption('allow_always', ['Yes, and bypass permissions'], 'permission.plan.bypass', true),
  planOption('allow_once', ['Yes, implement this plan'], 'permission.plan.implement', false),
  planOption('reject_once', ['No, and tell Codex what to do differently'], 'permission.feedbackCodex', false),
  planOption('allow_once', ['Yes, enter plan mode'], 'permission.plan.enter', false),
  planOption('reject_once', ['No, start implementing now'], 'permission.plan.start', false),
];

const onceLabels = /^(Allow|Allow once|Yes|Yes, allow once|Yes, proceed|Yes, just this once)$/i;
const rejectLabels = /^(Reject|Reject once|Deny|No|No, reject|Cancel|No, continue without running it)$/i;
const sessionLabels = /^(Yes, during this session|Yes, and (?:don't|do not) ask again for this command in this session|Allow for this session)$/i;
const clearContext = /^Yes, clear context(?: \((\d+(?:\.\d+)?)% used\))? and (use auto mode|bypass permissions|auto-accept edits)$/;
const sessionCommand = [
  /^Yes, allow `([^`]+)` commands \(this session\)$/i, /^Always allow `([^`]+)` in this session$/i,
  /^Yes, and (?:don't|do not) ask again for commands that start with `([^`]+)`$/i,
];
const dontAskCommand = /^Yes, and (?:don't|do not) ask again for (.+) commands$/i;
const projectCommand = /^Yes, always allow `([^`]+)` commands in `([^`]+)`$/i;
const allCommand = /^Yes, always allow `([^`]+)` commands in all projects$/i;

/**
 * permissionOption: ACP kinds say nothing of scope, so only complete, known labels are rewritten. `plan`: the option
 * belongs to a linked plan approval, where Build / Revise and Kimi's reject-and-exit are localized too
 */
export function permissionOption(option: PermissionChoice, plan: boolean): DisplayOption {
  const out = new DisplayOption();
  out.id = option.id;
  out.label = option.label;
  out.kind = option.kind;
  out.detail = option.detail;
  const label = option.label;
  const known = planOptions.find((o: PlanOption) => o.kind === option.kind && o.labels.includes(label));
  if (known !== undefined) {
    out.label = t(known.key);
    out.bypass = known.bypass;
    return out;
  }
  if (plan) {
    if (option.kind === 'allow_once' && label === 'Build') {
      out.label = t('plan.build');
      return out;
    }
    if (option.kind === 'reject_once' && label === 'Revise') {
      out.label = t('plan.revise');
      return out;
    }
    if (option.kind === 'allow_always' && label === 'Build with Bypass Permissions') {
      out.label = t('permission.plan.buildBypass');
      out.bypass = true;
      return out;
    }
    if (option.kind === 'reject_once' && label === 'Reject and exit Plan mode') {
      out.label = t('permission.plan.rejectExit');
      return out;
    }
  }
  if (option.kind === 'allow_once' && onceLabels.test(label)) {
    out.label = t('permission.once');
    return out;
  }
  if (option.kind === 'reject_once' && rejectLabels.test(label)) {
    out.label = t('permission.reject');
    return out;
  }
  if (option.kind !== 'allow_always') {
    return out;
  }
  if (sessionLabels.test(label)) {
    out.label = t('permission.session');
    return out;
  }
  const clear = label.match(clearContext);
  if (clear !== null) {
    const key = clear[2] === 'use auto mode' ? 'permission.plan.clearAuto' :
      clear[2] === 'bypass permissions' ? 'permission.plan.clearBypass' : 'permission.plan.clearAcceptEdits';
    out.label = tp(key, 'usage', clear[1] !== undefined ? tp('permission.plan.usage', 'percent', clear[1]) : '');
    out.bypass = clear[2] === 'bypass permissions';
    return out;
  }
  if (label === 'Yes, switch to bypass mode') {
    out.label = t('permission.bypass');
    out.bypass = true;
    return out;
  }
  let command: string | undefined = undefined;
  let scope = 'session';
  for (const p of sessionCommand) {
    const m = label.match(p);
    if (m !== null && command === undefined) {
      command = m[1];
    }
  }
  const dontAsk = label.match(dontAskCommand);
  if (command === undefined && dontAsk !== null && dontAsk[1] !== 'similar') {
    command = dontAsk[1];
  }
  const project = label.match(projectCommand);
  if (command === undefined && project !== null) {
    command = project[1];
    scope = project[2];
  }
  const all = label.match(allCommand);
  if (command === undefined && all !== null) {
    command = all[1];
    scope = '';
  }
  if (command === undefined) {
    return out;
  }
  out.label = scope === 'session' ? t('permission.session') :
    scope.length > 0 ? tp('permission.project', 'project', scope) : t('permission.allProjects');
  out.detail = tp('permission.commands', 'command', command);
  return out;
}

/** reviseOption: the plan approval's option that sends it back for another round */
export function reviseOption(options: PermissionChoice[]): PermissionChoice | undefined {
  return options.find((o: PermissionChoice) => o.kind === 'reject_once' && !o.id.toLowerCase().includes('exit') &&
    !o.label.includes('退出'));
}

const planH1 = /^#[ \t]+(.+?)[ \t]*#*[ \t]*(?:\r?\n|$)/;

/** The plan card's name and body: the engine names a plan after its first `# heading`, which the body drops */
export function planHeading(b: Block): string[] {
  const markdown = (b.markdown ?? '').replace(/^\s+/, '');
  const name = b.title ?? '';
  const title = name.length > 0 && name !== 'Plan' ? name : t('plan.title');
  const h1 = markdown.match(planH1);
  if (h1 !== null && h1[1] === name) {
    return [title, markdown.slice(h1[0].length).trim()];
  }
  return [title, markdown.trim()];
}

// Goals

export function goalDuration(seconds: number): string {
  const whole = Math.max(0, Math.round(seconds));
  const h = Math.floor(whole / 3600);
  const m = Math.floor(whole / 60) % 60;
  const s = whole % 60;
  if (h > 0) {
    return tp('goal.elapsed.hm', 'h', `${h}`, 'm', `${m}`.padStart(2, '0'));
  }
  if (m > 0) {
    return s > 0 ? tp('turns.elapsed.ms', 'm', `${m}`, 's', `${s}`) : tp('turns.elapsed.m', 'm', `${m}`);
  }
  return tp('turns.elapsed.s', 's', `${s}`);
}

/** compactTokens: the locale's compact notation ("184K", "18.4万") */
export function compactTokens(n: number): string {
  const cjk = t('common.listSep') === '、' || t('turns.done') !== 'Done';
  if (cjk && n >= 10000) {
    return `${trimFraction(n / 10000)}万`;
  }
  if (n >= 1000000) {
    return `${trimFraction(n / 1000000)}M`;
  }
  if (n >= 1000) {
    return `${trimFraction(n / 1000)}K`;
  }
  return `${n}`;
}

function trimFraction(v: number): string {
  const s = v.toFixed(1);
  return s.endsWith('.0') ? s.slice(0, -2) : s;
}

export function goalEventLabel(event: string | undefined): string {
  switch (event) {
    case 'paused':
    case 'resumed':
    case 'blocked':
    case 'limited':
    case 'complete':
    case 'cleared':
      return t(`goal.event.${event}`);
    default:
      return t('goal.event.set');
  }
}

/** EVENT_STATUS: the status a milestone row borrows its glyph from */
export function goalEventStatus(event: string | undefined): string {
  switch (event) {
    case 'paused':
    case 'cleared':
      return 'paused';
    case 'blocked':
      return 'blocked';
    case 'limited':
      return 'limited';
    case 'complete':
      return 'complete';
    default:
      return 'active';
  }
}

/** GoalRow's trailing spend: none for set / resumed, else the time (or rounds) and the tokens */
export function goalRowDetail(b: Block): string | undefined {
  if (b.event === 'set' || b.event === 'resumed') {
    return undefined;
  }
  const parts: string[] = [];
  if (b.timeUsedSeconds !== undefined && b.timeUsedSeconds > 0) {
    parts.push(goalDuration(b.timeUsedSeconds));
  } else if (b.iterations !== undefined) {
    parts.push(tp('goal.rounds', 'n', `${b.iterations}`));
  }
  if (b.tokensUsed !== undefined) {
    parts.push(tp('goal.tokens', 'used', compactTokens(b.tokensUsed)));
  }
  return parts.length > 0 ? parts.join(t('common.metaSep')) : undefined;
}

/** The first non-empty line of a text, trimmed */
export function firstLine(text: string | undefined): string {
  return (text ?? '').split('\n').find((l: string) => l.trim().length > 0)?.trim() ?? '';
}
