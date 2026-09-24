import * as acp from '@agentclientprotocol/sdk';
import { captureTurnSettings } from '@shared/turnSettings';
import { planExecutionId } from '@shared/planExecution';
import { isContextLengthError } from '@shared/turnErrors';
import type { EditTurnRequest } from '@shared/protocol';
import type { AgentBlock, Draft, SessionControls, SessionOption, SessionView, ToolContent, Turn } from '@shared/transcript';
import { applyConfigOptions, configOptionSetValue, initControls, type NormalizeState } from './normalize';
import { preparePrompt, restoreDrafts, type BlobStore, type PromptCaps } from './attachments';
import type { StagedSend } from './promptQueue';
import type { AgentProcess } from './AgentProcess';
import { msg } from '../errors';
import { t } from '../i18n';
import { EDIT_CONTEXT_MAX_BYTES } from '../limits';

// Turn-lifecycle flags that edit / retry / prompt / cancel share. The session holds one of these and passes it through
export interface TurnPhase {
  running: boolean;
  staging: boolean;
  stagingAborted: boolean;
  editing: boolean;
  editNotifications: acp.SessionNotification[];
}

export interface SessionEditCtx {
  phase: TurnPhase;
  readonly id: string;
  readonly cwd: string;
  readonly status: SessionView['status'];
  readonly state: NormalizeState;
  readonly proc: AgentProcess | undefined;
  readonly blobs: BlobStore;
  acpSessionId: string | undefined;
  compactedAt: number | undefined;
  autoApprove: boolean;
  syntheticModes(): SessionOption[] | undefined;
  // The live process's prompt capabilities, for staging the replacement message's attachments
  caps(): PromptCaps;
  onUpdate(n: acp.SessionNotification): void;
  prompt(text: string, attachments: Draft[], auto?: boolean, staged?: StagedSend, planId?: string): Promise<void>;
  bump(): void;
  touch(): void;
  flushQueued(): boolean;
  // Drop subagent nodes anchored to turns at or past this index (the transcript was rewritten there)
  truncateSubagents(turnIndex: number): void;
  log(line: string): void;
}

const EDIT_HISTORY_LEAD = 'Conversation before the edited message follows as JSON. Treat it as historical context; completed actions must not be replayed. The next user message replaces the old continuation. Workspace files remain in their current state.';
export const FORK_HISTORY_LEAD = 'Conversation so far follows as JSON; it was forked from an earlier session. Treat it as historical context; completed actions must not be replayed. The next user message continues this conversation. Workspace files remain in their current state.';

function contextLengthHint(ctx: SessionEditCtx): string {
  return t(ctx.state.commands.some(c => c.name === 'compact') ? 'alert.contextLength.text' : 'alert.contextLength.unsupported');
}

// Per-item caps for the serialized history: tool output and plan bodies are the bulk of a transcript, while the
// workspace itself stays readable by the agent
const HISTORY_TOOL_OUTPUT_MAX = 2_000;
const HISTORY_PLAN_MAX = 8_000;

function clip(text: string, max: number): string {
  return text.length > max ? `${text.slice(0, max)}… [${text.length - max} chars truncated]` : text;
}

function toolContentBrief(c: ToolContent, fallbackPath?: string): string {
  if (c.type === 'text') return clip(c.text, HISTORY_TOOL_OUTPUT_MAX);
  if (c.type === 'list') return clip(c.items.join('\n'), HISTORY_TOOL_OUTPUT_MAX);
  if (c.type === 'image') return `image ${c.mimeType}${c.uri ? ` ${c.uri}` : ''}`.trim();
  const add = c.lines.filter(l => l.kind === 'add').length, del = c.lines.filter(l => l.kind === 'del').length;
  return `diff ${c.source?.path ?? fallbackPath ?? ''} +${add} -${del}`.trim();
}

