// Normalized shape of the transcript: host-side normalize.ts reduces ACP session/update into these blocks; the webview only understands these

import type { SubagentSummary } from './subagents';
import type { ModelMemory, ModelShapes } from './modelShapes';
import type { AccountAction, AccountInfo, AgentId, AgentInfo, AgentInstallProgress, AuthMethodInfo, ForkOrigin, GoalAction, QueuedPrompt, SessionControls, SessionGoal, SessionStatus, SlashCommand, Turn, Usage } from './transcriptCore';

export type * from './transcriptCore';

// Public account metadata from the SSH engine that owns a session; secrets never cross the peer wire.
export interface RemoteAuth {
  agent: AgentInfo;
  // The host's CLI availability for new sessions in this folder; absent on older peers.
  agents?: AgentInfo[];
  accounts: AccountInfo[];
  action?: AccountAction;
  install?: AgentInstallProgress;
}

// Everything a session looks like to the webview: the host pushes the whole thing on every change (the transcript is small, not worth diffing)
// What a windowed view leaves out ahead of its first turn: `turns` whole turns, and the first `blocks` blocks of the first
// turn it does hold (an agent turn)
export interface ViewEarlier {
  turns: number;
  blocks: number;
}

export interface SessionView {
  id: string;
  remoteAuth?: RemoteAuth;
  agent: AgentId;
  // Bound account (only agents on the account layer have one); a session uses a single account from start to finish
  accountId?: string;
  title: string;
  cwd: string;
  status: SessionStatus;
  error?: string;
  // The error is a native session lock held by an agent another Acpira sidecar left running (takeOverSession ends it), or
  // this is a read-only copy of a session another Acpira engine has open (takeOverSession asks it to hand the session over)
  canTakeOver?: boolean;
  authMethods?: AuthMethodInfo[];
  turns: Turn[];
  // Only in a view sent to a page that said `window` in ready (sessionWindow.ts): what it leaves out ahead of turns[0].
  // Turn indices everywhere else (editTurn, forkSession, SubagentSummary.turnIndex, search hits) count from the session's
  // first turn: such a page adds `earlier.turns` to its own
  earlier?: ViewEarlier;
  running: boolean;
  // Monotonic snapshot id for this process. Turns are mutated in place and postMessage clones
  // asynchronously, so an older running:true payload can arrive after settle; the webview drops a
  // lower or equal rev. Not persisted.
  rev?: number;
  controls: SessionControls;
  // The agent's remembered per-model parameters (shared/modelShapes.ts), for the history editor's local model switch
  modelShapes?: ModelShapes;
  // What each option of the sidekick select runs with once picked, keyed by option id: its effort select and Fast switch
  // as they would read, so a sidekick row offers its parameters before it is picked
  sidekickShapes?: ModelShapes;
  // What the user last ran each of this agent's models with (engine prefs): `recent` holds model option ids, newest
  // first; `picks` the values of the model's own parameters (reasoning level, model_config) per model option id
  modelMemory?: ModelMemory;
  usage?: Usage;
  commands: SlashCommand[];
  // Prompts sent while a turn was in progress, in send order; the first goes out when the turn ends
  queued?: QueuedPrompt[];
  // The agent takes `_session/steering` and honours its idle fallback: a queued prompt can join the running turn
  canSteer?: boolean;
  // A running command row can be stopped on its own (`stopCommand`), the turn going on: the engine finds the command's
  // process under the agent's (macOS / Linux) or cancels the remote call (an SSH workspace)
  canStopCommands?: boolean;
  // The agent's current goal; a completed one stays only until its turn ends
  goal?: SessionGoal;
  // The goal controls the running agent advertises; absent when it has no goal support (or no process yet)
  goalActions?: GoalAction[];
  // Delegated child nodes announced during this session; each carries its own transcript via the `subagent` message
  subagents?: SubagentSummary[];
  // A fork whose copied turns are still in its transcript: the source session, the last copied turn's index, and the
  // source's agent when the fork moved to another one; the thread draws the boundary under that turn
  forkedFrom?: ForkOrigin;
  createdAt: string;
  updatedAt: string;
}
