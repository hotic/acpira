//! What a stand-in CLI is told: the note that its project is on the host, with the host's instructions, and the
//! session request that starts it in the stand-in, its native tools stopped where the request can stop them

use serde_json::{Map, Value, json};

use super::stand::Stand;
use super::tools::READ_ONLY;
use super::{Brief, Gate};

/// The note on where the project is: the host and its platform, the stand-in's meaning, the project tools, what stays
/// here, then each instructions file the host has (`instructions`: path and text), since the CLI cannot find them
pub fn note(stand: &Stand, places: Option<&Value>, instructions: &[(String, String)]) -> String {
  let s = |k: &str| places.and_then(|p| p.get(k)).and_then(Value::as_str).filter(|v| !v.is_empty());
  let platform = match (s("os"), s("arch"), s("shell")) {
    (Some(os), Some(arch), Some(shell)) => format!(" ({os} {arch}, shell {shell})"),
    _ => String::new(),
  };
  let prefix = &stand.profile.prefix;
  let mut out = format!(
    "# Remote project\n\nThis session's project is on the SSH host {alias}{platform}, in {host}. Your working directory {local} is an \
     empty stand-in on this machine for that folder: it holds none of the project's files, and it is not a git repository even when \
     the project is one. The project tools {prefix}Read, {prefix}Write, {prefix}Edit, {prefix}MultiEdit, {prefix}Bash, {prefix}Grep, \
     {prefix}Glob and {prefix}LS run on {alias}: a relative path resolves against {host}, a path under the stand-in means the same \
     file there, and commands (git, builds, tests) run there; one started with run_in_background keeps running there, read with \
     {prefix}BashOutput and stopped with {prefix}KillShell. Write paths as {host}/… To copy one of your own files on this machine \
     (such as a generated image or binary artifact) onto {alias}, call {prefix}Write with file_path and source_local_path instead \
     of encoding its bytes in text. {own}",
    alias = stand.alias,
    host = stand.host,
    local = stand.local.display(),
    own = stand.profile.own_note,
  );
  for (path, text) in instructions {
    out.push_str(&format!("\n\nThe project's instructions, from {path} on {}:\n\n<instructions>\n{}\n</instructions>", stand.alias, text.trim()));
  }
  out
}

/// The session request of a stand-in CLI: the stand-in as its cwd, the built-in agent's host `_meta` taken out, the note
/// where the profile's CLI reads one on the request, and claude-agent-acp's native tools switched off and aliased onto
/// the remote ones. A `Brief::StandFile` note goes into the stand-in instead (`stand_file`)
pub fn with_stand(mut req: Value, stand: &Stand, note: &str) -> Value {
  req["cwd"] = json!(stand.local);
  if let Some(meta) = req.get_mut("_meta").and_then(Value::as_object_mut) {
    meta.remove(acpira_shared::remote::META);
  }
  let profile = &stand.profile;
  if profile.gates.contains(&Gate::ClaudeOptions) {
    let options = &mut req["_meta"]["claudeCode"]["options"];
    let mut disallowed: Vec<Value> = options.get("disallowedTools").and_then(Value::as_array).cloned().unwrap_or_default();
    disallowed.extend(profile.native.shell.iter().chain(&profile.native.files).map(|t| json!(t)));
    options["disallowedTools"] = json!(disallowed);
    let aliases: Map<String, Value> = acpira_shared::host_tools::REMOTE.iter().map(|t| ((*t).to_owned(), json!(format!("{}{t}", profile.prefix)))).collect();
    options["toolAliases"] = Value::Object(aliases);
    let mut allowed: Vec<Value> = options.get("allowedTools").and_then(Value::as_array).cloned().unwrap_or_default();
    allowed.extend(READ_ONLY.iter().map(|t| json!(format!("{}{t}", profile.prefix))));
    options["allowedTools"] = json!(allowed);
  }
  match &profile.brief {
    Brief::ClaudeSystemPrompt => req["systemPrompt"] = json!({ "mode": "append", "content": [{ "type": "text", "text": note }] }),
    Brief::AntigravityMeta => {
      let sp = &mut req["_meta"]["antigravity"]["systemPrompt"];
      let next = match sp.get("append").and_then(Value::as_str).filter(|s| !s.is_empty()) {
        Some(prev) => format!("{prev}\n\n{note}"),
        None => note.to_owned(),
      };
      sp["append"] = json!(next);
    }
    Brief::StandFile(_) => {}
  }
  req
}

