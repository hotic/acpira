// A session's context window split by what fills it (the composer ring's card, the tool panel's context report tab). Only an agent
// that knows what it sent reports one: the built-in agent splits the request it built, part by part, and scales its own
// size estimates to the provider's count, so the parts add up to the figure the ring shows. For any other agent the
// engine estimates one from the transcript since the last compaction, scaled into the agent's figure, the remainder one
// `rest` part (`acpira-host/src/acp/session/context_estimate.rs`), always marked estimated. Rust mirror:
// `acpira-shared/src/context.rs`

// In the order a request carries them; `agent` is what an agent that reports no split keeps of its own, unsplit
export type ContextCategory = 'agent' | 'system' | 'tools' | 'docs' | 'skills' | 'mcp' | 'chat';
export const CONTEXT_CATEGORIES: readonly ContextCategory[] = ['agent', 'system', 'tools', 'docs', 'skills', 'mcp', 'chat'];

// What a node is, which picks its label and icon:
//   system:  base (the agent's instructions), environment, prompt (a system prompt a plugin rewrote, whole)
//   docs:    doc (an AGENTS.md file, `name` its path)
//   tools:   tool (a built-in tool's definition, `name`)
//   skills:  skill (one listed skill, `name`), skillTool (the skill tool's own text and schema)
//   mcp:     server (`name`), mcpTool (`name` the tool's own name)
//   chat:    summary (earlier turns compacted into one message), turn (`title` its prompt, `index` from 1), and in a turn
//            message (the user's words, steered messages too), note (what the agent stacked in front: mode, MCP
//            servers, sidekick, goal), thinking, reply, call (`name` the tool, `title` what it worked on, `toolKind`)
//   rest:    a category known only as a sum (the engine's estimate, `context_estimate.rs`): in agent, what an agent
//            that reports no split keeps of its own (system prompt, tool definitions, instruction files, a compaction
//            summary), the figure less the transcript; in another category, the sum the agent reported (Grok)
export type ContextNodeKind =
  | 'base' | 'environment' | 'prompt' | 'rest'
  | 'doc'
  | 'tool'
  | 'skill' | 'skillTool'
  | 'server' | 'mcpTool'
  | 'summary' | 'turn' | 'message' | 'note' | 'thinking' | 'reply' | 'call';

export interface ContextNode {
  // Stable while the conversation it describes stays as it is: the key a leaf's content is asked by
  id: string;
  kind: ContextNodeKind;
  name?: string;
  title?: string;
  // Turns count from the session's first, compacted ones included
  index?: number;
  // ACP tool kind of a call (read, edit, execute, search, fetch, other, ...), for its icon and verb
  toolKind?: string;
  // A group's tokens are its children's sum
  tokens: number;
  // Of `tokens`, what the provider served from its prompt cache
  cached?: number;
  children?: ContextNode[];
}

export interface ContextCategorySplit {
  id: ContextCategory;
  tokens: number;
  cached?: number;
  // Only in a detailed report (the report tab); the card takes the sums
  nodes?: ContextNode[];
}

export interface ContextReport {
  // The figure the ring shows (the last request and its reply) and the model's window
  used: number;
  size: number;
  // No provider count to scale by yet (a request since compaction, a provider that reported none): the parts are the
  // agent's own estimates
  estimated?: boolean;
  // Of `used`, what the provider served from its prompt cache on the last request
  cached?: number;
  // When the last request went out (ms since the epoch)
  at?: number;
  // The conversation this describes; a content request names it, and one for another gets nothing
  rev: string;
  categories: ContextCategorySplit[];
}

// What a leaf sent, as text the report shows (each piece cut at a limit the agent sets, then `truncated`)
export type ContextContent =
  | { kind: 'text'; text: string; truncated?: boolean }
  // A tool definition: its description and parameter schema
  | { kind: 'def'; description: string; schema: unknown; truncated?: boolean }
  // A tool call: its arguments as the model wrote them, and its result (none yet while the call runs); images are counted,
  // never sent
  | { kind: 'call'; input: string; output?: string; images?: number; error?: boolean; truncated?: boolean };

// The last request exactly as the agent sent it, the provider's wire format (images and signatures elided)
export interface ContextRaw {
  format: string;
  url?: string;
  body: unknown;
}

// A node's tokens, groups summed (the agent sends them summed already; this keeps pure logic independent of that)
export function nodeTokens(n: ContextNode): number {
  return n.children ? n.children.reduce((s, c) => s + nodeTokens(c), 0) : n.tokens;
}
