import { readFile, stat } from 'node:fs/promises';
import * as acp from '@agentclientprotocol/sdk';
import type { PermissionBlock, ToolCallBlock } from '@shared/transcript';
import { capturePlan, planDocuments, setPlanContent } from './plans';
import { activityOf, applyUpdate, commandFromRaw, permissionToolUpdate, type NormalizeState } from './normalize';
import { bestAllow } from './sessionErrors';
import { t } from '../i18n';
import { PLAN_PREVIEW_MAX_BYTES } from '../limits';

interface PendingPermission {
  resolve: (r: acp.RequestPermissionResponse) => void;
  blockId: string;
  options: acp.PermissionOption[];
  planId?: string;
  nodeId?: string;
  // The adapter asked for a deny-by-default card (`_meta.permission.defaultToNo`); a buildPlan click without an
  // explicit option then resolves the first reject option, not the first allow
  defaultToNo?: boolean;
}

export interface PermissionGateDeps {
  // Which transcript a request belongs to: the root session id → the root state; a child peer session id → that node's
  // state; bump marks the owning node dirty when a card mutates its transcript
  stateFor: (sessionId: string | undefined) => { state: NormalizeState; nodeId?: string; bump?: () => void } | undefined;
  // Every transcript that may hold cards (root + all child nodes): sweeps run over all of them
  states: () => { state: NormalizeState; bump?: () => void }[];
  touch: () => void;
  log: (line: string) => void;
}

// Holds in-flight permission cards and the yolo auto-approve flag. AcpSession owns orchestration (buildPlan / setMode)
export class PermissionGate {
  private pending = new Map<string, PendingPermission>();
  private permSeq = 0;
  epoch = 0;
  autoApprove = false;

  constructor(private deps: PermissionGateDeps) {}

  bumpEpoch() { this.epoch++; }

  has(blockId: string): boolean { return this.pending.has(blockId); }

  findByPlan(planId: string): PendingPermission | undefined {
    return [...this.pending.values()].find(p => p.planId === planId);
  }

  // When switching into yolo, approve the permission requests already waiting in one go, so the user doesn't have to click through each card
  flush() {
    for (const p of this.pending.values()) this.resolve(p.blockId, bestAllow(p.options));
  }

  cancelAll() {
    for (const p of this.pending.values()) p.resolve({ outcome: { outcome: 'cancelled' } });
    this.pending.clear();
    this.removeBlocks();
  }

  // One subagent was cancelled: its pending cards resolve as cancelled (RFD: the client answers the child's pending requests)
  cancelFor(nodeId: string) {
    for (const p of [...this.pending.values()]) {
      if (p.nodeId !== nodeId) continue;
      this.pending.delete(p.blockId);
      this.removeBlocks(p.blockId);
      p.resolve({ outcome: { outcome: 'cancelled' } });
    }
  }

  resolve(blockId: string, optionId: string) {
    const p = this.pending.get(blockId);
    if (!p) return;
    const option = p.options.find(o => o.optionId === optionId);
    if (!option) return;
    const plan = planDocuments(this.stateOfBlock(blockId)?.turns ?? []).find(b => b.id === p.planId);
    if (plan) plan.status = option.kind.startsWith('allow') ? 'approved' : 'rejected';
    this.pending.delete(blockId);
    this.removeBlocks(blockId);
    p.resolve({ outcome: { outcome: 'selected', optionId } });
    this.deps.touch();
  }

