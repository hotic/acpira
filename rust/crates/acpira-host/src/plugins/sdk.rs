//! The plugin SDKs (`sdk.mjs` for Node, `sdk.py` for Python, one file each without dependencies) and `acpira plugin
//! new`, which writes a plugin folder of the kinds asked for that passes the static check as written. The engine keeps
//! copies at `<plugins>/.sdk/` for plugins written by hand; a new folder gets its own copy, frozen with it, so an
//! engine upgrade never changes a plugin's code under it

use std::path::{Path, PathBuf};

use anyhow::{Context as _, Result, bail};
use serde_json::{Value, json};

use crate::plugins::manifest;

pub const SDK: &str = include_str!("sdk.mjs");
pub const SDK_FILE: &str = "acpira-plugin.mjs";
pub const SDK_PY: &str = include_str!("sdk.py");
pub const SDK_PY_FILE: &str = "acpira_plugin.py";
/// Beside the plugin folders; a name no plugin id can take
pub const SDK_DIR: &str = ".sdk";

/// What `acpira plugin new --kind` takes, the default first
pub const KINDS: &[&str] = &["runtime", "tool", "command", "skill", "provider", "theme"];

/// What `acpira plugin new --lang` takes for the runtime and the tool, the default first
pub const LANGS: &[&str] = &["node", "python"];

/// Keep `<plugins>/.sdk/` holding the SDKs this build ships; each written only when it differs. The folder
pub fn install(root: &Path) -> Result<PathBuf> {
  let dir = root.join(SDK_DIR);
  for (name, text) in [(SDK_FILE, SDK), (SDK_PY_FILE, SDK_PY)] {
    let path = dir.join(name);
    if std::fs::read_to_string(&path).is_ok_and(|t| t == text) {
      continue;
    }
    std::fs::create_dir_all(&dir).with_context(|| format!("create {}", dir.display()))?;
    let tmp = dir.join(format!(".{name}.{}.tmp", std::process::id()));
    std::fs::write(&tmp, text).and_then(|()| std::fs::rename(&tmp, &path)).with_context(|| format!("write {}", path.display()))?;
  }
  Ok(dir)
}

const RUNTIME: &str = r#"// The resident process: hooks, events and methods through the SDK (docs/dev/plugins.md, Runtime). Every name used
// here is declared in plugin.json; the engine calls only those.
import { plugin } from './acpira-plugin.mjs';

const p = plugin();

// Before every prompt of every agent: add a line of context, or return null to leave the prompt alone
p.hook('prompt', ({ text }) => (text.includes('TODO') ? { context: 'List the TODOs you leave behind.' } : null));

// After every turn: count them in the plugin's own store
p.on('turn.ended', async () => {
  const turns = ((await p.storage.get('turns')) ?? 0) + 1;
  await p.storage.set('turns', turns);
});

// Called by pages and other plugins
p.method('ping', async () => ({ pong: true, turns: (await p.storage.get('turns')) ?? 0 }));
"#;

const TOOL: &str = r#"// A script tool: the arguments arrive on stdin, stdout is the result, a non-zero exit is an error
import { readFileSync } from 'node:fs';

const { arguments: args } = JSON.parse(readFileSync(0, 'utf8'));
console.log(`Hello, ${args.name}!`);
"#;

const RUNTIME_PY: &str = r#"# The resident process: hooks, events and methods through the SDK (docs/dev/plugins.md, Runtime). Every name used
# here is declared in plugin.json; the engine calls only those.
from acpira_plugin import Plugin

p = Plugin()


# Before every prompt of every agent: add a line of context, or return None to leave the prompt alone
@p.hook("prompt")
def prompt(payload, ctx):
    return {"context": "List the TODOs you leave behind."} if "TODO" in payload.get("text", "") else None


# After every turn: count them in the plugin's own store
@p.on("turn.ended")
def ended(event):
    p.storage.set("turns", (p.storage.get("turns") or 0) + 1)


# Called by pages and other plugins
@p.method("ping")
def ping(params, ctx):
    return {"pong": True, "turns": p.storage.get("turns") or 0}


p.run()
"#;

const TOOL_PY: &str = r#"# A script tool: the arguments arrive on stdin, stdout is the result, a non-zero exit is an error
import json
import sys

args = json.load(sys.stdin)["arguments"]
print(f"Hello, {args['name']}!")
"#;

const SKILL: &str = "---\nname: hello\ndescription: Say what the plugin is for. Replace this with when an agent should use the skill.\n---\n\nThe steps an agent follows.\n";

