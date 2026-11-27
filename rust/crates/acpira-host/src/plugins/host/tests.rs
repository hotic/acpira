use super::*;

#[test]
fn a_style_names_the_hooks_it_restyles_in_order() {
  let css = "/* [data-part=\"composer\"] in a comment */\n[data-part='user-message'] > div { color: red }\n[data-session][data-state] {}\n[data-part = \"user-message\"] {}";
  assert_eq!(restyled_hooks(css), ["[data-part=\"user-message\"]", "[data-session]", "[data-session][data-state]"]);
  assert!(restyled_hooks("body { color: red }").is_empty());
}

fn host(root: &std::path::Path) -> Arc<PluginHost> {
  PluginHost::new(root.to_path_buf(), false, Arc::new(|_| {}))
}

fn write_plugin(root: &std::path::Path, id: &str, script: &str, check: Option<&str>) {
  let dir = root.join(id);
  std::fs::create_dir_all(&dir).unwrap();
  let mut m = json!({ "schema": 1, "id": id, "version": "0.1.0", "description": "Demo",
    "contributes": { "tools": [{ "name": "go", "description": "Go", "command": ["sh", "go.sh"], "readOnly": true }] } });
  if let Some(c) = check {
    m["check"] = json!(["sh", "-c", c]);
  }
  std::fs::write(dir.join("plugin.json"), m.to_string()).unwrap();
  std::fs::write(dir.join("go.sh"), script).unwrap();
}

#[tokio::test]
async fn account_rows_are_discovered_only_from_a_confirmed_runtime() {
  let tmp = tempfile::tempdir().unwrap();
  let h = host(tmp.path());
  h.start().await;
  let dir = tmp.path().join("demo");
  std::fs::create_dir_all(&dir).unwrap();
  let m = json!({ "schema": 1, "id": "demo", "version": "1", "description": "Accounts",
    "contributes": { "runtime": { "command": ["sh"], "methods": ["account.rows"] } } });
  std::fs::write(dir.join("plugin.json"), m.to_string()).unwrap();
  let (snap, _) = h.propose("demo", Provenance::cli()).await.unwrap();
  assert!(h.ui_slots().is_empty());
  // Commit a checked snapshot directly; no runtime is needed for slot discovery.
  h.store.enable("demo", &snap.hash, Provenance::user()).await.unwrap();
  assert!(h.ui_slots().iter().any(|s| s.plugin == "demo" && s.slot == "account.rows" && s.html.is_empty()));
  h.action(PluginAction::Disable { id: "demo".into() }).await.unwrap();
  assert!(h.ui_slots().is_empty());
}

#[tokio::test]
async fn reveal_uses_the_active_snapshot_when_the_source_is_gone() {
  let tmp = tempfile::tempdir().unwrap();
  let h = host(tmp.path());
  h.start().await;
  write_plugin(tmp.path(), "demo", "exit 0", None);
  let (frozen, _) = h.propose("demo", Provenance::cli()).await.unwrap();
  h.action(PluginAction::Remove { id: "demo".into() }).await.unwrap();
  h.action(PluginAction::Confirm { id: "demo".into(), snapshot: frozen.hash.clone() }).await.unwrap();
  let reveal = || h.action(PluginAction::Reveal { id: "demo".into() });
  assert_eq!(reveal().await.unwrap(), Some(frozen.dir));
  write_plugin(tmp.path(), "demo", "exit 0", None);
  assert_eq!(reveal().await.unwrap(), Some(tmp.path().join("demo")));
}

