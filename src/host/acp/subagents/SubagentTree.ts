// First-class subagent nodes for one AcpSession: per-dialect normalization (RFD/claude native sessions, Devin's
// nested cognition.ai updates, Claude legacy Agent, Kimi's receipt), lifecycle, early-update buffering and
// record round trips. All mutations are synchronous; the session only gets touch() callbacks.

import { randomUUID } from 'node:crypto';
import type * as acp from '@agentclientprotocol/sdk';
import type { SubagentRecord, SubagentState, SubagentSummary, SubagentVisibility } from '@shared/subagents';
import type { PermissionBlock, QuestionBlock, ToolCallBlock, Turn } from '@shared/transcript';
import { t } from '../../i18n';
import { activityOf, applyUpdate, emptyState, endTurn, type NormalizeState } from '../normalize';
import { restoreInterruptedTurns } from '../restoreTurns';
import type { SubagentLifecycle } from './wire';

export interface SubagentTreeDeps {
  log: (line: string) => void;
  now?: () => number;
}

export interface RootRouteCtx {
  turnIndex: number;
  findRootTool: (id: string) => ToolCallBlock | undefined;
}

// 'disconnected' is terminal: an agent-reported disconnect (RFD orphan recovery) ends the child as surely as a completion
const TERMINAL = new Set<SubagentState>(['completed', 'failed', 'cancelled', 'disconnected']);
const ORPHAN_MAX_IDS = 8;
const ORPHAN_MAX_UPDATES = 64;

interface PendingDelegation { toolCallId: string; title?: string; task?: string }
interface PendingLaunch { toolCallId: string; model?: string; title?: string }

interface SubagentNode {
  id: string;
  parentId?: string;
  turnIndex: number;
  visibility: SubagentVisibility;
  title?: string;
  task?: string;
  role?: string;
  status: SubagentState;
  stateSource: 'agent' | 'local';
  controls: { cancel: boolean };
  cancelRequested?: boolean;
  background?: boolean;
  announcedAt: number;
  endedAt?: number;
  model?: string;
  usage?: { used: number; size: number };
  peer: { sessionId?: string; agentId?: string; toolCallId?: string };
  toolCount: number;
  result?: string;
  meta?: Record<string, unknown>;
  // Dialects whose lifecycle is not driven by the parent call's status:
  // 'claude' reads it off the call's toolResponse, 'devin' only on subagent_completed
  dialect?: 'claude' | 'devin';
  state: NormalizeState;   // the child's own transcript
  rev: number;
  cached?: { rev: number; summary: SubagentSummary };
}

function record(v: unknown): Record<string, unknown> | undefined {
  return v !== null && typeof v === 'object' && !Array.isArray(v) ? v as Record<string, unknown> : undefined;
}

function str(v: unknown): string | undefined {
  return typeof v === 'string' && v.length > 0 ? v : undefined;
}

function toolBlocks(turns: Turn[]): ToolCallBlock[] {
  const out: ToolCallBlock[] = [];
  for (const turn of turns) if (turn.role === 'agent') for (const b of turn.blocks) if (b.type === 'tool_call') out.push(b);
  return out;
}

export class SubagentTree {
  private nodes: SubagentNode[] = [];
  // Rebuilt from nodes on reindex (peer ids survive session/load replay)
  readonly byPeerSession = new Map<string, SubagentNode>();
  readonly byPeerAgent = new Map<string, SubagentNode>();
  readonly byPeerTool = new Map<string, SubagentNode>();
  // Connection-local: child tool calls that were routed off the root stream; later updates often drop the _meta link
  private toolOwner = new Map<string, string>();
  private pendingDelegations: PendingDelegation[] = [];
  private pendingLaunches = new Map<string, PendingLaunch>();
  // Updates that arrived under a session id nobody announced yet, flushed on its lifecycle create
  private orphans = new Map<string, acp.SessionUpdate[]>();
  private orphanLogged = false;

