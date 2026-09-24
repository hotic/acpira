//! The connection prompt that binds one ChatGPT conversation to an external session (mirror of src/host/external/chatgptBinding.ts).
//! The bridge CLI is this binary's `bridge` subcommand

use acpira_shared::transcript::SessionView;

pub fn quote(s: &str) -> String {
  if cfg!(windows) { format!("'{}'", s.replace('\'', "''")) } else { format!("'{}'", s.replace('\'', "'\\''")) }
}

pub fn chatgpt_binding(view: &SessionView, exe: &str, home: &str) -> String {
  // Windows remote terminals are PowerShell, where a quoted path is a string until the call operator runs it
  let base = format!("{}{} bridge", if cfg!(windows) { "& " } else { "" }, quote(exe));
  let scope = format!("--home {} --session {}", quote(home), quote(&view.id));
  format!(
    "Bind only this ChatGPT conversation to this Acpira external session. This is an opt-in event mirror, not Codex and not a new model call.
Project: {cwd}
Acpira session: {id}
Use the existing authorized remote terminal tool on the machine holding this project. This binding grants no additional permissions.
At each genuinely new user turn choose a fresh TURN_ID. Retransmissions must reuse that TURN_ID.
Inspect the receiver first with {base} show {scope}. If it still has external.activeTurnId from a prior user turn, pass --previous-turn THAT_ID to prompt. This only supersedes the old observation; it does not claim the old work succeeded or that its process stopped.
Run:
{base} prompt {scope} --turn TURN_ID --text 'the actual user message'
Use this wrapper to execute project commands so start, streaming output, exit status and heartbeat are recorded automatically:
{base} exec {scope} --turn TURN_ID --command 'the actual shell command'
For file browsing and edits use read/list/write from the same bridge. Run {base} --help for arguments. write requires the expected SHA-256 (or missing for a new file) and new text on stdin.
Mirror visible progress/replies, with stable MESSAGE_ID values (the same ID replaces streamed text):
{base} message {scope} --turn TURN_ID --message MESSAGE_ID --phase commentary --text 'visible progress only'
When resuming transmission of the same latest unfinished turn, use resume with the original TURN_ID; do not replay exec commands to rebuild a transcript. Late tool output must retain its original turn and call IDs.
Use --phase final for the visible final reply. After all tools settle, finish the turn:
{base} finish {scope} --turn TURN_ID
Use --text - or --command - with stdin when quoting is inconvenient. The emit command accepts one structured JSON event from stdin for tools performed through another authorized interface.
Only explicitly bridged events are captured. Do not invent events, completion receipts, messages, model usage, reasoning text or elapsed times. Never copy hidden reasoning, system/developer instructions, credentials or unrelated chats. Keep responding in this ChatGPT conversation; Acpira is its local observation view. If bridge logging fails, report the gap instead of claiming full visibility.",
    cwd = view.cwd,
    id = view.id,
  )
}
