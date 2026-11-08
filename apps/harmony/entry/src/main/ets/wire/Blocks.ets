import { AgentBlock, SessionView, Turn } from './Wire';

// The transcript fields the chat's rows and cards read (packages/shared/transcript.ts), beyond the core ones Wire.ets
// declares: the same JSON objects, read through these wider shapes (`asBlock`, `asTurn`, `asView`). Kept apart from
// Wire.ets so the list and the composer extend that file without touching these.

export interface Location {
  path: string;
  line?: number;
}

export interface ReadRange {
  path: string;
  start: number;
  end?: number;
}

export interface DiffLine {
  kind: string;
  text: string;
  oldLine?: number;
  newLine?: number;
}

export interface DiffSource {
  path: string;
  oldText?: string;
  newText?: string;
  omitted?: boolean;
}

/** ToolContent: `type` says which fields carry it (text, diff, list, image) */
export interface ToolContent {
  type: string;
  text?: string;
  lines?: DiffLine[];
  source?: DiffSource;
  items?: string[];
  blob?: string;
  mimeType?: string;
  uri?: string;
}

export interface DiffStat {
  add: number;
  del: number;
}

export interface PlanEntry {
  title: string;
  status: string;
  priority?: string;
}

export interface AsyncTaskInfo {
  id: string;
  state: string;
  summary?: string;
}

export interface RetryAttempt {
  at: number;
  attempt?: number;
  text: string;
}

export interface RetryLog {
  status: string;
  max?: number;
  nextAt?: number;
  reconnect?: boolean;
  attempts: RetryAttempt[];
}

export interface PermissionChoice {
  id: string;
  label: string;
  kind: string;
  detail?: string;
}

export interface QuestionOption {
  id: string;
  label: string;
  description?: string;
}

export interface Question {
  id: string;
  title?: string;
  text: string;
  kind: string;
  options: QuestionOption[];
  other?: boolean;
  numeric?: boolean;
  required?: boolean;
}

export interface Attachment {
  kind: string;
  blob?: string;
  mimeType?: string;
  name?: string;
  uri?: string;
  startLine?: number;
  endLine?: number;
  text?: string;
}

export interface ImageRef {
  blob?: string;
  mimeType: string;
  uri?: string;
}

/** Every agent block's fields in one shape (transcript.ts AgentBlock); `type` says which carry it */
export interface Block extends AgentBlock {
  // text
  phase?: string;
  images?: ImageRef[];
  // thought
  startedAt?: number;
  durationSec?: number;
  // plan
  entries?: PlanEntry[];
  // tool_call
  observation?: string;
  verbKey?: string;
  targetMono?: boolean;
  locations?: Location[];
  readRange?: ReadRange;
  asyncTask?: AsyncTaskInfo;
  endedAt?: number;
  diffStat?: DiffStat;
  todoEntries?: PlanEntry[];
  content?: ToolContent;
  contents?: ToolContent[];
  subagentId?: string;
  // permission
  options?: PermissionChoice[];
  planId?: string;
  defaultToNo?: boolean;
  // plan_document
  path?: string;
  // compaction
  error?: string;
  // question
  toolCallId?: string;
  message?: string;
  questions?: Question[];
  outcome?: string;
  answers?: Record<string, string | string[]>;
  // notice
  category?: string;
  severity?: string;
  info?: boolean;
  details?: string;
  actions?: string[];
  retry?: RetryLog;
  // steer
  attachments?: Attachment[];
  // goal
  event?: string;
  objective?: string;
  iterations?: number;
  tokensUsed?: number;
  timeUsedSeconds?: number;
  // image
  mimeType?: string;
  blob?: string;
  uri?: string;
}

export interface TurnError {
  message: string;
  code?: number;
  kind?: string;
  actions?: string[];
}

export interface TurnCommandOption {
  name: string;
  value: string;
}

export interface TurnCommand {
  name: string;
  mode?: string;
  options?: TurnCommandOption[];
}

export interface UsageContext {
  used: number;
  size: number;
}

export interface TurnUsage {
  input?: number;
  output?: number;
  model?: string;
  requestId?: string;
  context?: UsageContext;
}

export interface TurnRetry {
  attempt?: number;
  max?: number;
  detail: string;
}

/** A user or agent turn with every field the transcript reads */
export interface TurnX extends Turn {
  // user
  command?: string;
  attachments?: Attachment[];
  auto?: boolean;
  // agent
  observation?: string;
  respondedAt?: number;
  error?: TurnError;
  retry?: TurnRetry;
  usage?: TurnUsage;
}

export interface SubagentSummary {
  id: string;
  parentId?: string;
  turnIndex: number;
  title?: string;
  task?: string;
  role?: string;
  state: string;
  announcedAt: number;
  endedAt?: number;
  model?: string;
  activity?: string;
  result?: string;
  permissions?: Block[];
  question?: Block;
}

export interface SessionViewX extends SessionView {
  subagents?: SubagentSummary[];
}

export function asBlock(b: AgentBlock): Block {
  return b as Block;
}

export function asTurn(t: Turn): TurnX {
  return t as TurnX;
}

export function asView(v: SessionView): SessionViewX {
  return v as SessionViewX;
}
