//! `~/.acpira/plugins/`: the source folders, their frozen copies and `state.json`, the single source of truth for what
//! is enabled (`docs/dev/plugins.md`). Every change is a commit under the state file's lock: the generation goes up,
//! the change is kept as the state's `last`, and `changes.jsonl` gets its line afterwards. A crash between the two
//! writes leaves the line missing, which the next commit (or `reconcile` at start) appends

use std::collections::{BTreeMap, BTreeSet};
use std::io::Write as _;
use std::path::{Path, PathBuf};

use anyhow::{Context as _, Result, anyhow, bail};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

use crate::plugins::manifest::{self, Findings, Manifest};
use crate::store::file_lock::{with_file_lock, write_atomic};
use crate::util::now_ms;

pub const STATE: &str = "state.json";
/// The start of the error a commit gives when the frozen copy went meanwhile (the storage page's clean): freezing again
/// and retrying is safe
pub const GONE: &str = "the frozen copy was removed meanwhile";
pub const CHANGES: &str = "changes.jsonl";
pub const VERSIONS: &str = ".versions";
pub const DATA: &str = ".data";
pub const REMOVED: &str = ".removed";
pub const STATE_SCHEMA: u32 = 1;
/// Earlier snapshots kept per plugin for rollback
const HISTORY: usize = 10;
/// Freezing stops past these: a plugin is a script and a manifest, not a checkout
pub const MAX_FILES: usize = 500;
pub const MAX_BYTES: u64 = 8 << 20;
/// What `node_modules` may add on top: npm packages the author installed are frozen with the plugin
pub const DEP_MAX_FILES: usize = 30_000;
pub const DEP_MAX_BYTES: u64 = 256 << 20;
/// Files of `node_modules` stored once by content and linked into every snapshot holding them
pub const BLOBS: &str = ".blobs";
/// Names never frozen, at any depth: version control, caches a run leaves, Python environments (links to the
/// interpreter)
const SKIPPED: &[&str] = &[".git", "__pycache__", ".DS_Store", ".venv"];
/// Inside `node_modules`: npm's links to package binaries and the tools' own caches
const SKIPPED_IN_DEPS: &[&str] = &[".bin", ".cache", ".vite"];
const SKIPPED_SUFFIXES: &[&str] = &[".pyc", ".tmp"];

/// Who made a change: the user on a page, a session's agent through its card, or the command line
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Provenance {
  /// `user`, `session` or `cli`
  pub by: String,
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub session: Option<String>,
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub agent: Option<String>,
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub model: Option<String>,
}

impl Provenance {
  pub fn user() -> Provenance {
    Provenance { by: "user".into(), ..Default::default() }
  }

