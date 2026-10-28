# Evaluation benches

Scripts under `scripts/eval/` that measure orchestration choices against real CLIs and models. Every real run spends model calls or subscription quota; `--dry` swaps the models for `test/fake-agent.ts` and spends nothing.

## Reading results

- Quality first: a difference counts only beyond the noise measured by rerunning the same cell (the flip rate). Time second, and only as a clear margin (the evolve loop asks for 10%). Quota and cost are recorded, not optimized.
- One evaluation's median wall time varies about as much as 10%, so a win on time alone needs a second evaluation.
- A saturated set (every cell finds the same bugs, every config passes) shows only what an option costs, not what it buys.

## Shared pieces

- `meter.ts`: the metering proxy gateway-backed harnesses point at (`/r/<run>/<upstream>`); one line per call in `calls.ndjson`. `OUTAGE` gateway errors move a run to `infra.ndjson` for a rerun instead of counting as failure.
- `harnesses.ts`: writes each run's fresh HOME. Native-login harnesses (`agy`, `claude-max`, `devin`) bypass the meter. `claude-max` links the keychain without `CLAUDE_CONFIG_DIR`, since the keychain item's name derives from the config dir. Devin's ACP mode takes its key only through `authenticate`, so its login is imported and the copy deleted.
- `native.ts`: quota through `acpira quota --json` and Claude Code's own transcript usage; Claude's usage endpoint rate-limits hard, so quota is read per batch. `prices.ts`: catalogue prices.

## Review bench

`scripts/eval/review/`: does the handoff pack help a summoned reviewer find real defects, and does a group of reviewers from three families find more than one?

- Targets are mined fix commits (`mine.ts`) plus planted cross-file contract breaks; a third is held out in the git-ignored `output/review-bench/holdout/`, out of the evolve loop's reach.
- A target's work tree (`bench.ts materialize`) is the introducing commit's parent as a fresh repository with that change uncommitted, so no later history gives the fix away. Bug files are paths in that tree; after a later move the target must be renamed or materialize refuses it.
- The fake agent leads and calls `ask_agent` through the session's own `acpira` MCP server. The request is the same in every variant (where the change is and the original requirement), never the author's account. Variants: A request alone, B plus the handoff pack, C plus uses of changed definitions (`refs`).
- Scoring (`judge.ts`): a fixed judge from a family no reviewer belongs to matches findings to known defects; a match citing a file outside the defect's files does not count (kept as `claimed`). The evolve loop never changes the judge. `rescore.ts` rejudges stored reviews after a target is corrected.
- Findings that match no known bug are counted, not graded.
- Outcome: neither B nor C gained beyond the noise, so reviewer personas keep `changes` off by default and the recipe keeps `refs` off. A group found no more than one reviewer and ran at its slowest member's pace.

```
pnpm exec tsx scripts/eval/review/run.ts --reviewers astra[,astra+argon+opus] [--variant A,B,C] [--set dev|holdout] [--reps N] [--dry]
pnpm exec tsx scripts/eval/review/report.ts DIR [--by-target]
```

## Harness comparison

`run.ts --harness …` runs coding tasks with hidden tests against the built-in agent and other harnesses on the same models. Only totals carry signal; a single task's difference is noise. Decisions it backs are in `builtin-agent.md`, Harness comparison.

```
pnpm exec tsx scripts/eval/run.ts --model MODEL --harness acpira,opencode,pi [--task calc-lang,conf-bugs] [--reps N] [--out DIR]
```

## Fusion bench

`run.ts --config …` (configurations in `configs.ts`): does Acpira's orchestration on the vendors' own CLIs match Devin's Fusion, and does the built-in agent's sidekick do as well as Fusion on the same model pair? Same tasks, hidden tests and permission handling as the harness comparison.

- A run waits out background work its lead started (summoned rounds and their automatic messages) within the task's time limit.
- Two harder tasks (`agent-gaps`, `steering`) come from this repository's history, because the first five sit near the ceiling. `rust-tests.sh` runs their hidden tests one target at a time under a watchdog, as a failing steering test can hang its runtime.
- A configuration's picks are checked on the new session: an option or value the session lacks fails the run rather than running another model.
- Devin reports no tokens and its sidekick is invisible over ACP; for it quota is all there is. A run holds at most one Claude session, since its usage is read from the transcripts in the run's HOME.
- Outcome: every configuration sat at the quality ceiling. A sidekick bought no hidden-test quality and cost wall time, as the lead hands over tests and waits; its only measurable value was a second look at a finished change, which other CLIs get through reviewer personas.
- Hence the sidekick slot stays in the built-in agent's model panel only, and the sidekick's `when` guidance asks for parallel work or a second look, not a step the lead would then wait for.
- `calc-lang` trees also run a post-hoc nesting probe (`tasks/calc-lang/probes/nesting.py`), since the hidden tests did not tell the leads apart.

```
pnpm exec tsx scripts/eval/run.ts --config claude-opus,devin-opus-luna,acpira-orch [--task …] [--reps N] [--out DIR] [--dry]
pnpm exec tsx scripts/eval/report.ts DIR [--by-task]
```

## Evolve loop

`evolve.ts` tunes the strategy file the review bench reads (`--policy`, schema in `policy.ts`): the handoff pack's recipe and the reviewer's brief.

- Each round a read-only proposer (Claude Code in the loop's own sidecar home, every permission card rejected) proposes a policy; validation refuses unknown keys, out-of-range values and repeats. Candidate and incumbent then run on the dev set side by side.
- Accept on more bugs beyond the noise; reject on fewer beyond it. Within the noise, accept only with at least as many bugs and a median wall time at most 90% of the incumbent's, confirmed by a second evaluation pooled with the first.
- The wrap-up runs the incumbent against the shipped defaults on the holdout set and writes `report.md` and `defaults.patch`; the patch is applied by hand once approved.
- Outcome: no policy found more bugs, and the proposer drifted into tailoring candidates to the one missed bug, so the shipped defaults stay.

```
pnpm exec tsx scripts/eval/evolve.ts --out DIR [--rounds 2] [--hours 10] [--max-rejects 3] [--proposer claude|stub] [--final-only] [--dry]
```

## Extension race

`extend/run.ts`: how quickly an agent extends Acpira, and whether plugins (`plugins.md`) beat today's ways. Tasks and their checks are in `extend/tasks.ts`.

- Groups: `today` (an engine from before plugins) and `plugins` (this tree's engine). Arms: `plugin`, `mcp` (project `.mcp.json`), `hooks` (`.agents/hooks.json`), `source` (rebuild from source, `--with-source`).
- Every build is checked through the real engine with the fake agent, so a verdict costs no model call. Plugin arms also check that `acpira plugin disable` makes the check fail, and arms both groups offer are checked on the other engine too.
- `--dry` writes each arm's reference build instead of a model; any failing check exits 1. Source arms have no reference build. No real run (`--agent`) has been made yet.

```
pnpm exec tsx scripts/eval/extend/run.ts --dry | --agent ID [--task fx-tool,lock-gate] [--arm plugin,mcp] [--group today,plugins] [--out DIR]
```
