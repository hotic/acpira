# Acpira

A chat shell for VS Code / Cursor, IntelliJ and a desktop app that drives official agent CLIs (`grok agent stdio`, `devin acp`, `kimi acp`, Codex, Claude, OpenCode, Google's `agy_acp_server`, or any ACP-compatible command) over [ACP](https://agentclientprotocol.com) (JSON-RPC over stdio). The shell owns UI, session organization, permission approvals, accounts, and context budget; model calls and agent execution stay in the CLIs.

## Working rules

- Done means: `pnpm typecheck` and `pnpm test` pass, plus `pnpm build` when a bundle entry or the webview changed and `cargo test --workspace` + `cargo clippy --workspace --all-targets --all-features -- -D warnings` (in `rust/`) when the engine changed, and no orphaned `fake-agent` process remains. Kotlin changes also pass `cd apps/intellij && ./gradlew test`; Swift changes under `apps/ios/` pass `apps/ios/scripts/check.sh`; `apps/desktop/` changes its clippy and `scripts/smoke.mjs`. Visual changes are checked in `pnpm harness` or the LAB when a browser is available; otherwise the summary says they were not.
- Keep going while the next step needs no decision. Stop and ask only when blocked on a decision, or before: deleting files this task did not create, force-pushing or rewriting history, deleting or rewriting data outside the repository (`~/.acpira`, the CLIs' own stores), or sending real prompts to a CLI (probe scripts, evals), since those spend model calls.
- Read the area's doc from the table below before changing it; the recorded wire behaviour outranks the ACP spec and memory of it.
- `docs/dev/` holds only what the code does not say quickly: invariants and cross-file contracts, the non-obvious why, external CLI wire behaviour with the version observed on, traps, open questions, and where an area lives. It never restates code (constants, timings, step-by-step logic, field lists, test lists, UI descriptions). Edit entries in place, terse bullets: no dates, change history, investigation stories, measurements, commit hashes, machine names or personal paths; git keeps the history. `docs/dev/` is published, so anything private or scratch goes in an untracked file under `docs/` (everything there outside the `.gitignore` whitelist stays local).
- This file stays an index under 12 KB (some hosts truncate at 16 KB); details belong in `docs/dev/`.
- Long multi-part work keeps its checklist in `docs/TASKS.md` (ignored by git) and ticks items as they finish.
- A run that changed files closes with a short summary: any decision still needed first, then what changed, the commands actually run, and what was left unverified (and why). Anything not confirmed against code, tests or a real CLI is called out as unverified, with where it was looked for.

## Commands

- `pnpm build` — the VS Code shell (`apps/vscode/dist/extension.cjs`), the picker (`dist/picker.js`) and the webview (`dist/webview/`)
- `pnpm typecheck` — `scripts/typecheck.mjs`: every project the root `tsconfig.json` references, plus a local `lab/tsconfig.json`. A new TypeScript directory gets its own tsconfig and a reference in the root solution file
- `pnpm test` — vitest: every package's and shell's suites plus the cross-package contracts in `test/` (envelopes against the Rust sidecar; `test/globalSetup.ts` rebuilds `rust/target/debug/acpira`, `ACPIRA_SIDECAR_BIN` overrides it). `test/fake-agent.ts` is a fake ACP agent run with plain `node`; after a run `ps -eo pid,ppid,command | awk '$2==1 && /fake-agent/'` prints nothing
- `cd rust && cargo test --workspace` — the engine: unit tests plus `crates/acpira-host/tests/engine/` against the fake agent (skipped without `node_modules`). `test/fixtures/engine-*.json` are golden normalize / diff output the webview suites render; `ACPIRA_UPDATE_FIXTURES=1 cargo test golden` rewrites them after an intended change
- `pnpm harness [port]` — the real webview bundle against the real sidecar and CLIs in a browser, no IDE (`test/host-preview/`); open the URL with the token printed on stderr, data under `~/.acpira/harness` unless `--home`. Build first
- Probes against real CLIs: `pnpm probe <agent> [...] [prompt]` (raw `initialize` + `session/new`, use first for protocol issues) and `scripts/probe/*-host.ts` (the webview's path through the sidecar). Flags and what each covers: "Host probes" in `docs/dev/host-architecture.md`. Everything but a prompt-less `pnpm probe` spends model calls
- Evals (`scripts/eval/`) and `acpira quota [--json]`: `docs/dev/evals.md`; real runs spend model calls or quota
- Engine subcommands: `acpira serve` (the persistent engine the VS Code shell connects to on macOS / Linux, `ACPIRA_ENGINE=stdio` falls back to the attached sidecar), `acpira install-agent`, `acpira handoff-pack`, `acpira agents --json`: `docs/dev/host-architecture.md`
- `pnpm catalog:update` — regenerate the built-in model catalogue from models.dev; refresh before a release
- `pnpm package` — page, picker, this machine's release sidecar, then its `.vsix` and, on an arm64 Mac, the desktop app and dmg; `pnpm install:local [--vsix | --desktop]` installs them. Cross-platform sidecars, per-platform VSIX and versioning: `docs/releasing.md`
- `apps/ios/scripts/check.sh [simulator]` / `shot.sh` — the iOS app's tests / a simulator screenshot (`docs/dev/ios.md`)
- `cd apps/desktop && node scripts/smoke.mjs` / `node scripts/package.mjs` — the desktop app end to end with the fake agent / its dmg (`docs/dev/desktop.md`)

## Hard rules

- `packages/ui/` must never import `vscode`. Only `apps/vscode/src/extension.ts`, `bridge.ts`, `vscodePlatform.ts` and `files.ts` import it, never `packages/engine-client/`
- All engine behaviour lives in the Rust sidecar; the TypeScript under `apps/vscode/src/` is the VS Code shell only (webview panels, platform RPCs, sidecar lifecycle, the one-time legacy migration). `HostRuntime::create` in `rust/crates/acpira-host/src/runtime.rs` is the engine's single composition root
- `packages/shared/transcript.ts` (`SessionView`) and `packages/shared/protocol.ts` (`HostMsg` / `WebviewMsg`) are the only host ↔ webview contract; `packages/shared/sidecar.ts` is the sidecar wire contract (`SIDECAR_PROTOCOL_VERSION`)
- Sidecar stdout carries envelopes only; logs go to stderr
- The root `tsconfig.json` holds `references` only
- `~/.acpira` is shared by every window and engine: files are written tmp + rename, and `accounts.json`, `secrets.json`, `settings.json` and `sessions/prefs.json` are edited under the file lock
- Every stream chunk pushes the session view, as a `sessionPatch` (only the changed tail) to the real webview; nothing may defeat the `shared/reuse.ts` reference reuse and `memo` chain (`docs/dev/webview.md`)
- Write code comments in English
- Components must not contain raw numbers: sizes, type scale, spacing, and radii all use token classes (`h-ctl`, `px-pad`, `gap-gap`, `rounded-md`, …)
- Every "row" (thought / plan / tool / status / session item) uses `ui/Row`; command execution is a row too (command on the row, output card below), not a separate block

## Layout

One pnpm workspace: each package declares its own dependencies (versions in the `catalog`) and imports another by name (`@acpira/shared/x`), never by relative path. Packages export their TypeScript sources; nothing imports from `apps/`. Shared build output goes to the root `dist/`.

- `rust/` the engine: `acpira-shared` contracts; `acpira-host` (`acp/` the ACP client: `transport/`, `agents/`, `session/`, `transcript/`, `vendors/`; plus `session_manager.rs`, `store/`, `accounts/`, `external/`, `sidecar/`); `acpira-agent` the built-in agent; binary `acpira`
- `packages/`: `shared/` contracts and pure logic; `ui/` the React UI; `engine-client/` the VS Code shell's engine client; `picker/` the browser panel's element picker
- `apps/vscode/` the extension; `apps/desktop/` the Tauri app (own Cargo workspace, window-free logic in `rust/crates/acpira-desktop`); `apps/intellij/` the IntelliJ plugin; `apps/ios/` the iOS app
- `test/` cross-package contracts, `fake-agent.ts` and shared fixtures; `scripts/` probes, evals, release; `lab/` the LAB (local only)

## Read before changing

| Area | Doc |
|---|---|
| Engine composition, sidecar protocol and shells, persistent engine and push bus, agent registry / launching / installs, browser panel, session links, cross-harness subagents, terminals, shared config, stores, accounts, probes | `docs/dev/host-architecture.md` |
| Transcript normalization: prompts and attachments, tool content, terminal / image output, session import, subagents, usage, forking, export | `docs/dev/transcript-pipeline.md` |
| `packages/ui` structure, stylesheets, markdown links, slash commands, render budget | `docs/dev/webview.md` |
| UI primitives, tokens, rows, menus, overlays, composer, model panels | `docs/dev/ui-conventions.md` |
| Motion, rails, sticky prompts, scrollbars, folding, appearance | `docs/dev/ui-motion-layout.md` |
| Edits and retries, model identity, plans, permissions, controls, queue, workspace hooks, scope / deletion, prefs, compaction and context usage, questions, to-dos | `docs/dev/protocol-gotchas.md` |
| The built-in agent (`acpira-agent`): providers, ACP surface, MCP, Plan mode, goals, compaction, session records | `docs/dev/builtin-agent.md` |
| Plugins: manifest, runtime, hooks, activation, safe mode | `docs/dev/plugins.md` |
| SSH remote workspaces: helper, deploy, connections, remote run | `docs/dev/remote.md` |
| Eval benches | `docs/dev/evals.md` |
| Anything specific to one agent CLI | `docs/dev/agent-quirks.md`, then `docs/acp-agents-compat.md` (the compatibility matrix) |
| `apps/intellij/` | `docs/dev/intellij.md` |
| `apps/desktop/` | `docs/dev/desktop.md` |
| `apps/ios/` | `docs/dev/ios.md` |
| Windows compatibility boundaries and regression checks | `docs/dev/windows-compatibility.md` |

Public docs: `docs/context-api.md` (the `acpira.context` contract other extensions call), `docs/chatgpt-bridge.md`, `docs/releasing.md`, `docs/roadmap.md`, `docs/harness-evaluations.md`. The rest of `docs/` is local only and may be absent (`HANDOFF.md`, `TASKS.md`, plans and scratch notes).
