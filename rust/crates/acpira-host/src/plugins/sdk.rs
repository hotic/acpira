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
pub const KINDS: &[&str] = &["runtime", "tool", "command", "skill", "provider", "theme", "ui", "agent"];

/// The backdrop runtime and the scenes `plugin new --kind theme --scene` starts from (`backdrop/`)
const BACKDROP_RUNTIME: &str = include_str!("backdrop/runtime.js");
const BACKDROP_HELPERS: &str = include_str!("backdrop/helpers.js");
const CAT: &str = include_str!("backdrop/cat.html");

/// A built-in scene: its script and the skin it comes with (name, base, seed tokens, one line)
pub struct Scene {
  pub id: &'static str,
  pub name: &'static str,
  pub description: &'static str,
  pub base: &'static str,
  /// `--bg-0`, `--fg-1`, `--brand`: the page derives the other colours from these
  pub seeds: [&'static str; 3],
  script: &'static str,
}

/// What `--scene` takes, the default first
pub const SCENES: &[Scene] = &[
  Scene {
    id: "deepsea",
    name: "Deep sea",
    description: "Jellyfish pulsing upward through the dark, kelp swaying below",
    base: "dark",
    seeds: ["#031420", "#e2eef4", "#6fd8f2"],
    script: include_str!("backdrop/deepsea.js"),
  },
  Scene {
    id: "inkhills",
    name: "Ink hills",
    description: "Ink-wash ranges fading into mist, a boat on the river",
    base: "light",
    seeds: ["#f3efe6", "#24221e", "#b4442f"],
    script: include_str!("backdrop/inkhills.js"),
  },
  Scene {
    id: "fireflies",
    name: "Fireflies",
    description: "A summer night meadow, fireflies over the grass",
    base: "dark",
    seeds: ["#0c1511", "#e6ede2", "#c6e86a"],
    script: include_str!("backdrop/fireflies.js"),
  },
  Scene {
    id: "aurora",
    name: "Aurora",
    description: "Northern lights over a snowy range, mirrored in a lake",
    base: "dark",
    seeds: ["#0a0f1b", "#e4ebf6", "#53dcb4"],
    script: include_str!("backdrop/aurora.js"),
  },
];

/// What a scaffolded scene file starts with: the contract it is drawn under
const SCENE_HEAD: &str = "// A scene for Acpira's glyph renderer (plugin.json `theme.scene`). Define `const SCENE` with:\n// paint(c, w, h, cols, rows) draws the still background once per size on a 2D canvas; setup(cols, rows) and\n// frame(t, cols, rows) prepare what moves; cell(x, y, t, o) sets one cell with set(o, glyph, r, g, b) (colours 0 to 1)\n// and adds soft light with light(o, r, g, b). Helpers: noise1, noise2, hash, fract, smooth, clamp, mix, ramp. CW and CH\n// are a cell's size in CSS pixels. The renderer (frame cap, pausing, WebGL or 2D) is Acpira's and is not in this file.\n";

/// A scene's backdrop document: the scene and the renderer that draws it, the running Acpira's (`theme.scene`)
pub fn backdrop_html(scene: &str) -> String {
  format!(
    "<!doctype html>\n<html>\n<head>\n<meta charset=\"utf-8\">\n<style>html,body{{margin:0;height:100%;overflow:hidden;background:#000}}canvas{{display:block;width:100vw;height:100vh}}</style>\n</head>\n<body>\n<canvas id=\"c\"></canvas>\n<script>\n// The scene: paint(c, w, h, cols, rows) draws the still background once per size on a 2D canvas; setup(cols, rows)\n// and frame(t, cols, rows) prepare what moves; cell(x, y, t, o) sets one cell with set(o, glyph, r, g, b) (colours\n// 0 to 1) and adds soft light with light(o, r, g, b). CW and CH are a cell's size in CSS pixels\n{BACKDROP_HELPERS}\n{scene}</script>\n<script>\n{BACKDROP_RUNTIME}</script>\n</body>\n</html>\n"
  )
}

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

/// The starting prompt of `--kind agent`: it replaces Acpira's own, so it says everything the agent is
const AGENT_PROMPT: &str = "# {id}

You are {id}, an assistant for one job: say here what it is, in one or two sentences.

## What you do
- The tasks you take, and the result each one ends with.

## What you never do
- Anything outside the job: say so in one line and stop.

## How you work
- Use only the tools you are given. When something needed is missing, ask with `question` instead of guessing.
- Keep answers short and concrete.
";

/// The manifest and files of a new plugin `id` contributing `kinds`, its runtime and tool in `lang`
pub fn files(id: &str, kinds: &[&str], lang: &str) -> Result<(Value, Vec<(&'static str, String)>)> {
  files_with(id, kinds, lang, SCENES[0].id)
}

/// `files`, the theme's backdrop drawn from the built-in scene `scene`
pub fn files_with(id: &str, kinds: &[&str], lang: &str, scene: &str) -> Result<(Value, Vec<(&'static str, String)>)> {
  let Some(scene) = SCENES.iter().find(|s| s.id == scene) else {
    bail!("`{scene}` is not a scene: {}", SCENES.iter().map(|s| s.id).collect::<Vec<_>>().join(", "));
  };
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
      // A skin: three seed colours the page derives the rest from, a backdrop behind frosted surfaces
      "theme" => {
        let [bg, fg, brand] = scene.seeds;
        contributes.insert(
          "theme".into(),
          json!({ "base": scene.base, "glass": "veil", "scene": "scene.js", "tokens": { "--bg-0": bg, "--fg-1": fg, "--brand": brand } }),
        );
        files.push(("scene.js", format!("{SCENE_HEAD}{}", scene.script)));
      }
      // An interface piece sized for the composer's top edge: a glyph cat that follows the session's state
      "ui" => {
        contributes.insert("ui".into(), json!([{ "id": "cat", "slot": "composer.above", "title": "Cat", "html": "cat.html", "width": 180, "height": 60 }]));
        files.push(("cat.html", CAT.into()));
      }
      // A specialised agent: Acpira's loop with this prompt and only these tools; the plugin's own tool when there is one
      "agent" => {
        let mut tools = vec![json!("read"), json!("grep"), json!("todo"), json!("question")];
        if kinds.contains(&"tool") {
          tools.push(json!("plugin:hello"));
        }
        if kinds.contains(&"skill") {
          tools.push(json!("skill"));
        }
        contributes.insert("agent".into(), json!({ "name": id, "builtin": { "prompt": "prompt.md", "tools": tools } }));
        files.push(("prompt.md", AGENT_PROMPT.replace("{id}", id)));
      }
      other => bail!("`{other}` is not a kind: {}", KINDS.join(", ")),
    }
  }
  requires.dedup();
  // With its own agent, the plugin's hooks guard that agent's sessions only
  if kinds.contains(&"agent")
    && let Some(rt) = contributes.get_mut("runtime")
  {
    rt["scope"] = json!(manifest::SCOPE_AGENT);
  }
  let mut m = json!({ "schema": manifest::SCHEMA, "id": id, "version": "0.1.0", "name": id, "description": "Say in one line what this plugin does", "contributes": contributes });
  if !requires.is_empty() {
    m["requires"] = json!(requires);
  }
  Ok((m, files))
}

/// Write the folder `<root>/<id>/`, which must not exist yet; the path of the folder
pub fn scaffold(root: &Path, id: &str, kinds: &[&str], lang: &str) -> Result<PathBuf> {
  scaffold_with(root, id, kinds, lang, SCENES[0].id)
}

/// `scaffold` with the theme's scene picked
pub fn scaffold_with(root: &Path, id: &str, kinds: &[&str], lang: &str, scene: &str) -> Result<PathBuf> {
  let (mut m, files) = files_with(id, kinds, lang, scene)?;
  // A plugin that is only a skin is named after its scene
  if kinds == ["theme"]
    && let Some(s) = SCENES.iter().find(|s| s.id == scene)
  {
    m["name"] = json!(s.name);
    m["description"] = json!(s.description);
  }
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
    assert_eq!(m.contributes.kinds(), ["tools", "theme", "agent", "runtime", "skills", "commands", "providers", "ui"]);
    // The agent lists the plugin's own tool and skill, and the runtime guards that agent alone
    let agent = m.contributes.agent.as_ref().unwrap();
    assert_eq!(agent["builtin"]["tools"], json!(["read", "grep", "todo", "question", "plugin:hello", "skill"]));
    assert_eq!(m.contributes.runtime.as_ref().unwrap().scope.as_deref(), Some(manifest::SCOPE_AGENT));
    assert!(acpira_agent::profile::Profile::load(&dir).unwrap().prompt.starts_with("# my-kit"));
    assert_eq!(std::fs::read_to_string(dir.join(SDK_FILE)).unwrap(), SDK);
    assert_eq!(m.contributes.providers[0]["keyEnv"], "MY_KIT_API_KEY");
    assert!(scaffold(tmp.path(), "my-kit", &["tool"], "node").unwrap_err().to_string().contains("exists already"));
    assert!(scaffold(tmp.path(), "Bad", &["tool"], "node").is_err() && scaffold(tmp.path(), "ok", &["widget"], "node").is_err());
  }