  pub fn cli() -> Provenance {
    Provenance { by: "cli".into(), ..Default::default() }
  }
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Entry {
  pub enabled: bool,
  /// The frozen copy in use, `.versions/<id>/<snapshot>`
  pub snapshot: String,
  /// Earlier snapshots, newest last
  #[serde(default, skip_serializing_if = "Vec::is_empty")]
  pub previous: Vec<String>,
  #[serde(default)]
  pub provenance: Provenance,
  #[serde(default)]
  pub committed_at: i64,
  #[serde(flatten)]
  pub extra: Map<String, Value>,
}

/// A frozen version waiting for the user (`acpira plugin propose`, a card nobody answered yet)
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Proposal {
  pub snapshot: String,
  #[serde(default)]
  pub provenance: Provenance,
  #[serde(default)]
  pub at: i64,
  #[serde(flatten)]
  pub extra: Map<String, Value>,
}

/// One committed change, as `changes.jsonl` and the state's `last` keep it
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Change {
  pub generation: u64,
  pub at: i64,
  /// The plugin, or the tool group / `safeMode` for the switches of the host itself
  pub id: String,
  /// `enable`, `disable`, `rollback`, `remove`, `propose`, `safeMode`, `toolGroup`
  pub action: String,
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub snapshot: Option<String>,
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub on: Option<bool>,
  #[serde(default)]
  pub provenance: Provenance,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct State {
  #[serde(default)]
  pub schema: u32,
  #[serde(default)]
  pub generation: u64,
  #[serde(default)]
  pub safe_mode: bool,
  #[serde(default)]
  pub plugins: BTreeMap<String, Entry>,
  #[serde(default)]
  pub proposals: BTreeMap<String, Proposal>,
  /// Built-in tool groups the user switched off (`false`); a group absent here is on
  #[serde(default)]
  pub tool_groups: BTreeMap<String, bool>,
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub last: Option<Change>,
  /// The plugin whose theme is the skin, one at a time; `""` for none. Absent in a state no engine with skins wrote:
  /// the theme enabled last is the skin then (`PluginHost::skin_id`)
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub skin: Option<String>,
  /// Fields of a newer engine, written back as they were
  #[serde(flatten)]
  pub extra: Map<String, Value>,
}

impl State {
  pub fn tool_group_on(&self, group: &str) -> bool {
    self.tool_groups.get(group).copied().unwrap_or(true)
  }
}

/// A source folder frozen into a content-addressed copy
#[derive(Debug, Clone)]
pub struct Frozen {
  pub id: String,
  pub hash: String,
  pub dir: PathBuf,
  pub manifest: Manifest,
  /// Relative paths of the frozen files, in path order
  pub files: Vec<String>,
  pub findings: Findings,
}

pub struct PluginStore {
  root: PathBuf,
}

impl PluginStore {
  /// `root` is `~/.acpira/plugins`
  pub fn new(root: PathBuf) -> PluginStore {
    PluginStore { root }
  }

  pub fn root(&self) -> &Path {
    &self.root
  }

  pub fn state_path(&self) -> PathBuf {
    self.root.join(STATE)
  }

  pub fn source(&self, id: &str) -> PathBuf {
    self.root.join(id)
  }

  pub fn snapshot_dir(&self, id: &str, hash: &str) -> PathBuf {
    self.root.join(VERSIONS).join(id).join(hash)
  }

  pub fn data_dir(&self, id: &str) -> PathBuf {
    self.root.join(DATA).join(id)
  }

  /// The source folders present (names that are valid ids), sorted
  pub fn sources(&self) -> Vec<String> {
    let Ok(rd) = std::fs::read_dir(&self.root) else { return vec![] };
    let mut ids: Vec<String> = rd
      .flatten()
      .filter(|e| e.file_type().is_ok_and(|t| t.is_dir()))
      .filter_map(|e| e.file_name().to_str().map(str::to_owned))
      .filter(|n| manifest::valid_id(n))
      .collect();
    ids.sort();
    ids
  }

  /// The state as on disk; a missing file is the empty state, an unreadable one an error (never treated as empty, which
  /// a commit would then write over)
  pub fn load(&self) -> Result<State> {
    read_state(&self.state_path())
  }

  /// Apply `change` to the state under its lock and commit it: generation + 1, `last` set, the log appended. `change`
  /// returns the record of what it did, or None for no change (nothing is written then)
  pub async fn commit(&self, change: impl FnOnce(&mut State) -> Result<Option<Change>>) -> Result<Option<Change>> {
    std::fs::create_dir_all(&self.root).with_context(|| format!("create {}", self.root.display()))?;
    let path = self.state_path();
    let log = self.root.join(CHANGES);
    let at = path.clone();
    with_file_lock(&at, move || async move {
      let mut state = read_state(&path)?;
      append_missing(&log, state.last.as_ref())?;
      let Some(mut record) = change(&mut state)? else { return Ok(None) };
      state.schema = STATE_SCHEMA;
      state.generation += 1;
      record.generation = state.generation;
      record.at = now_ms();
      state.last = Some(record.clone());
      let mut bytes = serde_json::to_vec_pretty(&state)?;
      bytes.push(b'\n');
      write_atomic(&path, &bytes, None).await?;
      append_missing(&log, Some(&record))?;
      Ok(Some(record))
    })
    .await
  }

  /// At start: the log line a crash after the last commit left out
  pub async fn reconcile(&self) -> Result<()> {
    let path = self.state_path();
    if !path.exists() {
      return Ok(());
    }
    let log = self.root.join(CHANGES);
    let at = path.clone();
    with_file_lock(&at, move || async move {
      let state = read_state(&path)?;
      append_missing(&log, state.last.as_ref())
    })
    .await
  }

  /// The newest `n` changes, newest last
  pub fn changes(&self, n: usize) -> Vec<Change> {
    let Ok(text) = std::fs::read_to_string(self.root.join(CHANGES)) else { return vec![] };
    let mut out: Vec<Change> = text.lines().rev().filter_map(|l| serde_json::from_str(l).ok()).take(n).collect();
    out.reverse();
    out
  }

  /// Freeze the source folder of `id`: hash its files, copy them to `.versions/<id>/<hash>/` read-only (an existing
  /// copy of the same hash is reused) and run the static check on the copy. Errors in the manifest still freeze, so
  /// the caller can report them; a folder that cannot be read or is too large does not
  pub fn freeze(&self, id: &str, which: &dyn Fn(&str) -> bool) -> Result<Frozen> {
    if !manifest::valid_id(id) {
      bail!("`{id}` is not a plugin id (lowercase letters, digits and `-`)");
    }
    let source = self.source(id);
    if !source.is_dir() {
      bail!("no plugin folder {}", source.display());
    }
    let files = list_files(&source)?;
    let hash = digest(&source, &files)?;
    let dir = self.snapshot_dir(id, &hash);
    if dir.is_dir() && verify(&dir, &hash).is_ok() {
      // Reused: the storage page reads the folder's time to leave a freeze under way alone. Best effort (Windows
      // cannot open a folder to set its time)
      let _ = std::fs::File::open(&dir).and_then(|f| f.set_modified(std::time::SystemTime::now()));
    } else {
      if dir.exists() {
        // A copy that no longer matches its name: put aside, never loaded (its folders writable; its files may be
        // links into the shared store, which keep their mode)
        let _ = make_removable(&dir);
        let aside = dir.with_extension(format!("bad-{}", crate::util::random_hex(4)));
        std::fs::rename(&dir, &aside)?;
      }
      let parent = dir.parent().ok_or_else(|| anyhow!("snapshot path"))?;
      std::fs::create_dir_all(parent)?;
      let tmp = parent.join(format!(".tmp-{}", crate::util::random_hex(8)));
      copy_files(&source, &tmp, &files, &self.root.join(BLOBS))?;
      make_read_only(&tmp)?;
      if let Err(e) = std::fs::rename(&tmp, &dir) {
        // Another engine froze the same files first
        let _ = make_removable(&tmp);
        let _ = std::fs::remove_dir_all(&tmp);
        if !dir.is_dir() {
          return Err(e.into());
        }
      }
    }
    let (manifest, mut findings) = manifest::read(&dir).map_err(|e| anyhow!(e))?;
    manifest::check(&manifest, id, &dir, which, &mut findings);
    // TypeScript / TSX sources compile now, from the frozen copy: a failure is an error of the check, and the output is
    // what the engine loads once the user confirms. Compiling only rewrites text; nothing of the plugin runs
    findings.errors.extend(crate::plugins::build::check(&dir, &manifest, &crate::plugins::build::output_dir(&self.root, id, &hash)));
    Ok(Frozen { id: id.to_owned(), hash, dir, manifest, files, findings })
  }

  /// The frozen copy `hash` of `id`, checked against its name: the manifest, or why it cannot be loaded
  pub fn open(&self, id: &str, hash: &str) -> Result<(PathBuf, Manifest)> {
    let dir = self.snapshot_dir(id, hash);
    verify(&dir, hash).with_context(|| format!("plugin {id}"))?;
    let (manifest, _) = manifest::read(&dir).map_err(|e| anyhow!("plugin {id}: {e}"))?;
    Ok((dir, manifest))
  }

  /// Whether the source folder differs from the snapshot in use (`None`: no source folder or nothing enabled)
  pub fn source_changed(&self, id: &str, snapshot: &str) -> Option<bool> {
    let source = self.source(id);
    let files = list_files(&source).ok()?;
    Some(digest(&source, &files).ok()? != snapshot)
  }

  /// Move the source folder of `id` to `.removed/<id>.<stamp>`; returns where it went
  pub fn retire_source(&self, id: &str) -> Result<Option<PathBuf>> {
    let source = self.source(id);
    if !source.exists() {
      return Ok(None);
    }
    let removed = self.root.join(REMOVED);
    std::fs::create_dir_all(&removed)?;
    let to = removed.join(format!("{id}.{}", now_ms()));
    std::fs::rename(&source, &to)?;
    Ok(Some(to))
  }

  // The commits the host and the command line make

  /// Point `id` at `hash` and enable it; the snapshot it had goes to its history
  pub async fn enable(&self, id: &str, hash: &str, provenance: Provenance) -> Result<Option<Change>> {
    self.enable_as(id, hash, provenance, false).await
  }

  /// `enable`, and with `skin` make the plugin's theme the skin in effect (in the same commit)
  pub async fn enable_as(&self, id: &str, hash: &str, provenance: Provenance, skin: bool) -> Result<Option<Change>> {
    let data = self.data_dir(id);
    std::fs::create_dir_all(&data).with_context(|| format!("create {}", data.display()))?;
    self
      .commit(|s| {
        let reskin = skin && s.skin.as_deref() != Some(id);
        if reskin {
          s.skin = Some(id.to_owned());
        }
        if s.plugins.get(id).is_some_and(|e| e.enabled && e.snapshot == hash) {
          s.proposals.remove(id);
          return Ok(reskin.then(|| Change { id: id.to_owned(), action: "skin".into(), snapshot: Some(hash.to_owned()), provenance, ..Default::default() }));
        }
        self.guard(s, id, hash)?;
        let entry = s.plugins.entry(id.to_owned()).or_default();
        if !entry.snapshot.is_empty() && entry.snapshot != hash {
          let old = std::mem::take(&mut entry.snapshot);
          entry.previous.retain(|p| *p != old && p != hash);
          entry.previous.push(old);
          let over = entry.previous.len().saturating_sub(HISTORY);
          entry.previous.drain(..over);
        }
        entry.previous.retain(|p| p != hash);
        entry.snapshot = hash.to_owned();
        entry.enabled = true;
        entry.provenance = provenance.clone();
        entry.committed_at = now_ms();
        s.proposals.remove(id);
        Ok(Some(Change { id: id.to_owned(), action: "enable".into(), snapshot: Some(hash.to_owned()), provenance, ..Default::default() }))
      })
      .await
  }

  pub async fn disable(&self, id: &str, provenance: Provenance) -> Result<Option<Change>> {
    self
      .commit(|s| {
        let Some(entry) = s.plugins.get_mut(id).filter(|e| e.enabled) else { return Ok(None) };
        entry.enabled = false;
        entry.provenance = provenance.clone();
        Ok(Some(Change { id: id.to_owned(), action: "disable".into(), snapshot: Some(entry.snapshot.clone()), provenance, ..Default::default() }))
      })
      .await
  }

  /// Put `id` back as a plugin card found it before applying (`PluginCardUndo`): the version enabled then, or off;
  /// and the skin of then, when the card was a skin's
  pub async fn restore(&self, id: &str, snapshot: Option<&str>, skin: Option<&str>, provenance: Provenance) -> Result<Option<Change>> {
    self
      .commit(|s| {
        let mut changed = false;
        if let Some(skin) = skin
          && s.skin.as_deref() != Some(skin)
        {
          s.skin = Some(skin.to_owned());
          changed = true;
        }
        if let Some(entry) = s.plugins.get_mut(id) {
          match snapshot {
            Some(back) if !(entry.enabled && entry.snapshot == back) => {
              let now = std::mem::replace(&mut entry.snapshot, back.to_owned());
              entry.previous.retain(|p| p != back && *p != now);
              if !now.is_empty() {
                entry.previous.push(now);
              }
              entry.enabled = true;
              entry.committed_at = now_ms();
              changed = true;
            }
            None if entry.enabled => {
              entry.enabled = false;
              changed = true;
            }
            _ => {}
          }
          if changed {
            entry.provenance = provenance.clone();
          }
        }
        Ok(changed.then(|| Change { id: id.to_owned(), action: "undo".into(), snapshot: snapshot.map(str::to_owned), provenance, ..Default::default() }))
      })
      .await
  }

  /// Make the theme of `id` the skin, or (`None`) show none; the caller checks `id` is a theme in effect
  pub async fn set_skin(&self, id: Option<&str>, provenance: Provenance) -> Result<Option<Change>> {
    self
      .commit(|s| {
        // `""` is an explicit none: an absent pick falls back to the theme enabled last
        let pick = id.unwrap_or("");
        if s.skin.as_deref() == Some(pick) {
          return Ok(None);
        }
        s.skin = Some(pick.to_owned());
        let snapshot = id.and_then(|id| s.plugins.get(id)).map(|e| e.snapshot.clone());
        Ok(Some(Change { id: id.unwrap_or_default().to_owned(), action: "skin".into(), snapshot, provenance, ..Default::default() }))
      })
      .await
  }

  /// Back to the snapshot before the current one (enabled)
  pub async fn rollback(&self, id: &str, provenance: Provenance) -> Result<Option<Change>> {
    self
      .commit(|s| {
        let entry = s.plugins.get(id).ok_or_else(|| anyhow!("plugin {id} was never enabled"))?;
        let Some(back) = entry.previous.last().cloned() else { bail!("plugin {id} has no earlier version") };
        self.guard(s, id, &back)?;
        let entry = s.plugins.get_mut(id).ok_or_else(|| anyhow!("plugin {id} was never enabled"))?;
        entry.previous.pop();
        entry.snapshot = back.clone();
        entry.enabled = true;
        entry.provenance = provenance.clone();
        entry.committed_at = now_ms();
        Ok(Some(Change { id: id.to_owned(), action: "rollback".into(), snapshot: Some(back), provenance, ..Default::default() }))
      })
      .await
  }

  /// Checked inside a commit, so against what every engine committed and with no clean running: the frozen copy is
  /// still there (the storage page removes versions under this lock), and enabling it closes no ring of plugins
  /// requiring each other (optional dependencies count: once both are in effect they call each other the same way)
  fn guard(&self, s: &State, id: &str, hash: &str) -> Result<()> {
    let dir = self.snapshot_dir(id, hash);
    if !dir.is_dir() {
      bail!("{GONE}: {}", dir.display());
    }
    let requires = |dir: &Path| {
      manifest::read(dir).map(|(m, _)| m.dependencies.into_keys().chain(m.optional_dependencies.into_keys()).collect::<Vec<_>>()).unwrap_or_default()
    };
    let mut seen = BTreeSet::new();
    let mut stack: Vec<(String, Vec<String>)> = requires(&dir).into_iter().map(|d| (d.clone(), vec![id.to_owned(), d])).collect();
    while let Some((dep, path)) = stack.pop() {
      if dep == id {
        bail!("its dependencies form a ring: {}", path.join(" -> "));
      }
      if !seen.insert(dep.clone()) {
        continue;
      }
      let Some(e) = s.plugins.get(&dep).filter(|e| e.enabled && !e.snapshot.is_empty()) else { continue };
      for next in requires(&self.snapshot_dir(&dep, &e.snapshot)) {
        let mut p = path.clone();
        p.push(next.clone());
        stack.push((next, p));
      }
    }
    Ok(())
  }

  /// Drop `id` from the state and move its source folder aside
  pub async fn remove(&self, id: &str, provenance: Provenance) -> Result<Option<Change>> {
    // The id becomes a path below: never let it leave the plugins folder
    if !manifest::valid_id(id) {
      bail!("`{id}` is not a plugin id");
    }
    let change = self
      .commit(|s| {
        let had = s.plugins.remove(id).is_some() | s.proposals.remove(id).is_some();
        let source = self.source(id).exists();
        Ok((had || source).then(|| Change { id: id.to_owned(), action: "remove".into(), provenance, ..Default::default() }))
      })
      .await?;
    self.retire_source(id)?;
    Ok(change)
  }

  /// File a frozen version for the user to confirm on the plugin page
  pub async fn propose(&self, id: &str, hash: &str, provenance: Provenance) -> Result<Option<Change>> {
    self
      .commit(|s| {
        if s.plugins.get(id).is_some_and(|e| e.enabled && e.snapshot == hash) || s.proposals.get(id).is_some_and(|p| p.snapshot == hash) {
          return Ok(None);
        }
        s.proposals.insert(id.to_owned(), Proposal { snapshot: hash.to_owned(), provenance: provenance.clone(), at: now_ms(), extra: Map::new() });
        Ok(Some(Change { id: id.to_owned(), action: "propose".into(), snapshot: Some(hash.to_owned()), provenance, ..Default::default() }))
      })
      .await
  }

  /// Turn down a proposal
  pub async fn dismiss(&self, id: &str, provenance: Provenance) -> Result<Option<Change>> {
    self
      .commit(|s| {
        let Some(p) = s.proposals.remove(id) else { return Ok(None) };
        Ok(Some(Change { id: id.to_owned(), action: "dismiss".into(), snapshot: Some(p.snapshot), provenance, ..Default::default() }))
      })
      .await
  }

  pub async fn set_safe_mode(&self, on: bool, provenance: Provenance) -> Result<Option<Change>> {
    self
      .commit(|s| {
        if s.safe_mode == on {
          return Ok(None);
        }
        s.safe_mode = on;
        Ok(Some(Change { id: "safeMode".into(), action: "safeMode".into(), on: Some(on), provenance, ..Default::default() }))
      })
      .await
  }

  pub async fn set_tool_group(&self, group: &str, on: bool, provenance: Provenance) -> Result<Option<Change>> {
    self
      .commit(|s| {
        if s.tool_group_on(group) == on {
          return Ok(None);
        }
        if on {
          s.tool_groups.remove(group);
        } else {
          s.tool_groups.insert(group.to_owned(), false);
        }
        Ok(Some(Change { id: group.to_owned(), action: "toolGroup".into(), on: Some(on), provenance, ..Default::default() }))
      })
      .await
  }
}

fn read_state(path: &Path) -> Result<State> {
  match std::fs::read(path) {
    Ok(bytes) => serde_json::from_slice(&bytes).with_context(|| format!("{} is not valid", path.display())),
    Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(State::default()),
    Err(e) => Err(e).with_context(|| format!("read {}", path.display())),
  }
}

/// Append `record` to the log unless the log already holds its generation (or a later one)
fn append_missing(log: &Path, record: Option<&Change>) -> Result<()> {
  let Some(record) = record else { return Ok(()) };
  let logged = last_logged(log);
  if logged.is_some_and(|g| g >= record.generation) {
    return Ok(());
  }
  let mut f = std::fs::OpenOptions::new().create(true).append(true).open(log).with_context(|| format!("open {}", log.display()))?;
  let mut line = serde_json::to_vec(record)?;
  line.push(b'\n');
  f.write_all(&line)?;
  Ok(())
}

/// The generation of the log's last readable line
fn last_logged(log: &Path) -> Option<u64> {
  let text = std::fs::read_to_string(log).ok()?;
  text.lines().rev().find_map(|l| serde_json::from_str::<Change>(l).ok()).map(|c| c.generation)
}

fn skipped(name: &str) -> bool {
  SKIPPED.contains(&name) || SKIPPED_SUFFIXES.iter().any(|s| name.ends_with(s))
}

/// The files to freeze, relative with `/`, sorted. Symbolic links are refused: a frozen copy holds only its own bytes.
/// `node_modules` counts apart, against its own limits
fn list_files(root: &Path) -> Result<Vec<String>> {
  let mut out = vec![];
  let (mut own, mut own_bytes, mut deps, mut dep_bytes) = (0usize, 0u64, 0usize, 0u64);
  let mut stack = vec![(PathBuf::new(), false)];
  while let Some((rel, in_deps)) = stack.pop() {
    let dir = root.join(&rel);
    for e in std::fs::read_dir(&dir).with_context(|| format!("read {}", dir.display()))? {
      let e = e?;
      let name = e.file_name().to_string_lossy().into_owned();
      if skipped(&name) || (in_deps && SKIPPED_IN_DEPS.contains(&name.as_str())) {
        continue;
      }
      let rel = rel.join(&name);
      let t = e.file_type()?;
      if t.is_symlink() {
        if in_deps {
          bail!("{} is a symbolic link: install the packages with npm (or pnpm with `node-linker=hoisted`), so node_modules holds the files", rel.display());
        }
        bail!("{} is a symbolic link; a plugin holds its own files", rel.display());
      }
      if t.is_dir() {
        stack.push((rel, in_deps || name == "node_modules"));
        continue;
      }
      let len = e.metadata()?.len();
      out.push(rel.to_string_lossy().replace('\\', "/"));
      if in_deps {
        deps += 1;
        dep_bytes += len;
        if deps > DEP_MAX_FILES || dep_bytes > DEP_MAX_BYTES {
          bail!("node_modules holds more than {DEP_MAX_FILES} files or {} MB; keep the plugin's packages few", DEP_MAX_BYTES >> 20);
        }
      } else {
        own += 1;
        own_bytes += len;
        if own > MAX_FILES || own_bytes > MAX_BYTES {
          bail!("the plugin folder holds more than {MAX_FILES} files or {} MB besides node_modules (run-time data belongs in .data)", MAX_BYTES >> 20);
        }
      }
    }
  }
  out.sort();
  Ok(out)
}

fn executable(meta: &std::fs::Metadata) -> bool {
  #[cfg(unix)]
  {
    use std::os::unix::fs::PermissionsExt;
    meta.permissions().mode() & 0o111 != 0
  }
  #[cfg(not(unix))]
  {
    let _ = meta;
    false
  }
}

/// SHA-256 over each file's path, executable bit and content in path order; the first 32 hex digits
fn digest(root: &Path, files: &[String]) -> Result<String> {
  use sha2::Digest as _;
  let mut h = sha2::Sha256::new();
  for rel in files {
    let path = root.join(rel);
    let meta = std::fs::metadata(&path)?;
    let content = std::fs::read(&path).with_context(|| format!("read {}", path.display()))?;
    h.update(rel.as_bytes());
    h.update([0, executable(&meta) as u8, 0]);
    h.update((content.len() as u64).to_be_bytes());
    h.update(&content);
  }
  Ok(h.finalize().iter().take(16).map(|b| format!("{b:02x}")).collect())
}

/// Copies this process has verified: a copy is read-only and hashed once per engine run, not on every reload (a copy
/// with a large node_modules would otherwise be read whole each time the state changes)
static VERIFIED: std::sync::LazyLock<parking_lot::Mutex<std::collections::HashSet<PathBuf>>> = std::sync::LazyLock::new(Default::default);

/// The copy still hashes to its name (files a run left behind under skipped names do not count)
fn verify(dir: &Path, hash: &str) -> Result<()> {
  if !dir.is_dir() {
    bail!("the frozen copy {} is missing", dir.display());
  }
  if VERIFIED.lock().contains(dir) {
    return Ok(());
  }
  let files = list_files(dir)?;
  let got = digest(dir, &files)?;
  if got != hash {
    bail!("the frozen copy {} was changed after it was frozen", dir.display());
  }
  VERIFIED.lock().insert(dir.to_path_buf());
  Ok(())
}

/// Copy the files into a new snapshot. A file of `node_modules` is stored once under `blobs` by its content and linked
/// in, so versions and plugins holding the same packages share their bytes; where links fail it is copied
fn copy_files(from: &Path, to: &Path, files: &[String], blobs: &Path) -> Result<()> {
  for rel in files {
    let dst = to.join(rel);
    if let Some(p) = dst.parent() {
      std::fs::create_dir_all(p)?;
    }
    let src = from.join(rel);
    // Unix only: on Windows the read-only flag belongs to the file, so freezing or removing one copy would flip it for
    // every snapshot linking the same bytes
    if cfg!(unix)
      && (rel.starts_with("node_modules/") || rel.contains("/node_modules/"))
      && let Ok(blob) = blob_of(&src, blobs)
      && std::fs::hard_link(&blob, &dst).is_ok()
    {
      continue;
    }
    std::fs::copy(&src, &dst).with_context(|| format!("copy {rel}"))?;
  }
  std::fs::create_dir_all(to)?;
  Ok(())
}

/// The shared copy of a file's content (its executable bit is part of the name, since links share it)
fn blob_of(src: &Path, blobs: &Path) -> Result<PathBuf> {
  use sha2::Digest as _;
  let meta = std::fs::metadata(src)?;
  let content = std::fs::read(src)?;
  let mut h = sha2::Sha256::new();
  h.update(&content);
  let name: String = h.finalize().iter().map(|b| format!("{b:02x}")).collect();
  let blob = blobs.join(name.get(..2).unwrap_or("00")).join(format!("{name}{}", if executable(&meta) { ".x" } else { "" }));
  if !blob.is_file() {
    if let Some(p) = blob.parent() {
      std::fs::create_dir_all(p)?;
    }
    let tmp = blob.with_extension(format!("tmp-{}", crate::util::random_hex(6)));
    std::fs::write(&tmp, &content)?;
    let mut perm = std::fs::metadata(&tmp)?.permissions();
    #[cfg(unix)]
    {
      use std::os::unix::fs::PermissionsExt;
      perm.set_mode(if executable(&meta) { 0o555 } else { 0o444 });
    }
    #[cfg(not(unix))]
    perm.set_readonly(true);
    std::fs::set_permissions(&tmp, perm)?;
    if std::fs::rename(&tmp, &blob).is_err() {
      let _ = std::fs::remove_file(&tmp);
    }
  }
  Ok(blob)
}

/// What the storage page may clean under the plugins root: per plugin id, the files nothing in effect needs (the
/// folders `.removed` keeps, frozen versions outside the state's current, earlier and proposed ones, compiled output
/// of those or of an older engine), and apart from them the data of a plugin that is gone (no entry, proposal or source)
#[derive(Debug, Clone, PartialEq)]
pub struct Leftover {
  pub id: String,
  pub data: bool,
  pub targets: Vec<PathBuf>,
}

/// Run `f` (blocking) holding the lock of the state under `root`, so no commit lands meanwhile
pub async fn with_state_lock<T: Send + 'static>(root: &Path, f: impl FnOnce() -> T + Send + 'static) -> Result<T> {
  std::fs::create_dir_all(root)?;
  with_file_lock(&root.join(STATE), move || async move { tokio::task::spawn_blocking(f).await.map_err(|e| anyhow!("{e}")) }).await
}

/// `fresh` holds back a version or output that may belong to a freeze under way (not yet in the state)
pub fn leftovers(root: &Path, fresh: &dyn Fn(&Path) -> bool) -> Vec<Leftover> {
  let store = PluginStore::new(root.to_path_buf());
  // An unreadable state names nothing as needed: offer nothing rather than everything
  let Ok(state) = store.load() else { return vec![] };
  let sources = store.sources();
  let known = |id: &str| state.plugins.contains_key(id) || state.proposals.contains_key(id) || sources.iter().any(|s| s == id);
  let needed = |id: &str, hash: &str| {
    state.plugins.get(id).is_some_and(|e| e.snapshot == hash || e.previous.iter().any(|p| p == hash)) || state.proposals.get(id).is_some_and(|p| p.snapshot == hash)
  };
  let dirs = |p: &Path| -> Vec<(String, PathBuf)> {
    let Ok(rd) = std::fs::read_dir(p) else { return vec![] };
    let mut out: Vec<_> = rd.flatten().map(|e| (e.file_name().to_string_lossy().into_owned(), e.path())).collect();
    out.sort();
    out
  };
  let mut files: BTreeMap<String, Vec<PathBuf>> = BTreeMap::new();
  for (name, path) in dirs(&root.join(REMOVED)) {
    // `<id>.<ms>`
    let id = name.rsplit_once('.').map_or(name.as_str(), |(id, _)| id).to_owned();
    files.entry(id).or_default().push(path);
  }
  for (id, dir) in dirs(&root.join(VERSIONS)) {
    if !known(&id) {
      files.entry(id).or_default().push(dir);
      continue;
    }
    for (hash, path) in dirs(&dir) {
      if !needed(&id, &hash) && !fresh(&path) {
        files.entry(id.clone()).or_default().push(path);
      }
    }
  }
  for (id, dir) in dirs(&root.join(crate::plugins::build::BUILD)) {
    if !known(&id) {
      files.entry(id).or_default().push(dir);
      continue;
    }
    for (name, path) in dirs(&dir) {
      let hash = name.split('-').next().unwrap_or_default();
      // Output of a needed version stays whichever engine built it: an older engine may still run from it
      if !needed(&id, hash) && !fresh(&path) && !fresh(&root.join(VERSIONS).join(&id).join(hash)) {
        files.entry(id.clone()).or_default().push(path);
      }
    }
  }
  let mut out: Vec<Leftover> = files.into_iter().map(|(id, targets)| Leftover { id, data: false, targets }).collect();
  let mut data: BTreeMap<String, Vec<PathBuf>> = BTreeMap::new();
  for (name, path) in dirs(&root.join(DATA)) {
    let id = name.strip_suffix(".kv.json").unwrap_or(&name).to_owned();
    if manifest::valid_id(&id) && !known(&id) {
      data.entry(id).or_default().push(path);
    }
  }
  out.extend(data.into_iter().map(|(id, targets)| Leftover { id, data: true, targets }));
  out
}

/// Let a frozen copy be deleted: its folders writable again. Its files keep their mode, since on Unix they are the
/// same inodes other versions and the shared store link (deleting a link needs only its folder writable); Windows
/// clears the read-only flag of everything, as deleting there requires
pub fn make_removable(dir: &Path) -> std::io::Result<()> {
  #[cfg(unix)]
  {
    use std::os::unix::fs::PermissionsExt;
    fn walk_dirs(d: &Path) -> std::io::Result<()> {
      let meta = std::fs::symlink_metadata(d)?;
      if !meta.is_dir() {
        return Ok(());
      }
      let mut perm = meta.permissions();
      perm.set_mode(perm.mode() | 0o700);
      std::fs::set_permissions(d, perm)?;
      for e in std::fs::read_dir(d)?.flatten() {
        if e.file_type()?.is_dir() {
          walk_dirs(&e.path())?;
        }
      }
      Ok(())
    }
    walk_dirs(dir)
  }
  #[cfg(not(unix))]
  {
    if std::fs::symlink_metadata(dir)?.is_dir() { make_writable(dir) } else { Ok(()) }
  }
}

/// Shared files no snapshot links any more (Unix: a link count of one); the bytes freed. Windows keeps them
pub fn prune_blobs(root: &Path) -> u64 {
  #[cfg(unix)]
  {
    use std::os::unix::fs::MetadataExt;
    let mut freed = 0;
    let Ok(dirs) = std::fs::read_dir(root.join(BLOBS)) else { return 0 };
    for d in dirs.flatten() {
      let Ok(files) = std::fs::read_dir(d.path()) else { continue };
      for f in files.flatten() {
        if let Ok(m) = f.metadata()
          && m.is_file()
          && m.nlink() <= 1
          && std::fs::remove_file(f.path()).is_ok()
        {
          freed += m.len();
        }
      }
    }
    freed
  }
  #[cfg(not(unix))]
  {
    let _ = root;
    0
  }
}

fn walk(dir: &Path, f: &mut dyn FnMut(&Path, bool) -> std::io::Result<()>) -> std::io::Result<()> {
  for e in std::fs::read_dir(dir)? {
    let e = e?;
    let p = e.path();
    if e.file_type()?.is_dir() {
      walk(&p, f)?;
      f(&p, true)?;
    } else {
      f(&p, false)?;
    }
  }
  Ok(())
}

/// Files and folders read-only; files keep their executable bit
fn make_read_only(dir: &Path) -> std::io::Result<()> {
  let mut set = |p: &Path, _is_dir: bool| {
    let mut perm = std::fs::metadata(p)?.permissions();
    #[cfg(unix)]
    {
      use std::os::unix::fs::PermissionsExt;
      let exec = _is_dir || perm.mode() & 0o111 != 0;
      perm.set_mode(if exec { 0o555 } else { 0o444 });
    }
    #[cfg(not(unix))]
    perm.set_readonly(true);
    std::fs::set_permissions(p, perm)
  };
  walk(dir, &mut set)?;
  set(dir, true)
}

/// Undo `make_read_only`, so the copy can be moved or deleted
pub fn make_writable(dir: &Path) -> std::io::Result<()> {
  let mut set = |p: &Path, _is_dir: bool| {
    let mut perm = std::fs::metadata(p)?.permissions();
    #[cfg(unix)]
    {
      use std::os::unix::fs::PermissionsExt;
      perm.set_mode(perm.mode() | 0o200);
    }
    #[cfg(not(unix))]
    #[allow(clippy::permissions_set_readonly_false, reason = "the copy is about to be deleted")]
    perm.set_readonly(false);
    std::fs::set_permissions(p, perm)
  };
  set(dir, true)?;
  walk(dir, &mut set)
}

#[cfg(test)]
mod tests {
  use super::*;
  use serde_json::json;

