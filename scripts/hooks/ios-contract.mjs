#!/usr/bin/env node
// Workspace afterTurn gate (.agents/hooks.json): a turn that changed a TypeScript contract the iOS app mirrors, or the
// app's own models, runs AcpiraKit's ContractTests on the Mac before the turn ends. The suite reads packages/shared
// itself and needs no simulator, so drift shows up in the turn that caused it instead of on the next iOS CI run.
// Passes silently off macOS, without `swift`, or when no such file changed.
import { spawnSync } from 'node:child_process';
import { readFileSync } from 'node:fs';
import { isAbsolute, join, relative } from 'node:path';

const root = process.env.ACPIRA_PROJECT_DIR || process.cwd();
const kit = join(root, 'apps/ios/Packages/AcpiraKit');

// The files ContractTests reads (`source(...)` in ContractTests.swift) and the Swift models it checks
const WATCHED = [
  /^packages\/shared\/(protocol|transcript|sidecar|remote|sessionPatch|settings|subagents)\.ts$/,
  /^apps\/ios\/Packages\/AcpiraKit\/Sources\/AcpiraModels\//,
];

let input = {};
try {
  input = JSON.parse(readFileSync(0, 'utf8') || '{}');
} catch {
  process.exit(0);
}
const files = (input.turn_files ?? [])
  .map(f => (typeof f === 'string' ? f : f?.path ?? ''))
  .map(f => (isAbsolute(f) ? relative(root, f) : f).replaceAll('\\', '/'));
if (!files.some(f => WATCHED.some(rx => rx.test(f)))) process.exit(0);
if (process.platform !== 'darwin' || spawnSync('swift', ['--version'], { stdio: 'ignore' }).status !== 0) process.exit(0);

// A scratch folder of its own and no networking XCFramework (as iOS CI builds it), so the gate never rebuilds the
// package other runs keep under .build with a different manifest environment
const run = spawnSync('swift', ['test', '--scratch-path', join(kit, '.build/contract-gate'), '--filter', 'ContractTests'], {
  cwd: kit,
  env: { ...process.env, ACPIRA_NET: '0' },
  encoding: 'utf8',
  maxBuffer: 64 * 1024 * 1024,
});
if (run.status === 0) process.exit(0);

// The failed expectations and their details, or the compiler errors when the tests did not build
const out = `${run.stdout ?? ''}\n${run.stderr ?? ''}`.split('\n');
const shown = out.filter(l => /✘|↳|error:/.test(l) && !/Testing Library Version|Target Platform/.test(l)).slice(0, 40);
const reason = [
  'iOS contract check failed (swift test --filter ContractTests in apps/ios/Packages/AcpiraKit, ACPIRA_NET=0):',
  ...(shown.length ? shown : out.slice(-30)),
  '',
  'The iOS app mirrors packages/shared in apps/ios/Packages/AcpiraKit/Sources/AcpiraModels. Mirror the change there,',
  'or, when the phone deliberately ignores a field, add it to the matching leftOut list in',
  'apps/ios/Packages/AcpiraKit/Tests/AcpiraKitTests/ContractTests.swift with a comment saying why.',
].join('\n');
process.stdout.write(JSON.stringify({ decision: 'block', reason }));