  constructor(private deps: SubagentTreeDeps, records?: SubagentRecord[], updatedAt?: string) {
    const at = updatedAt ?? new Date().toISOString();
    for (const r of records ?? []) {
      const wasRunning = r.state === 'running';
      const n: SubagentNode = {
        id: r.id,
        turnIndex: r.turnIndex,
        visibility: r.visibility,
        status: wasRunning ? 'disconnected' : r.state,
        stateSource: wasRunning ? 'local' : r.stateSource,
        controls: { cancel: r.controls.cancel },
        announcedAt: r.announcedAt,
        toolCount: r.toolCount,
        peer: { ...r.peer },
        state: { ...emptyState(), turns: restoreInterruptedTurns(r.turns, at) },
        rev: 1,
      };
      if (r.parentId !== undefined) n.parentId = r.parentId;
      if (r.title !== undefined) n.title = r.title;
      if (r.task !== undefined) n.task = r.task;
      if (r.role !== undefined) n.role = r.role;
      if (r.cancelRequested) n.cancelRequested = true;
      if (r.background) n.background = true;
      if (r.model !== undefined) n.model = r.model;
      if (r.usage !== undefined) n.usage = { ...r.usage };
      if (r.result !== undefined) n.result = r.result;
      if (wasRunning) n.endedAt = Date.parse(at) || this.now();
      else if (r.endedAt !== undefined) n.endedAt = r.endedAt;
      this.nodes.push(n);
    }
    this.reindex();
  }

  private now(): number { return this.deps.now?.() ?? Date.now(); }
  private bump(n: SubagentNode) { n.rev++; }
  get size(): number { return this.nodes.length; }
  private label(n: SubagentNode): string { return n.peer.sessionId ?? n.peer.agentId ?? n.peer.toolCallId ?? n.id.slice(0, 8); }

  reindex() {
    this.byPeerSession.clear();
    this.byPeerAgent.clear();
    this.byPeerTool.clear();
    this.toolOwner.clear();
    for (const n of this.nodes) {
      if (n.peer.sessionId !== undefined) this.byPeerSession.set(n.peer.sessionId, n);
      if (n.peer.agentId !== undefined) this.byPeerAgent.set(n.peer.agentId, n);
      if (n.peer.toolCallId !== undefined) this.byPeerTool.set(n.peer.toolCallId, n);
    }
  }

  // The request/notification arrived under this peer session id → which transcript it belongs to; bump marks the
  // summary dirty when a gate mutates the node's blocks (a permission card appearing or resolving)
  stateForPeer(peerSessionId: string): { state: NormalizeState; nodeId?: string; bump?: () => void } | undefined {
    const n = this.byPeerSession.get(peerSessionId);
    return n === undefined ? undefined : { state: n.state, nodeId: n.id, bump: () => this.bump(n) };
  }

  states(): { state: NormalizeState; bump?: () => void }[] {
    return this.nodes.map(n => ({ state: n.state, bump: () => this.bump(n) }));
  }

  // ---- lifecycle updates (RFD subagent_update / claude subagent_spawned + subagent_state_update) ----

  lifecycle(parentPeerSessionId: string, rootPeerSessionId: string | undefined, l: SubagentLifecycle, ctx: RootRouteCtx) {
    let parentId: string | undefined;
    if (parentPeerSessionId !== rootPeerSessionId) {
      const parent = this.byPeerSession.get(parentPeerSessionId);
      if (parent === undefined) {
        this.deps.log(`subagent ${l.peerSessionId} announced under unknown parent session ${parentPeerSessionId}`);
        return;
      }
      parentId = parent.id;
    }
    let n = this.byPeerSession.get(l.peerSessionId);
    if (n === undefined) {
      n = {
        id: randomUUID(),
        turnIndex: ctx.turnIndex,
        visibility: 'session',
        status: 'running',
        stateSource: 'agent',
        controls: { cancel: l.capabilities?.cancel === true },
        announcedAt: this.now(),
        toolCount: 0,
        peer: { sessionId: l.peerSessionId },
        state: emptyState(),
        rev: 0,
      };
      // The eager turn gives tool rows real startedAt/endedAt and endTurn something to seal
      n.state.turns.push({ role: 'agent', startedAt: n.announcedAt, blocks: [] });
      if (parentId !== undefined) n.parentId = parentId;
      this.nodes.push(n);
      this.byPeerSession.set(l.peerSessionId, n);
      // A claude async_launched receipt may have reached the root before the spawn
      const launch = this.pendingLaunches.get(l.peerSessionId);
      if (launch !== undefined) {
        this.pendingLaunches.delete(l.peerSessionId);
        n.peer.toolCallId = launch.toolCallId;
        this.byPeerTool.set(launch.toolCallId, n);
        n.model ??= launch.model;
        n.title ??= launch.title;
        const block = ctx.findRootTool(launch.toolCallId);
        if (block !== undefined) block.subagentId = n.id;
      }
    }
    if (l.title !== undefined) n.title = l.title;
    if (l.task !== undefined) n.task = l.task;
    if (l.capabilities?.cancel !== undefined) n.controls.cancel = l.capabilities.cancel;
    if (l.meta !== undefined) n.meta = l.meta;
    if (l.state !== undefined) this.transition(n, l.state);
    this.bump(n);
    const buffered = this.orphans.get(l.peerSessionId);
    if (buffered !== undefined) {
      this.orphans.delete(l.peerSessionId);
      for (const u of buffered) this.applyChild(n, u, ctx);
    }
  }

