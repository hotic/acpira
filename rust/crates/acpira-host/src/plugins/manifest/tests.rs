use std::path::Path;

use super::*;
use serde_json::json;

fn manifest(v: Value) -> (Manifest, Findings) {
  parse(v.to_string().as_bytes()).unwrap()
}

#[test]
fn a_script_tool_manifest_passes_and_names_its_tool() {
  let dir = tempfile::tempdir().unwrap();
  std::fs::write(dir.path().join("convert.py"), "print(1)").unwrap();
  let (m, mut f) = manifest(json!({
    "schema": 1, "id": "currency", "version": "0.1.0", "description": "x",
    "contributes": { "tools": [{ "name": "convert", "description": "Convert", "input": { "type": "object" }, "command": ["python3", "convert.py"], "readOnly": true }] }
  }));
  check(&m, "currency", dir.path(), &|p| p == "python3", &mut f);
  assert!(f.ok(), "{f:?}");
  assert_eq!(m.layer(), "tools");
  assert_eq!(served_name(&m.id, &m.contributes.tools[0].name), "currency_convert");
  assert_eq!(m.contributes.tools[0].timeout(), std::time::Duration::from_secs(45));
}

#[test]
fn an_agent_cli_not_installed_yet_passes_with_a_note_and_a_bad_mark_fails() {
  let dir = tempfile::tempdir().unwrap();
  let agent = |mark: Value| {
    json!({ "schema": 1, "id": "qoder", "version": "0.1.0", "description": "x",
      "contributes": { "agent": { "name": "Qoder", "command": "qoder", "args": ["--acp"], "vendor": "Alibaba", "mark": mark } } })
  };
  let (m, mut f) = manifest(agent(json!(["M3 3h18v18H3z"])));
  check(&m, "qoder", dir.path(), &|_| false, &mut f);
  assert!(f.ok(), "{f:?}");
  assert!(f.text().contains("not on PATH yet"), "{}", f.text());
  let (m, mut f) = manifest(agent(json!(["<script>"])));
  check(&m, "qoder", dir.path(), &|_| false, &mut f);
  assert!(f.text().contains("agent: a `mark` path"), "{}", f.text());
  // A program meant to ship inside the plugin still has to be there
  let (m, mut f) = manifest(json!({ "schema": 1, "id": "own", "version": "0.1.0", "description": "x",
    "contributes": { "agent": { "command": "${pluginDir}/bin/own" } } }));
  check(&m, "own", dir.path(), &|_| false, &mut f);
  assert!(!f.ok());
}

#[test]
fn the_static_check_names_every_problem_without_running_anything() {
  let dir = tempfile::tempdir().unwrap();
  let (m, mut f) = manifest(json!({
    "schema": 1, "id": "Bad_Id", "extra": 1, "requires": ["nope"],
    "contributes": {
      "tools": [{ "name": "Go", "command": ["../escape.sh"], "input": { "type": "string" } }, { "name": "Go", "command": [] }],
      "theme": { "tokens": { "--ctl-w": "1px", "--bg-0": "red;}" }, "css": "missing.css" },
      "mcp": { "acpira": {} },
      "widgets": []
    }
  }));
  check(&m, "other", dir.path(), &|_| false, &mut f);
  let text = f.text();
  for needle in ["id `Bad_Id`", "folder's name", "requires `nope`", "tool `Go`: the name", "declared twice", "a description is needed", "type object", "`../escape.sh`", "needs a `command`", "`--ctl-w`", "`--bg-0`", "missing.css", "Acpira's own", "unknown key `extra`", "unknown contribution `widgets`"] {
    assert!(text.contains(needle), "{needle} missing in\n{text}");
  }
}