  fn write_plugin(root: &Path, id: &str, script: &str) {
    let dir = root.join(id);
    std::fs::create_dir_all(&dir).unwrap();
    let m = json!({ "schema": 1, "id": id, "version": "0.1.0", "description": "x",
      "contributes": { "tools": [{ "name": "go", "description": "Go", "command": ["sh", "go.sh"] }] } });
    std::fs::write(dir.join("plugin.json"), m.to_string()).unwrap();
    std::fs::write(dir.join("go.sh"), script).unwrap();
  }

  fn any(_: &str) -> bool {
    true
  }

  #[tokio::test]
  async fn a_commit_refuses_a_ring_another_engine_closed_and_a_copy_cleaned_meanwhile() {
    let tmp = tempfile::tempdir().unwrap();
    for (id, dep, optional) in [("a", "b", false), ("b", "a", true)] {
      write_plugin(tmp.path(), id, "echo");
      let path = tmp.path().join(id).join("plugin.json");
      let mut m: serde_json::Value = serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
      m[if optional { "optionalDependencies" } else { "dependencies" }] = json!({ dep: "*" });
      std::fs::write(&path, m.to_string()).unwrap();
    }
    // Two engines, each with its own view: the second commit still sees the first through the shared state
    let (one, two) = (PluginStore::new(tmp.path().to_path_buf()), PluginStore::new(tmp.path().to_path_buf()));
    let (a, b) = (one.freeze("a", &any).unwrap(), two.freeze("b", &any).unwrap());
    one.enable_as("a", &a.hash, Provenance::user(), false).await.unwrap();
    let err = two.enable_as("b", &b.hash, Provenance::user(), false).await.unwrap_err().to_string();
    assert!(err.contains("ring: b -> a -> b"), "{err}");
    // A version the storage page removed after the freeze is refused at the commit, never enabled half there
    make_removable(&b.dir).unwrap();
    std::fs::remove_dir_all(&b.dir).unwrap();
    std::fs::remove_dir_all(tmp.path().join("a")).unwrap();
    one.remove("a", Provenance::user()).await.unwrap();
    assert!(two.enable_as("b", &b.hash, Provenance::user(), false).await.unwrap_err().to_string().starts_with(GONE));
  }

