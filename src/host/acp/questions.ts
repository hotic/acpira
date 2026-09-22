import type * as acp from '@agentclientprotocol/sdk';
import type { Question, QuestionAnswers, QuestionBlock, QuestionOption } from '@shared/transcript';
import { activityOf, type NormalizeState } from './normalize';
import type { GrokAnswers, GrokQuestionRequest, GrokQuestionResponse } from './grokQuestions';
import { t } from '../i18n';

// The agent's own tool input, as seen in its tool_call: the form schema flattens it (Devin puts the option label into `const` and the
// description into `title`; Kimi drops the question text into the elicitation `message`), so the tool input is the richer source for labels
export interface RawQuestion {
  header?: string;
  question?: string;
  options?: { label: string; description?: string }[];
}

type Pending =
  | { kind: 'form'; blockId: string; schema: acp.ElicitationSchema; questions: Question[]; nodeId?: string; resolve: (r: acp.CreateElicitationResponse) => void }
  | { kind: 'grok'; blockId: string; questions: Question[]; nodeId?: string; resolve: (r: GrokQuestionResponse) => void };

export interface QuestionGateDeps {
  // Which transcript a request belongs to: the root session id → the root state; a child peer session id → that node's
  // state; bump marks the owning node dirty when a card mutates its transcript
  stateFor: (sessionId: string | undefined) => { state: NormalizeState; nodeId?: string; bump?: () => void } | undefined;
  // Every transcript that may hold cards (root + all child nodes): sweeps run over all of them;
  // nodeId pins a card to its owner — undefined is the root — so a reopened child's fresh `q-1`
  // can never settle the old `q-1` still sitting in the root history
  states: () => { state: NormalizeState; nodeId?: string; bump?: () => void }[];
  touch: () => void;
  log: (line: string) => void;
}

// Remembered tool inputs: enough for the questions in flight, a handful is plenty
const RAW_KEEP = 8;

// Holds the question cards waiting for the user. Devin / Kimi ask through elicitation/create (mode form), Grok through its own request;
// both become one QuestionBlock in the current agent turn, and the webview answers by block id. A resolved block stays as the record
export class QuestionGate {
  private pending = new Map<string, Pending>();
  private seq = 0;
  private raw: { toolCallId: string; questions: RawQuestion[] }[] = [];

  constructor(private deps: QuestionGateDeps) {}

  has(blockId: string): boolean { return this.pending.has(blockId); }

  get waiting(): boolean { return this.pending.size > 0; }

  // A tool_call / tool_call_update whose rawInput carries `questions` is the agent's ask-user-question tool; keep it for label enrichment
  rememberToolInput(u: acp.ToolCall | acp.ToolCallUpdate) {
    const raw = u.rawInput as { questions?: unknown } | null | undefined;
    if (!raw || !Array.isArray(raw.questions)) return;
    const questions = raw.questions.flatMap((q): RawQuestion[] => {
      const item = q as Record<string, unknown> | null;
      if (!item || typeof item !== 'object') return [];
      const options = Array.isArray(item.options)
        ? item.options.flatMap(o => {
          const opt = o as Record<string, unknown> | null;
          return opt && typeof opt.label === 'string' ? [{ label: opt.label, ...(typeof opt.description === 'string' ? { description: opt.description } : {}) }] : [];
        })
        : undefined;
      return [{
        ...(typeof item.header === 'string' ? { header: item.header } : {}),
        ...(typeof item.question === 'string' ? { question: item.question } : {}),
        ...(options ? { options } : {}),
      }];
    });
    this.raw = [...this.raw.filter(r => r.toolCallId !== u.toolCallId), { toolCallId: u.toolCallId, questions }].slice(-RAW_KEEP);
  }

  // Devin sends no toolCallId with the form: fall back to the latest remembered input with the same number of questions
  private rawFor(toolCallId: string | null | undefined, count: number): RawQuestion[] | undefined {
    if (toolCallId) return this.raw.find(r => r.toolCallId === toolCallId)?.questions;
    for (let i = this.raw.length - 1; i >= 0; i--) if (this.raw[i]!.questions.length === count) return this.raw[i]!.questions;
    return undefined;
  }

  async onElicitation(req: acp.CreateElicitationRequest, signal: AbortSignal): Promise<acp.CreateElicitationResponse> {
    if (signal.aborted) return { action: 'cancel' };
    // The request union ends in a catch-all variant, so the form fields are read through a plain shape rather than narrowed on `mode`
    const { requestedSchema: schema, toolCallId, sessionId } = req as { requestedSchema?: acp.ElicitationSchema; toolCallId?: string | null; sessionId?: string };
    if (req.mode !== 'form' || !schema) return { action: 'decline' };
    const ref = this.deps.stateFor(sessionId);
    if (!ref) {
      this.deps.log(`elicitation request for unknown session ${sessionId ?? '(request scope)'}`);
      return { action: 'cancel' };
    }
    const questions = formQuestions(schema, req.message, req._meta, this.rawFor(toolCallId, Object.keys(schema.properties ?? {}).length));
    if (!questions.length) return { action: 'decline' };
    const block = this.open(ref.state, questions, toolCallId ?? undefined, spareMessage(req.message, questions));
    ref.bump?.();
    return new Promise(resolve => {
      this.pending.set(block.id, { kind: 'form', blockId: block.id, schema, questions, nodeId: ref.nodeId, resolve });
      this.arm(block, signal);
    });
  }

