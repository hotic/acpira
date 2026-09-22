// First-class subagent nodes shared by host and webview: one summary per delegated child, whatever the wire dialect

import type { PermissionBlock, QuestionBlock, Turn } from './transcript';

export type SubagentState = 'running' | 'completed' | 'failed' | 'cancelled' | 'disconnected';

// What Acpira can observe of the child: 'session' = the agent streams the child's own updates under its own session id
// (claude-agent-acp native sessions / RFD #1992); 'nested' = the child's tool calls arrive on the parent session with a
// structured parent link (Devin `cognition.ai/subagent_context`, Claude `claudeCode.parentToolUseId`); 'receipt' = only the
// delegation call and its result text (Kimi's Agent tool)
export type SubagentVisibility = 'session' | 'nested' | 'receipt';

export interface SubagentSummary {
  id: string;                 // Acpira's own stable id (randomUUID) — never a peer id
  parentId?: string;          // another summary's id; absent = child of the root session
  turnIndex: number;          // index in SessionView.turns of the root agent turn during which it was announced
  visibility: SubagentVisibility;
  title?: string;             // short label (RFD/claude `name`, Devin `title`, Claude legacy/Kimi `description`)
  task?: string;              // the delegated task text (RFD `task`, Devin `task`, Claude/Kimi `prompt`)
  role?: string;              // Devin `profile`, Claude/Kimi `subagent_type`; absent when not structured
  state: SubagentState;
  stateSource: 'agent' | 'local';   // 'local' = Acpira synthesized it (disconnected on connection end / parent prompt returned first)
  controls: { cancel: boolean };    // default false; only true when the agent said so for this child
  cancelRequested?: boolean;
  background?: boolean;       // the agent said the child runs detached (Devin isBackground, Claude toolResponse.isAsync)
  announcedAt: number;
  endedAt?: number;
  model?: string;             // only when reported (Devin subagent_started.model, Claude toolResponse.resolvedModel / rawInput.model)
  usage?: { used: number; size: number };  // only when a usage_update was attributed to this child
  peer: { sessionId?: string; agentId?: string; toolCallId?: string };  // the agent's own identifiers (diagnostics + replay re-association)
  activity?: string;          // latest activity label maintained host-side
  toolCount: number;
  result?: string;            // final result text the parent received (receipt content / Devin summary); for 'session' visibility the child's own last text
  permissions?: PermissionBlock[];  // this child's pending permission cards, mirrored so the root view can show them with provenance
  question?: QuestionBlock;         // this child's open question card, same reason
}

export interface SubagentRecord extends Omit<SubagentSummary, 'permissions' | 'question'> {
  turns: Turn[];
  rev?: number;   // the node's dirty counter at write time; absent on records written before it was persisted
}
