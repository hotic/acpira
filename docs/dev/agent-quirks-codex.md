# Agent quirks: Codex (codex-acp)

How codex-acp and the bundled codex behave on the ACP wire, with the version observed ("source" = read from the shipped package). Other CLIs and cross-agent notes: `agent-quirks.md`.

## Codex (codex-acp)

- codex-acp 1.13.0 (bundled codex 0.155.1, `CODEX_PATH` overrides): `loadSession` + list / resume / close / delete / fork; auth `api-key` and `chat-gpt` with no terminal methods, so sign-in goes through `authenticate`. Terminal fallback is `codex-acp cli login`; bare `codex-acp login` needs a separate `codex`.
- Controls: modes read-only / agent / agent-full-access (2.1.1 adds `workspace-write`), `model`, `reasoning_effort` (low … ultra), a `fast-mode` boolean, `collaboration_mode` (default / plan). Skills appear as `/$<name>` commands. Hidden models are not listed.
- A thread on a model no longer listed: the adapter lists it first but drops `reasoning_effort` and `fast-mode`, and an effort set fails invalid params until a listed model is picked (1.13.0 source).
- List / resume work only on the canonical cwd: the adapter canonicalizes (`/var` → `/private/var`) and string-compares per page, so `native_sessions.rs` retries with `realpath`.
- 2.1.1: a rollout is written only after the thread's first user message; once the process is replaced, resuming an unprompted thread fails `Internal error` with `data.details "no rollout found for thread id"`, meaning gone.
- 2.1.1 / codex 0.159.3: one writer per thread (`$CODEX_HOME/thread-writer-locks/`); resume while another process (another window, the Codex app's `app-server`) holds it fails `"thread <id> already has an active writer"`, classified locked. No PID, so no take-over.
- Compaction updates are gated on `clientCapabilities.session.compaction` (source).
- Tool calls open with `in_progress` (1.13.0), unlike Claude. 1.13.0 left a summoned child's `tool_call` `pending` forever with no permission request; `live_activity` stops naming an open tool once later output arrives.
- Image generation (1.13.0): a `kind: other` call titled `Image generation`, then a completed update with `Revised prompt:` text plus a full base64 `image` block, repeated in `rawOutput`.
- **Ultra** is a real `reasoning_effort` ("Maximum reasoning with automatic task delegation", GPT-6 Astra and others); delegated agents arrive as ordinary `session`-tier children, no workflow-specific wire (1.13.0). `:generation:` child ids after a restart stream normally.
- Steering: answers an idle steer `startedNewTurn` whatever the opt-in; `_meta.codex.threadStatus` gives the idle edge (`protocol-gotchas.md` Queue).
- **Codex accounts**: the login is `$CODEX_HOME/auth.json`, refreshed in place with rotating refresh tokens, so never copied. A saved account is a private `CODEX_HOME` with its own `auth.json` and symlinks for everything else, so threads stay one store.
- Quota: `GET chatgpt.com/backend-api/wham/usage` with the access token and `ChatGPT-Account-Id` → `rate_limit.primary_window` / `secondary_window` (Pro has one weekly window). Exhaustion is AIR `quota_exhausted` (source). Unverified: cross-account resume through the shared store.
- **Goals**: 1.13.0 advertises `_meta.goal { controlMethod: '_session/goal', actions: set/pause/resume/clear }` and snapshots at `session_info_update._meta.goal`; 2.x moves both under `_meta.jetbrains.air` (`vendors/goal.rs` reads both). `/goal …` prompts equal the requests.
- Goal behaviour (1.13.0 and 2.2.2, live): an unmet goal continues after the turn with no `session/prompt`, bracketed by `threadStatus` active … idle; the host runs it as a detached turn. After two continuations without progress Codex sets `blocked` itself.
- Pause / clear never interrupt a running turn, only further continuations (a paused goal can still reach `complete`). `/goal resume` on a `complete` goal reactivates it. `usageLimited` / `budgetLimited` map to `limited`.
- Open: codex-acp 2.x's "AIR tool call contract, exact diff patches" not run at turn level.