// Lean view of an agent block for the model: UI-only state (timers, streaming flags, diff sources, permission cards,
// thoughts) is dropped, long bodies are clipped
function compactBlock(b: AgentBlock): unknown {
  switch (b.type) {
    case 'text': return { text: b.markdown };
    case 'tool_call': {
      const items = b.contents ?? (b.content ? [b.content] : []);
      const output = items.map(c => toolContentBrief(c, b.target));
      return { tool: b.verb, kind: b.kind, ...(b.target ? { target: b.target } : {}), status: b.status, ...(output.length ? { output } : {}) };
    }
    case 'plan': return { plan: b.entries.map(e => `[${e.status}] ${e.title}`) };
    case 'plan_document': return { planDocument: b.title, status: b.status, ...(b.path ? { path: b.path } : {}), markdown: clip(b.markdown, HISTORY_PLAN_MAX) };
    case 'question': return { questions: b.questions.map(q => q.text), ...(b.outcome ? { outcome: b.outcome } : {}), ...(b.answers ? { answers: b.answers } : {}) };
    case 'image': return { image: b.mimeType, ...(b.uri ? { uri: b.uri } : {}) };
    default: return undefined;
  }
}

function compactTurn(turn: Turn): unknown {
  if (turn.role === 'user') {
    return { role: 'user', text: turn.text, ...(turn.attachments?.length ? { attachments: turn.attachments.map(a => a.name ?? a.kind) } : {}) };
  }
  return {
    role: 'agent',
    blocks: turn.blocks.map(compactBlock).filter(b => b !== undefined),
    ...(turn.stop && turn.stop !== 'end_turn' ? { stop: turn.stop } : {}),
    ...(turn.error ? { error: turn.error.message } : {}),
  };
}

// Index of the first turn to keep so the serialized history fits the budget, aligned to a user turn; 0 when everything
// fits, undefined when nothing does (or trimming is not allowed)
function fitStart(items: string[], turns: readonly Turn[], budget: number, trim: boolean): number | undefined {
  // Array brackets plus one comma per item
  let total = 2 + items.reduce((n, s) => n + Buffer.byteLength(s, 'utf8') + 1, 0);
  if (total <= budget) return 0;
  if (!trim) return undefined;
  for (let start = 0; start < items.length; start++) {
    total -= Buffer.byteLength(items[start]!, 'utf8') + 1;
    const next = start + 1;
    if (total <= budget && turns[next]?.role === 'user') return next;
  }
  return undefined;
}

// ACP cannot rewind to a message. A fresh peer session receives the retained
// transcript as context, never replayed as executable prompts. `trim` lets an oversized history keep only its most
// recent turns (fork); without it an oversized history yields undefined (edit falls back to the native session)
export async function historyContext(
  sessionId: string,
  allTurns: readonly Turn[],
  proc: AgentProcess,
  blobs: BlobStore,
  lead: string,
  caps: PromptCaps,
  trim = false,
): Promise<{ blocks: acp.ContentBlock[]; omitted: number } | undefined> {
  const source = allTurns.filter(turn => turn.role !== 'user' || !turn.auto);
  if (!source.length) return { blocks: [], omitted: 0 };
  const items = source.map(turn => JSON.stringify(compactTurn(turn)));
  const omitNote = (n: number) => `\n${n} earlier turns were omitted to fit the size limit.`;
  const start = fitStart(items, source, EDIT_CONTEXT_MAX_BYTES - Buffer.byteLength(lead + omitNote(source.length), 'utf8') - 1, trim);
  if (start === undefined) return undefined;
  const turns = source.slice(start);
  const history = `${lead}${start ? omitNote(start) : ''}\n[${items.slice(start).join(',')}]`;
  const context: acp.ContentBlock[] = [
    proc.init.agentCapabilities?.promptCapabilities?.embeddedContext
      ? { type: 'resource', resource: { uri: `acpira://history/${sessionId}`, mimeType: 'text/plain', text: history } }
      : { type: 'text', text: history },
  ];
  for (const turn of turns) {
    if (turn.role !== 'user' || !turn.attachments?.length) continue;
    const drafts = await restoreDrafts(sessionId, turn.attachments, blobs);
    if (drafts.length !== turn.attachments.length) throw new Error(t('history.missingAttachment'));
    const old = await preparePrompt(sessionId, '', drafts, blobs, caps);
    if (old.problems.length) throw new Error(old.problems.join('\n'));
    context.push({ type: 'text', text: `Attachments from earlier user message: ${turn.text}` }, ...old.blocks);
  }
  return { blocks: context, omitted: start };
}

