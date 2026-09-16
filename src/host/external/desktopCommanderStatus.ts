import { execFile } from 'node:child_process';
import { stat } from 'node:fs/promises';
import { homedir } from 'node:os';
import { join } from 'node:path';
import type { ChatGptIntegrationStatus } from '@shared/chatgptIntegration';
import { resolveCommand } from '../acp/AgentRegistry';

type Status = ChatGptIntegrationStatus['desktopCommander'];
export function commanderFacts(binary: boolean, configuration: boolean, running: boolean | undefined): Status {
  const evidence = running ? 'process' : binary ? 'executable' : configuration ? 'configuration' : 'none';
  return { installation: running || binary ? 'detected' : configuration || running === undefined ? 'unknown' : 'not_detected',
    process: running === undefined ? 'unknown' : running ? 'detected' : 'not_detected', pairing: 'unknown', evidence };
}

// Probe only executable/process presence and config metadata. Never read pairing tokens,
// browser cookies, credentials, tool logs or process arguments into a UI response.
export async function desktopCommanderStatus(): Promise<Status> {
  const [binary, configuration, running] = await Promise.all([
    resolveCommand('desktop-commander').then(Boolean, () => false),
    stat(join(homedir(), '.claude-server-commander', 'config.json')).then(s => s.isFile(), () => false),
    commanderProcess(),
  ]);
  return commanderFacts(binary, configuration, running);
}
function commanderProcess(): Promise<boolean | undefined> {
  if (process.platform === 'win32') return Promise.resolve(undefined);
  return new Promise(resolve => {
    execFile('ps', ['-ax', '-o', 'args='], { timeout: 1500, maxBuffer: 2_000_000, encoding: 'utf8' }, (error, stdout) => {
      if (error) { resolve(undefined); return; }
      resolve(stdout.split('\n').some(line => line.includes('/desktop-commander/') || /(?:^|\s)desktop-commander\s+remote(?:\s|$)/.test(line)));
    });
  });
}
