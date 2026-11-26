import { createContext } from 'react';
import type { AgentId, AgentInfo, SessionControls } from '@acpira/shared/transcript';

// What the turn action row needs from the shell: the session it belongs to, the control list for resolving the model
// name, and the fork action with the agents it can continue on — absent in hosts that do not wire it (LAB)
export const TurnActionsContext = createContext<{
  sessionId: string; controls: SessionControls;
  fork?: (turnIndex: number, agent: AgentId) => void; agents?: AgentInfo[]; agentId?: AgentId;
  share?: (turnIndex: number, messageEl: HTMLElement) => void;
} | undefined>(undefined);
