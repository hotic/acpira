// Named task suites for eval gating: separates dev tuning tasks from locked holdout tasks (1/3 holdout)
// so prompts and harness tweaks are never overfit to the gate set (docs/dev/evals.md).

export const SUITES: Record<string, { tasks: string[]; defaultReps: number; description: string }> = {
  // Non-saturated headroom suite: multi-file Rust and exact-semantics tasks, 1/3 held out
  'headroom-dev': {
    tasks: ['agent-gaps', 'steering', 'semver-port', 'calc-lang'],
    defaultReps: 5,
    description: 'Headroom dev split (4 tasks; 5 paired reps recommended)',
  },
  'headroom-holdout': {
    tasks: ['inv-lots', 'conf-bugs'],
    defaultReps: 5,
    description: 'Locked 1/3 holdout split (never tune prompts on these)',
  },
  headroom: {
    tasks: ['agent-gaps', 'steering', 'semver-port', 'calc-lang', 'inv-lots', 'conf-bugs'],
    defaultReps: 5,
    description: 'Full headroom suite (4 dev + 2 holdout, 5 paired reps)',
  },
};

export const HOLDOUT_TASKS = new Set<string>(SUITES['headroom-holdout']!.tasks);

export function isHoldoutTask(taskId: string): boolean {
  return HOLDOUT_TASKS.has(taskId);
}