  #[tokio::test]
  async fn freezing_is_content_addressed_and_ignores_run_leftovers() {
    let tmp = tempfile::tempdir().unwrap();
    let store = PluginStore::new(tmp.path().to_path_buf());
    write_plugin(tmp.path(), "demo", "echo 1");
    let a = store.freeze("demo", &any).unwrap();
    assert!(a.findings.ok(), "{:?}", a.findings);
    assert_eq!(a.files, ["go.sh", "plugin.json"]);
    // Caches and version control are not part of the plugin
    std::fs::create_dir_all(tmp.path().join("demo/__pycache__")).unwrap();
    std::fs::write(tmp.path().join("demo/__pycache__/x.pyc"), "x").unwrap();
    std::fs::create_dir_all(tmp.path().join("demo/.git")).unwrap();
    assert_eq!(store.freeze("demo", &any).unwrap().hash, a.hash);
    // The copy is read-only and loads; a run leaving a cache in it does not spoil it
    assert!(std::fs::metadata(a.dir.join("go.sh")).unwrap().permissions().readonly());
    assert!(store.open("demo", &a.hash).is_ok());
    // An edit is a new version
    std::fs::write(tmp.path().join("demo/go.sh"), "echo 2").unwrap();
    let b = store.freeze("demo", &any).unwrap();
    assert_ne!(a.hash, b.hash);
    assert_eq!(store.source_changed("demo", &b.hash), Some(false));
    assert_eq!(store.source_changed("demo", &a.hash), Some(true));
  }

