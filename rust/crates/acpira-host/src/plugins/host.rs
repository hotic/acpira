//! The engine's side of the plugins: what is loaded now (from `state.json`, re-read when it changes), the plugin
//! page's view and actions, activation (freeze, static check, the user's confirmation, dynamic check, commit) and
//! the script tools Acpira's MCP server serves (`docs/dev/plugins.md`)

use std::collections::{BTreeMap, BTreeSet};
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, SystemTime};

use anyhow::{Result, anyhow, bail};
use serde_json::{Value, json};

use acpira_shared::plugins::{PluginAction, PluginChange, PluginFindings, PluginInfo, PluginProposal, PluginProvenance, PluginsView, ToolGroupInfo};

use crate::platform::paths::wire_path;
use crate::plugins::manifest::{self, Findings, Manifest, ToolDef};
use crate::plugins::run::{self, CallContext};
use crate::plugins::store::{Change, Frozen, PluginStore, Provenance, State};
use crate::store::transcript_store::LogFn;

/// Longest a plugin's `check` may run once the user confirmed
const CHECK_TIMEOUT: Duration = Duration::from_secs(45);
/// The most of one plugin's theme style sheet a page gets
const THEME_CSS_MAX_BYTES: usize = 64 * 1024;
/// Changes the plugin page lists
const CHANGES_SHOWN: usize = 30;
/// `ACPIRA_SAFE_MODE=1` (or `--safe-mode`): the engine starts with every user plugin unloaded, whatever the switch says
pub const SAFE_MODE_ENV: &str = "ACPIRA_SAFE_MODE";

static FORCED: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

/// `--safe-mode` on the command line: the engine of this process starts with every user plugin unloaded
pub fn force_safe_mode() {
  FORCED.store(true, Ordering::Relaxed);
}

/// One enabled plugin in effect: its frozen copy and manifest
#[derive(Debug)]
pub struct Active {
  pub id: String,
  pub hash: String,
  pub dir: PathBuf,
  pub manifest: Manifest,
}

/// What is loaded now
#[derive(Debug, Default)]
pub struct Loaded {
  pub state: State,
  /// Effective: the switch or the launch
  pub safe_mode: bool,
  pub active: Vec<Arc<Active>>,
  /// Why an enabled plugin is not in effect
  pub errors: BTreeMap<String, String>,
}

impl Loaded {
  /// The script tools in effect, by the name they are served under
  pub fn tool(&self, served: &str) -> Option<(Arc<Active>, ToolDef)> {
    self.active.iter().find_map(|a| a.manifest.contributes.tools.iter().find(|t| manifest::served_name(&a.id, &t.name) == served).map(|t| (a.clone(), t.clone())))
  }

  /// The MCP tool definitions of every script tool in effect
  pub fn tool_defs(&self) -> Vec<Value> {
    let mut out = vec![];
    for a in &self.active {
      for t in &a.manifest.contributes.tools {
        let mut description = t.description.clone();
        description.push_str(&format!(" (Acpira plugin `{}`)", a.id));
        out.push(json!({
          "name": manifest::served_name(&a.id, &t.name),
          "description": description,
          "inputSchema": t.input,
          "annotations": { "title": format!("{}: {}", a.manifest.display_name(), t.name), "readOnlyHint": t.read_only },
        }));
      }
    }
    out
  }

  /// The built-in tool groups switched off
  pub fn groups_off(&self) -> Vec<String> {
    self.state.tool_groups.iter().filter(|(_, on)| !**on).map(|(g, _)| g.clone()).collect()
  }
}

pub type Listener = Arc<dyn Fn() + Send + Sync>;

pub struct PluginHost {
  store: PluginStore,
  /// Safe mode from the launch (`--safe-mode`, `ACPIRA_SAFE_MODE`)
  forced_safe: bool,
  loaded: parking_lot::RwLock<Arc<Loaded>>,
  /// `state.json`'s modification time and length at the last read
  stamp: parking_lot::Mutex<Option<(Option<SystemTime>, u64)>>,
  listeners: parking_lot::Mutex<Vec<(u64, Listener)>>,
  seq: AtomicU64,
  /// Tells the other engines on the bus that the state changed (`bus.touch("plugins")`)
  touch: parking_lot::RwLock<Option<Listener>>,
  log: LogFn,
  last_error: parking_lot::Mutex<Option<String>>,
}