  // Permission request → insert a card into the owning transcript's current assistant turn (root or the child's own)
  // and wait for the webview's answer; if the agent cancels, withdraw the card
  async onPermission(req: acp.RequestPermissionRequest, signal: AbortSignal): Promise<acp.RequestPermissionResponse> {
    if (signal.aborted) return Promise.resolve({ outcome: { outcome: 'cancelled' } });
    const ref = this.deps.stateFor(req.sessionId);
    if (!ref) {
      this.deps.log(`permission request for unknown session ${req.sessionId}`);
      return { outcome: { outcome: 'cancelled' } };
    }
    const epoch = this.epoch;
    const state = ref.state;
    const last = state.turns[state.turns.length - 1];
    // The verb / command on the card is taken from the corresponding tool row; the permission request itself often carries only a title
    const tool = last?.role === 'agent' ? last.blocks.find((b): b is ToolCallBlock => b.type === 'tool_call' && b.id === req.toolCall.toolCallId) : undefined;
    // OpenCode embeds a low-fidelity copy of the call (kind 'other', the parent dir as title); it must not downgrade the block
    applyUpdate(state, permissionToolUpdate(tool, req.toolCall));
    const plan = capturePlan(state.turns, req.toolCall);
    // A resumed Devin session may send only the plan path. Load that exact file
    // before presenting approval; missing files retain the normal permission UI.
    if (plan && !plan.markdown && plan.path) {
      try {
        const file = await stat(plan.path);
        if (file.isFile() && file.size <= PLAN_PREVIEW_MAX_BYTES) {
          setPlanContent(plan, await readFile(plan.path, 'utf8'));
          if (plan.markdown) plan.status = 'ready';
        }
      } catch { /* The permission choices remain usable without a local preview. */ }
    }
    if (signal.aborted || epoch !== this.epoch) return { outcome: { outcome: 'cancelled' } };
    // The owner may have gone terminal while the plan file was being read — a card for it must not appear
    if (this.deps.stateFor(req.sessionId) === undefined) return { outcome: { outcome: 'cancelled' } };
    // yolo: approve directly without showing a card, preferring allow_always so the same tool doesn't keep coming back
    if (this.autoApprove) {
      if (plan?.approvalToolCallId === req.toolCall.toolCallId) plan.status = 'approved';
      return { outcome: { outcome: 'selected', optionId: bestAllow(req.options) } };
    }
    const blockId = `perm-${++this.permSeq}`;
    const raw = req.toolCall.rawInput as Record<string, unknown> | undefined;
    // `_meta.permission` (version 1) carried by the claude / codex adapters: the adapter's own heading and reason
    // beat the generic "needs approval" phrasing; strings only, anything else ignored
    const meta = (req._meta as { permission?: unknown } | undefined)?.permission as Record<string, unknown> | undefined;
    const metaStr = (v: unknown) => typeof v === 'string' && v.trim() ? v.trim() : undefined;
    const metaTitle = meta && meta.version === 1 ? metaStr(meta.title) : undefined;
    const metaDesc = meta && meta.version === 1 ? metaStr(meta.description) : undefined;
    const defaultToNo = meta?.defaultToNo === true ? true : undefined;
    const block: PermissionBlock = {
      type: 'permission', id: blockId,
      planId: plan?.markdown && plan.approvalToolCallId === req.toolCall.toolCallId ? plan.id : undefined,
      title: metaTitle ?? (tool ? t('host.needApprovalFor', { what: `${tool.verb}${tool.kind !== 'execute' && tool.target ? ` ${tool.target}` : ''}` }) : req.toolCall.title ? t('host.needApprovalFor', { what: req.toolCall.title }) : t('host.needApproval')),
      command: commandFromRaw(raw) ?? (tool?.kind === 'execute' ? tool.target : undefined),
      description: metaDesc ?? (typeof raw?.description === 'string' ? raw.description : undefined),
      defaultToNo,
      options: req.options.map(o => ({
        id: o.optionId, label: o.name, kind: o.kind,
        // per-option `_meta.permission.description` (codex-acp annotates what each choice does)
        detail: metaStr((o._meta as { permission?: Record<string, unknown> } | null | undefined)?.permission?.description),
      })),
    };
    if (last?.role === 'agent') { last.blocks.push(block); last.activity = activityOf(state.turns); ref.bump?.(); }
    return new Promise(resolve => {
      this.pending.set(blockId, { resolve, blockId, options: req.options, planId: block.planId, nodeId: ref.nodeId, defaultToNo });
      signal.addEventListener('abort', () => {
        if (!this.pending.delete(blockId)) return;
        this.removeBlocks(blockId);
        resolve({ outcome: { outcome: 'cancelled' } });
        this.deps.touch();
      }, { once: true });
      this.deps.touch();
    });
  }

  private stateOfBlock(blockId: string): NormalizeState | undefined {
    for (const e of this.deps.states()) {
      for (const t of e.state.turns) {
        if (t.role === 'agent' && t.blocks.some(b => b.type === 'permission' && b.id === blockId)) return e.state;
      }
    }
    return undefined;
  }

  removeBlocks(onlyId?: string) {
    for (const e of this.deps.states()) {
      let removed = false;
      for (const t of e.state.turns) {
        if (t.role !== 'agent') continue;
        const before = t.blocks.length;
        t.blocks = t.blocks.filter(b => b.type !== 'permission' || (onlyId !== undefined && b.id !== onlyId));
        removed ||= t.blocks.length !== before;
      }
      if (removed) e.bump?.();
    }
  }
}
