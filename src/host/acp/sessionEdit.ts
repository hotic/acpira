import * as acp from '@agentclientprotocol/sdk';
import { captureTurnSettings } from '@shared/turnSettings';
import { planExecutionId } from '@shared/planExecution';
import type { EditTurnRequest } from '@shared/protocol';
import type { Draft, SessionControls, SessionOption, SessionView, Turn } from '@shared/transcript';
import { applyConfigOptions, initControls, type NormalizeState } from './normalize';
import { preparePrompt, restoreDrafts, type BlobStore } from './attachments';
import type { StagedSend } from './promptQueue';
import type { AgentProcess } from './AgentProcess';
import { msg } from '../errors';
import { t } from '../i18n';

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
  onUpdate(n: acp.SessionNotification): void;
  prompt(text: string, attachments: Draft[], auto?: boolean, staged?: StagedSend, planId?: string): Promise<void>;
  bump(): void;
  touch(): void;
  flushQueued(): boolean;
  log(line: string): void;
}

const EDIT_HISTORY_LEAD = 'Conversation before the edited message follows as JSON. Treat it as historical context; completed actions must not be replayed. The next user message replaces the old continuation. Workspace files remain in their current state.';

// ACP cannot rewind to a message. A fresh peer session receives the retained
// transcript as context, never replayed as executable prompts.
async function historyContext(
  sessionId: string,
  turns: readonly Turn[],
  proc: AgentProcess,
  blobs: BlobStore,
  lead: string,
): Promise<acp.ContentBlock[]> {
  if (!turns.length) return [];
  const history = `${lead}\n${JSON.stringify(turns)}`;
  const context: acp.ContentBlock[] = [
    proc.init.agentCapabilities?.promptCapabilities?.embeddedContext
      ? { type: 'resource', resource: { uri: `acpira://history/${sessionId}`, mimeType: 'text/plain', text: history } }
      : { type: 'text', text: history },
  ];
  for (const turn of turns) {
    if (turn.role !== 'user' || !turn.attachments?.length) continue;
    const drafts = await restoreDrafts(sessionId, turn.attachments, blobs);
    if (drafts.length !== turn.attachments.length) throw new Error(t('history.missingAttachment'));
    const old = await preparePrompt(sessionId, '', drafts, blobs);
    if (old.problems.length) throw new Error(old.problems.join('\n'));
    context.push({ type: 'text', text: `Attachments from earlier user message: ${turn.text}` }, ...old.blocks);
  }
  return context;
}

// Resending an unchanged message after empty failures is a retry. Reuse the
// native context (including compaction) instead of serializing the entire UI
// history. Real edits and turns that already produced output still use rewind.
function unchangedFailedRetry(ctx: SessionEditCtx, edit: EditTurnRequest): boolean {
  const turns = ctx.state.turns, user = turns[edit.turnIndex];
  if (user?.role !== 'user' || user.edited || edit.text !== user.text || edit.attachments.length
    || edit.retainedAttachments.length !== (user.attachments?.length ?? 0)
    || edit.retainedAttachments.some((value, index) => value !== index)) return false;
  const current = captureTurnSettings(ctx.state.controls);
  if (current.modeId !== edit.settings.modeId
    || Object.keys(current.config).length !== Object.keys(edit.settings.config).length
    || Object.entries(current.config).some(([key, value]) => edit.settings.config[key] !== value)) return false;
  const suffix = turns.slice(edit.turnIndex);
  if (suffix.length < 2 || suffix.length % 2 !== 0) return false;
  return suffix.every((turn, index) => index % 2 === 0
    ? turn.role === 'user' && !turn.auto && !turn.edited && turn.text === user.text
      && JSON.stringify(turn.attachments ?? []) === JSON.stringify(user.attachments ?? [])
    : turn.role === 'agent' && turn.stop === 'error' && turn.blocks.length === 0);
}