impl PluginHost {
  /// `root` is `~/.acpira/plugins`
  pub fn new(root: PathBuf, forced_safe: bool, log: LogFn) -> Arc<PluginHost> {
    Arc::new(PluginHost {
      store: PluginStore::new(root),
      forced_safe,
      loaded: Default::default(),
      stamp: Default::default(),
      listeners: Default::default(),
      seq: AtomicU64::new(1),
      touch: Default::default(),
      log,
      last_error: Default::default(),
    })
  }

  /// Safe mode from the launch of this process: `--safe-mode` (`force_safe_mode`) or `ACPIRA_SAFE_MODE`
  pub fn safe_mode_from_env() -> bool {
    FORCED.load(Ordering::Relaxed) || std::env::var(SAFE_MODE_ENV).is_ok_and(|v| !v.is_empty() && v != "0")
  }

  pub fn store(&self) -> &PluginStore {
    &self.store
  }

  pub fn loaded(&self) -> Arc<Loaded> {
    self.loaded.read().clone()
  }

  pub fn forced_safe(&self) -> bool {
    self.forced_safe
  }

  pub fn set_touch(&self, f: Listener) {
    *self.touch.write() = Some(f);
  }

  pub fn subscribe(&self, f: Listener) -> u64 {
    let id = self.seq.fetch_add(1, Ordering::Relaxed);
    self.listeners.lock().push((id, f));
    id
  }

  pub fn unsubscribe(&self, id: u64) {
    self.listeners.lock().retain(|(i, _)| *i != id);
  }

  fn notify(&self) {
    let listeners: Vec<Listener> = self.listeners.lock().iter().map(|(_, f)| f.clone()).collect();
    for f in listeners {
      f();
    }
  }

  /// At start: the log line a crash left out, then the first load
  pub async fn start(&self) {
    if let Err(e) = self.store.reconcile().await {
      (self.log)(&format!("plugins: {e:#}"));
    }
    self.reload();
  }

  /// Read `state.json` again when it changed since the last read (another engine's commit, a hand edit) and load what
  /// it enables; true when what is loaded changed. Cheap when nothing changed: one metadata call
  pub fn reload(&self) -> bool {
    let path = self.store.state_path();
    let stamp = std::fs::metadata(&path).ok().map(|m| (m.modified().ok(), m.len()));
    {
      let mut last = self.stamp.lock();
      if last.is_some() && *last == Some(stamp.unwrap_or((None, 0))) {
        return false;
      }
      *last = Some(stamp.unwrap_or((None, 0)));
    }
    let state = match self.store.load() {
      Ok(s) => s,
      Err(e) => {
        let e = format!("plugins: {e:#}");
        if self.last_error.lock().replace(e.clone()).as_deref() != Some(e.as_str()) {
          (self.log)(&e);
        }
        return false;
      }
    };
    *self.last_error.lock() = None;
    let safe_mode = self.forced_safe || state.safe_mode;
    let mut active = vec![];
    let mut errors = BTreeMap::new();
    if !safe_mode {
      for (id, e) in state.plugins.iter().filter(|(_, e)| e.enabled) {
        match self.store.open(id, &e.snapshot) {
          Ok((dir, manifest)) => active.push(Arc::new(Active { id: id.clone(), hash: e.snapshot.clone(), dir, manifest })),
          Err(err) => {
            (self.log)(&format!("plugins: {err:#}"));
            errors.insert(id.clone(), format!("{err:#}"));
          }
        }
      }
    }
    let next = Loaded { state, safe_mode, active, errors };
    let same = {
      let cur = self.loaded.read();
      cur.state == next.state && cur.safe_mode == next.safe_mode && cur.errors == next.errors
    };
    *self.loaded.write() = Arc::new(next);
    if !same {
      self.notify();
    }
    !same
  }

  /// After a commit here: load it, tell the pages and the other engines
  fn committed(&self, change: &Option<Change>) {
    if change.is_none() {
      return;
    }
    // The commit wrote the file within this second: the stamp alone may not tell
    *self.stamp.lock() = None;
    self.reload();
    if let Some(f) = self.touch.read().clone() {
      f();
    }
  }

  // Script tools

  /// Run the script tool served as `served`. Checked again right before: committed, enabled, not under safe mode
  pub async fn call(&self, served: &str, arguments: &Value, ctx: &CallContext) -> Result<String, String> {
    self.reload();
    let loaded = self.loaded();
    if loaded.safe_mode {
      return Err("Acpira is in safe mode: plugins are unloaded, so this tool is not available.".into());
    }
    let Some((plugin, tool)) = loaded.tool(served) else {
      return Err(format!("`{served}` is not a tool of an enabled plugin (it may have been switched off)."));
    };
    run::call(&tool, &plugin.dir, &self.store.data_dir(&plugin.id), arguments, ctx).await
  }

