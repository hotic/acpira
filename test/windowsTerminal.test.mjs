// The shell only delivers prepared launches. Native auth argv coverage lives in Rust platform::terminal.
// Node 22's type stripping loads the production helper without installing the webview dependencies.
import { strict as assert } from 'node:assert';
import { spawnSync } from 'node:child_process';
import { mkdtempSync, readFileSync, rmSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { test } from 'node:test';
import { terminalLaunch } from '../src/host/terminalLaunch.ts';

test('Windows terminal delivers an encoded launch unchanged to PowerShell 5.1', { skip: process.platform !== 'win32' }, () => {
  const root = mkdtempSync(join(tmpdir(), "acpira-terminal O'Brien 用户 "));
  try {
    const result = join(root, 'result.txt');
    const source = "[IO.File]::WriteAllText($env:ACPIRA_TERMINAL_RESULT, '中文 ''literal'' $variable & |')";
    const args = ['-NoLogo', '-NoProfile', '-NoExit', '-EncodedCommand', Buffer.from(source, 'utf16le').toString('base64')];
    const launch = terminalLaunch('powershell.exe', args, 'win32');
    assert.deepEqual(launch, { shellPath: 'powershell.exe', shellArgs: args });
    assert.equal(launch.shellArgs, args);
    const run = spawnSync(launch.shellPath, launch.shellArgs.filter(arg => arg !== '-NoExit'), {
      encoding: 'utf8', env: { ...process.env, ACPIRA_TERMINAL_RESULT: result }, timeout: 10_000,
    });
    assert.equal(run.status, 0, run.stderr);
    assert.equal(readFileSync(result, 'utf8'), "中文 'literal' $variable & |");
  } finally { rmSync(root, { recursive: true, force: true }); }
});
