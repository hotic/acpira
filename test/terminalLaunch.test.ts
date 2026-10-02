import { spawnSync } from 'node:child_process';
import { resolve } from 'node:path';
import { describe, expect, it } from 'vitest';
import { terminalLaunch } from '../src/host/terminalLaunch';

const powershell = process.env.ACPIRA_TEST_POWERSHELL ?? (process.platform === 'win32' ? 'powershell.exe' : 'pwsh');
const hasPowerShell = spawnSync(powershell, ['-NoProfile', '-Command', 'exit 0']).status === 0;

describe('terminal launch', () => {
  it.each([
    ['powershell.exe', ['-NoLogo', '-NoProfile', '-NoExit', '-EncodedCommand', 'ZQBuAGQAIAB0AG8AIABlAG4AZAA=']],
    [String.raw`C:\Program Files\PowerShell\7\pwsh.exe`, ['-NoProfile', '-Command', 'Write-Output "中文"']],
  ] as const)('forwards the prepared Windows shell %s without interpreting its arguments', (command, input) => {
    const args = [...input];
    const launch = terminalLaunch(command, args, 'win32');
    expect(launch).toEqual({ shellPath: command, shellArgs: args });
    expect(launch.shellArgs).toBe(args);
  });

  it('keeps POSIX shell quoting', () => {
    expect(terminalLaunch('bash', ['-c', "echo 'ok'"], 'darwin')).toEqual({ text: "bash -c 'echo '\\''ok'\\'''" });
  });

  it.skipIf(!hasPowerShell)('runs the Devin offline installer regressions', () => {
    const run = spawnSync(powershell, ['-NoProfile', '-ExecutionPolicy', 'Bypass', '-File', resolve('test/devin-install.test.ps1')], {
      encoding: 'utf8', timeout: 15_000,
    });
    expect(run.stderr, run.stdout).toBe('');
    expect(run.status, run.stdout).toBe(0);
    expect(run.stdout).toContain('PASS: Devin installer offline regressions');
  });
});
