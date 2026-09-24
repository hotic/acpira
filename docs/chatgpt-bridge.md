# ChatGPT conversation mirrors

Acpira can display an externally driven ChatGPT conversation as its own **ChatGPT** channel, alongside native ACP sessions. This is an opt-in local event bridge, **not Codex, not a second model, and not a complete ChatGPT transcript API**. The model loop continues in ChatGPT; Acpira renders received user messages, visible assistant text, tool calls, outputs and actual completion receipts.

## Connect from VS Code

1. Install or update Acpira and reload the extension host.
2. Open the project to bind. Run **Acpira: Connect ChatGPT Session** from the command palette. Acpira opens a new mirror and copies its connection instructions.
3. Paste the instructions into exactly one ChatGPT conversation with an already authorized remote terminal connection to this machine. The bridge does not grant extra access.
4. Continue chatting there. The agent must route its project operations through the bridge and explicitly mirror visible messages. The Acpira conversation updates without reloading the editor.

Alternatively, open **Settings → ChatGPT → Desktop Commander** and create a conversation binding, then use **Copy connection instructions** in the mirror. ChatGPT is deliberately absent from the launchable-agent menu and default-agent picker; existing mirrors remain in history and channel filtering. Each mirror has its own ID. Never bind two unrelated ChatGPT conversations to the same mirror.

Reinstalling the extension makes the channel and bridge available; it **does not by itself subscribe to all ChatGPT conversations**. A tool safety or authorization denial must not be bypassed; report the missing capture instead.

## What is and is not observable

- `exec` records a tool start before execution, streams combined stdout/stderr, sends heartbeats and records the real process exit status. Nonzero exit codes remain nonzero to the calling agent.
- `read`, `list` and `write` expose file operations. `write` checks the expected SHA-256 (or `missing`), then records the actual before/after diff. These file commands resolve paths against the bound project and reject symlink escapes. **`exec` is not a sandbox** and retains the invoking OS user's permissions.
- Assistant text is supplied explicitly as `commentary` or `final`. This bridge does not read browser profiles, cookies, credentials, other conversations or private reasoning.
- Calls made directly through Desktop Commander or another connector are **not automatically intercepted**. The caller can explicitly record them using `emit`, but that is cooperative reporting, not an independent audit.
- Token usage, model identity and remote inference state remain unknown unless supported by a real future event source. A heartbeat only proves the bridge is active, not that ChatGPT is thinking.
- After 45 seconds without a fresh event during an open turn, the UI says **remote state unknown**. It does not fabricate a successful finish or claim to stop the remote model. Reloading the IDE preserves the transcript and does not cancel ChatGPT.

There are intentionally no send, model-switch or stop controls for these sessions. Continue/cancel in ChatGPT. Rename, pin, history and project filtering use the normal Acpira surfaces.

## Local CLI

The CLI is the `bridge` subcommand of the sidecar binary every package carries (`bin/acpira` in the VSIX, `sidecar/bin/<os>-<arch>/acpira` in the IntelliJ plugin); no Node.js is needed. The copied instructions contain the installed path and the correct profile directory (`--home`). The CLI has no network listener and requires no model API key. A repository build also produces the TypeScript equivalent, `node dist/chatgpt-bridge.cjs`, with the same commands.

```sh
/path/to/extension/bin/acpira bridge --help
/path/to/extension/bin/acpira bridge open \
  --key explicit-source-key --cwd /absolute/project --title 'ChatGPT task'
```

`open` returns the `sessionId`. Commands take `--session ID`, and events inside a turn additionally take `--turn TURN_ID`:

```sh
/path/to/acpira bridge prompt --session ID --turn TURN_ID --text 'Actual user message'
/path/to/acpira bridge exec --session ID --turn TURN_ID --command 'git status --short'
/path/to/acpira bridge message --session ID --turn TURN_ID \
  --message MESSAGE_ID --phase final --text 'Actual visible reply'
/path/to/acpira bridge finish --session ID --turn TURN_ID
```

