import type { AgentTurn, SessionControls, TurnSettings, TurnUsage } from '@shared/transcript';
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

export function toolCallCount(turn: AgentTurn): number {
  return turn.blocks.filter(b => b.type === 'tool_call').length;
}