  #[tokio::test]
  async fn a_changed_copy_is_refused_and_frozen_again() {
    let tmp = tempfile::tempdir().unwrap();
    let store = PluginStore::new(tmp.path().to_path_buf());
    write_plugin(tmp.path(), "demo", "echo 1");
    let a = store.freeze("demo", &any).unwrap();
    make_writable(&a.dir).unwrap();
    std::fs::write(a.dir.join("go.sh"), "rm -rf /").unwrap();
    assert!(format!("{:#}", store.open("demo", &a.hash).unwrap_err()).contains("changed after it was frozen"));
    let again = store.freeze("demo", &any).unwrap();
    assert_eq!(again.hash, a.hash);
    assert!(store.open("demo", &a.hash).is_ok());
  }

  #[tokio::test]
  async fn commits_count_generations_log_each_change_and_roll_back() {
    let tmp = tempfile::tempdir().unwrap();
    let store = PluginStore::new(tmp.path().to_path_buf());
    write_plugin(tmp.path(), "demo", "echo 1");
    let a = store.freeze("demo", &any).unwrap().hash;
    store.enable("demo", &a, Provenance::user()).await.unwrap().unwrap();
    // The same version again is no change
    assert!(store.enable("demo", &a, Provenance::user()).await.unwrap().is_none());
    std::fs::write(tmp.path().join("demo/go.sh"), "echo 2").unwrap();
    let b = store.freeze("demo", &any).unwrap().hash;
    let by_session = Provenance { by: "session".into(), session: Some("s1".into()), agent: Some("claude".into()), model: None };
    store.enable("demo", &b, by_session.clone()).await.unwrap();
    let s = store.load().unwrap();
    assert_eq!((s.generation, s.plugins["demo"].snapshot.as_str(), s.plugins["demo"].previous.as_slice()), (2, b.as_str(), std::slice::from_ref(&a)));
    assert_eq!(s.plugins["demo"].provenance, by_session);
    assert!(tmp.path().join(".data/demo").is_dir());
    store.rollback("demo", Provenance::user()).await.unwrap();
    assert_eq!(store.load().unwrap().plugins["demo"].snapshot, a);
    assert!(store.rollback("demo", Provenance::user()).await.is_err());
    store.disable("demo", Provenance::user()).await.unwrap();
    let actions: Vec<_> = store.changes(10).into_iter().map(|c| (c.generation, c.action)).collect();
    assert_eq!(actions, [(1, "enable".into()), (2, "enable".into()), (3, "rollback".into()), (4, "disable".into())]);
  }