  private transition(n: SubagentNode, state: SubagentState) {
    if (TERMINAL.has(n.status)) {
      if (!TERMINAL.has(state)) {
        // A terminal child never returns to running
        this.deps.log(`subagent ${this.label(n)} is ${n.status}; wire state ${state} ignored`);
        return;
      }
      if (n.stateSource === 'agent') {
        // The agent MUST NOT send further updates after its terminal one (a same-state repeat is a benign replay)
        if (n.status !== state) this.deps.log(`subagent ${this.label(n)} is ${n.status}; wire state ${state} ignored`);
        return;
      }
      // A local 'disconnected' yields to the agent's late or replayed terminal word
    }
    n.status = state;
    n.stateSource = 'agent';
    if (TERMINAL.has(state)) this.sealNode(n, state);
    this.bump(n);
  }

  private sealNode(n: SubagentNode, state: SubagentState) {
    n.endedAt ??= this.now();
    // A native child's result is what the child last said — the parent receives no receipt for it
    if (n.visibility === 'session' && n.result === undefined) {
      for (let i = n.state.turns.length - 1; i >= 0 && n.result === undefined; i--) {
        const turn = n.state.turns[i];
        if (turn?.role !== 'agent') continue;
        for (let j = turn.blocks.length - 1; j >= 0; j--) {
          const b = turn.blocks[j];
          if (b?.type === 'text' && b.markdown.trim()) { n.result = b.markdown; break; }
        }
      }
    }
    const last = n.state.turns[n.state.turns.length - 1];
    // Already sealed (a local settle ran first): keep the transcript exactly as it is
    if (last?.role === 'agent' && last.stop === undefined) {
      endTurn(n.state, state === 'cancelled' || state === 'disconnected' ? 'cancelled' : 'end_turn');
    }
  }

  // ---- child-stream updates ----

  applyChild(n: SubagentNode, u: acp.SessionUpdate, ctx: RootRouteCtx) {
    switch (u.sessionUpdate) {
      // A child session's own session-level noise never becomes transcript content
      case 'user_message_chunk':
      case 'session_info_update':
      case 'available_commands_update':
      case 'current_mode_update':
      case 'config_option_update':
        return;
      case 'usage_update':
        n.usage = { used: u.used, size: u.size };
        this.bump(n);
        return;
    }
    // Claude native mirror link: when the root receipt lacked toolResponse, the child's own
    // updates still name the parent's Task call — take the link from whichever side arrives
    if (n.visibility === 'session' && n.peer.toolCallId === undefined) {
      const link = str(record(record(u._meta)?.claudeCode)?.parentToolUseId);
      if (link !== undefined) {
        n.peer.toolCallId = link;
        this.byPeerTool.set(link, n);
        const block = ctx.findRootTool(link);
        if (block !== undefined) block.subagentId = n.id;
      }
    }
    if (u.sessionUpdate === 'tool_call' || u.sessionUpdate === 'tool_call_update') this.toolOwner.set(u.toolCallId, n.id);
    applyUpdate(n.state, u);
    n.toolCount = toolBlocks(n.state.turns).length;
    this.bump(n);
  }