#[tokio::test]
async fn an_agent_and_a_prompt_folder_come_from_the_frozen_copy() {
  let tmp = tempfile::tempdir().unwrap();
  let h = host(tmp.path());
  h.start().await;
  for id in ["pal", "claude"] {
    let dir = tmp.path().join(id);
    std::fs::create_dir_all(dir.join("prompts")).unwrap();
    std::fs::write(dir.join("prompts/system.md"), "## Style\n- Short.\n").unwrap();
    std::fs::write(dir.join("run.sh"), "").unwrap();
    let m = json!({ "schema": 1, "id": id, "version": "1", "description": "Pal",
      "contributes": { "agent": { "command": "run.sh", "args": ["acp", "${ACPIRA_PLUGIN_DATA}"] }, "prompts": "prompts" } });
    std::fs::write(dir.join("plugin.json"), m.to_string()).unwrap();
    h.action(PluginAction::Enable { id: id.into() }).await.unwrap();
  }
  // A built-in agent's id is never taken; the other runs from its snapshot with the plugin's name
  let agents = h.agent_entries();
  assert_eq!(agents.keys().collect::<Vec<_>>(), ["pal"]);
  let frozen = h.loaded().active.iter().find(|a| a.id == "pal").unwrap().dir.clone();
  assert_eq!(agents["pal"]["command"], json!(frozen.join("run.sh").to_string_lossy()));
  assert_eq!(agents["pal"]["name"], "pal");
  assert_eq!(agents["pal"]["args"][1], json!(h.store().data_dir("pal").to_string_lossy()));
  assert_eq!(h.prompt_layers().len(), 2);
  assert!(h.prompt_layers().iter().all(|d| d.ends_with("prompts") && d.starts_with(tmp.path().join(".versions"))));
  h.action(PluginAction::SafeMode { on: true }).await.unwrap();
  assert!(h.agent_entries().is_empty() && h.prompt_layers().is_empty());
}

#[tokio::test]
async fn a_specialised_agent_is_entered_with_its_frozen_copy_as_the_profile() {
  let tmp = tempfile::tempdir().unwrap();
  let h = host(tmp.path());
  h.start().await;
  let dir = tmp.path().join("ledger");
  std::fs::create_dir_all(&dir).unwrap();
  std::fs::write(dir.join("prompt.md"), "You keep the ledger.").unwrap();
  let write = |agent: Value| {
    let m = json!({ "schema": 1, "id": "ledger", "version": "1", "description": "Ledger", "contributes": { "agent": agent } });
    std::fs::write(dir.join("plugin.json"), m.to_string()).unwrap();
  };
  // A profile whose prompt is not in the plugin, or one that also names a command, is refused before anything runs
  write(json!({ "builtin": { "prompt": "missing.md", "tools": [] } }));
  assert!(h.action(PluginAction::Enable { id: "ledger".into() }).await.unwrap_err().to_string().contains("missing.md"));
  write(json!({ "command": "run.sh", "builtin": { "prompt": "prompt.md", "tools": [] } }));
  assert!(h.action(PluginAction::Enable { id: "ledger".into() }).await.unwrap_err().to_string().contains("exclude each other"));
  write(json!({ "name": "Ledger", "builtin": { "prompt": "prompt.md", "tools": ["read"] } }));
  h.action(PluginAction::Enable { id: "ledger".into() }).await.unwrap();
  let frozen = h.loaded().active.iter().find(|a| a.id == "ledger").unwrap().dir.clone();
  assert_eq!(h.agent_entries()["ledger"], json!({ "name": "Ledger", PROFILE_KEY: frozen }));
}

#[tokio::test]
async fn a_theme_becomes_one_style_layer_that_safe_mode_takes_away() {
  let tmp = tempfile::tempdir().unwrap();
  let h = host(tmp.path());
  h.start().await;
  assert_eq!(h.theme_css(), "");
  let dir = tmp.path().join("warm");
  std::fs::create_dir_all(&dir).unwrap();
  let m = json!({ "schema": 1, "id": "warm", "version": "1", "description": "Warm",
    "contributes": { "theme": { "tokens": { "--bg-0": "#1b1a18", "--brand": "oklch(70% 0.1 60)" }, "css": "theme.css" } } });
  std::fs::write(dir.join("plugin.json"), m.to_string()).unwrap();
  std::fs::write(dir.join("theme.css"), ".acp-shell { letter-spacing: 0.01em; }").unwrap();
  h.action(PluginAction::Enable { id: "warm".into() }).await.unwrap();
  let css = h.theme_css();
  assert!(css.contains(":root, .acp-shell, [data-theme], .acp-shell.acp-shell[data-skin] {\n  --bg-0: #1b1a18;\n  --brand: oklch(70% 0.1 60);\n}"), "{css}");
  assert!(css.contains("--skin-ink: var(--brand);"), "{css}");
  assert!(css.trim_end().ends_with(".acp-shell { letter-spacing: 0.01em; }\n/* */"), "{css}");
  h.action(PluginAction::SafeMode { on: true }).await.unwrap();
  assert_eq!(h.theme_css(), "");
}