  #[tokio::test]
  async fn a_crash_between_the_two_writes_is_repaired() {
    let tmp = tempfile::tempdir().unwrap();
    let store = PluginStore::new(tmp.path().to_path_buf());
    store.set_safe_mode(true, Provenance::cli()).await.unwrap();
    // The log line of the last commit was lost
    std::fs::write(tmp.path().join(CHANGES), "").unwrap();
    store.reconcile().await.unwrap();
    assert_eq!(store.changes(5).iter().map(|c| c.generation).collect::<Vec<_>>(), [1]);
    std::fs::write(tmp.path().join(CHANGES), "").unwrap();
    // A later commit appends it first
    store.set_tool_group("summon", false, Provenance::user()).await.unwrap();
    assert_eq!(store.changes(5).iter().map(|c| (c.generation, c.action.as_str())).collect::<Vec<_>>(), [(1, "safeMode"), (2, "toolGroup")]);
    let s = store.load().unwrap();
    assert!(s.safe_mode && !s.tool_group_on("summon") && s.tool_group_on("images"));
  }

  #[tokio::test]
  async fn two_engines_committing_at_once_lose_nothing() {
    let tmp = tempfile::tempdir().unwrap();
    let a = std::sync::Arc::new(PluginStore::new(tmp.path().to_path_buf()));
    let b = std::sync::Arc::new(PluginStore::new(tmp.path().to_path_buf()));
    let mut tasks = vec![];
    for i in 0..8 {
      let store = if i % 2 == 0 { a.clone() } else { b.clone() };
      tasks.push(tokio::spawn(async move { store.set_tool_group(&format!("g{i}"), false, Provenance::user()).await.unwrap() }));
    }
    for t in tasks {
      t.await.unwrap();
    }
    let s = a.load().unwrap();
    assert_eq!((s.generation, s.tool_groups.len()), (8, 8));
    assert_eq!(a.changes(20).len(), 8);
  }

