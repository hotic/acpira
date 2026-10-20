# Evaluation benches

Scripts under `scripts/eval/` that measure orchestration choices against real CLIs and models: the review bench (what helps a summoned reviewer find real defects), the harness and Fusion comparison (`run.ts`, coding tasks with hidden tests; its first use is in `docs/dev/builtin-agent.md`, Harness comparison), and the evolve loop that tunes the strategy file between them. Every real run spends model calls or subscription quota; `--dry` runs swap the models for `test/fake-agent.ts` and spend nothing. The rule for reading results: quality first (a difference counts only beyond the noise measured by rerunning the same cell), time second, quota and cost recorded but not optimized.

## Shared pieces

- `meter.ts`: the metering proxy every gateway-backed harness points at (`/r/<run>/<upstream>`); one line per model call in `calls.ndjson`, the real key set by the proxy, `OUTAGE` the gateway errors that move a run to `infra.ndjson`.
- `harnesses.ts`: one function per harness writing the run's fresh HOME (and sidecar home). Native-login harnesses use the CLI's own login and bypass the meter: `agy`, `claude-max` (Claude Code on its claude.ai login: a keychain link and the onboarding / account entries of `~/.claude.json`, no `CLAUDE_CONFIG_DIR`, since the keychain item's name derives from the config dir; checked 2026-10-21 with Claude Code 2.1.299, `claude auth status` logged in from such a HOME), `devin` (Devin's `credentials.toml` copied into the run's HOME with mode 0600, then imported from there into the run's sidecar home through the account menu's Import, since Devin's ACP mode takes its key only through `authenticate`; the copy is deleted right after the import and again when the run ends). Codex's own login on this machine is the gateway itself (a custom provider in `~/.codex/config.toml`), so the metered `codex` harness stands for it.
- `native.ts`: `readQuota` runs `acpira quota --json` (the quota windows of the CLIs' own logins, read the way the account view reads them with nothing imported or stored; labels dropped) and `claudeUsage` sums Claude Code's own transcript usage under the run's HOME, one count per message id. Claude's usage endpoint rate-limits hard, so quota is read per batch.
- `prices.ts`: catalogue prices for the reports.

## Review bench

`scripts/eval/review/`. Does the handoff pack help a reviewer, and does a group of reviewers from three model families find more than one?

- Targets come from this repository's history. `mine.ts` lists the small fix commits (at most 40 changed source lines in at most 3 source files), blames the lines each removed in its parent (the anchor line for a pure insertion) and names the commit that last wrote them (`output/review-bench/candidates.json`; on 2026-10-21: 291 fix commits, 148 small enough and blamed back, 91 of them onto a change under 900 source lines). Kept by hand were defects visible from the introducing change's own code and docs: not ones that need a real CLI's wire behaviour, a later requirement, or runtime timing, and not blame that lands on a refactor or on an insertion's anchor line. Planted targets are real commits with a defect patched on top, each breaking a contract across files (a serde key, a wire argument, a TypeScript mirror).
- 24 targets, 18 mined and 6 planted, split by a seeded hash within each kind with a third held out (`bench.ts split`): the dev set (12 mined, 4 planted) is `targets.json` with `plants/`; the holdout set (6 mined, 2 planted) and its plants live in the git-ignored `output/review-bench/holdout/`, out of the evolve loop's reach. Each bug carries the files it lives in, a description written from the fix, and keywords for the stub judge.
- A target's work tree (`bench.ts materialize <id> <dir>`): `git archive` of the introducing commit's parent as the only commit of a new repository, then that commit's diff applied uncommitted (new files intent-to-add) and the plant on top, so no later history gives the fix away. A bug's files are paths in that tree, and materialize refuses a target whose bug file the tree lacks: a mined target named by the fix commit's paths breaks after a later move, as `stale-tool-activity` did (`acp/transcript/normalize.rs`, at the introducing commit still `acp/normalize.rs`) until 2026-10-21.
- A run (`run.ts`): the fake agent is the lead and calls `ask_agent` through the session's own `acpira` MCP server, the same path a model's call takes; the delivery comes back as the automatic message. Reviewers are personas with the reviewer duty, consult mode and the reviewer brief: `astra` (Codex, GPT-6 Astra, xhigh, metered), `argon` (the built-in agent, Gemini 4 Argon, high, metered), `opus` (Claude Code, `opus` at xhigh on its own Max login, unmetered). Several reviewers joined by `+` are one group summon. Every permission card is allowed once and questions are skipped. The request is the same in every variant: where the change is and the original requirement (the commit subject), never the author's account of it.
- Variants: A the request alone, B with the handoff pack (`changes`, the policy's recipe in the run's `handoff.json`), C the pack with the uses of changed definitions (`refs`).
- Scoring (`judge.ts`): a fixed judge from a family no reviewer belongs to, `deepseek-v4-pro` on the gateway at temperature 0, lists the review's findings and matches each to a known defect or to none. A match that cites a file outside the defect's files does not count; the verdict keeps such a refused match as `claimed`. A corrected target is rescored with `rescore.ts DIR --target id,… [--reviewers cell,…]`, which judges the stored reviews again with the current targets, rewrites their verdicts and result lines and keeps the original results as `results.before-rescore.ndjson`. Per run: bugs found, findings that match none (real or not, they are not graded), tools and their kinds, wall time, tokens and cost per reviewer. The evolve loop never changes the judge.
- Outputs under `--out`: `results.ndjson` (one line per run, resumable), `calls.ndjson`, `quota.ndjson`, and per run the reviews, verdicts, node transcripts and the root session. `report.ts DIR [--by-target]` prints the cells, the flip rate between reruns of one cell, and every pair of cells compared target by target.
- Dry run: `--dry` makes each reviewer the fake agent answering with a canned review that names the target's bugs (the fake agent's `review=<path>`, which also names the prompt's block types), judged by the keyword stub. Checked 2026-10-21: all 24 targets materialize, run and score; variant A's reviewer gets the request as one text block and B's and C's a text block plus the pack as an embedded resource (C's listing the uses of changed definitions); `astra+argon+opus` is one group summon whose three rounds each get the same pack.

```
pnpm exec tsx --tsconfig tsconfig.host.json scripts/eval/review/run.ts --reviewers astra[,astra+argon+opus] [--variant A,B,C]
  [--set dev|holdout] [--target id,…] [--reps N] [--parallel N] [--judge stub|MODEL] [--policy FILE] [--out DIR] [--dry]
pnpm exec tsx --tsconfig tsconfig.host.json scripts/eval/review/report.ts DIR [--by-target]
pnpm exec tsx --tsconfig tsconfig.host.json scripts/eval/review/rescore.ts DIR --target id,… [--reviewers cell,…]
pnpm exec tsx scripts/eval/review/mine.ts          # candidates, spends nothing
pnpm exec tsx scripts/eval/review/bench.ts materialize <target> <dir>
```

### Results, 2026-10-21

Engine pinned at ebf8ebb3 (Acpira 1.10.7). Reviewers: Codex 0.162.1 with codex-acp 2.2.2 (GPT-6 Astra, xhigh), the built-in agent (Gemini 4 Argon, high), Claude Code 2.1.299 with claude-agent-acp 0.93.0 (`opus` at xhigh on the Max login). Judge `deepseek-v4-pro`. Each cell ran the dev set twice; the scores are after the rescore of 62aed117.

| cell | bugs found | flips between reruns | median wall | $ per run |
|---|---|---|---|---|
| A astra | 30 of 32 | 0% | 3.0 min | 1.81 |
| B astra | 31 of 32 | 6% | 2.8 min | 1.78 |
| C astra | 30 of 32 | 0% | 3.0 min | 1.80 |
| B astra+argon+opus | 30 of 32 | 0% | 7.3 min | 3.39, plus Opus on its Max login (1.51 at list price) |

- The dev set is close to saturated: every run of every cell found the bug of 15 of the 16 targets. Only `stale-tool-activity` (an open tool call that keeps naming the activity after later output) tells the cells apart. The one B run credited with it cites the neighbouring `latest_step`, which the judge took for the same root cause.
- Neither the handoff pack (B) nor the uses of changed definitions (C) gains beyond the noise. B finds one bug more than A, within its 6% flip rate, and is 5% faster by median, short of the 10% the rule asks for. Reviewer personas therefore keep `changes` off by default, and the recipe keeps `refs` off.
- The group (one summon, the same pack to each member) found no more than one reviewer: each member found 30 of the 32 on its own, no bug was found by one member only, and all three missed `stale-tool-activity` both times. The slowest member sets its pace (Argon in 24 of the 32 runs, 6.7 min by median; Opus 4.6, Astra 2.9), so it took 2.6 times as long as Astra alone, and it reported 9.3 findings per run that match no known bug against 2.1 (Opus 5.1 of them). A saturated set cannot show what a group catches that one reviewer misses, only what a group costs.
- Before the rescore, two target definitions hid found bugs, so variant B looked two bugs worse than A. Both B reviews of `inherit-mode-sanitize` named `sanitizePersonas` but placed the finding at its caller in the settings page.

## Fusion bench

`run.ts --config …` (configurations in `configs.ts`): does Acpira's orchestration on the vendors' own CLIs match or beat Devin's Fusion, and does the built-in agent's sidekick do as well as Fusion on the same pair of models? Same tasks, hidden tests and permission handling as the harness comparison; a run waits out the background work its lead started (summoned rounds and the automatic messages delivering them, idle for 5 s with nothing running) within the task's time limit.

- Configurations: `claude-opus` (Claude Code, `opus` at high, Max login) and `codex-astra` (Codex, GPT-6 Astra, the gateway) as single-model baselines; `devin-opus-luna` and `devin-astra-swe` (Devin on its own login with the Fusion pairs `fusion-claude-opus-5-5-high-sidekick-gpt-6-luna-high` and `fusion-gpt-6-astra-high-sidekick-swe-2-medium`, ids of Devin 3000.11.92); `acpira-orch` (Claude Code with Opus 5.5 leads, with a review group of Astra on Codex and Argon in the built-in agent plus an executor on GPT-6 Luna in the run's subagents.json, for the lead to use or not); `acpira-opus-luna` and `acpira-astra-swe` (the built-in agent on the gateway with the same pairs as Devin's, the second model as its sidekick); `acpira-astra` (the built-in agent on Astra alone, which tells the sidekick's effect apart from the harness's).
- Two harder tasks come from this repository's own history, because the first five sit near the ceiling for the strongest models. The work tree is `git archive` of a past revision (`base` in `task.json`), with a `patch` of test scaffolding and the files under `add/` on top. The hidden tests are the crate's test files as the reference commit left them (`hidden`). `rust-tests.sh` runs them one target at a time under a watchdog, because a failing steering test can leave its tokio runtime waiting forever on a held mock reply. `env` gives these tasks the real `RUSTUP_HOME` and `CARGO_HOME`, since the run's HOME is fresh.
  - `agent-gaps`: seven bug reports against the built-in agent, drawn from 58aff9c0. Of its 50 hidden tests, 41 pass at the start.
  - `steering`: the `_session/steering` feature of 916c80b1, with a spec written from that commit's docs and tests. Of its 76 hidden tests, 69 pass at the start.
  - Both reference commits pass every hidden test through the same script (checked 2026-10-21).
- A configuration's picks are set on the new session (by option id, else category) and checked: an option or value the session does not offer fails the run rather than running another model. Devin's login is imported into the run's sidecar home through the account menu's Import and its key removed once the run ends.
- Usage: the lead's metered calls (or, for Claude Code on its Max login, the tokens of its own transcripts), each helper's calls on its own metering path (`/r/<run>.<helper>/gw`) priced at its model, the summoned rounds per run by persona, and for native logins the quota points spent per run (`acpira quota` around each run; Devin's on-demand balance is in the raw snapshots). Devin reports no tokens: quota is all there is.
- Dry run: `--dry` makes the lead and every helper's CLI the fake agent under its own id; a configuration with helpers summons its first group (else its first helper) once. Checked 2026-10-21 for all seven configurations on `conf-bugs` (the roster and per-helper providers written, the picks skipped); `acpira-orch` summons its review group, both members' rounds complete and come back in one automatic message.

```
pnpm exec tsx --tsconfig tsconfig.host.json scripts/eval/run.ts --config claude-opus,devin-opus-luna,acpira-orch [--task …] [--reps N] [--out DIR] [--dry]
pnpm exec tsx --tsconfig tsconfig.host.json scripts/eval/report.ts DIR [--by-task]
```

## Evolve loop

`evolve.ts` tunes the strategy file the review bench reads (`--policy`, schema in `policy.ts`): the handoff pack's recipe (`functionContext`, `fullTextLines`, `refs`, `refsMax`, `maxBytes`) and the reviewer's standing brief. `readPaths` and the scout and executor briefs are left out, since the review bench exercises none of them.

- A round starts with a proposer. It reads `schema.json`, `defaults.json`, `incumbent.json`, `ledger.md` and `analysis.md` (the bugs the last evaluation missed, with the reviews that missed them) in `DIR/round-N/brief`, and answers with one JSON block holding a hypothesis and a policy. The proposer is Claude Code on its Max login (`opus` at high) in a session of the loop's own sidecar home `DIR/proposer/acpira`, where every permission card is rejected so it only reads; `pnpm harness --home DIR/proposer/acpira` shows those sessions. `--proposer stub` takes the next change of a fixed list instead.
- Validation refuses unknown keys, wrong types, values out of range, and a candidate equal to the incumbent or to one tried before. Such a round is recorded as invalid.
- The candidate and the incumbent then run on the dev set as two review bench processes started together, with the same reviewers, variant and reps.
- The rule compares bugs found. The noise is the flip rate between reruns under one policy (the review A/B's reruns via `--noise-from` while the incumbent is the shipped default, and the incumbent's own evaluations in earlier rounds) times the bugs evaluated. A candidate that finds more than the noise and at least one more bug is accepted, one that finds fewer beyond the noise is rejected, and one within the noise is accepted only when it finds at least as many and its median wall time is at most 90% of the incumbent's. Such a win on time alone is checked by evaluating both sides once more (`round-N/confirm`) and deciding again on the two evaluations pooled, since one evaluation's median varies about as much as the 10% asked (Results below). More failed runs than the incumbent rejects it.
- The loop stops at `--rounds`, `--hours`, `--max-rejects` rounds in a row without an acceptance, or when Claude's weekly quota is under `--quota-floor` (read before each round in which Claude proposes or reviews). Every round goes to `DIR/ledger.ndjson`, and a rerun with the same `DIR` resumes, an interrupted round keeping its saved proposal.
- The wrap-up runs the incumbent against the shipped defaults on the held-out set and writes `output/evolve/<stamp>/report.md` and `defaults.patch` (`Recipe::default`, `REVIEWER_BRIEF`, `DUTY_BRIEFS.reviewer` and `test/fixtures/duty-presets.json`). The patch is applied by hand once approved. `--final-only` reruns just the wrap-up.
- Dry run, checked 2026-10-21: two stub rounds with fake reviewers on the dev set (16 runs a side, both rejected within the noise), then `--final-only` with a forced incumbent (8 held-out runs a side, the report, and a patch that `git apply --check` accepts). A separate check applied such a patch to copies of the four files and read the brief back from the TypeScript module, the JSON fixture and the Rust literal. The confirmation of a win on time ran in a dry round with the bar loosened to equal medians for the check: both sides evaluated twice and decided on 32 pooled runs.

```
pnpm exec tsx --tsconfig tsconfig.host.json scripts/eval/evolve.ts --out DIR [--rounds 2] [--hours 10] [--max-rejects 3]
  [--reviewers astra] [--variant B] [--reps 1] [--parallel 4] [--proposer claude|stub] [--noise-from DIR]
  [--quota-floor 0.25] [--no-final | --final-only] [--dry]
```

### Results, 2026-10-21

Engine pinned at ebf8ebb3.
- Cell `astra` B: Codex 0.162.1 with GPT-6 Astra at xhigh, given the handoff pack. Each round ran one run a target per side on the dev set, three at a time.
- Proposer: Claude Code 2.1.299 on its Max login (`opus` at high). Judge: `deepseek-v4-pro`.
- A first start was stopped when the place check turned out to refuse found bugs (the review A/B's rescore), and the loop restarted on the corrected targets.
- It ran five rounds and stopped at three rejections in a row.
- Medians are as the loop recorded them, the upper middle run, before 68936a7b. Each pair is incumbent / candidate.

| round | candidate | decision | bugs of 16 | median s | findings matching no known bug |
|---|---|---|---|---|---|
| 1 | a larger pack (`refs` up to 150, 800 lines whole, 160 KB) and a long method brief | rejected, slower | 15 / 15 | 150 / 328 | 35 / 98 |
| 2 | `refs` up to 100, 1500 lines whole, 200 KB; a short brief: review from the pack first, no build or tests when dependencies are missing, two questions (callers, stale state) | accepted, as many and faster | 15 / 15 | 172 / 138 | 30 / 59 |
| 3 | round 2's brief, plus every kind of later event that should end retained state, and keep looking after one finding | rejected, slower | 15 / 15 | 163 / 252 | 58 / 165 |
| 4 | up to 4000 lines whole, 320 KB, `refs` up to 150; recheck unchanged helpers in their new use, one pass | rejected, short of 10% by under a second | 15 / 15 | 172 / 155 | 56 / 56 |
| 5 | round 2's pack; break every selection rule the change adds, and assume outside inputs arrive incomplete | rejected, slower | 15 / 15 | 160 / 178 | 54 / 60 |

On the held-out set, round 2's policy ran against the shipped defaults, one run per target. Both found the same 4 of the 8 bugs. The final policy was faster: 204 s by median against 224 (221 against 247 by the upper middle run, as the report shows). It cost $18.57 against $22.23, and it reported 41 findings that match no known bug against 30.

The loop spent $308 at list price on the dev set and $41 on the held-out set, all of it Astra on the gateway, plus the proposer's share of Claude's quota.

- Every evaluation of every policy found the same 15 of the dev set's 16 bugs. Only `stale-tool-activity` was missed. From round 3 on, the proposer built each candidate around that one bug (its function, its file's length), although the brief asks it not to tailor the policy to the bugs listed.
- The one acceptance was a time win inside one policy's own spread. Round 2's policy measured 138 s by median as the candidate, then 163, 172 and 160 s as the incumbent of rounds 3 to 5. The shipped defaults measured 150 and 172. Since 68936a7b, a win on time alone is confirmed with a second evaluation.
- Decision: the shipped defaults stay, and `defaults.patch` is not applied.
  - Neither set shows a gain in bugs found.
  - The difference in time is inside the spread.
  - The final policy reports a third more findings that match no known bug, up to twice as many on the dev set.
  - Its brief describes the pack as present, but reviewer personas send none by default (Review bench results).
  - Its brief also skips builds and tests when dependencies are missing, which in the bench's work trees they always are.
