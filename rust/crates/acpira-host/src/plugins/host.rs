//! The engine's side of the plugins: what is loaded now (from `state.json`, re-read when it changes), the plugin
//! page's view and actions, activation (freeze, static check, the user's confirmation, dynamic check, commit) and
//! the script tools Acpira's MCP server serves (`docs/dev/plugins.md`)

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
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
const THEME_CSS_MAX_BYTES: usize = manifest::CSS_MAX_BYTES as usize;
/// Changes the plugin page lists
const CHANGES_SHOWN: usize = 30;
/// `ACPIRA_SAFE_MODE=1` (or `--safe-mode`): the engine starts with every user plugin unloaded, whatever the switch says
pub const SAFE_MODE_ENV: &str = "ACPIRA_SAFE_MODE";

static FORCED: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

/// `--safe-mode` on the command line: the engine of this process starts with every user plugin unloaded
pub fn force_safe_mode() {
  FORCED.store(true, Ordering::Relaxed);
}

/// The key of a plugin agent entry that names a profile folder for the built-in loop (`acpira agent --profile`)
pub const PROFILE_KEY: &str = "acpiraProfile";

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
  /// The theme plugin in effect as the skin: the state's pick; in a state written before skins, the theme enabled last
  pub fn skin(&self) -> Option<&Arc<Active>> {
    let themed = |a: &&Arc<Active>| a.manifest.contributes.theme.is_some();
    match self.state.skin.as_deref() {
      Some(id) => self.active.iter().filter(themed).find(|a| a.id == id),
      None => self.active.iter().filter(themed).max_by_key(|a| (self.state.plugins.get(&a.id).map_or(0, |e| e.committed_at), a.id.clone())),
    }
  }

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
  /// The resident processes of the plugins in effect (`contributes.runtime`)
  runtimes: Arc<crate::plugins::runtime::Runtimes>,
  /// The schedule loop started (once per engine, by `enable_runtimes`)
  scheduling: std::sync::atomic::AtomicBool,
}