Use `--text -` or `--command -` to read stdin rather than interpolate text into a shell command. `write` always takes new text on stdin. `show --session ID` prints the normalized view. `emit --session ID` takes one JSON event on stdin, with a unique `id`, `turnId`, and the payload defined in `src/host/external/chatgptEvents.ts`.

Replaying the exact same event ID and payload is idempotent. Reusing an ID with different content fails. A successful `end_turn` is refused while a tool has no completion receipt. An explicitly reported cancellation/error ends source generation without claiming that a local command was terminated. CLI convenience commands generate fresh event IDs; prompt retransmissions reuse the same turn ID and text, and generic event retries use `emit` with stable event IDs.

## Follow-ups and interrupted transmission

`show` exposes `external.activeTurnId`. A genuinely new user message can use `prompt --previous-turn THAT_ID --turn NEW_ID` when the preceding turn has no end receipt. The exact previous ID is checked transactionally. The older turn keeps its transcript and unconfirmed outcome; this is not a synthetic success or cancellation. Late command outputs, heartbeats and exit receipts remain associated with their original turn and cannot renew the newer turn's lease.

A repeated `prompt` with the same turn ID and identical text is idempotent, including after completion. Different text with that ID is refused. For the latest unfinished turn, `resume --turn ORIGINAL_ID` renews observation without appending a user prompt or executing its tools again. A completed/cancelled generation is not reopened; another generation needs a new source attempt/turn rather than replaying shell operations.

The current connector does not supply automatic browser stop-button events. A quiet source means **unknown**, not cancelled. A source cancellation receipt does not imply that local commands have exited. Only observed process receipts settle those tool cards.

## Integration status and image boundary

Settings separates local component/process evidence, cloud device pairing and received project messages. It never infers a valid cloud pairing from a process or configuration file. Failed or inconclusive detection remains explicit; the dashboard is the place to verify cloud pairing. Detection reads executable presence, process presence and config metadata, not credentials or other conversations.

Images are not transported by this CLI implementation. ChatGPT container paths are not paths on the executing machine, and web uploads are not copied automatically. Missing images must be indicated, not represented by invented thumbnails.

## Storage, privacy and limits

Mirrors live under `$ACPIRA_HOME/bridges/chatgpt`, defaulting to `~/.acpira/bridges/chatgpt`. Native ACP session storage is untouched. Files are written with mode `0600`, directories are created with `0700`, and writes use the existing cross-process lock plus atomic replacement. All local IDE hosts can observe the same session without becoming its writer.

Tool output is displayed up to 256,000 characters per tool, with an explicit truncation marker. An event is limited to 2 MB and a mirror record to 64 MiB / 100,000 event receipts. This is not a lossless forensic recorder. Text file helpers support files up to 1 MB; diffs reuse the existing preview limits. Do not put credentials or unrelated private data in commands/messages intended for the mirror.

Deleting a mirror stops accepting events for that ID. Undo is available for 30 seconds; after it expires, a running bridge/host removes transcript contents and retains a small tombstone to reject delayed writers. Closing every host before that cleanup postpones it until the next refresh. This is logical deletion, not secure disk erasure. During `exec`, an audit-write failure is reported and a termination signal is sent to that command's process group; no UI action can cancel ChatGPT inference.

## Verification

`test/chatgptBridge.test.ts` covers lifecycle, identity, isolation, concurrency, replay, stale-state honesty, output limits, diff receipts, deletion and malformed inputs. `test/chatgptCli.test.ts` runs real child processes, checks streaming before completion, nonzero exit codes, guarded edits and path confinement. `test/hostRuntime.test.ts` verifies live view delivery and that selecting ChatGPT never spawns an ACP process.

`test/chatgptContinuation.test.ts` covers transactional continuation, late receipts, source cancellation versus local command lifetime, idempotent prompt delivery and explicit unknown transport states. Early events missed before successful binding are not backfilled. Automatic transcript capture, source cancellation events, image ingestion and cloud pairing verification remain unsupported.