  #[tokio::test]
  async fn unknown_fields_survive_and_a_broken_state_is_never_overwritten() {
    let tmp = tempfile::tempdir().unwrap();
    let store = PluginStore::new(tmp.path().to_path_buf());
    std::fs::write(store.state_path(), json!({ "schema": 1, "generation": 4, "future": { "x": 1 }, "plugins": { "p": { "enabled": false, "snapshot": "h", "later": true } } }).to_string()).unwrap();
    store.set_safe_mode(true, Provenance::user()).await.unwrap();
    let v: Value = serde_json::from_slice(&std::fs::read(store.state_path()).unwrap()).unwrap();
    assert_eq!((v["future"]["x"].as_i64(), v["plugins"]["p"]["later"].as_bool(), v["generation"].as_u64()), (Some(1), Some(true), Some(5)));
    std::fs::write(store.state_path(), "{ half").unwrap();
    assert!(store.set_safe_mode(false, Provenance::user()).await.is_err());
    assert_eq!(std::fs::read_to_string(store.state_path()).unwrap(), "{ half");
  }

  #[tokio::test]
  async fn the_storage_page_may_clean_what_nothing_in_effect_needs() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path();
    let store = PluginStore::new(root.to_path_buf());
    // `keep`: a, then b in effect (a is its earlier version), and c frozen but never enabled
    write_plugin(root, "keep", "echo a");
    let a = store.freeze("keep", &any).unwrap().hash;
    store.enable("keep", &a, Provenance::user()).await.unwrap();
    write_plugin(root, "keep", "echo b");
    let b = store.freeze("keep", &any).unwrap().hash;
    store.enable("keep", &b, Provenance::user()).await.unwrap();
    write_plugin(root, "keep", "echo c");
    let c = store.freeze("keep", &any).unwrap();
    // `gone`: enabled, wrote data, then uninstalled
    write_plugin(root, "gone", "echo g");
    let g = store.freeze("gone", &any).unwrap().hash;
    store.enable("gone", &g, Provenance::user()).await.unwrap();
    std::fs::write(root.join(DATA).join("gone.kv.json"), "{}").unwrap();
    std::fs::write(root.join(DATA).join("keep.kv.json"), "{}").unwrap();
    store.remove("gone", Provenance::user()).await.unwrap();
    let removed = std::fs::read_dir(root.join(REMOVED)).unwrap().next().unwrap().unwrap().path();

