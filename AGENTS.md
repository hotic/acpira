# Acpira

A chat shell for VS Code / Cursor (and IntelliJ) that drives official agent CLIs (`grok agent stdio`, `devin acp`, `kimi acp`, or any ACP-compatible command) over [ACP](https://agentclientprotocol.com) (JSON-RPC over stdio). The shell owns UI, session organization, permission approvals, accounts, and context budget; model calls and agent execution stay in the CLIs.

## Working rules

- Done means: `pnpm typecheck` and `pnpm test` pass, plus `pnpm build` when a bundle entry or the webview changed and `cargo test --workspace` + `cargo clippy --workspace --all-targets --all-features -- -D warnings` (in `rust/`) when the engine changed, and no orphaned `fake-agent.ts` process remains (the check is under `pnpm test` below). Kotlin changes also pass `cd idea && ./gradlew test`. Visual changes are checked in `pnpm harness` or the LAB when a browser is available; otherwise the summary says they were not.
- Keep going while the next step needs no decision, and put status notes in the same message as the next action. Stop and ask only when blocked on a decision, or before: deleting files this task did not create, force-pushing or rewriting history, deleting or rewriting data outside the repository (`~/.acpira`, the CLIs' own stores), or sending real prompts to a CLI (the host probe scripts, `pnpm probe` with a prompt), since those spend model calls.
- Read the area's doc from the table below before changing it; the recorded wire behaviour outranks the ACP spec and memory of it. A newly verified fact goes into the matching `docs/dev/` file together with the agent version it was observed on. This file stays an index under 16 KB, because some hosts truncate it there.
- Long multi-part work keeps its checklist in `docs/TASKS.md` (ignored by git) and ticks items as they finish.
- A run ends with, in this order: **Blocked on** (decisions needed), **Changed**, **Verified** (commands actually run), **Not verified** (and why). Anything not confirmed against code, tests or a real CLI is marked unverified, with where it was looked for.

## Commands

- `pnpm build` — the VS Code shell (esbuild → `dist/extension.cjs`) + webview bundle (Vite → `dist/webview/main.{js,css}`); the engine is the Rust sidecar (`rust/`, binary `acpira`)
- `pnpm harness [port]` — `acpira --ws` via cargo (release build): the browser harness at the URL printed on stderr (`http://127.0.0.1:7357/?token=…`) runs the real webview bundle against the real sidecar and CLIs without any IDE (`test/host-preview/`, query also `host=editor&agent=kimi&theme=light&cwd=/abs/path`). The handshake token is required; data lives under `~/.acpira/harness` unless `--home` is given; hello `agents` from the page is ignored. Build first; the settings live in `localStorage`
- `pnpm typecheck` — two tsconfigs (`tsconfig.webview.json` covers `src/webview` + `src/shared` + the LAB in `src/lab` / `lab`; `tsconfig.host.json` the host; shared options in `tsconfig.base.json`); the root `tsconfig.json` is a solution file with `references` only, do not add `compilerOptions` to it. A source directory outside both `include` lists is neither type-checked nor alias-resolved by the IDE
- `pnpm test` — vitest: shell, webview and shared suites plus envelope contracts against the Rust sidecar (`test/sidecarShell.ts`; `test/globalSetup.ts` brings `rust/target/debug/acpira` up to date, `ACPIRA_SIDECAR_BIN` points at another binary). `test/fake-agent.ts` is a fake ACP agent built on the SDK; the contract suites start it through the `node_modules/.bin/tsx` wrapper, which relays SIGTERM to the real node, while a test that SIGKILLs the agent spawns `node --import tsx/dist/loader.mjs` directly (the Rust engine suites always do), or the kill hits the wrapper and the fixture lives on as an orphan (`ps -eo pid,ppid,command | awk '$2==1 && /fake-agent.ts/'` should print nothing after a run)
- `pnpm probe grok [--auth] [--api-key-env VAR] [--import-local] [--image PATH] [--link PATH] [--embed PATH] [--elicit] [--wait MS] [prompt]` — run `initialize` + `session/new` (+ one prompt, optionally with an inline image / `resource_link` / embedded `resource` block) against a CLI without VS Code; `--elicit` advertises form elicitation and prints / auto-answers `elicitation/create`; `--wait MS` keeps the process alive after the last response, which is where `available_commands_update` arrives (Grok / Kimi send it after `session/new` returns, Devin during). Use this first when debugging protocol issues
- `pnpm exec tsx --tsconfig tsconfig.host.json scripts/probe-agent-host.ts <agent> [--attach] [--shell] [--write] [--plan] [--image] [--raw]` — the same path the webview takes (envelopes into the Rust sidecar, `scripts/lib/host.ts`) against a real CLI: new session, controls, one prompt, boolean controls flipped on/off, optionally a dropped text attachment, a shell command, a permission-card write (`--write`), a plan-review reject (`--plan`) or an image view (`--image`), permission cards answered like a click; `scripts/probe-opencode-host.ts [--model provider/model]` adds OpenCode's mode / effort switching, a two-file write and the native-session import round trip on a second manager (`--model` when the CLI's default route is down — OpenCode retries a 503 silently, so a first prompt that never returns is the gateway, check it with curl before suspecting the host); `scripts/probe-restore-host.ts <agent>` is the generic restore round trip for any agent: a tiny prompt, a same-store reopen (same `acpSessionId`, no duplicated or re-sent turns, a follow-up that must recall the first reply — it logs whether `session/resume` or `session/load` ran), then `session/list` → import on a fresh store (≥2 sealed replayed turns, re-listing marks it imported). All spend real model calls; give the CLI a temp store (`DSH_HOME`, `PI_CODING_AGENT_DIR`) where it has one
- `pnpm exec tsx --tsconfig tsconfig.host.json scripts/probe-subagents.ts <agent> [--air] [--no-caps] [--import-local] [--cmd "<command line>"] [--wait MS]` — raw-ndjson subagent probe (what an agent really emits, SDK validation bypassed) and `scripts/probe-subagents-host.ts <claude|devin>` — the same through the sidecar; both spend real model calls and leave their logs / temp stores in place for inspection
- `pnpm package` — build everything and package this machine's platform `.vsix` (with its Rust sidecar at `bin/`); `pnpm build:sidecar [--all | <os>-<arch>…]` builds release sidecars into `dist/sidecar/`, `node scripts/package-vsix.mjs [--all | <vsce-target>…]` packages from them (one VSIX per platform, no universal package)
- `cd rust && cargo test --workspace` — the engine both shells ship (see `docs/dev/host-architecture.md`): unit tests plus `crates/acpira-host/tests/engine/` (sessions, manager, accounts, subagents … against `test/fake-agent.ts`; skipped without `node_modules`). `test/fixtures/engine-*.json` are its golden normalize / diff output, which the webview suites render; `ACPIRA_UPDATE_FIXTURES=1 cargo test golden` rewrites them after an intended change. `acpira agents --json` shows how each built-in CLI resolves and is initialized

## Hard rules

- `src/webview/` must never import `vscode`. Only `src/host/extension.ts`, `bridge.ts`, `vscodePlatform.ts` and `files.ts` import it; `src/host/shell/` (the extension's sidecar client) does not
- All engine behaviour lives in the Rust sidecar; the TypeScript under `src/host/` is the VS Code shell only (webview panels, platform RPCs, sidecar lifecycle, the one-time legacy migration). `HostRuntime::create` in `rust/crates/acpira-host/src/runtime.rs` is the engine's single composition root
- `src/shared/transcript.ts` (`SessionView`) and `src/shared/protocol.ts` (`HostMsg` / `WebviewMsg`) are the only host ↔ webview contract; `src/shared/sidecar.ts` is the sidecar wire contract (`SIDECAR_PROTOCOL_VERSION`)
- Sidecar stdout carries envelopes only; logs go to stderr
- The root `tsconfig.json` holds `references` only
- `~/.acpira` is shared by every VS Code / Cursor window and IDEA sidecar: files are written tmp + rename, and `accounts.json`, `secrets.json` and `sessions/prefs.json` are edited under `store/fileLock.ts`
- Every stream chunk pushes the whole `SessionView`; nothing may defeat the `shared/reuse.ts` reference reuse and `memo` chain (see the render budget in `docs/dev/webview.md`)
- Write code comments in English
- Components must not contain raw numbers: sizes, type scale, spacing, and radii all use token classes (`h-ctl`, `px-pad`, `gap-gap`, `rounded-md`, …)
- Every "row" (thought / plan / tool / status / session item) uses `ui/Row`; command execution is a row too (command on the row, output card below), not a separate block

## Layout

- `src/shared/` — contracts and pure logic shared by host and webview (transcript, protocol, sidecar, appearance, agent order, models, export)
- `src/host/` — the VS Code shell: `extension.ts`, `bridge.ts` (webview panels), `vscodePlatform.ts` (platform RPCs), `files.ts` / `fileRank.ts`, `shell/` the sidecar client and locator, `store/` + `accounts/AccountStore.ts` for the legacy migration
- `src/webview/` — React UI: `ui/` primitives, `chat/`, `settings/`, `effects/`, `styles/`
- `rust/` the engine (`acpira-shared` contracts, `acpira-host`: `acp/` the ACP client and session state machine, `session_manager.rs`, `store/`, `accounts/`, `external/` ChatGPT mirrors, `sidecar/` envelope server + harness; binary `acpira`)
- `test/` vitest suites and `fake-agent.ts`; `scripts/` probes; `idea/` the IntelliJ plugin; `lab/` + `src/lab/` the LAB (local only)

## Read before changing

| Area | Doc |
|---|---|
| Shells and the sidecar protocol, runtime, bridge core, VS Code platform, session manager viewers, agent registry / order / launching / availability, `probeControls`, data dir and `TranscriptStore`, accounts, file lock, quota monitoring | `docs/dev/host-architecture.md` |
| Prompt staging and `promptCapabilities`, tool content lists, terminal / image output, native session import, subagents, JetBrains AIR `sessionFailure` / `asyncTasks`, empty completions, attachments, turn usage, forking, export | `docs/dev/transcript-pipeline.md` |
| `src/webview` structure and stylesheet layering, file links in markdown, slash commands, transcript render budget (memo, lazy folds, stream glyphs) | `docs/dev/webview.md` |
| `ui/` primitives, tokens, rows, buttons, menus, overlays, model panels, composer, question card, toolbar chips, model visibility | `docs/dev/ui-conventions.md` |
| Orb and Working label, motion, rails, sticky user prompts, scrollbars, to-do presentation, folding modes, appearance axes, rendering preferences | `docs/dev/ui-motion-layout.md` |
| Historical edits and retries, model identity, plan approval / execution, permissions, controls, follow-up queue, session scope / deletion, remembered prefs, account binding, compaction, failed prompts, questions, to-do bar | `docs/dev/protocol-gotchas.md` |
| Anything specific to Devin, Grok, Kimi, Codex, Claude, OpenCode, DeepSeek Harness or Pi | `docs/dev/agent-quirks.md`, then `docs/acp-agents-compat.md` (verified / source / unverified matrix) |
| `idea/`: Kotlin modules, JCEF, native RPC, sidecar service, split mode, distribution, release | `docs/dev/intellij.md` |

Also committed: `docs/chatgpt-bridge.md` (ChatGPT conversation mirrors). The rest of `docs/` is local only and may be absent (`HANDOFF.md`, `history-editing.md`, `compaction-queue.md`, `plan-modes.md`, `grok-controls-verification.md`).