  bufferOrphan(peerSessionId: string, u: acp.SessionUpdate) {
    let list = this.orphans.get(peerSessionId);
    if (list === undefined) {
      if (this.orphans.size >= ORPHAN_MAX_IDS) { this.logOrphanDrop(); return; }
      list = [];
      this.orphans.set(peerSessionId, list);
    }
    if (list.length >= ORPHAN_MAX_UPDATES) { this.logOrphanDrop(); return; }
    list.push(u);
  }

  private logOrphanDrop() {
    if (this.orphanLogged) return;
    this.orphanLogged = true;
    this.deps.log('subagent updates dropped: too many buffered for unannounced sessions');
  }

  // ---- root-stream routing: the nested / receipt dialects ----

  routeRoot(u: acp.SessionUpdate, ctx: RootRouteCtx): 'consumed' | 'root' {
    const meta = record(u._meta);
    // Devin: every child-side update carries the parent agent link
    const parentAgentId = str(record(meta?.['cognition.ai/subagent_context'])?.parentAgentId);
    if (parentAgentId !== undefined && parentAgentId !== 'root') {
      const n = this.byPeerAgent.get(parentAgentId);
      if (n === undefined) {
        this.deps.log(`${u.sessionUpdate} for unknown subagent ${parentAgentId} left on the root`);
        return 'root';
      }
      this.applyChild(n, u, ctx);
      return 'consumed';
    }
    if (u.sessionUpdate !== 'tool_call' && u.sessionUpdate !== 'tool_call_update') return 'root';
    // A call already routed to a child keeps routing there even without _meta
    const owner = this.toolOwner.get(u.toolCallId);
    if (owner !== undefined) {
      const n = this.nodes.find(x => x.id === owner);
      if (n !== undefined) { this.applyChild(n, u, ctx); return 'consumed'; }
      this.toolOwner.delete(u.toolCallId);
    }
    // Devin lifecycle updates addressed to the agent id (toolCallId === agentId)
    const started = record(meta?.['cognition.ai/subagent_started']);
    if (started !== undefined) { this.devinStarted(started, ctx); return 'consumed'; }
    const completed = record(meta?.['cognition.ai/subagent_completed']);
    if (completed !== undefined) { this.devinCompleted(completed); return 'consumed'; }
    const otherSubMeta = Object.keys(meta ?? {}).find(k => k.startsWith('cognition.ai/subagent_'));
    if (otherSubMeta !== undefined && this.byPeerAgent.has(u.toolCallId)) {
      this.deps.log(`unhandled ${otherSubMeta} on subagent ${u.toolCallId}`);
      return 'consumed';
    }
    // Claude native: an async_launched receipt links the root call to the child's session id
    const claude = record(meta?.claudeCode);
    const toolResponse = record(claude?.toolResponse);
    if (toolResponse?.isAsync === true && str(toolResponse.agentId) !== undefined) {
      const agentId = str(toolResponse.agentId)!;
      const n = this.byPeerSession.get(agentId);
      if (n !== undefined) {
        n.peer.toolCallId = u.toolCallId;
        this.byPeerTool.set(u.toolCallId, n);
        n.model ??= str(toolResponse.resolvedModel);
        n.title ??= str(toolResponse.description);
        this.bump(n);
      } else {
        this.pendingLaunches.set(agentId, {
          toolCallId: u.toolCallId,
          ...(str(toolResponse.resolvedModel) !== undefined ? { model: str(toolResponse.resolvedModel) } : {}),
          ...(str(toolResponse.description) !== undefined ? { title: str(toolResponse.description) } : {}),
        });
      }
      return 'root';
    }
    // Claude legacy: child tool calls ride the root stream under claudeCode.parentToolUseId
    const parentToolUseId = str(claude?.parentToolUseId);
    if (parentToolUseId !== undefined) {
      const n = this.byPeerTool.get(parentToolUseId);
      if (n !== undefined) { this.applyChild(n, u, ctx); return 'consumed'; }
      return 'root';
    }
    // Devin run_subagent delegation calls stay on the root but are remembered for the subagent_started link
    if (str(meta?.['cognition.ai/inferenceToolName']) === 'run_subagent') {
      const raw = record(u.rawInput);
      if (raw !== undefined) {
        const d: PendingDelegation = { toolCallId: u.toolCallId };
        const title = str(raw.title);
        const task = str(raw.task);
        if (title !== undefined) d.title = title;
        if (task !== undefined) d.task = task;
        this.pendingDelegations.push(d);
      }
    }
    return 'root';
  }