    let found = leftovers(root, &|_| false);
    assert_eq!(
      found,
      [
        Leftover { id: "gone".into(), data: false, targets: vec![removed, root.join(VERSIONS).join("gone")] },
        Leftover { id: "keep".into(), data: false, targets: vec![c.dir.clone()] },
        Leftover { id: "gone".into(), data: true, targets: vec![root.join(DATA).join("gone"), root.join(DATA).join("gone.kv.json")] },
      ]
    );
    // A version frozen moments ago may be a card still waiting
    assert!(leftovers(root, &|_| true).iter().all(|l| !l.targets.contains(&c.dir)));
    // Read-only copies delete once their folders are writable; the versions in effect stay read-only
    for t in found.iter().filter(|l| !l.data).flat_map(|l| l.targets.iter()) {
      make_removable(t).unwrap();
      std::fs::remove_dir_all(t).unwrap();
    }
    assert!(std::fs::metadata(store.snapshot_dir("keep", &b).join("go.sh")).unwrap().permissions().readonly());
    assert!(store.open("keep", &a).is_ok() && store.open("keep", &b).is_ok());
  }

  #[tokio::test]
  async fn proposals_wait_and_removal_moves_the_source_aside() {
    let tmp = tempfile::tempdir().unwrap();
    let store = PluginStore::new(tmp.path().to_path_buf());
    write_plugin(tmp.path(), "demo", "echo 1");
    let h = store.freeze("demo", &any).unwrap().hash;
    store.propose("demo", &h, Provenance::cli()).await.unwrap().unwrap();
    assert!(store.propose("demo", &h, Provenance::cli()).await.unwrap().is_none());
    let s = store.load().unwrap();
    assert!(s.proposals.contains_key("demo") && !s.plugins.contains_key("demo"));
    store.enable("demo", &h, Provenance::user()).await.unwrap();
    assert!(store.load().unwrap().proposals.is_empty());
    store.remove("demo", Provenance::user()).await.unwrap();
    assert!(!tmp.path().join("demo").exists() && store.load().unwrap().plugins.is_empty());
    assert_eq!(std::fs::read_dir(tmp.path().join(REMOVED)).unwrap().count(), 1);
    assert_eq!(store.sources(), Vec::<String>::new());
  }

  #[test]
  fn links_and_huge_folders_are_not_frozen() {
    let tmp = tempfile::tempdir().unwrap();
    let store = PluginStore::new(tmp.path().to_path_buf());
    write_plugin(tmp.path(), "demo", "echo 1");
    #[cfg(unix)]
    {
      std::os::unix::fs::symlink("/etc/hosts", tmp.path().join("demo/hosts")).unwrap();
      assert!(store.freeze("demo", &any).unwrap_err().to_string().contains("symbolic link"));
      std::fs::remove_file(tmp.path().join("demo/hosts")).unwrap();
    }
    for i in 0..=MAX_FILES {
      std::fs::write(tmp.path().join(format!("demo/f{i}")), "").unwrap();
    }
    assert!(store.freeze("demo", &any).unwrap_err().to_string().contains("more than"));
    assert!(store.freeze("Nope", &any).is_err());
  }

  #[test]
  fn node_modules_freezes_with_the_plugin_and_versions_share_its_files() {
    let tmp = tempfile::tempdir().unwrap();
    let store = PluginStore::new(tmp.path().to_path_buf());
    write_plugin(tmp.path(), "demo", "echo 1");
    let pkg = tmp.path().join("demo/node_modules/left-pad");
    std::fs::create_dir_all(&pkg).unwrap();
    std::fs::write(pkg.join("index.js"), "module.exports = 1;").unwrap();
    std::fs::write(tmp.path().join("demo/package.json"), r#"{ "dependencies": { "left-pad": "1" } }"#).unwrap();
    std::fs::write(tmp.path().join("demo/package-lock.json"), "{}").unwrap();
    #[cfg(unix)]
    {
      // npm's links to package binaries are left out; any other link in node_modules says how to install instead
      std::fs::create_dir_all(tmp.path().join("demo/node_modules/.bin")).unwrap();
      std::os::unix::fs::symlink("../left-pad/index.js", tmp.path().join("demo/node_modules/.bin/left-pad")).unwrap();
    }
    let a = store.freeze("demo", &any).unwrap();
    assert!(a.findings.ok(), "{:?}", a.findings);
    assert!(a.files.iter().any(|f| f == "node_modules/left-pad/index.js"), "{:?}", a.files);
    assert!(!a.files.iter().any(|f| f.contains(".bin")));
    // A new version shares the package's bytes with the first
    std::fs::write(tmp.path().join("demo/go.sh"), "echo 2").unwrap();
    let b = store.freeze("demo", &any).unwrap();
    assert_ne!(a.hash, b.hash);
    #[cfg(unix)]
    {
      use std::os::unix::fs::MetadataExt;
      let ia = std::fs::metadata(a.dir.join("node_modules/left-pad/index.js")).unwrap().ino();
      let ib = std::fs::metadata(b.dir.join("node_modules/left-pad/index.js")).unwrap().ino();
      assert_eq!(ia, ib);
      std::os::unix::fs::symlink("index.js", pkg.join("main.js")).unwrap();
      assert!(store.freeze("demo", &any).unwrap_err().to_string().contains("node-linker"));
    }
    // Without a lockfile the check refuses it
    #[cfg(unix)]
    std::fs::remove_file(pkg.join("main.js")).unwrap();
    std::fs::remove_file(tmp.path().join("demo/package-lock.json")).unwrap();
    let c = store.freeze("demo", &any).unwrap();
    assert!(c.findings.text().contains("lockfile"), "{:?}", c.findings);
  }
}