impl PluginHost {
  /// `root` is `~/.acpira/plugins`
  pub fn new(root: PathBuf, forced_safe: bool, log: LogFn) -> Arc<PluginHost> {
    let runtimes = crate::plugins::runtime::Runtimes::new(root.join(crate::plugins::store::DATA), log.clone());
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
      runtimes,
      scheduling: Default::default(),
    })
  }

  /// The resident processes; they start only once the composition root called `Runtimes::enable`
  pub fn runtimes(&self) -> &Arc<crate::plugins::runtime::Runtimes> {
    &self.runtimes
  }

  /// Start what the plugins in effect want running and stop the rest; off the caller's task
  fn sync_runtimes(&self) {
    if !self.runtimes.enabled() {
      return;
    }
    let Ok(handle) = tokio::runtime::Handle::try_current() else { return };
    let runtimes = self.runtimes.clone();
    let active = self.loaded().active.clone();
    handle.spawn(async move { runtimes.sync(&active).await });
  }

  /// Turn the resident processes on (the composition root) and start those already in effect
  pub fn enable_runtimes(&self, host_call: Option<crate::plugins::runtime::HostCall>) {
    self.runtimes.enable(host_call);
    self.sync_runtimes();
    if !self.scheduling.swap(true, Ordering::Relaxed) {
      crate::plugins::schedule::spawn(Arc::downgrade(&self.runtimes), self.store.root().to_path_buf(), self.log.clone());
    }
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
    // The SDK copy plugins written by hand take theirs from, and the skill that teaches an agent to write the first
    // plugin, so it is there before any plugin is (`sdk.rs`)
    if let Err(e) = crate::plugins::sdk::install(self.store.root()) {
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
      self.runtimes.set_contributed(crate::plugins::runtime::Contributed::of(&self.loaded.read().active));
      self.sync_runtimes();
      self.notify();
    }
    !same
  }

  /// After a commit here: load it, tell the pages and the other engines
  fn committed(&self, change: &Option<Change>) {
    let Some(c) = change else { return };
    let (id, action) = (c.id.clone(), c.action.clone());
    self.runtimes.emit("plugin.changed", || json!({ "id": id, "action": action }));
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
    // A tool without a `command` is answered by the plugin's process
    if tool.command.is_empty() {
      return self.runtimes.tool(&plugin.id, &tool.name, arguments, ctx).await;
    }
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

  /// The style layer of the skin in effect, empty without one and in safe mode: its tokens on the page's token roots
  /// (the same specificity as `tokens.css`, and later) and on the skinned shell (above the automatic-theme derivation
  /// of `palette.css`), then its style sheet
  pub fn theme_css(&self) -> String {
    self.reload();
    theme_css_of(&self.loaded())
  }

  /// The theme push: the style layer, the skin and the generation, all of one loaded state, so a commit landing in
  /// between never pairs one skin's colours with another's backdrop
  pub fn theme(&self) -> (String, Option<acpira_shared::plugins::PluginSkin>, u64) {
    self.reload();
    let loaded = self.loaded();
    (theme_css_of(&loaded), skin_of(&loaded), loaded.state.generation)
  }

  /// The skin in effect as the page takes it, its backdrop read from the frozen copy (left out when gone or too large)
  pub fn skin(&self) -> Option<acpira_shared::plugins::PluginSkin> {
    self.reload();
    skin_of(&self.loaded())
  }

  /// The backdrop of the frozen version `snapshot` of `id`, for a card's thumbnail
  pub fn backdrop_of(&self, id: &str, snapshot: &str) -> Option<String> {
    let (dir, m) = self.store.open(id, snapshot).ok()?;
    backdrop_doc(&dir, m.contributes.theme.as_ref()?)
  }

  /// The card a session shows for a frozen version an agent offers
  pub fn card(&self, frozen: &Frozen, card_id: String) -> acpira_shared::transcript::PluginCard {
    let m = &frozen.manifest;
    let c = &m.contributes;
    let runs = c
      .tools
      .iter()
      .find(|t| !t.command.is_empty())
      .map(|t| t.command.join(" "))
      .or_else(|| c.runtime.as_ref().map(|r| r.main.as_ref().map(|m| format!("node {m}")).unwrap_or_else(|| r.command.join(" "))))
      .or_else(|| c.mcp.as_ref().and_then(|m| m.values().next()).and_then(|v| v.get("command")).and_then(Value::as_str).map(str::to_owned))
      .or_else(|| c.agent.as_ref().and_then(|a| a.get("command")).and_then(Value::as_str).map(str::to_owned))
      .or_else(|| m.check.as_ref().map(|c| c.join(" ")));
    acpira_shared::transcript::PluginCard {
      id: card_id,
      plugin: frozen.id.clone(),
      snapshot: frozen.hash.clone(),
      name: m.display_name().to_owned(),
      description: m.description.clone(),
      kind: card_kind(m).into(),
      runs: runs.filter(|r| !r.trim().is_empty()),
      // A command's button is a piece of the composer's toolbar, so it flies in like one
      slots: c.ui.iter().map(|u| u.slot.clone()).chain(c.commands.iter().any(|x| x.button.is_some()).then(|| "composer.toolbar".to_owned())).collect(),
      schedules: c.schedules.iter().map(|j| j.cron.clone()).collect(),
      folder: wire_path(&self.store.source(&frozen.id)),
      at: crate::util::now_ms(),
      undo: None,
    }
  }

  /// Apply a card: enable the version it offers (the dynamic check runs now) and, for a theme, make it the skin. What
  /// was in effect before, for the card's undo; none when the version is in effect already (a second click from a
  /// window that had not caught up), so the record of the first apply is kept
  pub async fn apply_card(&self, card: &acpira_shared::transcript::PluginCard, provenance: Provenance) -> Result<Option<acpira_shared::transcript::PluginCardUndo>> {
    self.reload();
    let themed = self.themed(card);
    let before = {
      let loaded = self.loaded();
      let entry = loaded.state.plugins.get(&card.plugin).filter(|e| e.enabled && !e.snapshot.is_empty());
      let skin = loaded.skin().map(|a| a.id.clone()).unwrap_or_default();
      if entry.is_some_and(|e| e.snapshot == card.snapshot) && (!themed || skin == card.plugin) {
        return Ok(None);
      }
      // By what the version contributes, not the card's kind: a theme with tools is shown as a program but still
      // takes the skin
      acpira_shared::transcript::PluginCardUndo { snapshot: entry.map(|e| e.snapshot.clone()), skin: themed.then_some(skin) }
    };
    self.confirm(&card.plugin, &card.snapshot, provenance).await?;
    Ok(Some(before))
  }

  /// The version a card offers is a theme
  fn themed(&self, card: &acpira_shared::transcript::PluginCard) -> bool {
    card.kind == "skin" || self.store.open(&card.plugin, &card.snapshot).is_ok_and(|(_, m)| m.contributes.theme.is_some())
  }

  /// Put back what applying a card changed; a card applied elsewhere (no record) switches its plugin off, and a skin
  /// leaves no skin
  pub async fn undo_card(&self, card: &acpira_shared::transcript::PluginCard, provenance: Provenance) -> Result<()> {
    let undo = card.undo.clone().unwrap_or_else(|| acpira_shared::transcript::PluginCardUndo { snapshot: None, skin: self.themed(card).then(String::new) });
    let change = self.store.restore(&card.plugin, undo.snapshot.as_deref(), undo.skin.as_deref(), provenance).await?;
    self.committed(&change);
    Ok(())
  }

  /// The interface pieces of the plugins in effect, in id order, each document read from the frozen copy (none in
  /// safe mode); a piece whose file is gone or too large is left out
  pub fn ui_slots(&self) -> Vec<acpira_shared::plugins::PluginSlot> {
    self.reload();
    let mut out = vec![];
    for a in &self.loaded().active {
      for u in &a.manifest.contributes.ui {
        let title = if u.title.trim().is_empty() { a.manifest.display_name().to_owned() } else { u.title.clone() };
        let piece = |html: String| acpira_shared::plugins::PluginSlot {
          plugin: a.id.clone(),
          id: u.id.clone(),
          slot: u.slot.clone(),
          title: title.clone(),
          html,
          width: u.width,
          height: u.height,
          method: u.method.clone(),
        };
        // A message action is a button: no document
        if u.method.is_some() {
          out.push(piece(String::new()));
          continue;
        }
        // A component `entry` is the compiled document (`build.rs`), built again if its output is gone
        let html = match u.entry.as_deref() {
          Some(entry) => {
            let out_dir = crate::plugins::build::output_dir(self.store.root(), &a.id, &a.hash);
            match crate::plugins::build::ui(&a.dir, entry, &u.id, &out_dir) {
              Ok(doc) => Some(doc),
              Err(e) => {
                (self.log)(&format!("plugin {}: ui {}: {e}", a.id, u.id));
                None
              }
            }
          }
          None => read_capped(&a.dir, &u.html, manifest::UI_HTML_MAX_BYTES),
        };
        let Some(html) = html else { continue };
        out.push(piece(html));
      }
    }
    out
  }

  /// The prompt folders of the plugins in effect, in id order: one more override layer of the built-in agent's system
  /// prompt, sent when a session opens (`acpira_agent::acp::LAYERS_META`)
  pub fn prompt_layers(&self) -> Vec<PathBuf> {
    self.reload();
    self
      .loaded()
      .active
      .iter()
      .filter_map(|a| a.manifest.contributes.prompts.as_deref().and_then(|p| manifest::inside(&a.dir, p)))
      .filter(|d| d.is_dir())
      .collect()
  }

  /// The agents of the plugins in effect as `agents.json` entries under the plugin's id, run from the frozen copy
  /// (`plugins::mcp::rewrite`); a built-in agent's id is never taken
  /// The agents the plugins in effect add, as agents.json entries; a specialised agent's carries its frozen copy under
  /// `PROFILE_KEY` instead of a command
  pub fn agent_entries(&self) -> serde_json::Map<String, Value> {
    self.reload();
    let builtin: Vec<String> = crate::acp::agents::registry::builtin_agents().into_iter().map(|d| d.id).collect();
    let mut out = serde_json::Map::new();
    for a in &self.loaded().active {
      let Some(spec) = a.manifest.contributes.agent.as_ref().and_then(Value::as_object) else { continue };
      if builtin.contains(&a.id) || a.id == crate::acp::agents::registry::SELF_AGENT_ID {
        continue;
      }
      // A specialised agent: the engine launches its own loop with the frozen copy as the profile (`runtime.rs`
      // `registry_of`), nothing of the plugin is run
      if spec.contains_key("builtin") {
        out.insert(a.id.clone(), json!({ "name": spec.get("name").cloned().unwrap_or_else(|| json!(a.manifest.display_name())), PROFILE_KEY: a.dir }));
        continue;
      }
      let data = self.store.data_dir(&a.id);
      let _ = std::fs::create_dir_all(&data);
      let mut entry = crate::plugins::mcp::rewrite(spec, &a.dir, &data);
      if !entry.contains_key("name") {
        entry.insert("name".into(), json!(a.manifest.display_name()));
      }
      out.insert(a.id.clone(), Value::Object(entry));
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
    self.check_dependencies(&manifest)?;
    // A runtime must start and answer `initialize` from the frozen copy before it is enabled
    if manifest.contributes.runtime.is_some() {
      let data_root = self.store.root().join(crate::plugins::store::DATA);
      crate::plugins::runtime::probe(id, hash, &dir, &manifest, &data_root, self.log.clone()).await.map_err(|e| anyhow!("the check failed: the runtime {e}"))?;
    }
    let change = self.store.enable_as(id, hash, provenance, manifest.contributes.theme.is_some()).await?;
    self.committed(&change);
    Ok(change)
  }

  /// Every plugin `manifest` requires is in effect at a version its range accepts. A ring of plugins requiring each
  /// other is refused by the commit itself (`PluginStore::guard`), against the state every engine shares
  fn check_dependencies(&self, manifest: &Manifest) -> Result<()> {
    let loaded = self.loaded();
    for (dep, range) in &manifest.dependencies {
      let Some(a) = loaded.active.iter().find(|a| a.id == *dep) else { bail!("it needs the plugin `{dep}` ({range}), which is not enabled") };
      if !manifest::version_meets(&a.manifest.version, range) {
        bail!("it needs the plugin `{dep}` {range}, but {} is enabled", a.manifest.version);
      }
    }
    Ok(())
  }

  /// Freeze, check and enable `id` in one go: what the plugin page's switch does (the switch is the confirmation)
  pub async fn activate(&self, id: &str, provenance: Provenance) -> Result<Option<Change>> {
    let mut retried = false;
    loop {
      let frozen = self.prepare(id)?;
      if !frozen.findings.ok() {
        bail!("{}", frozen.findings.text());
      }
      match self.confirm(id, &frozen.hash, provenance.clone()).await {
        // The storage page cleaned a reused copy between the freeze and the commit: freeze it again, once
        Err(e) if !retried && format!("{e:#}").contains(crate::plugins::store::GONE) => retried = true,
        done => return done,
      }
    }
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

  /// Copy the enabled version of every plugin the source of `id` depends on into its
  /// `node_modules/@acpira-plugin/<dep>/`, with a package.json whose `exports` is the dependency's `exports`, so
  /// `import { x } from '@acpira-plugin/<dep>'` resolves in the editor and compiles into the plugin (`acpira plugin
  /// vendor`). The copy is frozen with the plugin: it changes only when vendored again. Returns what was copied
  pub fn vendor(&self, id: &str) -> Result<Vec<String>> {
    if !manifest::valid_id(id) {
      bail!("`{id}` is not a plugin id");
    }
    let source = self.store.source(id);
    let (m, _) = manifest::read(&source).map_err(|e| anyhow!("{e}"))?;
    let loaded = self.loaded();
    let scope = source.join("node_modules").join(crate::plugins::build::VENDORED_SCOPE);
    // Copies are deleted and written below `scope`: a link on the way there could point them anywhere
    for dir in [source.join("node_modules"), scope.clone()] {
      if std::fs::symlink_metadata(&dir).is_ok_and(|m| m.file_type().is_symlink()) {
        bail!("{} is a link; vendoring writes only into real folders of the plugin", dir.display());
      }
    }
    let mut done = vec![];
    let deps = m.dependencies.iter().map(|d| (d, false)).chain(m.optional_dependencies.iter().map(|d| (d, true)));
    for ((dep, range), optional) in deps {
      if !manifest::valid_id(dep) {
        bail!("`{dep}` is not a plugin id");
      }
      let Some(a) = loaded.active.iter().find(|a| &a.id == dep) else {
        if optional {
          continue;
        }
        bail!("`{dep}` is not enabled: enable it first, then vendor again");
      };
      if !manifest::version_meets(&a.manifest.version, range) {
        bail!("`{dep}` {} is enabled, outside {range}", a.manifest.version);
      }
      let Some(exports) = a.manifest.exports.as_deref() else { bail!("`{dep}` shares no code: its plugin.json has no `exports`") };
      let to = scope.join(dep);
      if std::fs::symlink_metadata(&to).is_ok_and(|m| m.file_type().is_symlink()) {
        std::fs::remove_file(&to)?;
      } else if to.exists() {
        crate::plugins::store::make_writable(&to)?;
        std::fs::remove_dir_all(&to)?;
      }
      copy_tree(&a.dir, &to)?;
      crate::plugins::store::make_writable(&to)?;
      let pkg = json!({ "name": format!("{}/{dep}", crate::plugins::build::VENDORED_SCOPE), "version": a.manifest.version, "private": true, "type": "module", "exports": format!("./{}", exports.trim_start_matches("./")) });
      std::fs::write(to.join("package.json"), serde_json::to_vec_pretty(&pkg)?)?;
      done.push(format!("{dep} {}", a.manifest.version));
    }
    Ok(done)
  }

  /// One of the plugin page's actions (`Reveal` is the shell's; the source folder is returned for it)
  pub async fn action(&self, action: PluginAction) -> Result<Option<PathBuf>> {
    let user = Provenance::user();
    // Ids come from the page or the command line and some become paths: refuse anything that is not a plugin id
    let id = match &action {
      PluginAction::Enable { id } | PluginAction::Confirm { id, .. } | PluginAction::Dismiss { id } | PluginAction::Disable { id } => Some(id),
      PluginAction::Rollback { id } | PluginAction::Remove { id } | PluginAction::Reveal { id } => Some(id),
      PluginAction::Skin { id } => id.as_ref(),
      PluginAction::SafeMode { .. } | PluginAction::ToolGroup { .. } => None,
    };
    if let Some(id) = id.filter(|id| !manifest::valid_id(id)) {
      bail!("`{id}` is not a plugin id");
    }
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
      PluginAction::Skin { id } => {
        if let Some(id) = &id {
          self.reload();
          if !self.loaded().active.iter().any(|a| a.id == *id && a.manifest.contributes.theme.is_some()) {
            bail!("plugin {id} has no theme in effect");
          }
        }
        self.store.set_skin(id.as_deref(), user).await?
      }
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
      skin: loaded.skin().map(|a| a.id.clone()),
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
    if error.is_none() && active.is_some() {
      error = self.runtimes.error(id);
    }
    let tools = m.map(|m| m.contributes.tools.iter().map(|t| manifest::served_name(id, &t.name)).collect()).unwrap_or_default();
    let records = m.filter(|m| !m.contributes.schedules.is_empty()).map(|_| crate::plugins::schedule::Records::new(self.store.root()).read()).unwrap_or_default();
    let now = chrono::Local::now();
    let schedules = m.map(|m| m.contributes.schedules.iter().map(|d| crate::plugins::schedule::view(d, records.get(&crate::plugins::schedule::key(id, &d.id)), now)).collect()).unwrap_or_default();
    let dependencies = m.map(|m| dependency_view(m, &loaded.active)).unwrap_or_default();
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
      schedules,
      dependencies,
    }
  }
}

/// Each plugin `m` depends on, met when one in effect has a version its range accepts
fn dependency_view(m: &Manifest, active: &[Arc<Active>]) -> Vec<acpira_shared::plugins::PluginDependencyInfo> {
  let deps = m.dependencies.iter().map(|d| (d, false)).chain(m.optional_dependencies.iter().map(|d| (d, true)));
  deps
    .map(|((id, range), optional)| {
      let met = active.iter().find(|a| &a.id == id).is_some_and(|a| manifest::version_meets(&a.manifest.version, range));
      acpira_shared::plugins::PluginDependencyInfo { id: id.clone(), range: range.clone(), optional, met }
    })
    .collect()
}

fn copy_tree(from: &Path, to: &Path) -> std::io::Result<()> {
  std::fs::create_dir_all(to)?;
  for e in std::fs::read_dir(from)?.flatten() {
    let (src, dst) = (e.path(), to.join(e.file_name()));
    let kind = e.file_type()?;
    if kind.is_dir() {
      copy_tree(&src, &dst)?;
    } else if kind.is_symlink() {
      // A frozen copy holds no links; skip one rather than copy what it points at
      continue;
    } else {
      std::fs::copy(&src, &dst)?;
    }
  }
  Ok(())
}

fn provenance_view(p: &Provenance) -> PluginProvenance {
  PluginProvenance { by: p.by.clone(), session: p.session.clone(), agent: p.agent.clone(), model: p.model.clone() }
}

fn change_view(c: Change) -> PluginChange {
  PluginChange { generation: c.generation, at: c.at, id: c.id, action: c.action, snapshot: c.snapshot, on: c.on, provenance: provenance_view(&c.provenance) }
}

/// A file of a plugin's frozen copy as text, none when missing or over `max` bytes
fn theme_css_of(loaded: &Loaded) -> String {
  let mut out = String::new();
  if let Some(a) = loaded.skin()
    && let Some(theme) = &a.manifest.contributes.theme
  {
    out.push_str(&format!("/* Acpira plugin {} */\n", a.id));
    // Checked when it was enabled; checked again here, since this text goes into every page
    let tokens: Vec<String> = theme
      .tokens
      .iter()
      .filter(|(k, _)| manifest::THEME_TOKENS.contains(&k.as_str()))
      .filter_map(|(k, v)| v.as_str().filter(|v| manifest::safe_css_value(v)).map(|v| format!("  {k}: {v};")))
      .collect();
    if !tokens.is_empty() {
      out.push_str(&format!(":root, .acp-shell, [data-theme], .acp-shell.acp-shell[data-skin] {{\n{}\n}}\n", tokens.join("\n")));
    }
    if let Some(css) = theme.css.as_deref() {
      push_css(&mut out, &a.dir, css);
    }
  }
  // The styles stack over the skin, each plugin's in turn
  for a in loaded.active.iter() {
    if let Some(css) = a.manifest.contributes.style.as_deref() {
      out.push_str(&format!("/* Acpira plugin {} style */\n", a.id));
      push_css(&mut out, &a.dir, css);
    }
  }
  out
}

/// A plugin's stylesheet, cut at `THEME_CSS_MAX_BYTES`
fn push_css(out: &mut String, dir: &Path, rel: &str) {
  let Some(css) = manifest::inside(dir, rel).and_then(|p| std::fs::read_to_string(p).ok()) else { return };
  let mut end = css.len().min(THEME_CSS_MAX_BYTES);
  while !css.is_char_boundary(end) {
    end -= 1;
  }
  out.push_str(css.get(..end).unwrap_or_default());
  // Closes a comment the sheet (or the cut) left open, which would swallow the next plugin's sheet
  out.push_str("\n/* */\n");
}

fn skin_of(loaded: &Loaded) -> Option<acpira_shared::plugins::PluginSkin> {
  let a = loaded.skin()?;
  let theme = a.manifest.contributes.theme.as_ref()?;
  Some(acpira_shared::plugins::PluginSkin {
    plugin: a.id.clone(),
    name: a.manifest.display_name().to_owned(),
    snapshot: a.hash.clone(),
    base: theme.base.clone().filter(|b| manifest::THEME_BASES.contains(&b.as_str())),
    glass: theme.glass.clone().filter(|g| manifest::GLASS.contains(&g.as_str())),
    brand: theme.tokens.get("--brand").and_then(Value::as_str).filter(|v| manifest::safe_css_value(v)).map(str::to_owned),
    backdrop: backdrop_doc(&a.dir, theme),
  })

}

/// A theme's backdrop document: its own, or its scene inside the running Acpira's renderer
fn backdrop_doc(dir: &std::path::Path, theme: &manifest::Theme) -> Option<String> {
  match (&theme.scene, &theme.backdrop) {
    (Some(scene), _) => read_capped(dir, scene, manifest::SCENE_MAX_BYTES).map(|s| super::sdk::backdrop_html(&s)),
    (None, Some(b)) => read_capped(dir, b, manifest::BACKDROP_MAX_BYTES),
    (None, None) => None,
  }
}

fn read_capped(dir: &std::path::Path, rel: &str, max: u64) -> Option<String> {
  let path = manifest::inside(dir, rel)?;
  if std::fs::metadata(&path).ok()?.len() > max {
    return None;
  }
  std::fs::read_to_string(path).ok()
}

/// How a card presents a plugin and how much one click does: presentation (a theme, interface pieces in sandboxed
/// frames, a specialised agent of the engine's own loop) applies at once; anything that runs a program says what
fn card_kind(m: &manifest::Manifest) -> &'static str {
  let c = &m.contributes;
  // The self-test runs on apply too
  let runs = m.check.is_some()
    || !c.tools.is_empty()
    || c.mcp.as_ref().is_some_and(|m| !m.is_empty())
    || c.hooks.is_some()
    || c.runtime.is_some()
    || !c.providers.is_empty()
    || c.agent.as_ref().is_some_and(|a| a.get("builtin").is_none());
  if runs {
    "exec"
  } else if c.theme.is_some() {
    "skin"
  } else if !c.ui.is_empty() || c.style.is_some() || c.commands.iter().any(|x| x.button.is_some()) {
    "ui"
  } else if c.agent.is_some() {
    "agent"
  } else {
    "other"
  }
}