#[tokio::test]
async fn styles_stack_over_the_skin_and_survive_a_skin_change() {
  let tmp = tempfile::tempdir().unwrap();
  let h = host(tmp.path());
  h.start().await;
  write_theme(tmp.path(), "warm", "#201810", false);
  for id in ["rows", "dots"] {
    let dir = tmp.path().join(id);
    std::fs::create_dir_all(&dir).unwrap();
    let m = json!({ "schema": 1, "id": id, "version": "1", "description": id, "contributes": { "style": "style.css" } });
    std::fs::write(dir.join("plugin.json"), m.to_string()).unwrap();
    std::fs::write(dir.join("style.css"), format!("[data-session] {{ --{id}: 1; }}")).unwrap();
    h.action(PluginAction::Enable { id: id.into() }).await.unwrap();
  }
  // No skin yet: the styles alone
  let css = h.theme_css();
  assert!(css.contains("--rows: 1") && css.contains("--dots: 1"), "{css}");
  h.action(PluginAction::Enable { id: "warm".into() }).await.unwrap();
  let css = h.theme_css();
  let (skin, rows) = (css.find("#201810").unwrap(), css.find("--rows: 1").unwrap());
  assert!(skin < rows && css.contains("--dots: 1"), "{css}");
  h.action(PluginAction::Skin { id: None }).await.unwrap();
  let css = h.theme_css();
  assert!(!css.contains("#201810") && css.contains("--rows: 1"), "{css}");
  assert_eq!(h.view().plugins.iter().find(|p| p.id == "rows").map(|p| p.layer.as_str()), Some("interface"));
}

/// A skin with a document of its own, or (`scene`) a scene for Acpira's renderer
fn write_theme(root: &std::path::Path, id: &str, bg: &str, scene: bool) {
  let dir = root.join(id);
  std::fs::create_dir_all(&dir).unwrap();
  let (key, file, text) = if scene { ("scene", "scene.js", format!("const SCENE = {{ cell() {{}} }}; // {id}")) } else { ("backdrop", "backdrop.html", format!("<p>{id}</p>")) };
  let m = json!({ "schema": 1, "id": id, "version": "1", "description": id,
    "contributes": { "theme": { "base": "dark", "glass": "calm", key: file, "tokens": { "--bg-0": bg } } } });
  std::fs::write(dir.join("plugin.json"), m.to_string()).unwrap();
  std::fs::write(dir.join(file), text).unwrap();
}