  #[test]
  fn every_scene_scaffolds_a_skin_that_passes_the_check() {
    let tmp = tempfile::tempdir().unwrap();
    for scene in SCENES {
      let id = format!("skin-{}", scene.id);
      let dir = scaffold_with(tmp.path(), &id, &["theme"], "node", scene.id).unwrap();
      let (m, mut f) = manifest::parse(&std::fs::read(dir.join(manifest::FILE)).unwrap()).unwrap();
      manifest::check(&m, &id, &dir, &|_| false, &mut f);
      assert!(f.ok() && f.notes.is_empty(), "{}: {f:?}", scene.id);
      let theme = m.contributes.theme.as_ref().unwrap();
      assert_eq!(theme.base.as_deref(), Some(scene.base));
      // Only the scene is in the plugin; the document around it is the renderer's
      let script = std::fs::read_to_string(dir.join(theme.scene.as_ref().unwrap())).unwrap();
      assert!(theme.backdrop.is_none() && script.contains("const SCENE") && !script.contains("acpira: 'drawn'"), "{}", scene.id);
      assert!(backdrop_html(&script).contains("acpira: 'drawn'") && (script.len() as u64) < manifest::SCENE_MAX_BYTES, "{}", scene.id);
    }
    assert!(scaffold_with(tmp.path(), "x", &["theme"], "node", "rain").unwrap_err().to_string().contains("not a scene"));
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