  /// The hooks of the plugins in effect, in id order, each running in its frozen copy
  pub fn hook_configs(&self) -> Vec<crate::hooks::HooksConfig> {
    self.reload();
    let mut out = vec![];
    for a in &self.loaded().active {
      let Some(hooks) = &a.manifest.contributes.hooks else { continue };
      // Checked when it was enabled; a version that no longer parses is left out with a log line
      match crate::hooks::parse(&a.dir, &hooks.to_string()) {
        Ok(mut cfg) => {
          let data = self.store.data_dir(&a.id);
          let _ = std::fs::create_dir_all(&data);
          let env = vec![("ACPIRA_PLUGIN_DIR".to_owned(), a.dir.to_string_lossy().into_owned()), ("ACPIRA_PLUGIN_DATA".to_owned(), data.to_string_lossy().into_owned())];
          cfg.plugin = Some(crate::hooks::PluginSource { id: a.id.clone(), dir: a.dir.clone(), env });
          out.push(cfg);
        }
        Err(e) => (self.log)(&format!("plugins: {} hooks: {e}", a.id)),
      }
    }
    out
  }

  /// The style layer of the themes in effect, empty without one and in safe mode: each plugin's tokens on the page's
  /// token roots (the same specificity as `tokens.css`, and later), then its style sheet, in id order
  pub fn theme_css(&self) -> String {
    self.reload();
    let mut out = String::new();
    for a in &self.loaded().active {
      let Some(theme) = &a.manifest.contributes.theme else { continue };
      out.push_str(&format!("/* Acpira plugin {} */\n", a.id));
      // Checked when it was enabled; checked again here, since this text goes into every page
      let tokens: Vec<String> = theme
        .tokens
        .iter()
        .filter(|(k, _)| manifest::THEME_TOKENS.contains(&k.as_str()))
        .filter_map(|(k, v)| v.as_str().filter(|v| manifest::safe_css_value(v)).map(|v| format!("  {k}: {v};")))
        .collect();
      if !tokens.is_empty() {
        out.push_str(&format!(":root, .acp-shell, [data-theme] {{\n{}\n}}\n", tokens.join("\n")));
      }
      if let Some(css) = theme.css.as_deref().and_then(|c| manifest::inside(&a.dir, c)).and_then(|p| std::fs::read_to_string(p).ok()) {
        let mut end = css.len().min(THEME_CSS_MAX_BYTES);
        while !css.is_char_boundary(end) {
          end -= 1;
        }
        out.push_str(css.get(..end).unwrap_or_default());
        out.push('\n');
      }
    }
    out
  }

  /// The MCP servers of the plugins in effect, for a new local session
  pub fn mcp_servers(&self) -> Vec<crate::shared_config::mcp::SharedServer> {
    self.reload();
    crate::plugins::mcp::servers(&self.loaded().active, &|id| self.store.data_dir(id))
  }

  // Activation

  /// Freeze the source folder of `id` and check it statically; nothing of the plugin runs
  pub fn prepare(&self, id: &str) -> Result<Frozen> {
    self.store.freeze(id, &on_path)
  }

  /// The user confirmed the frozen version `hash` of `id`: the dynamic check on that copy, then the commit
  pub async fn confirm(&self, id: &str, hash: &str, provenance: Provenance) -> Result<Option<Change>> {
    let (dir, manifest) = self.store.open(id, hash)?;
    if let Some(check) = manifest.check.as_ref().filter(|c| !c.is_empty()) {
      let tool = ToolDef { name: "check".into(), description: String::new(), input: json!({}), command: check.clone(), read_only: true, timeout: Some(CHECK_TIMEOUT.as_secs_f64()) };
      run::call(&tool, &dir, &self.store.data_dir(id), &json!({}), &CallContext::default()).await.map_err(|e| anyhow!("the check failed: {e}"))?;
    }
    if let Some(mcp) = manifest.contributes.mcp.as_ref().filter(|m| !m.is_empty()) {
      crate::plugins::mcp::check(mcp, &dir, &self.store.data_dir(id), CHECK_TIMEOUT).await.map_err(|e| anyhow!("the check failed: {e}"))?;
    }
    let change = self.store.enable(id, hash, provenance).await?;
    self.committed(&change);
    Ok(change)
  }