// Resending an unchanged message after empty failures/cancellations is a retry. Reuse the
// native context (including compaction) instead of serializing the entire UI
// history. Real edits and turns that already produced output still use rewind.
function unchangedFailedRetry(ctx: SessionEditCtx, edit: EditTurnRequest): boolean {
  const turns = ctx.state.turns, user = turns[edit.turnIndex];
  if (user?.role !== 'user' || user.edited || edit.text !== user.text || edit.attachments.length
    || edit.retainedAttachments.length !== (user.attachments?.length ?? 0)
    || edit.retainedAttachments.some((value, index) => value !== index)) return false;
  const suffix = turns.slice(edit.turnIndex);
  if (suffix.length < 2 || suffix.length % 2 !== 0) return false;
  return suffix.every((turn, index) => index % 2 === 0
    ? turn.role === 'user' && !turn.auto && !turn.edited && turn.text === user.text
      && JSON.stringify(turn.attachments ?? []) === JSON.stringify(user.attachments ?? [])
    : turn.role === 'agent' && (turn.stop === 'error' || turn.stop === 'cancelled') && turn.blocks.length === 0);
}

function checkEditActive(ctx: SessionEditCtx): void {
  if (ctx.phase.stagingAborted || ctx.status !== 'ready') throw new Error(t('history.cancelled'));
}

async function applyEditSettings(ctx: SessionEditCtx, sessionId: string, controls: SessionControls, settings: EditTurnRequest['settings']): Promise<void> {
  const peer = ctx.proc!.agent;
  const live = sessionId === ctx.acpSessionId;
  const modeId = settings.modeId;
  if (modeId && !controls.modes.some(m => m.id === modeId)) throw new Error(t('history.optionUnavailable', { name: modeId }));
  // Model changes can replace the available effort options, so apply them first.
  const isModel = (id: string) => { const c = controls.options.find(c => c.id === id); return c?.category === 'model' || (!c?.category && id === 'model'); };
  const selections = Object.entries(settings.config).sort(([a], [b]) => Number(isModel(b)) - Number(isModel(a)));
  // The editor switches models locally, so its dependent controls still describe the previous model (Devin 3000.11.3:
  // GPT-6 Luna Fast → SWE-2 keeps `speed=fast`, yet SWE-2 has no speed control and no low effort). Only the model is
  // strict; a dependent value the chosen model no longer offers yields to the agent's own value, as a live switch would.
  const settled = new Set<string>();
  for (const [configId, value] of selections) {
    const c = controls.options.find(c => c.id === configId);
    if (!c?.options.some(o => o.id === value)) {
      if (isModel(configId)) throw new Error(t('history.optionUnavailable', { name: configId }));
      ctx.log(`edit: ${configId}=${value} is not offered after the model switch; keeping ${c?.value ?? 'no control'}`);
      settled.add(configId);
      continue;
    }
    if (c.value === value) continue;
    checkEditActive(ctx);
    const r = await peer.request(acp.methods.agent.session.setConfigOption, { sessionId, configId, ...configOptionSetValue(c, value) });
    applyConfigOptions(controls, r.configOptions);
    if (controls.options.find(c => c.id === configId)?.value !== value) throw new Error(t('history.optionUnavailable', { name: configId }));
  }
  if (modeId) {
    if (controls.modeConfigId) {
      if (controls.modeId !== modeId) {
        checkEditActive(ctx);
        const r = await peer.request(acp.methods.agent.session.setConfigOption, { sessionId, configId: controls.modeConfigId, value: modeId });
        applyConfigOptions(controls, r.configOptions);
        if (controls.modeId !== modeId) throw new Error(t('history.optionUnavailable', { name: modeId }));
      }
    } else if (controls.modeId !== modeId) {
      checkEditActive(ctx);
      await peer.request(acp.methods.agent.session.setMode, { sessionId, modeId: ctx.syntheticModes() && modeId === 'yolo' ? 'default' : modeId });
    }
    controls.modeId = modeId;
    if (live && ctx.syntheticModes()) ctx.autoApprove = modeId === 'yolo';
  }
  for (const [id, value] of selections) {
    if (!settled.has(id) && controls.options.find(c => c.id === id)?.value !== value) throw new Error(t('history.optionUnavailable', { name: id }));
  }
  checkEditActive(ctx);
}