#[test]
fn a_skin_names_its_look_and_a_piece_on_the_composer_its_box() {
  let dir = tempfile::tempdir().unwrap();
  std::fs::write(dir.path().join("cat.html"), "<p>cat</p>").unwrap();
  let skin = |theme: Value, ui: Value| {
    json!({ "schema": 1, "id": "sea", "version": "1", "description": "x", "contributes": { "theme": theme, "ui": ui } })
  };
  let (m, mut f) = manifest(skin(json!({ "base": "dim", "glass": "glass", "backdrop": "gone.html", "tokens": { "--bg-0": "#000" } }),
    json!([{ "id": "cat", "slot": "composer.above", "title": "Cat", "html": "cat.html" }, { "id": "big", "slot": "page", "title": "Big", "html": "cat.html", "width": 900 }])));
  check(&m, "sea", dir.path(), &|_| false, &mut f);
  let text = f.text();
  for needle in ["base `dim`", "glass `glass`", "`gone.html` not found", "`composer.above` needs `width` and `height`", "width must be 1 to 480"] {
    assert!(text.contains(needle), "{needle} missing in\n{text}");
  }
  // A backdrop without a base, and glass without a backdrop, pass with a note
  std::fs::write(dir.path().join("sea.html"), "<p>sea</p>").unwrap();
  let (m, mut f) = manifest(skin(json!({ "backdrop": "sea.html", "tokens": { "--bg-0": "#000" } }), json!([])));
  check(&m, "sea", dir.path(), &|_| false, &mut f);
  assert!(f.ok() && f.notes.iter().any(|n| n.contains("without `base`")), "{f:?}");
  let (m, mut f) = manifest(skin(json!({ "glass": "frost", "tokens": { "--bg-0": "#000" } }), json!([])));
  check(&m, "sea", dir.path(), &|_| false, &mut f);
  assert!(f.ok() && f.notes.iter().any(|n| n.contains("only applies with a `backdrop`")), "{f:?}");
  // A scene is drawn by Acpira's renderer; a plugin gives one or its own document, not both
  std::fs::write(dir.path().join("sea.js"), "const SCENE = {};").unwrap();
  let (m, mut f) = manifest(skin(json!({ "base": "dark", "scene": "sea.js", "tokens": { "--bg-0": "#000" } }), json!([])));
  check(&m, "sea", dir.path(), &|_| false, &mut f);
  assert!(f.ok() && f.notes.is_empty(), "{f:?}");
  let (m, mut f) = manifest(skin(json!({ "base": "dark", "scene": "sea.js", "backdrop": "sea.html" }), json!([])));
  check(&m, "sea", dir.path(), &|_| false, &mut f);
  assert!(f.text().contains("exclude each other"), "{f:?}");
}

#[test]
fn each_slot_asks_for_the_sides_it_does_not_size_and_a_message_action_is_a_button() {
  let dir = tempfile::tempdir().unwrap();
  std::fs::write(dir.path().join("p.html"), "<p>p</p>").unwrap();
  std::fs::write(dir.path().join("run.mjs"), "").unwrap();
  let ui = |ui: Value| {
    json!({ "schema": 1, "id": "kit", "version": "1", "description": "x",
      "contributes": { "ui": ui, "runtime": { "command": ["node", "run.mjs"], "methods": ["summarize"] } } })
  };
  let piece = |slot: &str, extra: Value| {
    let mut v = json!({ "id": slot.replace('.', "-"), "slot": slot, "title": "T", "html": "p.html" });
    v.as_object_mut().unwrap().extend(extra.as_object().unwrap().clone());
    v
  };
  let sized = json!([
    piece("composer.toolbar", json!({ "width": 80, "height": 24 })), piece("statusbar", json!({ "width": 60, "height": 20 })),
    piece("session.panel", json!({ "width": 40 })), piece("sidebar", json!({ "height": 120 })), piece("start", json!({ "height": 90 })),
    piece("panel.tab", json!({})), piece("page", json!({})),
    { "id": "sum", "slot": "message.actions", "title": "Summarize", "method": "summarize" },
    { "id": "explain", "slot": "selection.actions", "title": "Explain", "method": "summarize" },
    { "id": "stamp", "slot": "prompt.actions", "title": "Stamp", "method": "summarize" },
  ]);
  let (m, mut f) = manifest(ui(sized));
  check(&m, "kit", dir.path(), &|p| p == "node", &mut f);
  assert!(f.ok(), "{f:?}");
  let bare = json!([
    piece("statusbar", json!({ "width": 60 })), piece("session.panel", json!({})), piece("sidebar", json!({})),
    { "id": "a", "slot": "message.actions", "title": "A", "method": "nope" },
    { "id": "b", "slot": "message.actions", "title": "", "html": "p.html" },
    { "id": "c", "slot": "selection.actions", "title": "C", "entry": "p.tsx" },
    piece("start", json!({ "height": 90, "method": "summarize" })),
  ]);
  let (m, mut f) = manifest(ui(bare));
  check(&m, "kit", dir.path(), &|p| p == "node", &mut f);
  let text = f.text();
  for needle in [
    "`statusbar` needs `height`", "`session.panel` needs `width`", "`sidebar` needs `height`", "method `nope` is not one of",
    "names the runtime `method`", "not a document", "needs a `title`", "only a `message.actions`, `prompt.actions` or `selection.actions` piece calls",
    "a `selection.actions` piece names the runtime `method`", "ui `c`: a `selection.actions` piece is a button",
  ] {
    assert!(text.contains(needle), "{needle} missing in\n{text}");
  }
}