  /// Freeze, check and enable `id` in one go: what the plugin page's switch does (the switch is the confirmation)
  pub async fn activate(&self, id: &str, provenance: Provenance) -> Result<Option<Change>> {
    let frozen = self.prepare(id)?;
    if !frozen.findings.ok() {
      bail!("{}", frozen.findings.text());
    }
    self.confirm(id, &frozen.hash, provenance).await
  }

  /// File the current source of `id` for the user to confirm on the plugin page (`acpira plugin propose`)
  pub async fn propose(&self, id: &str, provenance: Provenance) -> Result<(Frozen, Option<Change>)> {
    let frozen = self.prepare(id)?;
    if !frozen.findings.ok() {
      bail!("{}", frozen.findings.text());
    }
    let change = self.store.propose(id, &frozen.hash, provenance).await?;
    self.committed(&change);
    Ok((frozen, change))
  }

  /// One of the plugin page's actions (`Reveal` is the shell's; the source folder is returned for it)
  pub async fn action(&self, action: PluginAction) -> Result<Option<PathBuf>> {
    let user = Provenance::user();
    let change = match action {
      PluginAction::Enable { id } => self.activate(&id, user).await?,
      PluginAction::Confirm { id, snapshot } => self.confirm(&id, &snapshot, user).await?,
      PluginAction::Dismiss { id } => self.store.dismiss(&id, user).await?,
      PluginAction::Disable { id } => self.store.disable(&id, user).await?,
      PluginAction::Rollback { id } => self.store.rollback(&id, user).await?,
      PluginAction::Remove { id } => self.store.remove(&id, user).await?,
      PluginAction::Reveal { id } => {
        let dir = self.store.source(&id);
        return if dir.is_dir() { Ok(Some(dir)) } else { Err(anyhow!("no plugin folder {}", dir.display())) };
      }
      PluginAction::SafeMode { on } => self.store.set_safe_mode(on, user).await?,
      PluginAction::ToolGroup { id, on } => {
        if !acpira_shared::host_tools::GROUPS.iter().any(|(g, _)| *g == id) {
          bail!("no tool group `{id}`");
        }
        self.store.set_tool_group(&id, on, user).await?
      }
    };
    self.committed(&change);
    Ok(None)
  }

  pub async fn set_safe_mode(&self, on: bool, provenance: Provenance) -> Result<Option<Change>> {
    let change = self.store.set_safe_mode(on, provenance).await?;
    self.committed(&change);
    Ok(change)
  }

  // The plugin page

  /// The page's view: every source folder and every plugin the state knows, the tool groups, the newest changes
  pub fn view(&self) -> PluginsView {
    self.reload();
    let loaded = self.loaded();
    let state = &loaded.state;
    let mut ids: BTreeSet<String> = self.store.sources().into_iter().collect();
    ids.extend(state.plugins.keys().cloned());
    ids.extend(state.proposals.keys().cloned());
    let plugins = ids.iter().map(|id| self.info(id, &loaded)).collect();
    let tool_groups = acpira_shared::host_tools::GROUPS
      .iter()
      .map(|(g, tools)| ToolGroupInfo { id: (*g).into(), on: state.tool_group_on(g), tools: tools.iter().map(|t| (*t).into()).collect() })
      .collect();
    let mut changes: Vec<PluginChange> = self.store.changes(CHANGES_SHOWN).into_iter().map(change_view).collect();
    changes.reverse();
    PluginsView {
      root: wire_path(self.store.root()),
      generation: state.generation,
      safe_mode: loaded.safe_mode,
      safe_mode_forced: self.forced_safe,
      plugins,
      tool_groups,
      changes,
    }
  }

