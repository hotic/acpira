import { createHash } from 'node:crypto';
import { spawnSync } from 'node:child_process';
import { existsSync, mkdirSync, readFileSync, rmSync } from 'node:fs';
import { join, resolve } from 'node:path';

// Review bench (docs/dev/evals.md): targets, their split, and the work tree a reviewer gets for one of them.
// A target is a commit of this repository that introduced a defect a later fix commit removed (mined), or a real commit
// with a defect planted on top of it (planted). The reviewer gets the commit's parent as a fresh repository with the
// commit's change (and any plant) left uncommitted, so no later history can give the fix away

export const ROOT = resolve(import.meta.dirname, '../../..');
export const BENCH_DIR = import.meta.dirname;
// Holdout targets and their answers stay out of git (and out of reach of the evolve loop's proposer)
export const HOLDOUT_DIR = join(ROOT, 'output/review-bench/holdout');

export interface Bug {
  // Where the defect lives in the changed tree: findings outside these files are not matched against it
  files: string[];
  // What is wrong and how it fails, written from the fix; the judge matches findings against this
  description: string;
  // The fix commit (mined) or how the plant breaks the change (planted)
  fix?: string;
  // Words a finding about this bug is likely to use: the stub judge's whole rule
  keywords: string[];
}

export interface Target {
  id: string;
  kind: 'mined' | 'planted';
  // The commit whose change is under review; the work tree is its parent plus its diff
  intro: string;
  // The original requirement as the reviewer is told it: the commit's subject, not its author's explanation
  requirement: string;
  // A planted target's patch file, applied after the commit's diff (relative to the target file's directory)
  plant?: string;
  bugs: Bug[];
  // Rough difficulty from the change's size and the bug's reach, for the report
  difficulty: 'easy' | 'medium' | 'hard';
}

interface TargetFile { targets: Target[] }

function readTargets(file: string): Target[] {
  if (!existsSync(file)) return [];
  return (JSON.parse(readFileSync(file, 'utf8')) as TargetFile).targets;
}

export function targetsOf(set: 'dev' | 'holdout'): { targets: Target[]; dir: string } {
  const dir = set === 'dev' ? BENCH_DIR : HOLDOUT_DIR;
  return { targets: readTargets(join(dir, 'targets.json')), dir };
}

// The split is a seeded hash order within each kind, so it does not depend on who picked the targets: the first third
// of each kind (rounded) is held out
export function splitOf(targets: Target[], seed = 'review-bench-2026-10-21'): { dev: Target[]; holdout: Target[] } {
  const order = (t: Target) => createHash('sha256').update(`${seed}:${t.id}`).digest('hex');
  const dev: Target[] = [];
  const holdout: Target[] = [];
  for (const kind of ['mined', 'planted'] as const) {
    const list = targets.filter(t => t.kind === kind).sort((a, b) => order(a).localeCompare(order(b)));
    const held = Math.round(list.length / 3);
    holdout.push(...list.slice(0, held));
    dev.push(...list.slice(held));
  }
  return { dev, holdout };
}

function run(cmd: string, args: string[], cwd: string, input?: Buffer | string) {
  const r = spawnSync(cmd, args, { cwd, input, maxBuffer: 512 << 20 });
  if (r.status !== 0) throw new Error(`${cmd} ${args.join(' ')} (in ${cwd}): ${r.stderr?.toString().trim()}`);
  return r.stdout;
}

const GIT_ID = ['-c', 'user.name=eval', '-c', 'user.email=eval@localhost', '-c', 'commit.gpgsign=false'];

// The work tree for one target under `dir/work`: the commit's parent as the only commit of a new repository, then the
// commit's change and the plant on top, uncommitted. Returns the work tree's path
export function materialize(t: Target, targetDir: string, dir: string, repo = ROOT): string {
  const work = join(dir, 'work');
  rmSync(work, { recursive: true, force: true });
  mkdirSync(work, { recursive: true });
  const tar = run('git', ['-C', repo, 'archive', '--format=tar', `${t.intro}^`], repo);
  run('tar', ['-x', '-f', '-', '-C', work], work, tar);
  // The bench itself (its targets and their answers) never goes into a work tree, should a target's commit carry it
  rmSync(join(work, 'scripts/eval/review'), { recursive: true, force: true });
  run('git', ['init', '-q'], work);
  run('git', ['add', '-A'], work);
  run('git', [...GIT_ID, 'commit', '-q', '-m', 'Base'], work);
  const diff = run('git', ['-C', repo, 'diff', '--binary', `${t.intro}^`, t.intro], repo);
  run('git', ['apply', '--whitespace=nowarn', '--intent-to-add', '-'], work, diff);
  if (t.plant) run('git', ['apply', '--whitespace=nowarn', join(targetDir, t.plant)], work);
  return work;
}

// `pnpm exec tsx scripts/eval/review/bench.ts materialize <target id> <dir>` (a look at a target by hand) or `split`
// (prints which targets each set holds, for targets.json and holdout/targets.json)
if (import.meta.url === `file://${process.argv[1]}`) {
  const [cmd, id, out] = process.argv.slice(2);
  if (cmd === 'materialize' && id && out) {
    for (const set of ['dev', 'holdout'] as const) {
      const { targets, dir } = targetsOf(set);
      const t = targets.find(x => x.id === id);
      if (t) {
        console.log(materialize(t, dir, resolve(out)));
        process.exit(0);
      }
    }
    console.error(`no target ${id}`);
    process.exit(2);
  } else if (cmd === 'split') {
    const all = [...targetsOf('dev').targets, ...targetsOf('holdout').targets];
    const { dev, holdout } = splitOf(all);
    console.log(JSON.stringify({ dev: dev.map(t => t.id), holdout: holdout.map(t => t.id) }, null, 2));
  } else {
    console.error('usage: bench.ts materialize <target> <dir> | split');
    process.exit(2);
  }
}