#[test]
fn commands_need_a_prompt_or_the_command_hook_and_skills_a_folder() {
  let dir = tempfile::tempdir().unwrap();
  std::fs::create_dir_all(dir.path().join("skills")).unwrap();
  std::fs::write(dir.path().join("run.mjs"), "").unwrap();
  let base = |commands: Value, runtime: Value, skills: &str| {
    json!({ "schema": 1, "id": "cmds", "version": "0.1.0", "description": "x", "contributes": { "commands": commands, "runtime": runtime, "skills": skills } })
  };
  let ok = base(json!([{ "name": "review", "prompt": "Review $ARGUMENTS", "button": "Review" }, { "name": "ping-me", "button": "Polish", "draft": true, "icon": "wand", "look": "icon" }]), json!({ "command": ["node", "run.mjs"], "hooks": ["command"] }), "skills");
  let (m, mut f) = manifest(ok);
  check(&m, "cmds", dir.path(), &|p| p == "node", &mut f);
  assert!(f.ok(), "{f:?}");
  assert_eq!(m.contributes.kinds(), ["runtime", "skills", "commands"]);
  let bad = base(json!([{ "name": "Review" }, { "name": "x", "prompt": "a" }, { "name": "x", "prompt": "b" }, { "name": "y", "prompt": "a", "button": " " }, { "name": "z", "prompt": "a", "draft": true }, { "name": "w", "prompt": "a", "button": "W", "icon": "unicorn", "look": "neon" }]), json!({ "command": ["node", "run.mjs"], "events": ["turn.*"] }), "nope");
  let (m, mut f) = manifest(bad);
  check(&m, "cmds", dir.path(), &|p| p == "node", &mut f);
  let text = f.text();
  for needle in ["command `/Review`: the name", "needs a `prompt`", "`/x` is declared twice", "skills folder `nope`", "`/y`: `button` is the label", "`/z`: `draft`, `icon`, `look` and `input` need a `button`", "icon `unicorn`", "look `neon`"] {
    assert!(text.contains(needle), "{needle} missing in\n{text}");
  }
}

