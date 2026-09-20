import * as acp from '@agentclientprotocol/sdk';
import type { AgentDef } from './AgentRegistry';
import { AgentProcess } from './AgentProcess';
import { idleHandlers } from './AgentPool';
import { t } from '../i18n';

export interface ListNativeInput {
  def: AgentDef;
  binary: string;
  cwd: string;
  extraEnv?: Record<string, string>;
  log: (line: string) => void;
  timeoutMs?: number;
}

// The history list's "Import from <agent>": a throwaway spawn runs initialize + session/list for this workspace, then dies.
// session/new is never called — every one of these CLIs persists a session the moment it is created (the settings-page
// probe already leaves those behind), and the import list must not add more.
export async function listNativeSessions(input: ListNativeInput): Promise<acp.SessionInfo[]> {
  const { def, binary, cwd, extraEnv, log } = input;
  const timeoutMs = input.timeoutMs ?? 20_000;
  const proc = await AgentProcess.spawn(def, binary, cwd, idleHandlers(line => log(`native list ${def.command} stderr: ${line}`)), extraEnv);
  try {
    if (!proc.init.agentCapabilities?.sessionCapabilities?.list) {
      throw new Error(t('session.import.unsupported', { agent: def.name }));
    }
    let timer: ReturnType<typeof setTimeout> | undefined;
    const deadline = new Promise<never>((_, reject) => {
      timer = setTimeout(() => reject(new Error(`session/list timed out after ${timeoutMs}ms`)), timeoutMs);
    });
    try {
      const sessions: acp.SessionInfo[] = [];
      let cursor: string | undefined;
      for (let page = 0; page < 5 && sessions.length < 200; page++) {
        const r: acp.ListSessionsResponse = await Promise.race([
          proc.agent.request(acp.methods.agent.session.list, { cwd, ...(cursor ? { cursor } : {}) }),
          deadline,
        ]);
        sessions.push(...r.sessions);
        cursor = r.nextCursor ?? undefined;
        if (!cursor) break;
      }
      log(`native list ${def.command}: ${sessions.length} session(s) in ${cwd}`);
      return sessions;
    } finally {
      if (timer) clearTimeout(timer);
    }
  } finally {
    await proc.kill();
  }
}