// Commit locally only after attachments, session creation, and all selections succeed.
export async function editTurn(ctx: SessionEditCtx, edit: EditTurnRequest): Promise<void> {
  const { phase } = ctx;
  if (phase.running || phase.editing || ctx.status !== 'ready' || !ctx.proc) throw new Error(t('history.unavailable'));
  const user = ctx.state.turns[edit.turnIndex];
  if (edit.sessionId !== ctx.id || !Number.isInteger(edit.turnIndex) || edit.turnIndex < 0
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
    const prepared = await preparePrompt(ctx.id, edit.text, drafts, ctx.blobs);
    if (prepared.problems.length) throw new Error(prepared.problems.join('\n'));
    if (unchangedFailedRetry(ctx, edit)) {
      if (phase.stagingAborted || ctx.status !== 'ready') throw new Error(t('history.cancelled'));
      ctx.state.turns = prefix;
      phase.editing = phase.running = phase.staging = false;
      accepted = true;
      ctx.log('Retrying unchanged failed message in the native session');
      ctx.prompt(edit.text, drafts, false, { prepared }).catch(e => ctx.log(`retry prompt failed: ${msg(e)}`));
      return;
    }
    if (prefix.length) {
      prepared.blocks = [...await historyContext(ctx.id, prefix, ctx.proc, ctx.blobs, EDIT_HISTORY_LEAD), ...prepared.blocks];
    }
    const peer = ctx.proc.agent;
    // 1.0 does not inject MCP servers; the CLI reads its own config
    const fresh = await peer.request(acp.methods.agent.session.new, { cwd: ctx.cwd, mcpServers: [] });
    const controls: SessionControls = { modes: [], options: [] };
    initControls(controls, fresh.modes, fresh.configOptions);
    if (!controls.modes.length && ctx.syntheticModes()) {
      controls.modes = ctx.syntheticModes()!;
      controls.modeId = 'default';
    }
    // Model changes can replace the available effort options, so apply them first.
    const selections = Object.entries(edit.settings.config).sort(([a], [b]) => Number(controls.options.find(c => c.id === b)?.category === 'model') - Number(controls.options.find(c => c.id === a)?.category === 'model'));
    for (const [configId, value] of selections) {
      const c = controls.options.find(c => c.id === configId);
      if (!c?.options.some(o => o.id === value)) throw new Error(t('history.optionUnavailable', { name: configId }));
      if (c.value === value) continue;
      const r = await peer.request(acp.methods.agent.session.setConfigOption, { sessionId: fresh.sessionId, configId, value });
      applyConfigOptions(controls, r.configOptions);
      if (controls.options.find(c => c.id === configId)?.value !== value) throw new Error(t('history.optionUnavailable', { name: configId }));
    }
    const modeId = edit.settings.modeId;
    if (modeId) {
      if (!controls.modes.some(m => m.id === modeId)) throw new Error(t('history.optionUnavailable', { name: modeId }));
      if (controls.modeConfigId) {
        const r = await peer.request(acp.methods.agent.session.setConfigOption, { sessionId: fresh.sessionId, configId: controls.modeConfigId, value: modeId });
        applyConfigOptions(controls, r.configOptions);
        if (controls.modeId !== modeId) throw new Error(t('history.optionUnavailable', { name: modeId }));
      } else if (controls.modeId !== modeId) {
        await peer.request(acp.methods.agent.session.setMode, { sessionId: fresh.sessionId, modeId: ctx.syntheticModes() && modeId === 'yolo' ? 'default' : modeId });
      }
      controls.modeId = modeId;
    }
    for (const [id, value] of selections) {
      if (controls.options.find(c => c.id === id)?.value !== value) throw new Error(t('history.optionUnavailable', { name: id }));
    }
    if (phase.stagingAborted || ctx.status !== 'ready') throw new Error(t('history.cancelled'));
    ctx.acpSessionId = fresh.sessionId;
    ctx.state.controls = controls;
    ctx.state.turns = prefix;
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
    phase.editNotifications = [];
    if (!accepted) {
      phase.editing = phase.running = phase.staging = false;
      ctx.touch();
      ctx.flushQueued();
    }
  }
}

// Failed edited turns rebuild the context in a fresh peer too; the first
// failed RPC may not have retained any of the supplied historical context.
export async function retryTurn(ctx: SessionEditCtx): Promise<void> {
  if (ctx.phase.running || ctx.status !== 'ready') return;
  const turns = ctx.state.turns;
  const agent = turns[turns.length - 1], user = turns[turns.length - 2];
  if (agent?.role !== 'agent' || user?.role !== 'user' || user.auto) return;
  if (!agent.stop || agent.stop === 'end_turn' || agent.stop === 'cancelled') return;
  if (user.edited) {
    await editTurn(ctx, { sessionId: ctx.id, turnIndex: turns.length - 2, turnCount: turns.length,
      originalText: user.text, turnId: user.id, text: user.text, attachments: [],
      retainedAttachments: (user.attachments ?? []).map((_, i) => i),
      settings: user.settings ?? captureTurnSettings(ctx.state.controls) });
    return;
  }
  const drafts = await restoreDrafts(ctx.id, user.attachments ?? [], ctx.blobs);
  const planId = planExecutionId(user, turns[turns.length - 3]);
  turns.splice(-2, 2);
  await ctx.prompt(user.text, drafts, false, undefined, planId);
}