#[test]
fn a_command_menu_is_checked_and_resolves_picks_with_defaults() {
  let dir = tempfile::tempdir().unwrap();
  let menu = json!([
    { "id": "on", "kind": "toggle", "label": "Dispatch", "default": true },
    { "kind": "separator" },
    { "id": "normal", "kind": "radio", "group": "level", "label": "Normal" },
    { "id": "eager", "kind": "radio", "group": "level", "label": "Eager", "default": true },
    { "id": "reset", "kind": "action", "label": "Reset" }
  ]);
  let base = |menu: Value, runtime: Value| {
    json!({ "schema": 1, "id": "subs", "version": "0.1.0", "description": "x", "contributes": { "commands": [{ "name": "subagent", "button": "Sub", "menu": menu }], "runtime": runtime } })
  };
  // A menu command needs no prompt; an action needs the `menu` hook
  let (m, mut f) = manifest(base(menu.clone(), json!({ "command": ["node", "run.mjs"], "hooks": ["menu"] })));
  check(&m, "subs", dir.path(), &|p| p == "node", &mut f);
  assert!(f.ok(), "{f:?}");
  let def = &m.contributes.commands[0];
  let picks = def.menu_picks(None);
  assert_eq!(Value::Object(picks.clone()), json!({ "on": true, "level": "eager" }));
  let stored = def.menu_pick(&picks, "normal").unwrap();
  assert_eq!(Value::Object(def.menu_pick(&stored, "on").unwrap()), json!({ "on": false, "level": "normal" }));
  assert!(def.menu_pick(&picks, "reset").is_none());
  // A pick the manifest no longer has falls back to the default
  let gone = json!({ "level": "max", "on": false });
  assert_eq!(Value::Object(def.menu_picks(gone.as_object())), json!({ "on": false, "level": "eager" }));

  // A split chip (`draft` with a menu) runs the command from its body, so it needs the `command` hook
  let split = |runtime: Value| {
    json!({ "schema": 1, "id": "subs", "version": "0.1.0", "description": "x", "contributes": { "commands": [{ "name": "polish", "button": "Polish", "draft": true, "menu": menu.clone() }], "runtime": runtime } })
  };
  let (m, mut f) = manifest(split(json!({ "command": ["node", "run.mjs"], "hooks": ["command", "menu"] })));
  check(&m, "subs", dir.path(), &|p| p == "node", &mut f);
  assert!(f.ok(), "{f:?}");
  let (m, mut f) = manifest(split(json!({ "command": ["node", "run.mjs"], "hooks": ["menu"] })));
  check(&m, "subs", dir.path(), &|p| p == "node", &mut f);
  assert!(f.text().contains("needs a `prompt`"), "{}", f.text());

  let bad = json!([
    { "id": "a", "kind": "radio", "label": "A" },
    { "id": "b", "kind": "radio", "group": "g", "label": "B", "default": true },
    { "id": "c", "kind": "radio", "group": "g", "label": "C", "default": true },
    { "id": "g", "kind": "toggle", "label": "G" },
    { "id": "x", "kind": "action", "label": "" },
    { "id": "y", "kind": "slider", "label": "Y" }
  ]);
  let (m, mut f) = manifest(base(bad, json!({ "command": ["node", "run.mjs"], "hooks": ["command"] })));
  check(&m, "subs", dir.path(), &|p| p == "node", &mut f);
  let text = f.text();
  for needle in ["row `a`: a radio row names its `group`", "more than one `default`", "row `g`: the id is also a group's name", "row `x`: `label` is 1 to", "an action needs a runtime that answers the `menu` hook", "kind `slider`"] {
    assert!(text.contains(needle), "{needle} missing in\n{text}");
  }
}

#[test]
fn providers_need_a_usable_source_and_never_a_key() {
  let dir = tempfile::tempdir().unwrap();
  let base = |providers: Value| json!({ "schema": 1, "id": "gw", "version": "0.1.0", "description": "x", "contributes": { "providers": providers } });
  let good = json!({ "id": "gw-claude", "format": "anthropic", "baseUrl": "http://gw:8080", "keyEnv": "GW_KEY", "models": [{ "id": "claude-x" }] });
  let (m, mut f) = manifest(base(json!([good.clone()])));
  check(&m, "gw", dir.path(), &|_| false, &mut f);
  assert!(f.ok(), "{f:?}");
  assert_eq!((m.contributes.kinds(), m.layer()), (vec!["providers"], "agents"));
  let p = provider_of(&good, "gw").unwrap();
  assert_eq!((p.name.as_str(), p.extra["plugin"].as_str(), p.extra["keyEnv"].as_str()), ("gw-claude", Some("gw"), Some("GW_KEY")));
  assert!(p.models[0].input_guessed(), "omitted `input` falls back to the catalogue");
  let explicit = provider_of(&json!({ "id": "gw-text", "baseUrl": "http://gw:8080", "models": [{ "id": "m", "input": ["text"] }] }), "gw").unwrap();
  assert!(!explicit.models[0].input_guessed(), "explicit `input` stays confirmed");
  let bad = base(json!([
    good,
    { "id": "gw-claude", "format": "anthropic", "baseUrl": "http://x", "models": [{ "id": "m" }] },
    { "id": "a/b", "baseUrl": "http://x", "models": [{ "id": "m" }] },
    { "id": "k", "baseUrl": "http://x", "apiKey": "sk-1", "models": [{ "id": "m" }] },
    { "id": "f", "format": "gemini", "baseUrl": "http://x", "models": [{ "id": "m" }] },
    { "id": "u", "models": [{ "id": "m" }] },
    { "id": "n", "baseUrl": "http://x", "models": [] },
    { "id": "e", "baseUrl": "http://x", "keyEnv": "lower", "models": [{ "id": "m" }] },
    { "id": "h", "baseUrl": "http://x", "headers": { "Authorization": "Bearer sk" }, "models": [{ "id": "m" }] },
    { "id": "logo", "baseUrl": "http://x", "mark": ["<path/>"], "models": [{ "id": "m" }] },
  ]));
  let (m, mut f) = manifest(bad);
  check(&m, "gw", dir.path(), &|_| false, &mut f);
  let text = f.text();
  for needle in ["`gw-claude` is declared twice", "`a/b`: the id", "`apiKey` is not allowed", "format `gemini`", "`baseUrl` is needed", "at least one model", "environment variable name", "header `Authorization` is a credential", "provider `logo`: a `mark` path holds"] {
    assert!(text.contains(needle), "{needle} missing in\n{text}");
  }
}