  async onGrokQuestion(req: GrokQuestionRequest, signal: AbortSignal): Promise<GrokQuestionResponse> {
    if (signal.aborted) return { outcome: 'skip_interview' };
    const questions = req.questions.map((q): Question => ({
      id: q.question, text: q.question, kind: q.multiSelect ? 'multiple' : 'single',
      options: q.options.map(o => ({ id: o.label, label: o.label, ...(o.description && o.description !== o.label ? { description: o.description } : {}) })),
      other: true,
    }));
    if (!questions.length) return { outcome: 'accepted', answers: {} };
    const ref = this.deps.stateFor(req.sessionId);
    if (!ref) {
      this.deps.log(`question request for unknown session ${req.sessionId}`);
      return { outcome: 'skip_interview' };
    }
    const block = this.open(ref.state, questions, req.toolCallId);
    ref.bump?.();
    return new Promise(resolve => {
      this.pending.set(block.id, { kind: 'grok', blockId: block.id, questions, nodeId: ref.nodeId, resolve });
      this.arm(block, signal);
    });
  }

  // The webview closed the card. Only answered questions travel; skip tells the agent to go on with what it has (Grok's skip_interview,
  // a form's decline when nothing was answered — with partial answers a form is accepted as far as it goes, which is how Devin and Kimi read a partial form anyway)
  resolve(blockId: string, answers: QuestionAnswers, skip = false) {
    const p = this.pending.get(blockId);
    if (!p) return;
    const given = cleanAnswers(p.questions, answers);
    const empty = Object.keys(given).length === 0;
    this.pending.delete(blockId);
    this.settle(p, skip || empty ? 'skipped' : 'answered', given);
    if (p.kind === 'grok') {
      p.resolve(skip || empty ? { outcome: 'skip_interview', ...(empty ? {} : { partial_answers: given as GrokAnswers }) } : { outcome: 'accepted', answers: given as GrokAnswers });
    } else {
      p.resolve(empty ? { action: 'decline' } : { action: 'accept', content: formContent(p.schema, p.questions, given) });
    }
    this.deps.touch();
  }

  // The turn is over (cancelled, failed, closed): every open card is withdrawn and the agent told so
  cancelAll() {
    for (const p of this.pending.values()) {
      this.settle(p, 'cancelled');
      if (p.kind === 'grok') p.resolve({ outcome: 'skip_interview' }); else p.resolve({ action: 'cancel' });
    }
    this.pending.clear();
  }

  // One subagent was cancelled: its open cards close as cancelled and the agent is told so
  cancelFor(nodeId: string) {
    for (const p of [...this.pending.values()]) {
      if (p.nodeId !== nodeId) continue;
      this.pending.delete(p.blockId);
      this.settle(p, 'cancelled');
      if (p.kind === 'grok') p.resolve({ outcome: 'skip_interview' }); else p.resolve({ action: 'cancel' });
    }
  }

  private open(state: NormalizeState, questions: Question[], toolCallId?: string, message?: string): QuestionBlock {
    const block: QuestionBlock = { type: 'question', id: `q-${++this.seq}`, ...(toolCallId ? { toolCallId } : {}), ...(message ? { message } : {}), questions };
    const last = state.turns[state.turns.length - 1];
    if (last?.role === 'agent') { last.blocks.push(block); last.activity = activityOf(state.turns); }
    return block;
  }

  // The agent withdrew its request (session/cancel, a timeout on its side): the card closes as cancelled
  private arm(block: QuestionBlock, signal: AbortSignal) {
    signal.addEventListener('abort', () => {
      const p = this.pending.get(block.id);
      if (!p) return;
      this.pending.delete(block.id);
      this.settle(p, 'cancelled');
      if (p.kind === 'grok') p.resolve({ outcome: 'skip_interview' }); else p.resolve({ action: 'cancel' });
      this.deps.touch();
    }, { once: true });
    this.deps.touch();
  }

  private settle(p: Pending, outcome: QuestionBlock['outcome'], answers?: QuestionAnswers) {
    for (const e of this.deps.states()) {
      if (e.nodeId !== p.nodeId) continue;
      for (let i = e.state.turns.length - 1; i >= 0; i--) {
        const turn = e.state.turns[i];
        if (turn?.role !== 'agent') continue;
        const b = turn.blocks.find(b => b.type === 'question' && b.id === p.blockId);
        if (!b || b.type !== 'question') continue;
        b.outcome = outcome;
        if (answers && Object.keys(answers).length) b.answers = answers;
        turn.activity = activityOf(e.state.turns);
        e.bump?.();
        return;
      }
    }
  }
}

type Prop = Record<string, unknown>;

