import type { AgentId, AgentInfo, AgentTurn, SessionControls, TurnSettings, TurnUsage } from '@shared/transcript';
import { launchable } from '@shared/agentOrder';
import { commandDuration } from './commandDuration';
import { splitCodexBlocks } from './folding';

// The reply text the copy action hands out: what renders outside the process fold (commentary before a tool call is
// process history, not part of the reply). A turn whose tail heuristic finds nothing falls back to its plain text blocks.
export function replyMarkdown(turn: AgentTurn): string {
  const reply = splitCodexBlocks(turn.blocks).reply.map(b => b.markdown).join('\n\n').trim();
  if (reply) return reply;
  return turn.blocks.filter(b => b.type === 'text').map(b => b.markdown).join('\n\n').trim();
}

// The model that answered: the peer's own id when it reported one (Grok _meta.modelId), else the selection captured on
// the user turn. Resolved through the model control's option list for the display name; an unknown id shows as-is
export function modelLabel(usage: TurnUsage | undefined, settings: TurnSettings | undefined, controls: SessionControls): string | undefined {
  const control = controls.options.find(c => c.category === 'model');
  const id = usage?.model ?? (control ? settings?.config[control.id] : undefined);
  if (!id) return undefined;
  return control?.options.find(o => o.id === id)?.name ?? id;
}

// Time from sending the prompt to the first streamed block (TTFT). Only live-streamed turns carry both timestamps;
// replayed / imported history has neither, and a failed turn that never streamed has no `respondedAt`.
export function turnTtft(turn: AgentTurn): string | undefined {
  if (turn.startedAt === undefined || turn.respondedAt === undefined || turn.respondedAt < turn.startedAt) return undefined;
  return commandDuration(turn.respondedAt - turn.startedAt);
}

// Total generated tokens for the turn: `output` already includes `reasoning` on OpenAI-style endpoints (`input + output === total`),
// while peers that report `thoughtTokens` separately have `reasoning > output` or `input + output + reasoning === total`.
function generatedTokens(u: TurnUsage | undefined): number | undefined {
  if (!u || u.output === undefined) return undefined;
  const r = u.reasoning ?? 0;
  if (r <= 0) return u.output;
  if (r > u.output) return u.output + r;
  if (u.total !== undefined && u.input !== undefined && Math.abs(u.input + u.output + r - u.total) < Math.abs(u.input + u.output - u.total)) {
    return u.output + r;
  }
  return u.output;
}

// Foreground tool execution time inside `[from, to]`, with overlapping parallel tool runs merged so concurrent calls
// are subtracted once rather than per call.
function foregroundToolMs(turn: AgentTurn, from: number, to: number): number {
  const spans: [number, number][] = [];
  for (const b of turn.blocks) {
    if (b.type !== 'tool_call' || b.background || b.startedAt === undefined || b.endedAt === undefined) continue;
    const s = Math.max(from, b.startedAt);
    const e = Math.min(to, b.endedAt);
    if (e > s) spans.push([s, e]);
  }
  if (spans.length === 0) return 0;
  spans.sort((a, b) => a[0] - b[0]);
  let total = 0;
  let [curS, curE] = spans[0]!;
  for (let i = 1; i < spans.length; i++) {
    const [s, e] = spans[i]!;
    if (s <= curE) curE = Math.max(curE, e);
    else { total += curE - curS; curS = s; curE = e; }
  }
  return total + (curE - curS);
}

// Per-request decode throughput (`(tokens - 1) / decodeSeconds`, NVIDIA AIPerf's Output Token Throughput Per User),
// measured from the first streamed block (`respondedAt`) to `endedAt` minus foreground tool execution. Omitted when
// fewer than 5 tokens were generated or the active decode window is under 200 ms (single-chunk / buffered dumps).
const MIN_TPS_TOKENS = 5;
const MIN_DECODE_MS = 200;

export function turnOutputTps(turn: AgentTurn): number | undefined {
  if (turn.respondedAt === undefined || turn.endedAt === undefined) return undefined;
  const tokens = generatedTokens(turn.usage);
  if (tokens === undefined || tokens < MIN_TPS_TOKENS) return undefined;
  const activeMs = turn.endedAt - turn.respondedAt - foregroundToolMs(turn, turn.respondedAt, turn.endedAt);
  if (activeMs < MIN_DECODE_MS) return undefined;
  const tps = Math.round(((tokens - 1) * 1000) / activeMs);
  return tps > 0 ? tps : undefined;
}

// The fork menu's agents: the ones the plus menu offers, in its order, with the session's own agent first (forking
// onto it is the plain fork). An agent the list no longer has (switched off since) is still offered first
export function forkTargets(agents: AgentInfo[], current: AgentId): AgentInfo[] {
  const open = launchable(agents);
  const own = open.find(a => a.id === current) ?? agents.find(a => a.id === current);
  return own ? [own, ...open.filter(a => a.id !== current)] : open;
}