#[test]
fn interface_pieces_need_a_known_slot_and_a_small_html_file() {
  let dir = tempfile::tempdir().unwrap();
  std::fs::write(dir.path().join("panel.html"), "<p>hi</p>").unwrap();
  std::fs::write(dir.path().join("big.html"), "x".repeat(UI_HTML_MAX_BYTES as usize + 1)).unwrap();
  let base = |ui: Value| json!({ "schema": 1, "id": "look", "version": "0.1.0", "description": "x", "contributes": { "ui": ui } });
  let (m, mut f) = manifest(base(json!([{ "id": "rates", "slot": "panel.tab", "title": "Rates", "html": "panel.html" }])));
  check(&m, "look", dir.path(), &|_| false, &mut f);
  assert!(f.ok(), "{f:?}");
  assert_eq!((m.contributes.kinds(), m.layer()), (vec!["ui"], "interface"));
  let (m, mut f) = manifest(base(json!([
    { "id": "a", "slot": "toolbar", "html": "panel.html" },
    { "id": "a", "slot": "page", "html": "panel.html" },
    { "id": "b", "slot": "page", "html": "../escape.html" },
    { "id": "c", "slot": "page", "html": "big.html" },
  ])));
  check(&m, "look", dir.path(), &|_| false, &mut f);
  let text = f.text();
  for needle in ["slot `toolbar` is not one of", "ui `a` is declared twice", "`../escape.html` not found", "`big.html` is over 256 KB"] {
    assert!(text.contains(needle), "{needle} missing in\n{text}");
  }
}

#[test]
fn an_unknown_schema_is_refused_whole() {
  assert!(parse(br#"{"schema":2,"id":"x"}"#).unwrap_err().contains("schema 2"));
  assert!(parse(b"{").unwrap_err().contains("invalid JSON"));
}

#[test]
fn ranges_read_as_npm_reads_them() {
  let meets = |v: &str, r: &str| version_meets(v, r);
  // A bare partial version pins what it names, never a caret
  assert!(meets("1.2.9", "1.2") && !meets("1.3.0", "1.2") && meets("1.9.0", "1") && !meets("2.0.0", "1"));
  assert!(meets("1.2.3", "1.2.3") && !meets("1.2.4", "1.2.3"));
  // Space-separated comparators, an operator apart from its version, `x` wildcards
  assert!(meets("1.5.0", ">=1.2 <2") && !meets("2.0.0", ">=1.2 <2") && meets("1.5.0", ">= 1.2, < 2"));
  assert!(meets("0.3.4", "0.3.x") && meets("0.3.4", "^0.3") && !meets("0.4.0", "~0.3.1"));
  // Free-form versions in a manifest
  assert!(meets("1", "^1") && meets("v1.2", "~1.2") && !meets("soon", "*") && meets("1.0.0", ""));
  assert!(version_req("1 || 2").is_err() && version_req("1 - 2").is_err() && version_req(">=").is_err());
}

#[test]
fn ids_and_tool_names() {
  assert!(valid_id("currency") && valid_id("a-1") && !valid_id("a_b") && !valid_id("-a") && !valid_id(""));
  assert!(valid_tool_name("convert_rate") && !valid_tool_name("1x") && !valid_tool_name("a-b"));
  assert!(inside(Path::new("/p"), "a/b.css").is_some() && inside(Path::new("/p"), "../x").is_none() && inside(Path::new("/p"), "/etc/x").is_none());
}
