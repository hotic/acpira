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

use acpira_shared::plugins::{PluginAction, PluginChange, PluginFindings, PluginInfo, PluginOverlap, PluginProposal, PluginProvenance, PluginsView, ToolGroupInfo};

use crate::platform::paths::wire_path;
use crate::plugins::manifest::{self, Findings, Manifest, ToolDef};
use crate::plugins::run::{self, CallContext};
use crate::plugins::store::{Change, Frozen, PluginStore, Provenance, State};
use crate::store::transcript_store::LogFn;

mod overlaps;
mod theme;

use overlaps::compute_overlaps;
use theme::{backdrop_doc, read_capped, restyled_hooks, skin_of, theme_css_of};

/// Longest a plugin's `check` may run once the user confirmed
const CHECK_TIMEOUT: Duration = Duration::from_secs(45);
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
    acpira_shared::host_tools::GROUPS.iter().map(|(g, _)| *g).filter(|g| !self.state.tool_group_on(g)).map(str::to_owned).collect()
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
    runtimes.stage(&active);
    handle.spawn(async move { runtimes.apply().await });
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

  /// A card was offered: send the pages' views again. Offering commits nothing and source folders are never watched,
  /// so a page holding a view from before the plugin's folder existed would read the new card as removed
  pub fn offered(&self) {
    self.notify();
  }

  /// At start: the log line a crash left out, then the first load
  pub async fn start(&self) {
    if let Err(e) = self.store.reconcile().await {
      (self.log)(&format!("plugins: {e:#}"));
    }
    // The SDK copy plugins written by hand take theirs from, and the skill the built-in agent ships (`sdk.rs`)
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
      // Load enabled plugins in the saved tile order, with any unlisted plugin following in id order
      let mut enabled_ids: BTreeSet<String> = state.plugins.iter().filter(|(_, e)| e.enabled).map(|(id, _)| id.clone()).collect();
      let mut load_ids = Vec::with_capacity(enabled_ids.len());
      for id in &state.order {
        if enabled_ids.remove(id) {
          load_ids.push(id.clone());
        }
      }
      load_ids.extend(enabled_ids);
      for id in load_ids {
        let Some(e) = state.plugins.get(&id) else { continue };
        match self.store.open(&id, &e.snapshot) {
          Ok((dir, manifest)) => active.push(Arc::new(Active { id, hash: e.snapshot.clone(), dir, manifest })),
          Err(err) => {
            (self.log)(&format!("plugins: {err:#}"));
            errors.insert(id, format!("{err:#}"));
          }
        }
      }
    }
    let next = Loaded { state, safe_mode, active, errors };
    let (same, topology_same) = {
      let cur = self.loaded.read();
      let same = cur.state == next.state && cur.safe_mode == next.safe_mode && cur.errors == next.errors;
      // Reordering changes dispatch order and contributed precedence without restarting running plugin processes
      let topology_same = cur.safe_mode == next.safe_mode
        && cur.errors == next.errors
        && cur.state.safe_mode == next.state.safe_mode
        && cur.state.plugins == next.state.plugins
        && cur.state.proposals == next.state.proposals
        && cur.state.tool_groups == next.state.tool_groups
        && cur.state.skin == next.state.skin;
      (same, topology_same)
    };
    let next = Arc::new(next);
    *self.loaded.write() = next.clone();
    if !same {
      self.runtimes.set_order(next.active.iter().map(|a| a.id.clone()).collect());
      self.runtimes.set_contributed(crate::plugins::runtime::Contributed::of(&next.active));
      if !topology_same {
        self.sync_runtimes();
      }
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
      adds: c.kinds().into_iter().map(str::to_owned).collect(),
      restyles: c.style.as_deref().and_then(|rel| manifest::inside(&frozen.dir, rel)).and_then(|p| std::fs::read_to_string(p).ok()).map(|css| restyled_hooks(&css)).unwrap_or_default(),
      folder: wire_path(&self.store.source(&frozen.id)),
      at: crate::util::now_ms(),
      undo: None,
      after: None,
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
      // A well-known read-only method: older hosts accept the manifest and just omit its account rows.
      if a.manifest.contributes.runtime.as_ref().is_some_and(|r| r.methods.iter().any(|m| m == "account.rows")) {
        out.push(acpira_shared::plugins::PluginSlot {
          plugin: a.id.clone(),
          id: "account.rows".into(),
          slot: "account.rows".into(),
          title: a.manifest.display_name().to_owned(),
          snapshot: None,
          html: String::new(),
          width: None,
          height: None,
          method: Some("account.rows".into()),
        });
      }
      for u in &a.manifest.contributes.ui {
        let title = if u.title.trim().is_empty() { a.manifest.display_name().to_owned() } else { u.title.clone() };
        let piece = |html: String| acpira_shared::plugins::PluginSlot {
          plugin: a.id.clone(),
          id: u.id.clone(),
          slot: u.slot.clone(),
          title: title.clone(),
          snapshot: (u.slot == "page").then(|| a.hash.clone()),
          html,
          width: u.width,
          height: u.height,
          method: u.method.clone(),
        };
        // A message or selection action is a button: no document
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

  /// The prompt folders of the plugins in effect, in reverse load order: `Places::prompt_dirs` lets later layers
  /// override earlier ones, so plugins earlier in the user's tile order win
  pub fn prompt_layers(&self) -> Vec<PathBuf> {
    self.reload();
    self
      .loaded()
      .active
      .iter()
      .rev()
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
    let manifest = self.check_activation(id, hash).await?;
    let change = self.store.enable_as(id, hash, provenance, manifest.contributes.theme.is_some()).await?;
    self.committed(&change);
    Ok(change)
  }

  async fn check_activation(&self, id: &str, hash: &str) -> Result<Manifest> {
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
    Ok(manifest)
  }

  /// Reload the settings page's enabled snapshot from its local source after a user gesture.
  pub(crate) async fn reload_plugin(&self, id: &str, snapshot: &str) -> Result<bool> {
    let state = self.store.load()?;
    let Some(expected) = state.plugins.get(id).filter(|e| e.enabled && e.snapshot == snapshot) else {
      bail!("the enabled plugin version changed; reopen its settings and try again");
    };
    self.reload();
    if self.forced_safe
      || state.safe_mode
      || !self
        .loaded()
        .active
        .iter()
        .any(|a| a.id == id && a.hash == snapshot && a.manifest.contributes.ui.iter().any(|u| u.slot == "page"))
    {
      bail!("this plugin has no active settings page");
    }

    let store = PluginStore::new(self.store.root().to_path_buf());
    let id_owned = id.to_owned();
    let frozen = tokio::task::spawn_blocking(move || store.freeze(&id_owned, &on_path)).await??;
    if !frozen.findings.ok() {
      bail!("{}", frozen.findings.text());
    }
    if frozen.hash == snapshot {
      return Ok(false);
    }
    self.check_activation(id, &frozen.hash).await?;
    let change = self.store.replace_enabled(id, &frozen.hash, expected).await?;
    self.committed(&change);
    if frozen.manifest.contributes.runtime.is_some() {
      self.runtimes.wait_for_snapshot(id, &frozen.hash).await.map_err(anyhow::Error::msg)?;
    }
    Ok(true)
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

  /// One of the plugin page's actions (`Reveal` returns the folder for the shell to show)
  pub async fn action(&self, action: PluginAction) -> Result<Option<PathBuf>> {
    let user = Provenance::user();
    // Ids come from the page or the command line and some become paths: refuse anything that is not a plugin id
    let id = match &action {
      PluginAction::Enable { id } | PluginAction::Confirm { id, .. } | PluginAction::Dismiss { id } | PluginAction::Disable { id } => Some(id),
      PluginAction::Rollback { id } | PluginAction::Remove { id } | PluginAction::Reveal { id } => Some(id),
      PluginAction::Skin { id } => id.as_ref(),
      PluginAction::SafeMode { .. } | PluginAction::ToolGroup { .. } | PluginAction::Reorder { .. } => None,
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
        if dir.is_dir() {
          return Ok(Some(dir));
        }
        // A card can re-enable its frozen copy after removal retired the editable source.
        self.reload();
        if let Some(active) = self.loaded().active.iter().find(|a| a.id == id && a.dir.is_dir()) {
          return Ok(Some(active.dir.clone()));
        }
        bail!("no plugin folder {}", dir.display());
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
      PluginAction::Reorder { ids } => {
        for id in &ids {
          if !valid_order_id(id) {
            bail!("`{id}` is not a plugin id");
          }
        }
        if self.store.set_order(ids).await? {
          *self.stamp.lock() = None;
          self.reload();
          if let Some(f) = self.touch.read().clone() {
            f();
          }
        }
        return Ok(None);
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
    let mut ordered = Vec::with_capacity(ids.len());
    for id in &state.order {
      if ids.remove(id) {
        ordered.push(id.clone());
      }
    }
    ordered.extend(ids);
    let overlaps = compute_overlaps(&loaded.active);
    let plugins = ordered.iter().map(|id| self.info(id, &loaded, overlaps.get(id).cloned().unwrap_or_default())).collect();
    let tool_groups = acpira_shared::host_tools::GROUPS
      .iter()
      // Computer use is offered only where this engine can drive apps
      .filter(|(g, _)| *g != acpira_shared::host_tools::COMPUTER_GROUP || acpira_computer::SUPPORTED)
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
      order: state.order.clone(),
    }
  }

  fn info(&self, id: &str, loaded: &Loaded, overlaps: Vec<PluginOverlap>) -> PluginInfo {
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
      overlaps,
    }
  }
}

/// Display-order entries are either user plugin ids or `builtin:<tool-group>` keys
fn valid_order_id(id: &str) -> bool {
  manifest::valid_id(id)
    || id
      .strip_prefix(acpira_shared::plugins::BUILTIN_ORDER_PREFIX)
      .is_some_and(|g| acpira_shared::host_tools::GROUPS.iter().any(|(x, _)| *x == g))
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

impl crate::acp::session::ext::PluginCardPort for PluginHost {
  fn prepare_offer(self: Arc<Self>, id: String, card_id: String) -> crate::acp::transport::rpc::BoxFuture<crate::acp::session::ext::PluginOfferOutcome> {
    Box::pin(async move {
      use crate::acp::session::ext::PluginOfferOutcome;
      let prepared = {
        let (host, id) = (self.clone(), id.clone());
        tokio::task::spawn_blocking(move || host.prepare(&id)).await
      };
      let frozen = match prepared {
        Ok(Ok(f)) => f,
        Ok(Err(e)) => return PluginOfferOutcome::Error(format!("{e:#}")),
        Err(e) => return PluginOfferOutcome::Error(format!("the check did not finish: {e}")),
      };
      if !frozen.findings.ok() {
        return PluginOfferOutcome::Error(format!(
          "The plugin was not offered to the user. Fix these in {} and call plugin_activate again:\n{}",
          self.store().source(&id).display(),
          frozen.findings.text()
        ));
      }
      // In effect already: enabled at this version, and the skin when it is a theme
      let loaded = self.loaded();
      let enabled = loaded.state.plugins.get(&id).is_some_and(|e| e.enabled && e.snapshot == frozen.hash);
      let skinned = frozen.manifest.contributes.theme.is_none() || loaded.skin().is_some_and(|a| a.id == id);
      if enabled && skinned {
        return PluginOfferOutcome::AlreadyActive(format!("This version of {id} is in effect already."));
      }
      PluginOfferOutcome::Offer(Box::new(self.card(&frozen, card_id)))
    })
  }

  fn offered(&self) {
    PluginHost::offered(self);
  }

  fn apply_card<'a>(
    &'a self,
    card: &'a acpira_shared::transcript::PluginCard,
    provenance: Provenance,
  ) -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<Option<acpira_shared::transcript::PluginCardUndo>>> + Send + 'a>> {
    Box::pin(PluginHost::apply_card(self, card, provenance))
  }

  fn undo_card<'a>(
    &'a self,
    card: &'a acpira_shared::transcript::PluginCard,
    provenance: Provenance,
  ) -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<()>> + Send + 'a>> {
    Box::pin(PluginHost::undo_card(self, card, provenance))
  }
}

#[cfg(test)]
#[cfg(unix)]
mod tests;