#[tokio::test]
async fn one_theme_is_the_skin_and_a_card_puts_back_the_one_before() {
  let tmp = tempfile::tempdir().unwrap();
  let h = host(tmp.path());
  h.start().await;
  write_theme(tmp.path(), "warm", "#201810", false);
  write_theme(tmp.path(), "cool", "#101820", true);
  h.action(PluginAction::Enable { id: "warm".into() }).await.unwrap();
  h.action(PluginAction::Enable { id: "cool".into() }).await.unwrap();
  // Only the skin's tokens are on the page, and it comes with its look and backdrop
  let css = h.theme_css();
  assert!(css.contains("#101820") && !css.contains("#201810"), "{css}");
  let skin = h.skin().unwrap();
  assert_eq!((skin.plugin.as_str(), skin.base.as_deref(), skin.glass.as_deref()), ("cool", Some("dark"), Some("calm")));
  // A scene comes inside the renderer this build ships
  let doc = skin.backdrop.unwrap();
  assert!(doc.contains("// cool") && doc.contains("acpira: 'drawn'"), "{doc}");
  assert_eq!(h.view().skin.as_deref(), Some("cool"));
  // A card for warm's version makes it the skin again and remembers cool; undoing puts cool back, warm stays on
  let warm = h.view().plugins.into_iter().find(|p| p.id == "warm").unwrap().snapshot.unwrap();
  assert_eq!(h.backdrop_of("warm", &warm).as_deref(), Some("<p>warm</p>"));
  let mut card = acpira_shared::transcript::PluginCard { id: "c1".into(), plugin: "warm".into(), snapshot: warm.clone(), kind: "skin".into(), ..Default::default() };
  let undo = h.apply_card(&card, Provenance::default()).await.unwrap().unwrap();
  // A second click before the page caught up changes nothing and keeps that record
  assert!(h.apply_card(&card, Provenance::default()).await.unwrap().is_none());
  assert_eq!((undo.snapshot.as_deref(), undo.skin.as_deref()), (Some(warm.as_str()), Some("cool")));
  assert_eq!(h.view().skin.as_deref(), Some("warm"));
  card.undo = Some(undo);
  h.undo_card(&card, Provenance::default()).await.unwrap();
  let view = h.view();
  assert_eq!(view.skin.as_deref(), Some("cool"));
  assert!(view.plugins.iter().find(|p| p.id == "warm").unwrap().enabled);
  // A card applied elsewhere has no record: undoing switches its plugin off and leaves no skin
  card.undo = None;
  h.apply_card(&card, Provenance::default()).await.unwrap();
  h.undo_card(&card, Provenance::default()).await.unwrap();
  let view = h.view();
  assert!(view.skin.is_none() && !view.plugins.iter().find(|p| p.id == "warm").unwrap().enabled, "{view:?}");
  assert_eq!(h.theme_css(), "");
}

#[tokio::test]
async fn the_page_switch_enables_a_tool_every_agent_can_call() {
  let tmp = tempfile::tempdir().unwrap();
  let h = host(tmp.path());
  h.start().await;
  write_plugin(tmp.path(), "demo", "echo hi", None);
  let view = h.view();
  assert_eq!(view.plugins.len(), 1);
  assert!(!view.plugins[0].enabled && view.plugins[0].findings.as_ref().unwrap().errors.is_empty());
  assert!(h.call("demo_go", &json!({}), &CallContext::default()).await.is_err());
  h.action(PluginAction::Enable { id: "demo".into() }).await.unwrap();
  assert_eq!(h.call("demo_go", &json!({}), &CallContext::default()).await.unwrap().trim(), "hi");
  let defs = h.loaded().tool_defs();
  assert_eq!((defs[0]["name"].as_str(), defs[0]["annotations"]["readOnlyHint"].as_bool()), (Some("demo_go"), Some(true)));
  // An edit after the freeze changes nothing until it is enabled
  std::fs::write(tmp.path().join("demo/go.sh"), "echo changed").unwrap();
  let info = &h.view().plugins[0];
  assert!(info.enabled && info.loaded && info.changed && info.provenance.as_ref().unwrap().by == "user");
  assert_eq!(h.call("demo_go", &json!({}), &CallContext::default()).await.unwrap().trim(), "hi");
  h.action(PluginAction::Enable { id: "demo".into() }).await.unwrap();
  assert_eq!(h.call("demo_go", &json!({}), &CallContext::default()).await.unwrap().trim(), "changed");
  h.action(PluginAction::Rollback { id: "demo".into() }).await.unwrap();
  assert_eq!(h.call("demo_go", &json!({}), &CallContext::default()).await.unwrap().trim(), "hi");
  // Switched off: a call made before the client listed again is refused
  h.action(PluginAction::Disable { id: "demo".into() }).await.unwrap();
  assert!(h.call("demo_go", &json!({}), &CallContext::default()).await.unwrap_err().contains("not a tool of an enabled plugin"));
  let actions: Vec<_> = h.view().changes.into_iter().map(|c| c.action).collect();
  assert_eq!(actions, ["disable", "rollback", "enable", "enable"]);
}