/// The manifest and files of a new plugin `id` contributing `kinds`, its runtime and tool in `lang`
pub fn files(id: &str, kinds: &[&str], lang: &str) -> Result<(Value, Vec<(&'static str, String)>)> {
  let py = match lang {
    "node" => false,
    "python" => true,
    other => bail!("`{other}` is not a language: {}", LANGS.join(", ")),
  };
  if !manifest::valid_id(id) {
    bail!("`{id}` is not a plugin id: lowercase letters, digits and `-`, at most {} characters", manifest::NAME_MAX);
  }
  let mut contributes = serde_json::Map::new();
  let mut files: Vec<(&'static str, String)> = vec![];
  let mut requires = vec![];
  for kind in kinds {
    match *kind {
      "runtime" => {
        let (program, main) = if py { ("python3", "main.py") } else { ("node", "main.mjs") };
        contributes.insert("runtime".into(), json!({ "command": [program, main], "hooks": ["prompt"], "events": ["turn.ended"], "methods": ["ping"] }));
        if py {
          files.push(("main.py", RUNTIME_PY.into()));
          files.push((SDK_PY_FILE, SDK_PY.into()));
        } else {
          files.push(("main.mjs", RUNTIME.into()));
          files.push((SDK_FILE, SDK.into()));
        }
        requires.push(program);
      }
      "tool" => {
        let (program, script) = if py { ("python3", "hello.py") } else { ("node", "hello.mjs") };
        contributes.insert(
          "tools".into(),
          json!([{ "name": "hello", "description": "Greet someone by name", "readOnly": true, "command": [program, script],
            "input": { "type": "object", "properties": { "name": { "type": "string", "description": "Who to greet" } }, "required": ["name"] } }]),
        );
        files.push((script, if py { TOOL_PY.into() } else { TOOL.into() }));
        requires.push(program);
      }
      "command" => {
        contributes.insert("commands".into(), json!([{ "name": "hello", "description": "Ask for a short greeting", "hint": "name", "prompt": "Write a one-line greeting for $ARGUMENTS." }]));
      }
      "skill" => {
        contributes.insert("skills".into(), json!("skills"));
        files.push(("skills/hello/SKILL.md", SKILL.into()));
      }
      "provider" => {
        let env = format!("{}_API_KEY", id.to_ascii_uppercase().replace('-', "_"));
        contributes.insert(
          "providers".into(),
          json!([{ "id": id, "name": id, "format": "openai-chat", "baseUrl": "http://127.0.0.1:8080/v1", "keyEnv": env, "models": [{ "id": "model-id", "context": 128000 }] }]),
        );
      }
      "theme" => {
        contributes.insert("theme".into(), json!({ "tokens": { "--bg-0": "#1b1a18" } }));
      }
      other => bail!("`{other}` is not a kind: {}", KINDS.join(", ")),
    }
  }
  requires.dedup();
  let mut m = json!({ "schema": manifest::SCHEMA, "id": id, "version": "0.1.0", "name": id, "description": "Say in one line what this plugin does", "contributes": contributes });
  if !requires.is_empty() {
    m["requires"] = json!(requires);
  }
  Ok((m, files))
}

/// Write the folder `<root>/<id>/`, which must not exist yet; the path of the folder
pub fn scaffold(root: &Path, id: &str, kinds: &[&str], lang: &str) -> Result<PathBuf> {
  let (m, files) = files(id, kinds, lang)?;
  let dir = root.join(id);
  if dir.exists() {
    bail!("{} exists already", dir.display());
  }
  for (name, text) in std::iter::once((manifest::FILE, format!("{}\n", serde_json::to_string_pretty(&m)?))).chain(files) {
    let path = dir.join(name);
    if let Some(parent) = path.parent() {
      std::fs::create_dir_all(parent).with_context(|| format!("create {}", parent.display()))?;
    }
    std::fs::write(&path, text).with_context(|| format!("write {}", path.display()))?;
  }
  Ok(dir)
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn every_kind_scaffolds_a_folder_that_passes_the_check() {
    let tmp = tempfile::tempdir().unwrap();
    let dir = scaffold(tmp.path(), "my-kit", KINDS, "node").unwrap();
    let (m, mut f) = manifest::parse(&std::fs::read(dir.join(manifest::FILE)).unwrap()).unwrap();
    manifest::check(&m, "my-kit", &dir, &|p| p == "node", &mut f);
    assert!(f.ok() && f.notes.is_empty(), "{f:?}");
    let py = scaffold(tmp.path(), "py-kit", &["runtime", "tool"], "python").unwrap();
    let (pm, mut pf) = manifest::parse(&std::fs::read(py.join(manifest::FILE)).unwrap()).unwrap();
    manifest::check(&pm, "py-kit", &py, &|p| p == "python3", &mut pf);
    assert!(pf.ok() && pf.notes.is_empty(), "{pf:?}");
    assert_eq!(std::fs::read_to_string(py.join(SDK_PY_FILE)).unwrap(), SDK_PY);
    assert!(scaffold(tmp.path(), "rb", &["tool"], "ruby").unwrap_err().to_string().contains("not a language"));
    assert_eq!(m.contributes.kinds(), ["tools", "theme", "runtime", "skills", "commands", "providers"]);
    assert_eq!(std::fs::read_to_string(dir.join(SDK_FILE)).unwrap(), SDK);
    assert_eq!(m.contributes.providers[0]["keyEnv"], "MY_KIT_API_KEY");
    assert!(scaffold(tmp.path(), "my-kit", &["tool"], "node").unwrap_err().to_string().contains("exists already"));
    assert!(scaffold(tmp.path(), "Bad", &["tool"], "node").is_err() && scaffold(tmp.path(), "ok", &["widget"], "node").is_err());
  }

  #[test]
  fn the_shared_copy_is_written_once_and_kept_current() {
    let tmp = tempfile::tempdir().unwrap();
    let dir = install(tmp.path()).unwrap();
    assert_eq!(std::fs::read_to_string(dir.join(SDK_PY_FILE)).unwrap(), SDK_PY);
    std::fs::write(dir.join(SDK_FILE), "old").unwrap();
    install(tmp.path()).unwrap();
    assert_eq!(std::fs::read_to_string(dir.join(SDK_FILE)).unwrap(), SDK);
    // Not a plugin folder: ids never start with `.`
    assert!(!manifest::valid_id(SDK_DIR));
  }
}
