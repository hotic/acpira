# Evaluation benches

Scripts under `scripts/eval/` measuring orchestration choices against real CLIs and models (options: each script's argv parsing). Every real run spends model calls or subscription quota; `--dry` swaps the models for `test/fake-agent.ts`.

## Reading results

- Quality first: a difference counts only beyond the flip rate of rerunning the same cell. Time second, as a clear margin only: one evaluation's median wall time varies about 10%, so a time-only win needs a second evaluation.
- A saturated set (every cell finds the same bugs, every config passes) shows only what an option costs, not what it buys.
- `OUTAGE` gateway errors (`meter.ts`) go to `infra.ndjson` for a rerun instead of counting as failure.

## Harness homes and quota

- Native-login harnesses (`agy`, `claude-max`, `devin`) bypass the metering proxy. `claude-max` links the keychain without `CLAUDE_CONFIG_DIR`, since the keychain item's name derives from the config dir. Devin's ACP mode takes its key only through `authenticate`, so its login is imported and the copy deleted (`harnesses.ts`).
- Claude's usage endpoint rate-limits hard, so quota is read per batch (`native.ts`).
- Devin reports no tokens and its sidekick is invisible over ACP; only quota measures it. A run holds at most one Claude session, as usage is read from the transcripts in the run's HOME.

## Review bench

`scripts/eval/review/`: does the handoff pack help a summoned reviewer find real defects, and does a three-family group find more than one reviewer?

- A third of the targets is held out in the git-ignored `output/review-bench/holdout/`, out of the evolve loop's reach.
- A target's work tree is the introducing commit's parent as a fresh repository with that change uncommitted, so no later history gives the fix away. After a file move the target must be renamed or `bench.ts materialize` refuses it.
- Every variant gets the same request (where the change is, the original requirement), never the author's account.
- The judge is fixed, from a family no reviewer belongs to, and the evolve loop never changes it; a match citing a file outside the defect's files does not count. Findings matching no known bug are counted, not graded.
- Outcome: neither the handoff pack nor `refs` gained beyond the noise, so reviewer personas keep `changes` off by default and the recipe keeps `refs` off. A group found no more than one reviewer and ran at its slowest member's pace.

## Harness comparison

`run.ts --harness`: hidden-test coding tasks across harnesses on the same models. Only totals carry signal. Decisions: `builtin-agent.md`, Harness comparison.

## Fusion bench

`run.ts --config` (`configs.ts`): does Acpira's orchestration on vendor CLIs, or the built-in agent's sidekick, match Devin's Fusion on the same models?

- Picks are checked on the new session: a missing option or value fails the run rather than running another model.
- `agent-gaps` and `steering` come from this repository's history, as the first five tasks sit near the ceiling; their hidden tests run one target at a time under a watchdog, as a failing steering test can hang its runtime. `calc-lang` adds a post-hoc nesting probe, since its hidden tests did not tell leads apart.
- Outcome: every configuration sat near the quality ceiling, so the comparison is cost and time. Orchestration on vendor CLIs bought nothing over their own model, hence the sidekick slot stays in the built-in agent's panel only.
- A sidekick saves only the lead's own steps: each re-reads the lead's whole context at the lead's price, while the sidekick's tokens are cheap whatever it does. Guidance that kept the lead working (parallel pieces or a second look only) cost more than the lead alone. Fusion's split (`SIDEKICK_LEAD`, `SIDEKICK_BRIEF` in `acpira-shared/src/subagents.rs`) on GPT-6 Astra with GPT-6 Luna passed what Astra alone passed except `semver-port`, cheaper in step with the task's share of implementation (most on `steering` and `agent-gaps`, about even on the smallest tasks, where briefing and review cost what doing would), at two to four times the wall time.
- What raised the lead's cost, each answered in the guidance: review in several rounds, probe scripts the lead wrote itself, taking over a fix while the sidekick's thread was busy, a separate call to wait.
- A brief carries the lead's reading of the task: `semver-port`'s JavaScript numbers as floats, a misreading Astra alone made now and then, went into every sidekick brief as a requirement.
- Unmeasured: Claude leads under this split, and the cache a long round lets expire (small at the lead's context on the gateway's GPT route).

## Ledger build

`run.ts --config … --task ledger-build,ledger-solo`: a lead splits four work items of a small TypeScript API across workers in worktrees and merges them on `main`; hidden tests grade five groups (one per item, one for all together). `ledger-solo` is the same work for one agent.

- The task folders are local only (git-ignored): their repo and prompts are third-party material, whose origin and licence `tasks/ledger-build/NOTICE` records. `hidden/` is checked against `solution/` (every group passes) and the untouched repo (none does).
- `grade.mjs` grades `main`: the work tree while it is on `main`, else `main` as committed. Work left on a branch scores nothing.
- A configuration's `trigger` goes first on its own line and `mode` picks the session mode; no persona or trigger ships by default, so every orchestrated cell runs on a roster the bench wrote.
- The tasks set a git identity: a run's HOME has none, and workers stop to ask their lead whose name to commit under.
- Outcome: every cell passed every group, so the set is saturated and shows cost and time only. The built-in agent alone matched Codex on the same model. A Claude Code lead with built-in workers on a cheaper model cost less than Claude Code alone at about its time, and well under its own subagents.
- Team mode took about twice the built-in agent's own time and three times its cost: `main` already passed everything once the four branches were merged, and the follow-up, review and fix rounds the mode asks for changed no group.
- A worker's question reaches its lead only once the lead's turn ends or it waits: a Codex lead that kept working left all four workers blocked for minutes on a question each (a worktree its sandbox had refused to create), which was most of that cell's time.
- Leads put worktrees wherever they liked: beside the project, in the system temp folder, or inside the project where `git status` lists them.
- Unmeasured: Opus 5.5 on the built-in agent and as its workers (the gateway's Claude routes were down), more than one or two runs per cell.

## Evolve loop

`evolve.ts` tunes the review bench's policy (`policy.ts`): the handoff pack's recipe and the reviewer's brief.

- The proposer is read-only (Claude Code in the loop's own sidecar home, every card rejected). Within the noise a candidate is accepted only with at least as many bugs and a clearly faster median, confirmed by a pooled second evaluation.
- Outcome: no policy found more bugs, and the proposer drifted into tailoring candidates to the one missed bug, so the shipped defaults stay.

## Extension race

`extend/run.ts`: how fast and reliably a model extends a harness from inside it, and what each harness lets it reach.

- The `plugin` and `opencode` arms share one plain request per task naming no file, id or contribution kind, so a check finds what was built by what it does. Repair rounds send back only the symptom a user would see.
- No check calls a model: the fake agent drives the engine, a scripted Chat Completions model (`extend/fakeModel.ts`) drives OpenCode.
- A writer gets nothing a user lacks: runs use a temp folder, copied binaries and an environment naming nothing of this repository, since a writer that reaches the source tree reads the bench's checks and reference builds instead of the plugin guide. Process lists still show paths, so the engine (host of the built-in agent's shell) and `opencode run` run under a per-run `sandbox-exec` profile (`extend/jail.ts`), denied the repository, the owner's own Acpira, OpenCode and agent data, the gateway key and every other run's folder. The sandbox matches resolved paths and fails open, so each bench run checks it first.
- Each run has its own TMPDIR, and the sandbox denies `/tmp` and the user temp folder: writers drop probes there (OpenCode's scratch is `/tmp/opencode` when TMPDIR is unset, `mktemp` ignores TMPDIR), and every earlier bench run lives there. xcrun's cache stays allowed, or each `/usr/bin/git` call costs a second. OpenCode's writer is also denied the bench's own programs (engine, fake agent, terminal driver); the engine's sandbox reads the engine and the fake agent only, since the built-in agent's shell runs inside it.
- Each run ends by killing whatever is left under its HOME, such as OpenCode's background service, which `opencode plugin list` starts on one machine-wide port, so a leftover would serve the next run's writer.
- OpenCode is pinned 2.x (`@opencode/cli`, `extend/opencode.ts`; its cache dir name does not reveal a bench). The 1.x `opencode` a user may have on PATH loads neither 2.x plugins nor the reverse, so a model writing for the wrong one fails silently; a run's PATH puts the pinned one first.

OpenCode 2.2.5 as the bench found it:

- A running server reloads the project's `.opencode/plugins` within seconds, the global `~/.config/opencode/plugins` only on `opencode reload`; a theme (`cli.json` `theme.name`) only in a new interface.
- A plugin tool is a Code Mode tool unless it sets `codemode: false`. The check layer denies `execute`, which turns every tool direct, and adds no permission rule a plugin's own deny would lose to (the last matching rule wins).
- `--auto` approves whatever no rule denies; a rule nothing matches asks.
- Its `opencode` skill points the model at its docs (the counterpart of `plugin_guide`).
- A configured model sends no reasoning level unless a variant carries one (`provider/model#variant` in `run -m`); the gateway's default for DeepSeek V4.1 Flash is none, so both writers get an explicit level.
- The `claude-agent` provider runs Claude Code with only OpenCode's tools (over MCP) and its own `claude_code` prompt; it connects with `auth login claude-agent --method claude-login`, finds Claude through `OPENCODE_CLAUDE_PATH` or PATH, and refuses a release that does not report its version (the one `claude-agent-acp` bundles). Subscription runs therefore point both sides at the same `claude` (`CLAUDE_CODE_EXECUTABLE` for the adapter) and pass on `CLAUDE_SECURESTORAGE_CONFIG_DIR`, which picks the keychain login.
- Claude Code keeps temp files and sockets under `/tmp/claude-<uid>`, shared with every Claude session on the machine, so the sandbox keeps it denied and each run gets a short `/tmp` folder of its own as `CLAUDE_CODE_TMPDIR` (socket paths stop near 104 bytes).
- The terminal interface hit-tests a click on the pointer's last motion, so a driver must move before it presses (`extend/tui_driver.py`).