#[tokio::test]
async fn a_failing_check_or_static_error_commits_nothing() {
  let tmp = tempfile::tempdir().unwrap();
  let h = host(tmp.path());
  write_plugin(tmp.path(), "demo", "echo hi", Some("echo selftest broke >&2; exit 1"));
  let err = h.action(PluginAction::Enable { id: "demo".into() }).await.unwrap_err();
  assert!(format!("{err:#}").contains("selftest broke"), "{err:#}");
  std::fs::write(tmp.path().join("demo/plugin.json"), json!({ "schema": 1, "id": "demo", "contributes": {} }).to_string()).unwrap();
  assert!(format!("{:#}", h.action(PluginAction::Enable { id: "demo".into() }).await.unwrap_err()).contains("contributes nothing"));
  assert_eq!(h.store().load().unwrap().generation, 0);
}

#[tokio::test]
async fn safe_mode_unloads_every_plugin_and_another_engine_follows() {
  let tmp = tempfile::tempdir().unwrap();
  let (a, b) = (host(tmp.path()), host(tmp.path()));
  write_plugin(tmp.path(), "demo", "echo hi", None);
  a.action(PluginAction::Enable { id: "demo".into() }).await.unwrap();
  b.reload();
  assert_eq!(b.loaded().active.len(), 1);
  a.action(PluginAction::SafeMode { on: true }).await.unwrap();
  assert!(a.call("demo_go", &json!({}), &CallContext::default()).await.unwrap_err().contains("safe mode"));
  // The other engine's next poll (or the bus touch) reads the new state
  *b.stamp.lock() = None;
  b.reload();
  assert!(b.loaded().safe_mode && b.loaded().active.is_empty());
  // Launched in safe mode: the switch does not bring plugins back
  let forced = PluginHost::new(tmp.path().to_path_buf(), true, Arc::new(|_| {}));
  a.action(PluginAction::SafeMode { on: false }).await.unwrap();
  forced.reload();
  assert!(forced.loaded().safe_mode && forced.view().safe_mode_forced);
}

#[tokio::test]
async fn a_proposal_waits_for_the_page() {
  let tmp = tempfile::tempdir().unwrap();
  let h = host(tmp.path());
  write_plugin(tmp.path(), "demo", "echo hi", None);
  let (frozen, _) = h.propose("demo", Provenance::cli()).await.unwrap();
  assert!(describe(&frozen).contains("tool demo_go: sh go.sh"));
  let info = &h.view().plugins[0];
  assert!(!info.enabled && info.proposal.as_ref().unwrap().provenance.by == "cli");
  assert!(h.call("demo_go", &json!({}), &CallContext::default()).await.is_err());
  h.action(PluginAction::Confirm { id: "demo".into(), snapshot: frozen.hash.clone() }).await.unwrap();
  assert!(h.view().plugins[0].proposal.is_none() && h.loaded().active.len() == 1);
  h.action(PluginAction::ToolGroup { id: "summon".into(), on: false }).await.unwrap();
  // Computer use is off until the user switches it on
  assert_eq!(h.loaded().groups_off(), ["summon", "computer"]);
  h.action(PluginAction::ToolGroup { id: "computer".into(), on: true }).await.unwrap();
  assert_eq!(h.loaded().groups_off(), ["summon"]);
  h.action(PluginAction::ToolGroup { id: "computer".into(), on: false }).await.unwrap();
  assert!(!h.loaded().state.tool_groups.contains_key("computer"));
  assert!(h.action(PluginAction::ToolGroup { id: "nope".into(), on: false }).await.is_err());
}