// One question per form property, in schema order. Labels come from the agent's tool input when it lines up with the schema
// (same count, and header / question text agreeing where both exist); the schema alone still yields a usable question
export function formQuestions(schema: acp.ElicitationSchema, message: string, meta: Record<string, unknown> | null | undefined, raw?: RawQuestion[]): Question[] {
  const props = schema.properties ?? {};
  const keys = Object.keys(props);
  const required = new Set(schema.required ?? []);
  const allowOther = meta?.['cognition.ai/allowOther'] === true;
  // Kimi joins the question texts into the message, one per line
  const lines = message.split('\n').map(l => l.trim()).filter(Boolean);
  const aligned = raw && raw.length === keys.length ? raw : undefined;
  return keys.flatMap((key, i) => {
    const prop = props[key] as Prop | undefined;
    if (!prop) return [];
    const rq = aligned?.[i];
    const usable = rq && (!str(prop.title) || !rq.header || prop.title === rq.header) && (!str(prop.description) || !rq.question || prop.description === rq.question) ? rq : undefined;
    const title = str(prop.title) ?? usable?.header;
    const text = str(prop.description) ?? usable?.question ?? (lines.length === keys.length ? lines[i] : undefined) ?? (keys.length === 1 ? str(message) : undefined) ?? title ?? key;
    const q: Question = { id: key, ...(title && title !== text ? { title } : {}), text, kind: 'single', options: [], ...(required.has(key) ? { required: true } : {}) };
    const items = prop.type === 'array' && prop.items && typeof prop.items === 'object' ? prop.items as Prop : undefined;
    if (prop.type === 'boolean') {
      q.options = [{ id: 'true', label: t('question.yes') }, { id: 'false', label: t('question.no') }];
    } else if (items) {
      q.kind = 'multiple';
      q.options = Array.isArray(items.anyOf) ? titled(items.anyOf, usable) : Array.isArray(items.enum) ? plain(items.enum) : [];
    } else if (Array.isArray(prop.oneOf)) {
      q.options = titled(prop.oneOf, usable);
    } else if (Array.isArray(prop.enum)) {
      q.options = plain(prop.enum);
    } else {
      q.kind = 'text';
      if (prop.type === 'number' || prop.type === 'integer') q.numeric = true;
    }
    if (q.kind !== 'text' && prop.type !== 'boolean' && (allowOther || (prop._meta as Prop | undefined)?.['cognition.ai/allowOther'] === true)) q.other = true;
    if (q.kind !== 'text' && q.options.length === 0) q.kind = 'text';
    return [q];
  });
}

function str(v: unknown): string | undefined {
  return typeof v === 'string' && v.trim() ? v : undefined;
}

function plain(values: unknown[]): QuestionOption[] {
  return values.flatMap(v => (typeof v === 'string' || typeof v === 'number' ? [{ id: String(v), label: String(v) }] : []));
}

// Titled choices ({ const, title, description }); a tool-input option with the same label as `const` supplies the label / description the agent meant
function titled(entries: unknown[], rq?: RawQuestion): QuestionOption[] {
  return entries.flatMap(e => {
    const o = e as Prop | null;
    if (!o || typeof o.const !== 'string') return [];
    const ro = rq?.options?.find(x => x.label === o.const);
    const label = ro?.label ?? str(o.title) ?? o.const;
    const description = ro ? ro.description : str(o.description);
    return [{ id: o.const, label, ...(description && description !== label ? { description } : {}) }];
  });
}

// The form's message earns a line of its own only when it says something the questions don't already
export function spareMessage(message: string, questions: Question[]): string | undefined {
  const m = message.trim();
  if (!m) return undefined;
  const texts = questions.map(q => q.text.trim());
  if (texts.includes(m) || texts.join('\n') === m.split('\n').map(l => l.trim()).filter(Boolean).join('\n')) return undefined;
  return m;
}

// Keep only answers to known questions with something in them; a single-select answer is one string, a multi-select a non-empty list
function cleanAnswers(questions: Question[], answers: QuestionAnswers): QuestionAnswers {
  const out: QuestionAnswers = {};
  for (const q of questions) {
    const a = answers[q.id];
    if (a === undefined) continue;
    if (q.kind === 'multiple') {
      const list = (Array.isArray(a) ? a : [a]).map(s => s.trim()).filter(Boolean);
      if (list.length) out[q.id] = list;
    } else {
      const s = (Array.isArray(a) ? a.join(', ') : a).trim();
      if (s) out[q.id] = s;
    }
  }
  return out;
}

// Answers go back typed the way the schema declared them: booleans and numbers are converted, lists stay lists, everything else is a string
function formContent(schema: acp.ElicitationSchema, questions: Question[], answers: QuestionAnswers): Record<string, acp.ElicitationContentValue> {
  const content: Record<string, acp.ElicitationContentValue> = {};
  for (const q of questions) {
    const a = answers[q.id];
    if (a === undefined) continue;
    const prop = schema.properties?.[q.id] as Prop | undefined;
    if (Array.isArray(a)) { content[q.id] = a; continue; }
    if (prop?.type === 'boolean') content[q.id] = a === 'true';
    else if (q.numeric) { const n = Number(a); if (Number.isFinite(n)) content[q.id] = n; }
    else content[q.id] = a;
  }
  return content;
}