  private devinStarted(started: Record<string, unknown>, ctx: RootRouteCtx) {
    const agentId = str(started.agentId);
    if (agentId === undefined) { this.deps.log('subagent_started without agentId dropped'); return; }
    let n = this.byPeerAgent.get(agentId);
    if (n === undefined) {
      n = {
        id: randomUUID(),
        turnIndex: ctx.turnIndex,
        visibility: 'nested',
        status: 'running',
        stateSource: 'agent',
        controls: { cancel: false },
        announcedAt: this.now(),
        toolCount: 0,
        peer: { agentId },
        state: emptyState(),
        rev: 0,
        dialect: 'devin',
      };
      n.state.turns.push({ role: 'agent', startedAt: n.announcedAt, blocks: [] });
      this.nodes.push(n);
      this.byPeerAgent.set(agentId, n);
    }
    const title = str(started.title);
    const task = str(started.task);
    const profile = str(started.profile);
    const model = str(started.model);
    if (title !== undefined) n.title = title;
    if (task !== undefined) n.task = task;
    if (profile !== undefined) n.role = profile;
    if (model !== undefined) n.model = model;
    if (started.isBackground === true) n.background = true;
    n.meta = started;
    // Link to the run_subagent delegation call: title always matches, task only when both carry one
    const di = this.pendingDelegations.findIndex(d => d.title === title && (d.task === undefined || task === undefined || d.task === task));
    if (di >= 0) {
      const d = this.pendingDelegations.splice(di, 1)[0]!;
      n.peer.toolCallId = d.toolCallId;
      this.byPeerTool.set(d.toolCallId, n);
      const block = ctx.findRootTool(d.toolCallId);
      if (block !== undefined) block.subagentId = n.id;
    } else if (n.peer.toolCallId === undefined) {
      this.deps.log(`subagent ${agentId} has no matching run_subagent call`);
    }
    this.bump(n);
  }

  private devinCompleted(done: Record<string, unknown>) {
    const agentId = str(done.agentId);
    const n = agentId !== undefined ? this.byPeerAgent.get(agentId) : undefined;
    if (n === undefined) { this.deps.log(`subagent_completed for unknown agent ${agentId ?? '?'}`); return; }
    const summary = str(done.summary);
    if (summary !== undefined) n.result = summary;
    this.transition(n, done.success === false ? 'failed' : 'completed');
    this.bump(n);
  }

  // After a root tool_call/tool_call_update is applied: stamp delegation links, keep nested/receipt nodes in sync
  annotateRoot(block: ToolCallBlock | undefined, u: acp.SessionUpdate, ctx: RootRouteCtx) {
    if (block === undefined || (u.sessionUpdate !== 'tool_call' && u.sessionUpdate !== 'tool_call_update')) return;
    const meta = record(u._meta);
    const raw = record(u.rawInput);
    // Devin read_subagent: the parent blocking on a child — it is what the parent is doing, so the row stays
    if (str(meta?.['cognition.ai/inferenceToolName']) === 'read_subagent') {
      block.verbKey = 'verb.awaitSubagent';
      block.verb = t('verb.awaitSubagent');
      const n = str(raw?.agent_id) !== undefined ? this.byPeerAgent.get(str(raw!.agent_id)!) : undefined;
      if (n?.title !== undefined) { block.target = n.title; delete block.targetMono; }
      return;
    }
    const claude = record(meta?.claudeCode);
    let n = this.byPeerTool.get(u.toolCallId);
    if (claude?.subagent === true) {
      n ??= this.upsertCallNode(u.toolCallId, 'nested', ctx);
      n.dialect = 'claude';
    } else if (n === undefined && raw !== undefined && str(raw.subagent_type) !== undefined && str(raw.prompt) !== undefined) {
      // Generic delegation receipt (Kimi's Agent tool)
      n = this.upsertCallNode(u.toolCallId, 'receipt', ctx);
    }
    if (n === undefined) return;
    block.subagentId = n.id;
    if (n.visibility === 'session') {
      // The launch receipt is the delegation call returning (async_launched); the adapter never sends
      // a terminal status for it, so an unswept pending row would be marked failed at turn end
      if (block.status === 'pending' || block.status === 'in_progress') {
        block.status = 'completed';
        if (block.startedAt !== undefined) block.endedAt ??= this.now();
      }
      this.bump(n);
      return;
    }
    block.verbKey = 'verb.delegate';
    block.verb = t('verb.delegate');
    const description = str(raw?.description);
    const prompt = str(raw?.prompt);
    const subagentType = str(raw?.subagent_type);
    const model = str(raw?.model);
    if (description !== undefined) n.title = description;
    if (prompt !== undefined) n.task = prompt;
    if (subagentType !== undefined) n.role = subagentType;
    if (model !== undefined) n.model = model;
    // Devin's run_subagent call returning only acknowledges the delegation — a background child keeps running
    // until subagent_completed says otherwise
    if (n.dialect === 'devin') { this.bump(n); return; }
    if (u.status === 'completed') {
      const toolResponse = record(claude?.toolResponse);
      if (n.dialect === 'claude' && toolResponse?.isAsync === true) {
        n.background = true;
        const agentId = str(toolResponse.agentId);
        if (agentId !== undefined) { n.peer.agentId = agentId; this.byPeerAgent.set(agentId, n); }
        this.transition(n, 'running');
      } else {
        n.result = this.resultText(u, block) ?? n.result;
        this.transition(n, 'completed');
      }
    } else if (u.status === 'failed') {
      this.transition(n, 'failed');
    } else if (u.status as unknown === 'cancelled') {
      // The SDK union has no 'cancelled' tool status; agents send it anyway
      this.transition(n, 'cancelled');
    }
    this.bump(n);
  }