#[tokio::test]
async fn reorder_controls_load_order_and_reports_overlaps() {
  let tmp = tempfile::tempdir().unwrap();
  let h = host(tmp.path());
  for (id, style_hook, cmd) in [("alpha", "[data-session]", "review"), ("beta", "[data-session]", "review"), ("gamma", "[data-part=\"header\"]", "other")] {
    let dir = tmp.path().join(id);
    std::fs::create_dir_all(&dir).unwrap();
    let m = json!({
      "schema": 1, "id": id, "version": "1", "description": id,
      "contributes": {
        "style": "style.css",
        "commands": [{ "name": cmd, "description": cmd, "prompt": format!("from {id}") }]
      }
    });
    std::fs::write(dir.join("plugin.json"), m.to_string()).unwrap();
    std::fs::write(dir.join("style.css"), format!("{style_hook} {{ --{id}: 1; }}")).unwrap();
  }
  h.action(PluginAction::Enable { id: "alpha".into() }).await.unwrap();
  h.action(PluginAction::Enable { id: "beta".into() }).await.unwrap();
  h.action(PluginAction::Enable { id: "gamma".into() }).await.unwrap();
  let gen_before = h.view().generation;
  let changes_before = h.view().changes.len();

  // Default order is alpha, beta, gamma: alpha wins CSS (appended later) and `/review`, and overlaps only with beta
  let css_before = h.theme_css();
  assert!(css_before.find("--beta: 1").unwrap() < css_before.find("--alpha: 1").unwrap(), "{css_before}");
  let view_before = h.view();
  let alpha_overlaps = &view_before.plugins.iter().find(|p| p.id == "alpha").unwrap().overlaps;
  assert_eq!(alpha_overlaps.len(), 2, "{alpha_overlaps:?}");
  assert!(alpha_overlaps.iter().any(|o| o.plugin == "beta" && o.kind == "command" && o.items == ["/review"]));
  assert!(alpha_overlaps.iter().any(|o| o.plugin == "beta" && o.kind == "style" && o.items == ["[data-session]"]));
  assert!(h.view().plugins.iter().find(|p| p.id == "gamma").unwrap().overlaps.is_empty());

  // Moving beta ahead of alpha flips active order, CSS cascade precedence, and contributed commands without touching generation/audit log
  h.action(PluginAction::Reorder { ids: vec!["gamma".into(), "builtin:computer".into(), "beta".into(), "gamma".into()] }).await.unwrap();
  let view = h.view();
  assert_eq!(view.order, ["gamma", "builtin:computer", "beta"]);
  assert_eq!(view.plugins.iter().map(|p| p.id.as_str()).collect::<Vec<_>>(), ["gamma", "beta", "alpha"]);
  assert_eq!(h.loaded().active.iter().map(|a| a.id.as_str()).collect::<Vec<_>>(), ["gamma", "beta", "alpha"]);
  let css_after = h.theme_css();
  assert!(css_after.find("--alpha: 1").unwrap() < css_after.find("--beta: 1").unwrap(), "{css_after}");
  assert_eq!(h.runtimes().contributed().commands.iter().find(|c| c.def.name == "review").map(|c| c.plugin.as_str()), Some("beta"));
  assert_eq!((view.generation, view.changes.len()), (gen_before, changes_before));
  // Invalid ids are rejected
  assert!(h.action(PluginAction::Reorder { ids: vec!["bad_id".into()] }).await.is_err());
  assert!(h.action(PluginAction::Reorder { ids: vec!["builtin:nope".into()] }).await.is_err());
  // Removing a plugin drops it from the saved order and clears its overlaps
  h.action(PluginAction::Remove { id: "beta".into() }).await.unwrap();
  assert_eq!(h.view().order, ["gamma", "builtin:computer"]);
  assert!(h.view().plugins.iter().find(|p| p.id == "alpha").unwrap().overlaps.is_empty());
}