/// A frozen plugin as the card and the guide describe it: what it adds, its files and hash, the static check
pub fn describe(frozen: &Frozen) -> String {
  let m = &frozen.manifest;
  let mut lines = vec![format!("{} {} ({})", m.display_name(), m.version, frozen.id)];
  if !m.description.is_empty() {
    lines.push(m.description.clone());
  }
  for t in &m.contributes.tools {
    let by = if t.command.is_empty() { "answered by the runtime".to_owned() } else { t.command.join(" ") };
    lines.push(format!("tool {}: {by}", manifest::served_name(&m.id, &t.name)));
  }
  for k in m.contributes.kinds().into_iter().filter(|k| *k != "tools" && *k != "schedules") {
    lines.push(format!("contributes {k}"));
  }
  for j in &m.contributes.schedules {
    lines.push(format!("schedule {}: {} (runs unattended while an Acpira engine runs)", j.id, j.cron));
  }
  for (dep, range) in &m.dependencies {
    lines.push(format!("needs plugin {dep} {range}"));
  }
  for (dep, range) in &m.optional_dependencies {
    lines.push(format!("uses plugin {dep} {range} when enabled"));
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
  async fn an_agent_and_a_prompt_folder_come_from_the_frozen_copy() {
    let tmp = tempfile::tempdir().unwrap();
    let h = host(tmp.path());
    h.start().await;
    for id in ["pal", "claude"] {
      let dir = tmp.path().join(id);
      std::fs::create_dir_all(dir.join("prompts")).unwrap();
      std::fs::write(dir.join("prompts/base.md"), "## Style\n- Short.\n").unwrap();
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
    assert_eq!(h.loaded().groups_off(), ["summon"]);
    assert!(h.action(PluginAction::ToolGroup { id: "nope".into(), on: false }).await.is_err());
  }
}
