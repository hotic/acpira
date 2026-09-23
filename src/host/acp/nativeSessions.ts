import { realpath } from 'node:fs/promises';
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

// An adapter can hand out many empty pages (codex-acp filters each 25-thread page by cwd, so a page that only
// held other projects still comes with a nextCursor) — keep paging while there is a cursor
const MAX_PAGES = 40;
const MAX_SESSIONS = 200;

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
      const listPages = async (listCwd: string): Promise<acp.SessionInfo[]> => {
        const sessions: acp.SessionInfo[] = [];
        let cursor: string | undefined;
        for (let page = 0; page < MAX_PAGES && sessions.length < MAX_SESSIONS; page++) {
          const r: acp.ListSessionsResponse = await Promise.race([
            proc.agent.request(acp.methods.agent.session.list, { cwd: listCwd, ...(cursor ? { cursor } : {}) }),
            deadline,
          ]);
          sessions.push(...r.sessions);
          cursor = r.nextCursor ?? undefined;
          if (!cursor) break;
        }
        return sessions;
      };
      let sessions = await listPages(cwd);
      // codex-acp stores the canonicalized thread cwd (macOS /var → /private/var) and compares strings: a project
      // reached through a symlink filters every page to nothing, so retry once with the resolved path
      const real = await realpath(cwd).catch(() => cwd);
      if (!sessions.length && real !== cwd) sessions = await listPages(real);
      log(`native list ${def.command}: ${sessions.length} session(s) in ${cwd}`);
      return sessions;
    } finally {
      if (timer) clearTimeout(timer);
    }
  } finally {
    await proc.kill();
  }
}
