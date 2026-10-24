import { spawn } from 'node:child_process';
import { existsSync, readFileSync } from 'node:fs';
import { join } from 'node:path';
import { fileURLToPath } from 'node:url';

// `pnpm typecheck`: every TypeScript project the root tsconfig.json references (each package, each Node shell, test/,
// scripts/), plus the local LAB when lab/tsconfig.json exists (git-ignored, so CI never has it). The projects run in
// parallel; each one's output is printed whole once it finishes, and any failure fails the run
const root = fileURLToPath(new URL('..', import.meta.url));
const tsc = join(root, 'node_modules', 'typescript', 'bin', 'tsc');
const projects = JSON.parse(readFileSync(join(root, 'tsconfig.json'), 'utf8')).references.map(r => r.path.replace(/^\.\//, ''));
if (existsSync(join(root, 'lab', 'tsconfig.json'))) projects.push('lab');

function check(project) {
  return new Promise(resolve => {
    const child = spawn(process.execPath, [tsc, '-p', project, '--noEmit'], { cwd: root });
    let out = '';
    child.stdout.on('data', d => { out += d; });
    child.stderr.on('data', d => { out += d; });
    child.on('close', code => resolve({ project, code, out }));
  });
}

const results = await Promise.all(projects.map(check));
for (const { project, code, out } of results) {
  console.log(`${code === 0 ? '✓' : '✗'} ${project}`);
  if (out.trim()) console.log(out.trimEnd());
}
process.exitCode = results.some(r => r.code !== 0) ? 1 : 0;