  fn info(&self, id: &str, loaded: &Loaded) -> PluginInfo {
    let source = self.store.source(id);
    let entry = loaded.state.plugins.get(id);
    let active = loaded.active.iter().find(|a| a.id == id);
    // The source's manifest describes the plugin as it is written; the frozen one when the source is gone
    let mut error = None;
    let mut findings = None;
    let from_source = if source.is_dir() {
      match manifest::read(&source) {
        Ok((m, mut f)) => {
          manifest::check(&m, id, &source, &on_path, &mut f);
          findings = Some(PluginFindings { errors: f.errors, notes: f.notes });
          Some(m)
        }
        Err(e) => {
          error = Some(e);
          None
        }
      }
    } else {
      None
    };
    let m = from_source.as_ref().or(active.map(|a| &a.manifest));
    if let Some(e) = loaded.errors.get(id) {
      error = Some(e.clone());
    }
    let tools = m.map(|m| m.contributes.tools.iter().map(|t| manifest::served_name(id, &t.name)).collect()).unwrap_or_default();
    PluginInfo {
      id: id.to_owned(),
      name: m.map(|m| m.display_name().to_owned()).unwrap_or_else(|| id.to_owned()),
      description: m.map(|m| m.description.clone()).unwrap_or_default(),
      version: m.map(|m| m.version.clone()).unwrap_or_default(),
      layer: m.map(|m| m.layer().to_owned()).unwrap_or_else(|| "tools".into()),
      contributes: m.map(|m| m.contributes.kinds().into_iter().map(str::to_owned).collect()).unwrap_or_default(),
      tools,
      enabled: entry.is_some_and(|e| e.enabled),
      loaded: active.is_some(),
      snapshot: entry.map(|e| e.snapshot.clone()).filter(|s| !s.is_empty()),
      can_rollback: entry.is_some_and(|e| !e.previous.is_empty()),
      provenance: entry.map(|e| provenance_view(&e.provenance)),
      committed_at: entry.map(|e| e.committed_at).filter(|t| *t > 0),
      proposal: loaded.state.proposals.get(id).map(|p| PluginProposal { snapshot: p.snapshot.clone(), provenance: provenance_view(&p.provenance), at: p.at }),
      changed: entry.filter(|e| !e.snapshot.is_empty()).and_then(|e| self.store.source_changed(id, &e.snapshot)).unwrap_or(false),
      error,
      findings,
      path: wire_path(&source),
    }
  }
}

fn provenance_view(p: &Provenance) -> PluginProvenance {
  PluginProvenance { by: p.by.clone(), session: p.session.clone(), agent: p.agent.clone(), model: p.model.clone() }
}

fn change_view(c: Change) -> PluginChange {
  PluginChange { generation: c.generation, at: c.at, id: c.id, action: c.action, snapshot: c.snapshot, on: c.on, provenance: provenance_view(&c.provenance) }
}

/// A frozen plugin as the card and the guide describe it: what it adds, its files and hash, the static check
pub fn describe(frozen: &Frozen) -> String {
  let m = &frozen.manifest;
  let mut lines = vec![format!("{} {} ({})", m.display_name(), m.version, frozen.id)];
  if !m.description.is_empty() {
    lines.push(m.description.clone());
  }
  for t in &m.contributes.tools {
    lines.push(format!("tool {}: {}", manifest::served_name(&m.id, &t.name), t.command.join(" ")));
  }
  for k in m.contributes.kinds().into_iter().filter(|k| *k != "tools") {
    lines.push(format!("contributes {k}"));
  }
  lines.push(format!("files: {}", frozen.files.join(", ")));
  lines.push(format!("version {}", frozen.hash));
  let check: &Findings = &frozen.findings;
  if !check.notes.is_empty() {
    lines.push(check.text());
  }
  lines.join("\n")
}

/// Whether `program` is on the PATH agents are launched with (a sync lookup, for the static check)
pub fn on_path(program: &str) -> bool {
  let Some(path) = crate::acp::agents::login_path::effective_path() else { return false };
  let exts: Vec<String> = if cfg!(windows) {
    std::env::var("PATHEXT").unwrap_or_else(|_| ".EXE;.CMD;.BAT;.COM".into()).split(';').map(str::to_owned).collect()
  } else {
    vec![String::new()]
  };
  std::env::split_paths(&path).any(|dir| {
    exts.iter().any(|ext| {
      let p = dir.join(format!("{program}{ext}"));
      std::fs::metadata(&p).is_ok_and(|m| m.is_file() && executable(&m))
    })
  })
}

fn executable(m: &std::fs::Metadata) -> bool {
  #[cfg(unix)]
  {
    use std::os::unix::fs::PermissionsExt;
    m.permissions().mode() & 0o111 != 0
  }
  #[cfg(not(unix))]
  {
    let _ = m;
    true
  }
}

#[cfg(test)]
#[cfg(unix)]
mod tests {
  use super::*;

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
    assert!(css.contains(":root, .acp-shell, [data-theme] {\n  --bg-0: #1b1a18;\n  --brand: oklch(70% 0.1 60);\n}"), "{css}");
    assert!(css.trim_end().ends_with(".acp-shell { letter-spacing: 0.01em; }"), "{css}");
    h.action(PluginAction::SafeMode { on: true }).await.unwrap();
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
    assert_eq!(h.loaded().groups_off(), ["summon"]);
    assert!(h.action(PluginAction::ToolGroup { id: "nope".into(), on: false }).await.is_err());
  }
}