  private upsertCallNode(toolCallId: string, visibility: SubagentVisibility, ctx: RootRouteCtx): SubagentNode {
    let n = this.byPeerTool.get(toolCallId);
    if (n === undefined) {
      n = {
        id: randomUUID(),
        turnIndex: ctx.turnIndex,
        visibility,
        status: 'running',
        stateSource: 'agent',
        controls: { cancel: false },
        announcedAt: this.now(),
        toolCount: 0,
        peer: { toolCallId },
        state: emptyState(),
        rev: 0,
      };
      n.state.turns.push({ role: 'agent', startedAt: n.announcedAt, blocks: [] });
      this.nodes.push(n);
      this.byPeerTool.set(toolCallId, n);
    }
    return n;
  }

  private resultText(u: acp.ToolCall | acp.ToolCallUpdate, block: ToolCallBlock): string | undefined {
    const raw = record(u.rawOutput) ?? u.rawOutput;
    if (typeof raw === 'string' && raw.trim()) return raw;
    if (Array.isArray(raw)) {
      const parts = raw.map(c => { const r = record(c); return r?.type === 'text' && typeof r.text === 'string' ? r.text : ''; }).filter(s => s.trim());
      if (parts.length) return parts.join('\n');
    }
    const content = u.content;
    if (Array.isArray(content)) {
      const parts = content.map(item => {
        const c = record(item);
        const inner = record(c?.content);
        return c?.type === 'content' && inner?.type === 'text' && typeof inner.text === 'string' ? inner.text : '';
      }).filter(s => s.trim());
      if (parts.length) return parts.join('\n');
    }
    return block.content?.type === 'text' ? block.content.text : undefined;
  }

  // ---- end of the connection / turn ----

  settle(reason: 'prompt-returned' | 'connection-lost' | 'disposed') {
    for (const n of this.nodes) {
      if (n.status !== 'running') continue;
      // Never claim failed / cancelled for an outcome the agent did not report
      n.status = 'disconnected';
      n.stateSource = 'local';
      n.endedAt = this.now();
      endTurn(n.state, 'cancelled');
      this.bump(n);
    }
    const dropped = [...this.orphans.values()].reduce((a, l) => a + l.length, 0);
    if (dropped > 0) this.deps.log(`${dropped} buffered subagent update(s) dropped on ${reason}`);
    this.orphans.clear();
    this.orphanLogged = false;
    this.pendingDelegations.length = 0;
    this.pendingLaunches.clear();
    this.toolOwner.clear();
  }

