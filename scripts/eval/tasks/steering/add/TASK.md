# Steering the built-in agent

Acpira's engine can send a message into a turn that is still running: the queue's Steer button, or a message sent
while the agent works. It does this for every agent that advertises the ACP extension `_session/steering`
(claude-agent-acp and codex-acp implement it). Implement it in Acpira's built-in agent, the Rust crate in
`rust/crates/acpira-agent` (`acpira agent`, described in `docs/dev/builtin-agent.md`).

## Wire

- `initialize` advertises it at the top level of its result, next to what it already returns:
  `"_meta": { "steering": { "supported": true } }`.
- Request `_session/steering` with params
  `{ "sessionId": …, "prompt": [ContentBlock, …], "_meta": { "steering": { "idleBehavior": "promptRequired" } } }`.
  The engine always sends `idleBehavior`.
  - While a turn runs and can still take the message, answer `{ "outcome": "injected" }` at once. The message waits for
    the turn's next model call.
  - With no turn running, while a `/compact` runs (it calls no model a message could reach), or once the running turn
    has decided to end: answer `{ "outcome": "promptRequired", "reason": "noRunningTurn" }` when `idleBehavior` is
    `promptRequired`, and keep nothing (the client then sends it as an ordinary prompt). Without that opt-in, refuse
    with JSON-RPC error -32603: this agent never starts a turn of its own.
  - An empty `prompt` is invalid params (-32602).

## What the model gets

- Everything steered in since the turn's last model call goes in front of its next call as one user message, behind
  the tool results of the round in flight (or behind the reply that was streaming). The turn's first call carries the
  prompt alone: a message sent before it goes with the call after it.
- That message's text is a fixed note inside a `<steer>` element, then each message, separated by blank lines:

  ```
  <steer>
  The user wrote this while you were working. (the rest of the note in your own words)
  </steer>

  first message

  second message
  ```

  The note must begin with exactly `The user wrote this while you were working.` Each message's blocks become parts
  the way a prompt's blocks do (attachments, images when the model takes them, `/skill` mentions expanded).
- On Chat Completions and Responses it is a user message of its own. On Anthropic Messages it rides in the user message
  that carries the round's `tool_result` blocks, as a text block after them.
- Nothing running is cut short for it: a running command finishes, and a streaming reply streams to its end.
- A reply without tool calls would end the turn, and so would a rejected permission card. With a message waiting, the
  turn makes one more call instead, so the model answers the message in the same turn. Deciding to end and closing the
  turn to new messages happen together: a message sent after the turn decided to end is handed back
  (`promptRequired`), never dropped or left behind.
- A message no call carried (the turn was cancelled, failed or stopped at a limit) goes into the history when the turn
  ends, behind what the turn left (after a cancel: behind the cancelled calls' results), so the next turn's request
  has it before the new prompt.
- The history stays a stable prefix: a request after a steered message repeats every earlier message byte for byte,
  and so does the first request after the session is reopened (`session/load` or `session/resume`).
- A steered message does not start a turn: whatever the agent does per turn (compaction's count of turns, the reasoning
  it echoes for the current turn, the running turn's prompt) still takes the user's prompt as the turn's start.

## What the client sees

- Live, the agent sends no `user_message_chunk` for a steered message: the client shows the message itself.
- The session log keeps it, so `session/load` replays it where the model got it (after the round's tool call updates,
  before the agent's next message) as `user_message_chunk` updates: one per content block of each message, without the
  note, and a `user_message_chunk` with the text `"\n\n"` between two messages.

## Checks

The crate's tests run with `cd rust && cargo test -p acpira-agent`. For tests that steer into a reply that is still
streaming or a round that waits on a card, `mock::Reply::Held` holds a scripted reply until the test opens its
`mock::Gate`, and `support::Answer::Wait` holds a permission card until `Harness::release`.