// Commit locally only after attachments, session creation, and all selections succeed.
export async function editTurn(ctx: SessionEditCtx, edit: EditTurnRequest): Promise<void> {
  const { phase } = ctx;
  if (phase.running || phase.editing || ctx.status !== 'ready' || !ctx.proc) throw new Error(t('history.unavailable'));
  const user = ctx.state.turns[edit.turnIndex];
  if ((edit.intent !== undefined && edit.intent !== 'replace' && edit.intent !== 'continue')
    || edit.sessionId !== ctx.id || !Number.isInteger(edit.turnIndex) || edit.turnIndex < 0
    || edit.turnCount !== ctx.state.turns.length || user?.role !== 'user' || user.auto
    || planExecutionId(user, ctx.state.turns[edit.turnIndex - 1])
    || user.text !== edit.originalText || user.id !== edit.turnId) throw new Error(t('history.stale'));
  const kept = edit.retainedAttachments;
  if (new Set(kept).size !== kept.length || kept.some(i => !Number.isInteger(i) || i < 0 || i >= (user.attachments?.length ?? 0))) throw new Error(t('history.stale'));
  if (!edit.text.trim() && !kept.length && !edit.attachments.length) throw new Error(t('history.empty'));
  phase.editing = phase.running = phase.staging = true;
  phase.editNotifications = [];
  phase.stagingAborted = false;
  ctx.bump();
  let accepted = false;
  try {
    const prefix = ctx.state.turns.slice(0, edit.turnIndex);
    const restore = async (attachments: NonNullable<typeof user.attachments>) => {
      const drafts = await restoreDrafts(ctx.id, attachments, ctx.blobs);
      if (drafts.length !== attachments.length) throw new Error(t('history.missingAttachment'));
      return drafts;
    };
    const drafts = [...await restore(kept.map(i => user.attachments![i]!)), ...edit.attachments];
    const prepared = await preparePrompt(ctx.id, edit.text, drafts, ctx.blobs, ctx.caps());
    if (prepared.problems.length) throw new Error(prepared.problems.join('\n'));
    const retry = unchangedFailedRetry(ctx, edit);
    let continuing = edit.intent === 'continue';
    let rebuilt: acp.ContentBlock[] | undefined;
    if (!continuing && !retry && prefix.length) {
      const history = await historyContext(ctx.id, prefix, ctx.proc, ctx.blobs, EDIT_HISTORY_LEAD, ctx.caps());
      const blocks = history && [...history.blocks, ...prepared.blocks];
      // Include expanded historical attachments and the replacement message.
      continuing = !blocks || Buffer.byteLength(JSON.stringify(blocks), 'utf8') > EDIT_CONTEXT_MAX_BYTES;
      if (!continuing) rebuilt = blocks;
    }
    if (continuing || retry) {
      const last = ctx.state.turns.at(-1);
      if (!continuing && last?.role === 'agent' && isContextLengthError(last.error)) throw new Error(contextLengthHint(ctx));
      await applyEditSettings(ctx, ctx.acpSessionId!, ctx.state.controls, edit.settings);
      checkEditActive(ctx);
      if (!continuing) { ctx.state.turns = prefix; ctx.truncateSubagents(prefix.length); }
      phase.editing = phase.running = phase.staging = false;
      for (const n of phase.editNotifications) {
        if (n.sessionId === ctx.acpSessionId && (n.update.sessionUpdate === 'available_commands_update' || n.update.sessionUpdate === 'usage_update')) ctx.onUpdate(n);
      }
      accepted = true;
      ctx.log(continuing ? 'Continuing in the native session without rebuilding history' : 'Retrying unchanged failed or cancelled message in the native session');
      ctx.prompt(edit.text, drafts, false, { prepared }).catch(e => ctx.log(`native prompt failed: ${msg(e)}`));
      return;
    }
    if (rebuilt) prepared.blocks = rebuilt;
    const peer = ctx.proc.agent;
    // 1.0 does not inject MCP servers; the CLI reads its own config
    const fresh = await peer.request(acp.methods.agent.session.new, { cwd: ctx.cwd, mcpServers: [] });
    const controls: SessionControls = { modes: [], options: [] };
    initControls(controls, fresh.modes, fresh.configOptions);
    if (!controls.modes.length && ctx.syntheticModes()) {
      controls.modes = ctx.syntheticModes()!;
      controls.modeId = 'default';
    }
    const modeId = edit.settings.modeId;
    await applyEditSettings(ctx, fresh.sessionId, controls, edit.settings);
    ctx.acpSessionId = fresh.sessionId;
    ctx.state.controls = controls;
    ctx.state.turns = prefix;
    ctx.truncateSubagents(prefix.length);
    ctx.state.usage = undefined;
    ctx.state.commands = [];
    ctx.compactedAt = undefined;
    ctx.autoApprove = !!ctx.syntheticModes() && modeId === 'yolo';
    phase.editing = phase.running = phase.staging = false;
    // Some peers advertise slash commands before session/new returns. Only
    // replay the new session's command inventory, never old content or usage.
    for (const n of phase.editNotifications) {
      if (n.sessionId === fresh.sessionId && n.update.sessionUpdate === 'available_commands_update') ctx.onUpdate(n);
    }
    accepted = true;
    ctx.prompt(edit.text, drafts, false, { prepared, edited: true }).catch(e => ctx.log(`edited prompt failed: ${msg(e)}`));
  } finally {
    if (!accepted) {
      phase.editing = phase.running = phase.staging = false;
      for (const n of phase.editNotifications) {
        if (n.sessionId === ctx.acpSessionId && (n.update.sessionUpdate === 'available_commands_update' || n.update.sessionUpdate === 'usage_update')) ctx.onUpdate(n);
      }
      ctx.touch();
      ctx.flushQueued();
    }
    phase.editNotifications = [];
  }
}