/// Where a `Brief::StandFile` note goes, with its text: the stand-in's instructions file, which the CLI reads as the
/// project's own. None for a profile whose note travels on the request
pub fn stand_file(stand: &Stand, note: &str) -> Option<(std::path::PathBuf, String)> {
  match &stand.profile.brief {
    Brief::StandFile(name) => Some((stand.local.join(name), format!("{note}\n"))),
    _ => None,
  }
}

#[cfg(test)]
mod tests {
  use super::*;
  use crate::acp::local_run::{builtin, generic};
  use std::path::Path;
  use std::sync::Arc;

  fn stand(id: &str) -> Stand {
    let profile = builtin(id).and_then(|l| l.stand().cloned()).unwrap();
    Stand::new(profile, "example-host", "/home/u/proj/", Path::new("/Users/me/.acpira"))
  }

  #[test]
  fn the_request_runs_in_the_stand_in_with_the_native_tools_off() {
    let s = stand("claude");
    let req = json!({ "cwd": "/home/u/proj", "_meta": { "acpira/remote": { "alias": "example-host" }, "claudeCode": { "options": { "thinking": { "type": "adaptive" } } } } });
    let places = json!({ "os": "linux", "arch": "x86_64", "shell": "/bin/bash" });
    let text = note(&s, Some(&places), &[("/home/u/proj/CLAUDE.md".into(), "Use pnpm.".into())]);
    let out = with_stand(req, &s, &text);
    assert_eq!(out["cwd"], json!(s.local));
    assert!(out["_meta"].get("acpira/remote").is_none());
    let o = &out["_meta"]["claudeCode"]["options"];
    assert_eq!(o["thinking"]["type"], "adaptive");
    assert!(o["disallowedTools"].as_array().unwrap().iter().any(|t| t == "Bash"));
    assert!(o["disallowedTools"].as_array().unwrap().iter().any(|t| t == "Monitor"));
    assert!(o["disallowedTools"].as_array().unwrap().iter().any(|t| t == "NotebookEdit"));
    assert_eq!(o["toolAliases"]["Edit"], "mcp__acpira__Edit");
    assert_eq!(o["toolAliases"]["MultiEdit"], "mcp__acpira__MultiEdit");
    assert_eq!(
      o["allowedTools"],
      json!(["mcp__acpira__Read", "mcp__acpira__Grep", "mcp__acpira__Glob", "mcp__acpira__LS", "mcp__acpira__BashOutput", "mcp__acpira__KillShell"])
    );
    let text = out["systemPrompt"]["content"][0]["text"].as_str().unwrap();
    assert_eq!(out["systemPrompt"]["mode"], "append");
    assert!(text.contains("SSH host example-host (linux x86_64, shell /bin/bash), in /home/u/proj"));
    assert!(text.contains("Use pnpm."));
    assert!(stand_file(&s, text).is_none());

    let agy = stand("antigravity");
    let agy_req = json!({ "cwd": "/home/u/proj", "_meta": { "acpira/remote": { "alias": "example-host" } } });
    let agy_note = note(&agy, Some(&places), &[("/home/u/proj/GEMINI.md".into(), "Use cargo.".into())]);
    let agy_out = with_stand(agy_req, &agy, &agy_note);
    assert_eq!(agy_out["cwd"], json!(agy.local));
    assert!(agy_out["_meta"].get("acpira/remote").is_none());
    assert!(agy_out["_meta"].get("claudeCode").is_none());
    let agy_text = agy_out["_meta"]["antigravity"]["systemPrompt"]["append"].as_str().unwrap();
    assert!(agy_text.contains("mcp_acpira_Read"));
    assert!(agy_text.contains("source_local_path"));
    assert!(agy_text.contains("Use cargo."));
  }

  #[test]
  fn another_cli_reads_the_note_from_the_stand_in() {
    let s = Stand::new(Arc::new(generic("mine", "Mine")), "example-host", "/home/u/proj", Path::new("/Users/me/.acpira"));
    let text = note(&s, None, &[("/home/u/proj/AGENTS.md".into(), "Use make.".into())]);
    let out = with_stand(json!({ "cwd": "/home/u/proj", "_meta": { "acpira/remote": {} } }), &s, &text);
    assert_eq!(out["cwd"], json!(s.local));
    assert!(out.get("systemPrompt").is_none() && out["_meta"].as_object().unwrap().is_empty(), "{out}");
    let (path, body) = stand_file(&s, &text).unwrap();
    assert_eq!(path, Path::new("/Users/me/.acpira/remote/stand/mine/example-host/home/u/proj/AGENTS.md"));
    assert!(body.contains("SSH host example-host, in /home/u/proj") && body.contains("Use make.") && body.contains("mcp__acpira__Bash"));
  }
}
