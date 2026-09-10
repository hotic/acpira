import type * as acp from '@agentclientprotocol/sdk';
import type { PlanDocumentBlock, Turn } from '@shared/transcript';

type ToolUpdate = acp.ToolCall | acp.ToolCallUpdate;
const record = (v: unknown): Record<string, unknown> => v && typeof v === 'object' ? v as Record<string, unknown> : {};
const str = (v: unknown): string | undefined => typeof v === 'string' ? v : undefined;

export function planDocuments(turns: Turn[]): PlanDocumentBlock[] {
  return turns.flatMap(t => t.role === 'agent' ? t.blocks.filter((b): b is PlanDocumentBlock => b.type === 'plan_document') : []);
}

export function setPlanContent(plan: PlanDocumentBlock, markdown: string) {
  // YAML metadata belongs to the file; the preview starts at the document body.
  plan.markdown = markdown.replace(/^---\r?\n[\s\S]*?\r?\n---\r?\n/, '').trim();
  plan.title = /^#\s+(.+)$/m.exec(plan.markdown)?.[1] ?? 'Plan';
}

export function isPlanApproval(u: ToolUpdate): boolean {
  return u._meta?.['acpira/planApproval'] === true || u._meta?.['cognition.ai/isExitPlan'] === true
    || u._meta?.['cognition.ai/inferenceToolName'] === 'exit_plan_mode'
    || /^(exit_plan_mode|ExitPlanMode)$/.test(u.title ?? '');
}

// Keep full plan content before tool normalization reduces a diff to display lines.
// Only vendor plan paths or explicit plan metadata classify a write as a plan.
export function capturePlan(turns: Turn[], u: ToolUpdate): PlanDocumentBlock | undefined {
  const plans = planDocuments(turns);
  const raw = record(u.rawInput);
  const meta = u._meta ?? {};
  const diff = u.content?.find(c => c.type === 'diff');
  const text = u.content?.flatMap(c => c.type === 'content' && c.content.type === 'text' ? [c.content.text] : []).join('\n') ?? '';
  const saved = /^Plan saved to: (.+)\r?\n\r?\n([\s\S]*)/.exec(text);
  const ready = record(record(u.rawOutput).PlanReady);
  const path = str(meta['cognition.ai/planFilePath']) ?? str(ready.plan_file_path)
    ?? saved?.[1] ?? str(raw.file_path) ?? str(raw.path) ?? diff?.path ?? u.locations?.[0]?.path;
  const exit = isPlanApproval(u) || plans.some(p => p.approvalToolCallId === u.toolCallId);
  const write = meta['cognition.ai/isPlanFileEdit'] === true || meta['cognition.ai/inferenceToolName'] === 'write_plan';
  const knownPath = path && /\/(?:\.grok\/sessions\/.*\/plan\.md|\.kimi-code\/sessions\/.*\/plans\/[^/]+\.md)$/.test(path);
  let p = plans.find(p => p.toolCallId === u.toolCallId || p.approvalToolCallId === u.toolCallId || (path && p.path === path));
  if (!p && !exit && !knownPath && !write) return;
  // Devin can announce exit_plan_mode with rawInput.plan before write_plan's
  // packets arrive. Attach that first file to the single unbound approval in
  // this turn, preserving the decision already made on its inline preview.
  let fillsApproval = false;
  const last = turns.at(-1);
  if (!p && !exit && write && last?.role === 'agent') {
    const unbound = last.blocks.filter((b): b is PlanDocumentBlock => b.type === 'plan_document'
      && !b.path && !!b.approvalToolCallId && b.toolCallId === b.approvalToolCallId);
    if (unbound.length === 1) {
      p = unbound[0]!;
      p.toolCallId = u.toolCallId;
      fillsApproval = true;
    }
  }
  if (!p && exit && !path) p = plans.at(-1);
  const markdown = str(raw.planContent) ?? str(ready.plan_content) ?? saved?.[2]
    ?? diff?.newText ?? str(raw.content) ?? (exit && !p?.markdown ? str(raw.plan) : undefined);
  if (!p) {
    if (last?.role !== 'agent') return;
    p = { type: 'plan_document', id: `plan-${u.toolCallId}`, title: 'Plan', markdown: '', path, toolCallId: u.toolCallId, status: 'draft' };
    last.blocks.push(p);
  }
  if (path) p.path = path;
  if (markdown !== undefined) {
    const before = p.markdown;
    setPlanContent(p, markdown);
    if (!exit && !fillsApproval && p.markdown !== before) { p.status = 'draft'; p.toolCallId = u.toolCallId; }
  }
  if (exit) {
    p.approvalToolCallId = u.toolCallId;
    if (p.markdown && p.status === 'draft') p.status = 'ready';
  }
  else if (u.status === 'completed' && p.status === 'draft') p.status = 'ready';
  return p;
}