// An empty edited failure may not have reached the peer, so rebuild its context.
// Once output exists, keep the entire attempt and continue on the same native
// session: completed tools and partial replies are history even if the RPC failed.
export async function retryTurn(ctx: SessionEditCtx): Promise<void> {
  if (ctx.phase.running || ctx.status !== 'ready') return;
  const turns = ctx.state.turns;
  const agent = turns[turns.length - 1], user = turns[turns.length - 2];
  if (agent?.role !== 'agent' || user?.role !== 'user' || user.auto) return;
  if (isContextLengthError(agent.error)) throw new Error(contextLengthHint(ctx));
  if (!agent.stop || agent.stop === 'end_turn' || agent.stop === 'cancelled') return;
  const hasOutput = agent.blocks.some(block => block.type !== 'text' || !!block.markdown.trim());
  if (user.edited && !hasOutput) {
    await editTurn(ctx, { sessionId: ctx.id, turnIndex: turns.length - 2, turnCount: turns.length,
      originalText: user.text, turnId: user.id, text: user.text, attachments: [],
      retainedAttachments: (user.attachments ?? []).map((_, i) => i),
      settings: user.settings ?? captureTurnSettings(ctx.state.controls) });
    return;
  }
  const drafts = await restoreDrafts(ctx.id, user.attachments ?? [], ctx.blobs);
  // Attachment reads yield; a second click or another send may have claimed the turn.
  if (ctx.phase.running || ctx.status !== 'ready' || ctx.state.turns.at(-1) !== agent) return;
  const planId = planExecutionId(user, turns[turns.length - 3]);
  if (!hasOutput) { turns.splice(-2, 2); ctx.truncateSubagents(turns.length); }
  await ctx.prompt(user.text, drafts, false, undefined, planId);
}