  // Edit / retry rewrote history at turnIndex: nodes announced in the removed turns go with them, along with the
  // routing state that belonged to those turns
  truncate(turnIndex: number) {
    // Pending delegations / launches belong to the turn being rewritten — drop them even when no node goes
    this.pendingDelegations.length = 0;
    this.pendingLaunches.clear();
    const removed = new Set(this.nodes.filter(n => n.turnIndex >= turnIndex).map(n => n.id));
    if (removed.size === 0) return;
    this.nodes = this.nodes.filter(n => !removed.has(n.id));
    for (const [toolCallId, nodeId] of this.toolOwner) if (removed.has(nodeId)) this.toolOwner.delete(toolCallId);
    this.reindex();
  }

  // ---- views ----

  private activityLabel(n: SubagentNode): string | undefined {
    if (n.status !== 'running') return undefined;
    return activityOf(n.state.turns)?.label;
  }

  summaries(): SubagentSummary[] {
    return this.nodes.map(n => {
      if (n.cached === undefined || n.cached.rev !== n.rev) {
        const permissions: PermissionBlock[] = [];
        let question: QuestionBlock | undefined;
        for (const turn of n.state.turns) {
          if (turn.role !== 'agent') continue;
          for (const b of turn.blocks) {
            if (b.type === 'permission') permissions.push(b);
            else if (b.type === 'question' && b.outcome === undefined) question = b;
          }
        }
        const activity = this.activityLabel(n);
        const summary: SubagentSummary = {
          id: n.id,
          turnIndex: n.turnIndex,
          visibility: n.visibility,
          state: n.status,
          stateSource: n.stateSource,
          controls: { cancel: n.controls.cancel },
          announcedAt: n.announcedAt,
          peer: { ...n.peer },
          toolCount: n.toolCount,
        };
        if (n.parentId !== undefined) summary.parentId = n.parentId;
        if (n.title !== undefined) summary.title = n.title;
        if (n.task !== undefined) summary.task = n.task;
        if (n.role !== undefined) summary.role = n.role;
        if (n.cancelRequested) summary.cancelRequested = true;
        if (n.background) summary.background = true;
        if (n.endedAt !== undefined) summary.endedAt = n.endedAt;
        if (n.model !== undefined) summary.model = n.model;
        if (n.usage !== undefined) summary.usage = { ...n.usage };
        if (activity !== undefined) summary.activity = activity;
        if (n.result !== undefined) summary.result = n.result;
        if (permissions.length > 0) summary.permissions = permissions;
        if (question !== undefined) summary.question = question;
        n.cached = { rev: n.rev, summary };
      }
      return n.cached.summary;
    });
  }

  transcript(id: string): { turns: Turn[]; rev: number; running: boolean } | undefined {
    const n = this.nodes.find(x => x.id === id);
    if (n === undefined) return undefined;
    return { turns: n.state.turns, rev: n.rev, running: n.status === 'running' };
  }

  cancel(id: string): { peerSessionId: string } | undefined {
    const n = this.nodes.find(x => x.id === id);
    if (n === undefined || !n.controls.cancel || n.status !== 'running' || n.peer.sessionId === undefined) return undefined;
    n.cancelRequested = true;
    this.bump(n);
    return { peerSessionId: n.peer.sessionId };
  }

  toRecords(): SubagentRecord[] {
    return this.nodes.map(n => {
      const r: SubagentRecord = {
        id: n.id,
        turnIndex: n.turnIndex,
        visibility: n.visibility,
        state: n.status,
        stateSource: n.stateSource,
        controls: { cancel: n.controls.cancel },
        announcedAt: n.announcedAt,
        peer: { ...n.peer },
        toolCount: n.toolCount,
        turns: n.state.turns,
      };
      if (n.parentId !== undefined) r.parentId = n.parentId;
      if (n.title !== undefined) r.title = n.title;
      if (n.task !== undefined) r.task = n.task;
      if (n.role !== undefined) r.role = n.role;
      if (n.cancelRequested) r.cancelRequested = true;
      if (n.background) r.background = true;
      if (n.endedAt !== undefined) r.endedAt = n.endedAt;
      if (n.model !== undefined) r.model = n.model;
      if (n.usage !== undefined) r.usage = { ...n.usage };
      if (n.result !== undefined) r.result = n.result;
      return r;
    });
  }
}
