//! What the Shared tab's buttons do. Every change that puts something where an agent looks is recorded in the ledger,
//! and anything in the way is moved into `~/.acpira/backups/shared/<time>/`, never deleted. Real files with content of
//! their own are only touched on an explicit decision (the link panel, a resolve button, the overwrite switch).
//!
//! Two levels behave differently. User level (`~/.agents` into the agents' home folders) is linked only through the
//! link panel, item by item, and kept up to date afterwards when the panel's "auto" was on. Project level needs work for
//! Claude alone (every other agent reads `.agents/skills` and `AGENTS.md` itself), so its links are made on their own,
//! relative and kept out of git status, unless turned off

use std::path::{Path, PathBuf};
use std::sync::Arc;

use anyhow::{Context, Result, anyhow, bail};

use acpira_shared::inventory::McpCaps;
use acpira_shared::shared_config::{Choice, Keep, Pick, ReachState, SharedAction, SharedScope, SharedTarget, SharedView};

use super::ledger::{Entry, EntryKind, Ledger, LedgerFile};
use super::links::{self, Spot};
use super::view::{self, CLAUDE_GLOBAL_IMPORT, CLAUDE_PROJECT_IMPORT, Wire, WireKind};
use super::{AGENTS, Places, mcp, pi_trust, scope_of_template};
use crate::agent_ext::{RuleWire, agent_ext};
use crate::store::file_lock::with_file_lock;
use crate::store::transcript_store::LogFn;

/// What a finished action asks the shell to do next
#[derive(Debug, Default, PartialEq)]
pub struct Outcome {
  /// A file to open in the editor, or a directory to reveal
  pub open: Option<PathBuf>,
}

pub struct SharedConfig {
  data_root: PathBuf,
  ledger: LedgerFile,
  log: LogFn,
}

impl SharedConfig {
  pub fn new(data_root: PathBuf, log: LogFn) -> Arc<Self> {
    Arc::new(SharedConfig { ledger: LedgerFile::new(&data_root), data_root, log })
  }

  fn backups(&self) -> PathBuf {
    let stamp: String = crate::util::now_iso().chars().map(|c| if c.is_ascii_alphanumeric() { c } else { '-' }).collect();
    self.data_root.join("backups").join("shared").join(stamp)
  }

  pub async fn view(&self, places: Places, agents: Vec<String>, caps: Arc<dyn Fn(&str) -> Option<McpCaps> + Send + Sync>) -> SharedView {
    if let Err(e) = self.maintain(&places, &agents).await {
      (self.log)(&format!("shared config: keeping links failed: {e:#}"));
    }
    let ledger = self.ledger.read().await;
    blocking(move || Ok(view::build(&places, &agents, &*caps, &ledger))).await.unwrap_or_else(|_| empty_view())
  }

  /// Create links that are simply missing (nothing at the link point) and drop the ones whose shared source is gone:
  /// user level when "auto" is on, skipped points excepted; project level for the given project and every project
  /// that got links before, unless project linking is off. Nothing that exists is replaced here
  pub async fn maintain(&self, places: &Places, agents: &[String]) -> Result<()> {
    let ledger = self.ledger.read().await;
    if ledger.overwrite {
      self.force_global(places, agents).await?;
    } else {
      // Overwrite is off but some of what it did could not be undone when it was turned off: try again
      if ledger.entries.iter().any(|e| e.overwrite)
        && let Err(e) = self.undo(|e| e.overwrite).await
      {
        (self.log)(&format!("shared config: {e:#}"));
      }
      if ledger.auto {
        self.link_missing(places, agents, SharedScope::Global, &ledger).await?;
      }
    }
    if !ledger.project_manual {
      let mut roots: Vec<PathBuf> = places.root.iter().cloned().collect();
      for p in &ledger.projects {
        if Path::new(p).is_dir() && !roots.iter().any(|r| r.to_string_lossy() == *p) {
          roots.push(PathBuf::from(p));
        }
      }
      for r in roots {
        self.link_missing(&Places { root: Some(r), ..places.clone() }, agents, SharedScope::Project, &ledger).await?;
      }
    }
    self.prune().await
  }

  /// Agents turned off in the settings lose the links / imports made for them (backups go back in place); turned on
  /// again, the automatic levels link them anew
  pub async fn retire(&self, off: &[String]) {
    let off = off.to_vec();
    let of_off = move |e: &Entry| e.agent.as_ref().is_some_and(|a| off.contains(a));
    if !self.ledger.read().await.entries.iter().any(&of_off) {
      return;
    }
    if let Err(e) = self.undo(of_off).await {
      (self.log)(&format!("shared config: taking back a turned-off agent's links failed: {e:#}"));
    }
  }

  /// Links whose shared source was deleted point at nothing now
  async fn prune(&self) -> Result<()> {
    let ledger = self.ledger.read().await;
    let dead: Vec<Entry> = ledger
      .entries
      .iter()
      .filter(|e| e.kind == EntryKind::Link && !Path::new(&e.target).exists() && links::is_link(Path::new(&e.path)))
      .cloned()
      .collect();
    if dead.is_empty() {
      return Ok(());
    }
    for e in &dead {
      let _ = links::remove_link(Path::new(&e.path));
      if let Some(repo) = &e.repo {
        let _ = links::git_unexclude(Path::new(repo), &rel(Path::new(&e.path), Path::new(repo)));
      }
    }
    // An entry holding a backup stays: undoing it later (unlink, overwrite off) still puts the agent's own copy back
    self.ledger.update(move |l| dead.iter().filter(|e| e.backup.is_none()).for_each(|e| l.drop_path(Path::new(&e.path)))).await
  }

  /// Wire every point of one scope that has nothing at it yet. At user level an exact copy of the shared skill or
  /// prompt is replaced too (moved to the backups), which is what brings a turned-off agent back without a decision;
  /// inside a project a copy may be committed, so it stays
  async fn link_missing(&self, places: &Places, agents: &[String], scope: SharedScope, ledger: &Ledger) -> Result<()> {
    let (p, a) = (places.clone(), agents.to_vec());
    let skipped = ledger.skipped.clone();
    let free = move |w: &Wire| {
      std::fs::symlink_metadata(&w.at).is_err()
        || (scope == SharedScope::Global && w.kind == WireKind::Link && links::inspect(&w.at, &w.target) == Spot::Same)
    };
    let todo: Vec<Wire> = blocking(move || {
      Ok(
        view::wires(&p, &a)
          .into_iter()
          .filter(|w| w.scope == scope && w.target.exists() && free(w))
          .filter(|w| !skipped.iter().any(|x| Path::new(x) == w.at))
          .collect(),
      )
    })
    .await?;
    if todo.is_empty() {
      return Ok(());
    }
    let exclude = places.root.as_ref().is_none_or(|r| !ledger.is_shared_project(r));
    let (root, b, log) = (places.root.clone(), self.backups(), self.log.clone());
    let made: Vec<Entry> = blocking(move || {
      Ok(
        todo
          .iter()
          .filter_map(|w| wire(w, root.as_deref(), &b, exclude).map_err(|e| log(&format!("shared config: {e:#}"))).ok())
          .collect(),
      )
    })
    .await?;
    if made.is_empty() {
      return Ok(());
    }
    let root = places.root.clone().filter(|_| scope == SharedScope::Project);
    self
      .ledger
      .update(move |l| {
        if let Some(r) = &root {
          l.add_project(r);
        }
        made.into_iter().for_each(|e| l.put(e));
      })
      .await
  }

  /// Overwrite's half of `maintain`. First the agents' own user-level skills join `~/.agents/skills` (see
  /// `adopt_private_skills`); then every user-level link point whose shared source exists ends up wired, skipped ones
  /// and conflicts included (an import file with instructions of its own besides the import line too), whatever stood
  /// there going to the backups. A point wired already is left alone. Every entry made here is overwrite's: one that
  /// overwrite wired before and that changed since keeps its first backup, the state before overwrite, which is what
  /// turning it off brings back; a link panel entry re-wired here becomes overwrite's, with what stood there as backup
  async fn force_global(&self, places: &Places, agents: &[String]) -> Result<()> {
    let (p, a, b, log) = (places.clone(), agents.to_vec(), self.backups(), self.log.clone());
    let made: Vec<Entry> = blocking(move || {
      let mut made = adopt_private_skills(&p, &a, &b, &log);
      for w in view::wires(&p, &a) {
        if w.scope != SharedScope::Global || !w.target.exists() || !matches!(w.overwrite_state(), ReachState::Missing | ReachState::Conflict) {
          continue;
        }
        let r = (|| -> Result<Entry> {
          // A mixed import file goes aside whole; the import line is then written on its own
          let aside = if w.mixed_import() { Some(links::move_aside(&w.at, &b)?) } else { None };
          let mut e = wire(&w, None, &b, false).inspect_err(|_| {
            if let Some(a) = &aside {
              let _ = links::move_to(a, &w.at);
            }
          })?;
          if let Some(a) = aside {
            e.backup = Some(s(&a));
          }
          Ok(e)
        })();
        match r {
          Ok(e) => made.push(e),
          Err(e) => log(&format!("shared config: overwrite: {e:#}")),
        }
      }
      Ok(made)
    })
    .await?;
    if made.is_empty() {
      return Ok(());
    }
    self
      .ledger
      .update(move |l| {
        for mut e in made {
          // What drifted in meanwhile stays in the backups folder; the ledger keeps the state from before overwrite
          if let Some(old) = l.find(Path::new(&e.path))
            && old.overwrite
          {
            e.backup = old.backup.clone();
          }
          e.overwrite = true;
          l.put(e);
        }
      })
      .await
  }

  /// The link panel's decisions. Conflicts that make an agent's own version the shared one go first, so the links
  /// made after them point at the chosen content
  async fn link(&self, picks: Vec<Pick>, auto: bool, places: &Places, agents: &[String]) -> Result<()> {
    let (p, a, b, log) = (places.clone(), agents.to_vec(), self.backups(), self.log.clone());
    let mut ordered = picks.clone();
    ordered.sort_by_key(|x| x.choice != Choice::KeepPrivate);
    let (made, errors) = blocking(move || {
      let (mut made, mut errors) = (vec![], vec![]);
      for pick in ordered {
        let at = PathBuf::from(&pick.at);
        let r = (|| -> Result<Option<Entry>> {
          let Some(w) = view::wires(&p, &a).into_iter().find(|w| w.scope == SharedScope::Global && w.at == at) else {
            bail!("{} is not a link point any more", at.display())
          };
          match (pick.choice, &w.skill) {
            (Choice::Skip, _) => Ok(None),
            (Choice::Link, _) => {
              if w.state() == ReachState::Conflict {
                bail!("{} holds content of its own; pick which version to keep", at.display());
              }
              if !w.target.exists() {
                bail!("{} does not exist yet", w.target.display());
              }
              wire(&w, None, &b, false).map(Some)
            }
            (choice, Some(_)) => resolve_skill(&p, &a, &at, keep_of(choice), &b, false),
            (choice, None) => resolve_prompt(&p, &a, &at, keep_of(choice), &b).map(Some),
          }
        })();
        match r {
          Ok(Some(e)) => made.push(e),
          Ok(None) => {}
          Err(e) => {
            log(&format!("shared config: {e:#}"));
            errors.push(format!("{e:#}"));
          }
        }
      }
      Ok((made, errors))
    })
    .await?;
    self
      .ledger
      .update(move |l| {
        l.auto = auto;
        for pick in &picks {
          l.skipped.retain(|x| Path::new(x) != Path::new(&pick.at));
          if pick.choice == Choice::Skip {
            l.skipped.push(pick.at.clone());
          }
        }
        made.into_iter().for_each(|e| l.put(e));
      })
      .await?;
    if !errors.is_empty() {
      bail!("{}", errors.join("; "));
    }
    Ok(())
  }

  /// Undo the recorded entries `which` selects and forget them
  async fn undo(&self, which: impl Fn(&Entry) -> bool + Send + 'static) -> Result<()> {
    let ledger = self.ledger.read().await;
    let chosen: Vec<Entry> = ledger.entries.iter().filter(|e| which(e)).cloned().collect();
    let log = self.log.clone();
    let c = chosen.clone();
    let failed = blocking(move || Ok(unlink(&c, &log))).await?;
    // Only what was undone is forgotten; a failed entry keeps its backup on record for the next try
    let done: Vec<Entry> = chosen.into_iter().filter(|e| !failed.contains(e)).collect();
    self.ledger.update(move |l| done.iter().for_each(|e| l.drop_path(Path::new(&e.path)))).await?;
    if !failed.is_empty() {
      bail!("could not undo {}", failed.iter().map(|e| e.path.as_str()).collect::<Vec<_>>().join(", "));
    }
    Ok(())
  }

  pub async fn apply(&self, action: SharedAction, places: &Places, agents: &[String]) -> Result<Outcome> {
    match action {
      SharedAction::Link { picks, auto } => {
        self.link(picks, auto, places, agents).await?;
        Ok(Outcome::default())
      }
      SharedAction::Unlink => {
        self.undo(|e| e.project_root().is_none()).await?;
        self.ledger.update(|l| l.auto = false).await?;
        Ok(Outcome::default())
      }
      SharedAction::Overwrite { on: true, merge } => {
        let file = places.prompt_file(SharedScope::Global).ok_or_else(|| anyhow!("no home"))?;
        let (p, a, b) = (places.clone(), agents.to_vec(), self.backups());
        locked_edit(file, move |f| merge_prompts(&p, &a, f, &merge, &b)).await?;
        self.ledger.update(|l| l.overwrite = true).await?;
        self.maintain(places, agents).await?;
        Ok(Outcome::default())
      }
      SharedAction::Overwrite { on: false, .. } => {
        self.ledger.update(|l| l.overwrite = false).await?;
        self.undo(|e| e.overwrite).await?;
        Ok(Outcome::default())
      }
      SharedAction::SavePrompt { scope, text, base } => {
        let file = places.prompt_file(scope).ok_or_else(|| anyhow!("no project"))?;
        locked_edit(file, move |f| {
          let now = read_text(f)?.unwrap_or_default();
          if now != base {
            bail!("{} changed on disk after editing began; reload it before saving", f.display());
          }
          if let Some(dir) = f.parent() {
            std::fs::create_dir_all(dir)?;
          }
          // In place rather than tmp + rename: the agents' links (hard links on Windows) keep pointing at this file
          Ok(std::fs::write(f, text)?)
        })
        .await?;
        // A first save creates the shared prompt, which the waiting link points now have a source for
        self.maintain(places, agents).await?;
        Ok(Outcome::default())
      }
      SharedAction::ProjectAuto { on } => {
        self.ledger.update(move |l| l.project_manual = !on).await?;
        if on {
          self.maintain(places, agents).await?;
        } else {
          // The CLAUDE.md import is an explicit edit of its own and stays
          self.undo(|e| e.project_root().is_some() && e.kind == EntryKind::Link).await?;
        }
        Ok(Outcome::default())
      }
      SharedAction::ShareProject { share } => {
        let root = places.root.clone().ok_or_else(|| anyhow!("no project"))?;
        let ledger = self.ledger.read().await;
        let key = s(&root);
        let mine: Vec<Entry> = ledger.entries.iter().filter(|e| e.project_root() == Some(&key) && e.repo.is_some()).cloned().collect();
        blocking(move || {
          for e in &mine {
            let repo = Path::new(e.repo.as_deref().unwrap_or_default());
            let line = rel(Path::new(&e.path), repo);
            if share { links::git_unexclude(repo, &line)? } else { links::git_exclude(repo, &line)? }
          }
          Ok(())
        })
        .await?;
        self
          .ledger
          .update(move |l| {
            l.shared_projects.retain(|x| *x != key);
            if share {
              l.shared_projects.push(key);
            }
          })
          .await?;
        Ok(Outcome::default())
      }
      SharedAction::TrustPi => {
        let (p, root) = (places.clone(), places.root.clone().ok_or_else(|| anyhow!("no project"))?);
        blocking(move || pi_trust::trust(&p, &root)).await?;
        Ok(Outcome::default())
      }
      SharedAction::ResolveSkill { path, keep } => {
        let ledger = self.ledger.read().await;
        let exclude = places.root.as_ref().is_none_or(|r| !ledger.is_shared_project(r));
        let (p, a, b) = (places.clone(), agents.to_vec(), self.backups());
        let entry = blocking(move || resolve_skill(&p, &a, Path::new(&path), keep, &b, exclude)).await?;
        self.record(entry, places).await
      }
      SharedAction::ClaudeImport => {
        let root = places.root.clone().ok_or_else(|| anyhow!("no project"))?;
        let file = root.join("CLAUDE.md");
        let f = file.clone();
        blocking(move || Ok(links::add_import(&f, CLAUDE_PROJECT_IMPORT)?)).await?;
        let e = Entry {
          path: s(&file),
          target: s(&root.join("AGENTS.md")),
          kind: EntryKind::Import,
          backup: None,
          repo: None,
          project: Some(s(&root)),
          agent: Some("claude".into()),
          overwrite: false,
        };
        self.record(Some(e), places).await
      }
      SharedAction::CreateSkill { scope, name } => {
        let name = name.trim().to_owned();
        if name.is_empty() || name.starts_with('.') || name.contains(['/', '\\', ':']) {
          bail!("invalid skill name");
        }
        let dir = places.skills_dir(scope).ok_or_else(|| anyhow!("no project"))?.join(&name);
        let d = dir.clone();
        blocking(move || {
          if d.exists() {
            bail!("{} already exists", d.display());
          }
          std::fs::create_dir_all(&d)?;
          let body = format!("---\nname: {name}\ndescription: What this skill does and when to use it.\n---\n\n# {name}\n\n");
          Ok(std::fs::write(d.join("SKILL.md"), body)?)
        })
        .await?;
        self.maintain(places, agents).await?;
        Ok(Outcome { open: Some(dir.join("SKILL.md")) })
      }
      SharedAction::RemoveSkill { path } => {
        let (p, a, b) = (places.clone(), agents.to_vec(), self.backups());
        blocking(move || remove_skill(&p, &a, Path::new(&path), &b)).await?;
        // Links that pointed at a deleted shared skill dangle now and are dropped here
        self.prune().await?;
        Ok(Outcome::default())
      }
      SharedAction::Open { scope, target } => {
        let path = match target {
          SharedTarget::Skills => places.skills_dir(scope),
          SharedTarget::Mcp => places.mcp_file(scope),
          SharedTarget::Prompt => places.prompt_file(scope),
        }
        .ok_or_else(|| anyhow!("no project"))?;
        let p = path.clone();
        blocking(move || {
          if p.exists() {
            return Ok(());
          }
          if target == SharedTarget::Skills {
            return Ok(std::fs::create_dir_all(&p)?);
          }
          if let Some(dir) = p.parent() {
            std::fs::create_dir_all(dir)?;
          }
          let body = if target == SharedTarget::Mcp { "{\n  \"mcpServers\": {}\n}\n" } else { "" };
          Ok(std::fs::write(&p, body)?)
        })
        .await?;
        Ok(Outcome { open: Some(path) })
      }
      SharedAction::AddMcp { scope, json, name } => {
        let file = places.mcp_file(scope).ok_or_else(|| anyhow!("no project"))?;
        locked_edit(file, move |f| mcp::add(f, &json, name.as_deref()).map(|_| ())).await?;
        Ok(Outcome::default())
      }
      SharedAction::ToggleMcp { scope, name, enabled } => {
        let file = mcp_file_of(places, scope, &name)?;
        locked_edit(file, move |f| mcp::toggle(f, &name, enabled)).await?;
        Ok(Outcome::default())
      }
      SharedAction::RemoveMcp { scope, name } => {
        let file = mcp_file_of(places, scope, &name)?;
        locked_edit(file, move |f| mcp::remove(f, &name)).await?;
        Ok(Outcome::default())
      }
    }
  }

  async fn record(&self, entry: Option<Entry>, places: &Places) -> Result<Outcome> {
    let root = places.root.clone();
    self
      .ledger
      .update(move |l| {
        if let Some(e) = entry {
          if let (Some(r), Some(_)) = (&root, e.project_root()) {
            l.add_project(r);
          }
          l.put(e);
        }
      })
      .await?;
    Ok(Outcome::default())
  }
}

fn empty_view() -> SharedView {
  SharedView {
    root: None,
    home: String::new(),
    auto: false,
    user_linked: false,
    project_auto: true,
    project_shared: false,
    project_linked: false,
    pi_untrusted: false,
    shared_prompt: false,
    overwrite: false,
    takeover: vec![],
    skills: vec![],
    mcp: vec![],
    no_mcp: vec![],
    prompts: vec![],
    private_skills: vec![],
    plan: vec![],
  }
}

fn keep_of(choice: Choice) -> Keep {
  if choice == Choice::KeepPrivate { Keep::Private } else { Keep::Shared }
}

fn s(p: &Path) -> String {
  p.to_string_lossy().into_owned()
}

fn rel(path: &Path, root: &Path) -> String {
  path.strip_prefix(root).map(|r| r.to_string_lossy().replace('\\', "/")).unwrap_or_default()
}

async fn blocking<T: Send + 'static>(f: impl FnOnce() -> Result<T> + Send + 'static) -> Result<T> {
  tokio::task::spawn_blocking(f).await.map_err(|e| anyhow!("{e}"))?
}

/// A read-modify-write of a shared config file, under the cross-process file lock: every window's sidecar edits the
/// same file, and two unlocked edits would each write back a copy missing the other's change
async fn locked_edit<T: Send + 'static>(file: PathBuf, f: impl FnOnce(&Path) -> Result<T> + Send + 'static) -> Result<T> {
  if let Some(dir) = file.parent() {
    tokio::fs::create_dir_all(dir).await?;
  }
  let at = file.clone();
  with_file_lock(&at, move || blocking(move || f(&file))).await
}

/// Put one wire in place; whatever sits there (an identical copy, or on an explicit resolve anything) is moved aside first.
/// `exclude`: a project link also goes into the repository's `info/exclude` (not for a project shared with the team)
fn wire(w: &Wire, root: Option<&Path>, backups: &Path, exclude: bool) -> Result<Entry> {
  let mut backup = None;
  // Whatever was moved aside goes back in place when the link / import line cannot be written after all
  let undo_aside = |backup: &Option<PathBuf>| {
    if let Some(b) = backup {
      let _ = links::move_to(b, &w.at);
    }
  };
  match w.kind {
    WireKind::Link => {
      if std::fs::symlink_metadata(&w.at).is_ok() {
        if links::inspect(&w.at, &w.target) == Spot::Linked {
          return Ok(entry(w, None, root));
        }
        backup = Some(links::move_aside(&w.at, backups).with_context(|| format!("moving {} aside", w.at.display()))?);
      }
      links::make_link(&w.at, &w.target, w.relative())
        .with_context(|| format!("linking {}", w.at.display()))
        .inspect_err(|_| undo_aside(&backup))?;
    }
    WireKind::Import => {
      // An import file that holds a copy of the shared prompt starts over as just the import line. One that exists but
      // cannot be read as text (another encoding) is content of its own and goes aside, never overwritten
      if std::fs::symlink_metadata(&w.at).is_ok() {
        let keep = std::fs::read_to_string(&w.at).is_ok_and(|t| t.trim().is_empty() || links::has_import(&t, &view::import_forms(&w.target)));
        if !keep {
          backup = Some(links::move_aside(&w.at, backups)?);
        }
      }
      links::add_import(&w.at, CLAUDE_GLOBAL_IMPORT).inspect_err(|_| undo_aside(&backup))?;
    }
  }
  let e = entry(w, backup.as_deref(), root);
  if let Some(repo) = &e.repo
    && exclude
  {
    links::git_exclude(Path::new(repo), &rel(&w.at, Path::new(repo)))?;
  }
  Ok(e)
}

/// The file an edit of one server goes to (a project server may still live in the legacy file)
fn mcp_file_of(places: &Places, scope: SharedScope, name: &str) -> Result<PathBuf> {
  match scope {
    SharedScope::Global => places.mcp_file(scope),
    SharedScope::Project => mcp::project_file_of(places, name),
  }
  .ok_or_else(|| anyhow!("no project"))
}

fn entry(w: &Wire, backup: Option<&Path>, root: Option<&Path>) -> Entry {
  let project = root.filter(|r| w.relative() && w.at.starts_with(r)).map(s);
  Entry {
    path: s(&w.at),
    target: s(&w.target),
    kind: if w.kind == WireKind::Import { EntryKind::Import } else { EntryKind::Link },
    backup: backup.map(s),
    repo: project.clone(),
    project,
    agent: Some(w.agent.to_owned()),
    overwrite: false,
  }
}

/// Overwrite's first step: `~/.agents/AGENTS.md` exists (created empty when missing) and ends with the unique lines
/// of each chosen agent file, each compared with the text so far, so a line two files share lands once. A changed
/// prompt is copied to the backups first and written in place, so links to it keep working
fn merge_prompts(places: &Places, agents: &[String], file: &Path, merge: &[String], backups: &Path) -> Result<()> {
  let before = read_text(file)?;
  let mut text = before.clone().unwrap_or_default();
  let forms = view::import_forms(file);
  let points: Vec<Wire> = view::wires(places, agents).into_iter().filter(|w| w.skill.is_none()).collect();
  for path in merge {
    let Some(w) = points.iter().find(|w| w.at == Path::new(path)) else { bail!("{path} is not an agent's global instruction file") };
    let Ok(own) = std::fs::read_to_string(&w.at) else { continue };
    let add = view::unique_lines(&own, &text, &forms);
    if add.is_empty() {
      continue;
    }
    if !text.trim().is_empty() {
      text = format!("{}\n\n", text.trim_end());
    }
    text.push_str(&add);
    text.push('\n');
  }
  if before.as_deref() == Some(text.as_str()) {
    return Ok(());
  }
  if before.is_some() {
    links::copy_aside(file, backups)?;
  }
  if let Some(dir) = file.parent() {
    std::fs::create_dir_all(dir)?;
  }
  Ok(std::fs::write(file, text)?)
}

/// A prompt file's text; None when it does not exist. Any other read failure (another encoding, no permission) is an
/// error, so content that cannot be read is never taken for an empty file and written over
fn read_text(file: &Path) -> Result<Option<String>> {
  match std::fs::read_to_string(file) {
    Ok(t) => Ok(Some(t)),
    Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
    Err(e) => Err(anyhow!("reading {}: {e}", file.display())),
  }
}

/// Overwrite's skill step: every user-level skill folder in an agent's own skills folder joins `~/.agents/skills`.
/// One the shared folder lacks is moved there (`Adopted`, moved back on off), so every agent reads it from then on;
/// one the shared folder already has, same or not, is moved into the backups (`Aside`), the shared one winning.
/// Link points (Claude's, Antigravity's) are left to the wires, and links of any kind are left alone
fn adopt_private_skills(places: &Places, agents: &[String], backups: &Path, log: &LogFn) -> Vec<Entry> {
  let Some(shared_dir) = places.skills_dir(SharedScope::Global) else { return vec![] };
  let points: Vec<PathBuf> = view::wires(places, agents).into_iter().filter(|w| w.scope == SharedScope::Global && w.skill.is_some()).map(|w| w.at).collect();
  let shared_real = std::fs::canonicalize(&shared_dir).ok();
  let mut out = vec![];
  for id in AGENTS.iter().filter(|id| agents.iter().any(|a| a == *id)) {
    let Some(ext) = agent_ext(id) else { continue };
    for tpl in ext.shared.own_skills.iter().filter(|t| scope_of_template(t) == SharedScope::Global) {
      let Some(dir) = places.expand(tpl) else { continue };
      if shared_real.is_some() && std::fs::canonicalize(&dir).ok() == shared_real {
        continue;
      }
      for (name, path, _) in view::skill_dirs(&dir) {
        if links::is_link(&path) || points.contains(&path) {
          continue;
        }
        let shared = shared_dir.join(&name);
        let r = (|| -> Result<Entry> {
          let (kind, at, target, backup) = if std::fs::symlink_metadata(&shared).is_ok() {
            (EntryKind::Aside, &path, &shared, Some(links::move_aside(&path, backups)?))
          } else {
            links::move_to(&path, &shared)?;
            (EntryKind::Adopted, &shared, &path, None)
          };
          Ok(Entry { path: s(at), target: s(target), kind, backup: backup.as_deref().map(s), repo: None, project: None, agent: Some((*id).to_owned()), overwrite: true })
        })();
        match r {
          Ok(e) => out.push(e),
          Err(e) => log(&format!("shared config: overwrite: taking in {}: {e:#}", path.display())),
        }
      }
    }
  }
  out
}

/// Undo recorded changes, newest first (a link made to an adopted skill goes before the skill moves back); a link that
/// was replaced by something else meanwhile is left alone. Returns the entries that could not be undone, which stay
/// recorded
fn unlink(entries: &[Entry], log: &LogFn) -> Vec<Entry> {
  let mut failed = vec![];
  for e in entries.iter().rev() {
    let path = Path::new(&e.path);
    let r = (|| -> Result<()> {
      match e.kind {
        // The shared copy goes back to the agent's own folder, unless something took that place since
        EntryKind::Adopted => {
          let own = Path::new(&e.target);
          if path.exists() && std::fs::symlink_metadata(own).is_err() {
            links::move_to(path, own)?;
          }
        }
        EntryKind::Aside => {}
        EntryKind::Link => {
          // Only a link that still resolves to the recorded target; one pointed elsewhere since is the user's
          if links::inspect(path, Path::new(&e.target)) == Spot::Linked {
            links::remove_link(path)?;
          }
        }
        EntryKind::Import => {
          let line = if e.target.ends_with("AGENTS.md") && path.file_name().is_some_and(|n| n == "CLAUDE.md") && path.parent() == Path::new(&e.target).parent() {
            CLAUDE_PROJECT_IMPORT
          } else {
            CLAUDE_GLOBAL_IMPORT
          };
          links::remove_import(path, line)?;
        }
      }
      if let Some(b) = &e.backup
        && Path::new(b).exists()
        && std::fs::symlink_metadata(path).is_err()
      {
        links::move_to(Path::new(b), path)?;
      }
      if let Some(repo) = &e.repo {
        links::git_unexclude(Path::new(repo), &rel(path, Path::new(repo)))?;
      }
      Ok(())
    })();
    if let Err(err) = r {
      log(&format!("shared config: undoing {} failed: {err:#}", e.path));
      failed.push(e.clone());
    }
  }
  failed
}

/// Which agent's own skills folder holds `path`, and in which scope
fn owner_of(places: &Places, agents: &[String], path: &Path) -> Option<(&'static str, SharedScope)> {
  let parent = path.parent()?;
  for id in AGENTS {
    if !agents.iter().any(|a| a == id) {
      continue;
    }
    let ext = agent_ext(id)?;
    for tpl in ext.shared.own_skills {
      if places.expand(tpl).is_some_and(|d| d == parent) {
        return Some((id, scope_of_template(tpl)));
      }
    }
  }
  None
}

fn resolve_skill(places: &Places, agents: &[String], path: &Path, keep: Keep, backups: &Path, exclude: bool) -> Result<Option<Entry>> {
  let (agent, scope) = owner_of(places, agents, path).ok_or_else(|| anyhow!("{} is not in an agent's skills folder", path.display()))?;
  let name = path.file_name().ok_or_else(|| anyhow!("no name"))?.to_owned();
  let shared = places.skills_dir(scope).ok_or_else(|| anyhow!("no project"))?.join(&name);
  let mut private_backup = None;
  match keep {
    Keep::Private => {
      if shared.exists() {
        links::move_aside(&shared, backups)?;
      }
      links::move_to(path, &shared)?;
    }
    Keep::Shared => {
      if !shared.exists() {
        bail!("no shared skill named {}", name.to_string_lossy());
      }
      private_backup = Some(links::move_aside(path, backups)?);
    }
  }
  // An agent that cannot read .agents gets its skill back as a link; one reading this scope itself (Antigravity in a
  // project) needs none, and a link would show it the skill twice
  let ext = agent_ext(agent);
  if ext.is_some_and(|e| e.reads_shared_skills(scope == SharedScope::Global)) {
    return Ok(None);
  }
  let Some((user, project)) = ext.and_then(|e| e.shared.skill_links) else { return Ok(None) };
  let base = places.expand(if scope == SharedScope::Global { user } else { project }).ok_or_else(|| anyhow!("no project"))?;
  let w = Wire { agent, scope, target: shared, at: base.join(&name), kind: WireKind::Link, skill: Some(name.to_string_lossy().into_owned()) };
  let mut e = wire(&w, places.root.as_deref(), backups, exclude)?;
  // The private copy set aside for the shared one comes back when this link is undone
  if e.backup.is_none()
    && w.at == path
  {
    e.backup = private_backup.as_deref().map(s);
  }
  Ok(Some(e))
}

/// Move one skill folder into the backups. Only a direct child of a shared skills folder or of an agent's own skills
/// folder is accepted, so a stray path from the page can never take anything else with it
fn remove_skill(places: &Places, agents: &[String], path: &Path, backups: &Path) -> Result<()> {
  let parent = path.parent().ok_or_else(|| anyhow!("no parent"))?;
  let shared = [SharedScope::Global, SharedScope::Project].into_iter().filter_map(|sc| places.skills_dir(sc)).any(|d| d == parent);
  if !shared && owner_of(places, agents, path).is_none() {
    bail!("{} is not in a skills folder", path.display());
  }
  if std::fs::symlink_metadata(path).is_err() {
    bail!("{} no longer exists", path.display());
  }
  links::move_aside(path, backups)?;
  Ok(())
}

fn resolve_prompt(places: &Places, agents: &[String], path: &Path, keep: Keep, backups: &Path) -> Result<Entry> {
  let shared = places.prompt_file(SharedScope::Global).ok_or_else(|| anyhow!("no home"))?;
  let w = view::wires(places, agents)
    .into_iter()
    .find(|w| w.skill.is_none() && w.at == path)
    .ok_or_else(|| anyhow!("{} is not an agent's global instruction file", path.display()))?;
  match keep {
    Keep::Private => {
      let mut text = std::fs::read_to_string(path)?;
      if agent_ext(w.agent).and_then(|e| e.shared.global_rules).is_some_and(|(_, k)| k == RuleWire::Import) {
        text = text.lines().filter(|l| !view::import_forms(&shared).iter().any(|f| l.trim() == f)).collect::<Vec<_>>().join("\n");
      }
      if shared.exists() {
        links::move_aside(&shared, backups)?;
      }
      if let Some(dir) = shared.parent() {
        std::fs::create_dir_all(dir)?;
      }
      std::fs::write(&shared, format!("{}\n", text.trim_end()))?;
    }
    Keep::Shared => {
      if !shared.exists() {
        bail!("no shared prompt yet");
      }
    }
  }
  // The file itself: out of the way, then wired
  if std::fs::symlink_metadata(path).is_ok() && links::inspect(path, &shared) != Spot::Linked {
    let b = links::move_aside(path, backups)?;
    let mut e = wire(&w, None, backups, false)?;
    e.backup = Some(s(&b));
    return Ok(e);
  }
  wire(&w, None, backups, false)
}

#[cfg(test)]
mod tests {
  use super::*;
  use acpira_shared::shared_config::PrivateMatch;
  use links::Spot;

  // A link point wired to `target` in whatever form the platform made: a symlink, or on Windows without symlink
  // rights a junction (directories) or a hard link (files). `is_symlink` alone would only hold on some machines
  fn wired(at: &Path, target: &Path) -> bool {
    links::inspect(at, target) == Spot::Linked
  }

  fn setup() -> (tempfile::TempDir, Places, Arc<SharedConfig>, Vec<String>) {
    let t = tempfile::tempdir().unwrap();
    let home = t.path().join("home");
    let root = t.path().join("repo");
    std::fs::create_dir_all(root.join(".git/info")).unwrap();
    for d in [".claude", ".codex", ".agents/skills/dig", ".grok"] {
      std::fs::create_dir_all(home.join(d)).unwrap();
    }
    std::fs::write(home.join(".agents/skills/dig/SKILL.md"), "---\nname: dig\ndescription: d\n---\n").unwrap();
    std::fs::create_dir_all(root.join(".agents/skills/release")).unwrap();
    std::fs::write(root.join(".agents/skills/release/SKILL.md"), "---\nname: release\n---\n").unwrap();
    let places = Places { home: home.clone(), config: home.join(".config"), root: Some(root) };
    let log: LogFn = Arc::new(|l| eprintln!("{l}"));
    let cfg = SharedConfig::new(t.path().join("data"), log);
    let agents = ["claude", "codex", "grok"].map(String::from).to_vec();
    (t, places, cfg, agents)
  }

  fn caps() -> Arc<dyn Fn(&str) -> Option<McpCaps> + Send + Sync> {
    Arc::new(|_| None)
  }

  fn pick(at: &Path, choice: Choice) -> Pick {
    Pick { at: s(at), choice }
  }

  #[tokio::test]
  async fn project_links_come_on_their_own_and_user_links_follow_the_panel() {
    let (_t, p, cfg, agents) = setup();
    let home = p.home.clone();
    let root = p.root.clone().unwrap();
    std::fs::write(home.join(".agents/AGENTS.md"), "# shared\n").unwrap();
    std::fs::write(home.join(".codex/AGENTS.md"), "# shared\n").unwrap();
    std::fs::write(home.join(".grok/AGENTS.md"), "# mine\n").unwrap();

    // Looking is enough for the project: Claude's link to .agents/skills appears, out of git status
    let v = cfg.view(p.clone(), agents.clone(), caps()).await;
    assert!(wired(&root.join(".claude/skills/release"), &root.join(".agents/skills/release")));
    // Relative on unix, so a moved repository keeps its links (a Windows junction is always absolute)
    #[cfg(unix)]
    assert_eq!(std::fs::read_link(root.join(".claude/skills/release")).unwrap(), PathBuf::from("../../.agents/skills/release"));
    assert_eq!(std::fs::read_to_string(root.join(".git/info/exclude")).unwrap(), "/.claude/skills/release\n");
    assert!(v.project_auto && v.project_linked && !v.user_linked && !v.auto);
    // User level waits for the panel, which lists the Claude skill link, the three prompt files and nothing native
    assert!(std::fs::symlink_metadata(home.join(".claude/skills/dig")).is_err());
    let plan = |v: &SharedView| v.plan.iter().map(|x| (x.agent.clone(), x.name.clone(), x.state)).collect::<Vec<_>>();
    assert_eq!(
      plan(&v),
      [
        ("claude".into(), "dig".into(), ReachState::Missing),
        ("claude".into(), "CLAUDE.md".into(), ReachState::Missing),
        ("codex".into(), "AGENTS.md".into(), ReachState::Missing),
        ("grok".into(), "AGENTS.md".into(), ReachState::Conflict),
      ]
    );

    // Link the skill and Codex, keep the shared prompt over Grok's, skip Claude's prompt
    let picks = vec![
      pick(&home.join(".claude/skills/dig"), Choice::Link),
      pick(&home.join(".codex/AGENTS.md"), Choice::Link),
      pick(&home.join(".grok/AGENTS.md"), Choice::KeepShared),
      pick(&home.join(".claude/CLAUDE.md"), Choice::Skip),
    ];
    cfg.apply(SharedAction::Link { picks, auto: true }, &p, &agents).await.unwrap();
    let shared = home.join(".agents/AGENTS.md");
    assert!(wired(&home.join(".claude/skills/dig"), &home.join(".agents/skills/dig")));
    assert!(wired(&home.join(".codex/AGENTS.md"), &shared) && wired(&home.join(".grok/AGENTS.md"), &shared));
    assert!(!home.join(".claude/CLAUDE.md").exists());
    let v = cfg.view(p.clone(), agents.clone(), caps()).await;
    assert!(v.auto && v.user_linked);
    assert_eq!(plan(&v), [("claude".into(), "CLAUDE.md".into(), ReachState::Missing)]);
    assert!(v.plan[0].skipped);

    // Auto: a new skill is linked on the next look, the skipped prompt stays untouched
    std::fs::create_dir_all(home.join(".agents/skills/new")).unwrap();
    std::fs::write(home.join(".agents/skills/new/SKILL.md"), "x").unwrap();
    cfg.view(p.clone(), agents.clone(), caps()).await;
    assert!(wired(&home.join(".claude/skills/new"), &home.join(".agents/skills/new")));
    assert!(!home.join(".claude/CLAUDE.md").exists());
    // A link to a deleted skill goes away
    std::fs::remove_dir_all(home.join(".agents/skills/new")).unwrap();
    cfg.maintain(&p, &agents).await.unwrap();
    assert!(std::fs::symlink_metadata(home.join(".claude/skills/new")).is_err());

    // Sharing the project with the team takes its links out of info/exclude, and back
    cfg.apply(SharedAction::ShareProject { share: true }, &p, &agents).await.unwrap();
    assert_eq!(std::fs::read_to_string(root.join(".git/info/exclude")).unwrap(), "");
    assert!(cfg.view(p.clone(), agents.clone(), caps()).await.project_shared);
    cfg.apply(SharedAction::ShareProject { share: false }, &p, &agents).await.unwrap();
    assert_eq!(std::fs::read_to_string(root.join(".git/info/exclude")).unwrap(), "/.claude/skills/release\n");

    // Undo is user level only; the backed-up Grok file comes back byte for byte
    cfg.apply(SharedAction::Unlink, &p, &agents).await.unwrap();
    assert!(std::fs::symlink_metadata(home.join(".claude/skills/dig")).is_err());
    assert_eq!(std::fs::read_to_string(home.join(".grok/AGENTS.md")).unwrap(), "# mine\n");
    assert_eq!(std::fs::read_to_string(home.join(".codex/AGENTS.md")).unwrap(), "# shared\n");
    assert!(!wired(&home.join(".codex/AGENTS.md"), &shared));
    assert!(wired(&root.join(".claude/skills/release"), &root.join(".agents/skills/release")));
    assert!(!cfg.view(p.clone(), agents.clone(), caps()).await.auto);

    // Project level off removes the project links and keeps them away
    cfg.apply(SharedAction::ProjectAuto { on: false }, &p, &agents).await.unwrap();
    assert!(std::fs::symlink_metadata(root.join(".claude/skills/release")).is_err());
    assert_eq!(std::fs::read_to_string(root.join(".git/info/exclude")).unwrap(), "");
    let v = cfg.view(p.clone(), agents.clone(), caps()).await;
    assert!(!v.project_auto && !v.project_linked);
    assert!(std::fs::symlink_metadata(root.join(".claude/skills/release")).is_err());
  }

  #[tokio::test]
  async fn resolves_private_skills_and_prompts() {
    let (t, p, cfg, agents) = setup();
    let home = p.home.clone();
    // A Claude-only skill, a differing Claude copy at a link point and an identical Codex duplicate
    for (dir, body) in [(".claude/skills/solo", "solo"), (".claude/skills/dig", "changed"), (".codex/skills/dig", "---\nname: dig\ndescription: d\n---\n")] {
      std::fs::create_dir_all(home.join(dir)).unwrap();
      std::fs::write(home.join(dir).join("SKILL.md"), body).unwrap();
    }
    let v = cfg.view(p.clone(), agents.clone(), caps()).await;
    let m = |n: &str, a: &str| v.private_skills.iter().find(|s| s.name == n && s.agent == a).map(|s| s.matches);
    // Claude's differing dig sits where the link goes, so the panel owns it rather than the private list
    assert_eq!((m("solo", "claude"), m("dig", "claude"), m("dig", "codex")), (Some(PrivateMatch::Unique), None, Some(PrivateMatch::Same)));
    assert!(v.plan.iter().any(|x| x.name == "dig" && x.state == ReachState::Conflict));

    let path = |d: &str| s(&home.join(d));
    cfg.apply(SharedAction::ResolveSkill { path: path(".claude/skills/solo"), keep: Keep::Private }, &p, &agents).await.unwrap();
    assert!(home.join(".agents/skills/solo/SKILL.md").exists());
    assert!(wired(&home.join(".claude/skills/solo"), &home.join(".agents/skills/solo")));
    cfg.apply(SharedAction::Link { picks: vec![pick(&home.join(".claude/skills/dig"), Choice::KeepPrivate)], auto: false }, &p, &agents).await.unwrap();
    assert_eq!(std::fs::read_to_string(home.join(".agents/skills/dig/SKILL.md")).unwrap(), "changed");
    assert!(wired(&home.join(".claude/skills/dig"), &home.join(".agents/skills/dig")));
    cfg.apply(SharedAction::ResolveSkill { path: path(".codex/skills/dig"), keep: Keep::Shared }, &p, &agents).await.unwrap();
    assert!(!home.join(".codex/skills/dig").exists());
    assert!(cfg.view(p.clone(), agents.clone(), caps()).await.private_skills.is_empty());

    // Deleting a shared skill backs it up and prunes the agents' links to it; a private one goes the same way
    std::fs::create_dir_all(home.join(".codex/skills/own")).unwrap();
    std::fs::write(home.join(".codex/skills/own/SKILL.md"), "own").unwrap();
    cfg.apply(SharedAction::RemoveSkill { path: path(".agents/skills/solo") }, &p, &agents).await.unwrap();
    assert!(!home.join(".agents/skills/solo").exists());
    assert!(std::fs::symlink_metadata(home.join(".claude/skills/solo")).is_err());
    cfg.apply(SharedAction::RemoveSkill { path: path(".codex/skills/own") }, &p, &agents).await.unwrap();
    assert!(!home.join(".codex/skills/own").exists());
    assert!(cfg.apply(SharedAction::RemoveSkill { path: path(".agents") }, &p, &agents).await.is_err());
    let backups = t.path().join("data/backups/shared");
    let kept: Vec<_> = std::fs::read_dir(&backups).unwrap().flatten().flat_map(|d| std::fs::read_dir(d.path()).unwrap().flatten()).map(|e| e.file_name().to_string_lossy().into_owned()).collect();
    assert!(kept.iter().any(|n| n.ends_with(".agents_skills_solo")) && kept.iter().any(|n| n.ends_with(".codex_skills_own")));

    // No shared prompt yet: Grok's own file becomes it and Claude's differing one imports it, in one submit
    std::fs::write(home.join(".grok/AGENTS.md"), "# grok rules\n").unwrap();
    std::fs::write(home.join(".claude/CLAUDE.md"), "# claude rules\n").unwrap();
    let v = cfg.view(p.clone(), agents.clone(), caps()).await;
    assert!(!v.shared_prompt);
    let picks = vec![pick(&home.join(".claude/CLAUDE.md"), Choice::KeepShared), pick(&home.join(".grok/AGENTS.md"), Choice::KeepPrivate)];
    cfg.apply(SharedAction::Link { picks, auto: false }, &p, &agents).await.unwrap();
    assert_eq!(std::fs::read_to_string(home.join(".agents/AGENTS.md")).unwrap(), "# grok rules\n");
    assert!(wired(&home.join(".grok/AGENTS.md"), &home.join(".agents/AGENTS.md")));
    assert_eq!(std::fs::read_to_string(home.join(".claude/CLAUDE.md")).unwrap(), "@~/.agents/AGENTS.md\n");
    // Undo puts Claude's own file back
    cfg.apply(SharedAction::Unlink, &p, &agents).await.unwrap();
    assert_eq!(std::fs::read_to_string(home.join(".claude/CLAUDE.md")).unwrap(), "# claude rules\n");
  }

  // Antigravity reads a project's .agents/skills but not ~/.agents/skills: adopting its project skill leaves no link
  // behind in .gemini/skills, adopting a global one links it back into ~/.gemini/config/skills
  #[tokio::test]
  async fn an_adopted_antigravity_skill_is_linked_back_only_where_it_does_not_read_agents() {
    let (_t, p, cfg, _) = setup();
    let agents = vec!["antigravity".to_owned()];
    let (home, root) = (p.home.clone(), p.root.clone().unwrap());
    for dir in [root.join(".gemini/skills/lint"), home.join(".gemini/config/skills/notes")] {
      std::fs::create_dir_all(&dir).unwrap();
      std::fs::write(dir.join("SKILL.md"), "---\nname: x\n---\n").unwrap();
    }
    cfg.apply(SharedAction::ResolveSkill { path: s(&root.join(".gemini/skills/lint")), keep: Keep::Private }, &p, &agents).await.unwrap();
    assert!(root.join(".agents/skills/lint/SKILL.md").exists());
    assert!(std::fs::symlink_metadata(root.join(".gemini/skills/lint")).is_err());
    cfg.apply(SharedAction::ResolveSkill { path: s(&home.join(".gemini/config/skills/notes")), keep: Keep::Private }, &p, &agents).await.unwrap();
    assert!(home.join(".agents/skills/notes/SKILL.md").exists());
    assert!(wired(&home.join(".gemini/config/skills/notes"), &home.join(".agents/skills/notes")));
  }

  #[tokio::test]
  async fn undo_restores_a_private_skill_and_spares_a_repointed_link() {
    let (_t, p, cfg, agents) = setup();
    let home = p.home.clone();
    // Claude's own `dig` differs from the shared one; keeping the shared one sets it aside behind a link
    std::fs::create_dir_all(home.join(".claude/skills/dig")).unwrap();
    std::fs::write(home.join(".claude/skills/dig/SKILL.md"), "mine").unwrap();
    let solo = home.join(".claude/skills/solo");
    std::fs::create_dir_all(&solo).unwrap();
    std::fs::write(solo.join("SKILL.md"), "solo").unwrap();
    let path = |d: &str| s(&home.join(d));
    cfg.apply(SharedAction::ResolveSkill { path: path(".claude/skills/dig"), keep: Keep::Shared }, &p, &agents).await.unwrap();
    assert!(wired(&home.join(".claude/skills/dig"), &home.join(".agents/skills/dig")));
    cfg.apply(SharedAction::ResolveSkill { path: path(".claude/skills/solo"), keep: Keep::Private }, &p, &agents).await.unwrap();
    assert!(wired(&solo, &home.join(".agents/skills/solo")));
    // The user points the solo link at a skill of their own
    let custom = home.join("custom-solo");
    std::fs::create_dir_all(&custom).unwrap();
    crate::platform::files::remove_link(&solo).unwrap();
    crate::platform::files::create_link(&solo, &custom, &custom).unwrap();

    cfg.apply(SharedAction::Unlink, &p, &agents).await.unwrap();
    assert!(!links::is_link(&home.join(".claude/skills/dig")));
    assert_eq!(std::fs::read_to_string(home.join(".claude/skills/dig/SKILL.md")).unwrap(), "mine");
    assert!(wired(&solo, &custom));
  }

  #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
  async fn concurrent_mcp_adds_all_land() {
    let (_t, p, cfg, agents) = setup();
    let mut tasks = tokio::task::JoinSet::new();
    for i in 0..16 {
      let (p, cfg, agents) = (p.clone(), cfg.clone(), agents.clone());
      tasks.spawn(async move {
        let json = format!(r#"{{ "command": "srv{i}" }}"#);
        cfg.apply(SharedAction::AddMcp { scope: SharedScope::Global, json, name: Some(format!("s{i}")) }, &p, &agents).await.unwrap();
      });
    }
    while let Some(r) = tasks.join_next().await {
      r.unwrap();
    }
    let text = std::fs::read_to_string(p.mcp_file(SharedScope::Global).unwrap()).unwrap();
    let data: serde_json::Value = serde_json::from_str(&text).unwrap();
    assert_eq!(data["mcpServers"].as_object().unwrap().len(), 16);
  }

  #[tokio::test]
  async fn a_turned_off_agent_loses_its_links_and_gets_them_back() {
    let (_t, p, cfg, agents) = setup();
    let home = p.home.clone();
    let root = p.root.clone().unwrap();
    std::fs::write(home.join(".agents/AGENTS.md"), "# shared\n").unwrap();
    std::fs::write(home.join(".codex/AGENTS.md"), "# shared\n").unwrap();
    let (dig, release, codex, shared) = (home.join(".claude/skills/dig"), root.join(".claude/skills/release"), home.join(".codex/AGENTS.md"), home.join(".agents/AGENTS.md"));
    let dig_ok = || wired(&dig, &home.join(".agents/skills/dig"));
    let release_ok = || wired(&release, &root.join(".agents/skills/release"));
    let picks = vec![pick(&home.join(".claude/skills/dig"), Choice::Link), pick(&home.join(".codex/AGENTS.md"), Choice::Link)];
    cfg.view(p.clone(), agents.clone(), caps()).await;
    cfg.apply(SharedAction::Link { picks, auto: true }, &p, &agents).await.unwrap();
    assert!(release_ok() && dig_ok());

    // Claude off: its user and project links go, Codex's stays
    cfg.retire(&["claude".into()]).await;
    let on: Vec<String> = agents.iter().filter(|a| *a != "claude").cloned().collect();
    let v = cfg.view(p.clone(), on, caps()).await;
    assert!(std::fs::symlink_metadata(home.join(".claude/skills/dig")).is_err());
    assert!(std::fs::symlink_metadata(root.join(".claude/skills/release")).is_err());
    assert!(wired(&codex, &shared));
    assert!(!v.plan.iter().any(|x| x.agent == "claude"));
    // The identical Codex copy that was moved aside comes back when Codex is turned off
    cfg.retire(&["codex".into()]).await;
    assert_eq!(std::fs::read_to_string(home.join(".codex/AGENTS.md")).unwrap(), "# shared\n");
    assert!(!wired(&codex, &shared));

    // Back on: auto links them again
    cfg.view(p.clone(), agents.clone(), caps()).await;
    assert!(dig_ok() && release_ok());
    assert!(wired(&codex, &shared));
  }

  #[tokio::test]
  async fn pi_sees_project_skills_once_trusted() {
    // The env var would point the write at a real Pi store
    if std::env::var_os("PI_CODING_AGENT_DIR").is_some() {
      return;
    }
    let (_t, p, cfg, mut agents) = setup();
    agents.push("pi".into());
    let pi_reach = |v: &SharedView| {
      let skill = v.skills.iter().find(|x| x.name == "release").unwrap();
      skill.reach.iter().find(|r| r.agent == "pi").map(|r| r.state)
    };
    let v = cfg.view(p.clone(), agents.clone(), caps()).await;
    assert!(v.pi_untrusted);
    assert_eq!(pi_reach(&v), Some(ReachState::Untrusted));
    cfg.apply(SharedAction::TrustPi, &p, &agents).await.unwrap();
    let v = cfg.view(p.clone(), agents.clone(), caps()).await;
    assert!(!v.pi_untrusted);
    assert_eq!(pi_reach(&v), Some(ReachState::Native));
    // Global skills never need trust
    let dig = v.skills.iter().find(|x| x.name == "dig").unwrap();
    assert!(dig.reach.iter().any(|r| r.agent == "pi" && r.state == ReachState::Native));
  }

  #[tokio::test]
  async fn overwrite_wires_every_agent_and_turning_it_off_restores_them() {
    let (_t, p, cfg, agents) = setup();
    let home = p.home.clone();
    let (codex, claude, grok) = (home.join(".codex/AGENTS.md"), home.join(".claude/CLAUDE.md"), home.join(".grok/AGENTS.md"));
    let shared = home.join(".agents/AGENTS.md");
    let codex_own = "# 全局指令\n\n- 称呼：老板\n\n## 提交风格\n\n默认中文\n";
    let claude_own = "# 全局指令\n\n- 称呼：老板\n\n## Git\n\n- 不加水印\n```\nx\n```\n";
    std::fs::write(&codex, codex_own).unwrap();
    std::fs::write(&claude, claude_own).unwrap();
    // Claude's differing dig sits at a link point; Claude and Codex have skills of their own, Codex a differing dig too
    for (dir, body) in [(".claude/skills/dig", "mine"), (".claude/skills/only", "claude only"), (".codex/skills/solo", "codex solo"), (".codex/skills/dig", "codex dig")] {
      std::fs::create_dir_all(home.join(dir)).unwrap();
      std::fs::write(home.join(dir).join("SKILL.md"), body).unwrap();
    }
    // Grok's prompt was skipped in the link panel once
    cfg.apply(SharedAction::Link { picks: vec![pick(&grok, Choice::Skip)], auto: false }, &p, &agents).await.unwrap();

    // Without a shared prompt every line of the agents' own files is theirs alone
    let v = cfg.view(p.clone(), agents.clone(), caps()).await;
    let unique = |v: &SharedView, f: &Path| v.takeover.iter().find(|x| Path::new(&x.path) == f).map(|x| x.unique.clone());
    assert_eq!(unique(&v, &codex).as_deref(), Some(codex_own.trim_end()));
    assert!(unique(&v, &claude).is_some());

    // Merging both: Claude adds only what Codex lacks, fences kept; then everything is wired, the skipped point too
    let merge = vec![s(&codex), s(&claude)];
    cfg.apply(SharedAction::Overwrite { on: true, merge }, &p, &agents).await.unwrap();
    assert_eq!(std::fs::read_to_string(&shared).unwrap(), "# 全局指令\n\n- 称呼：老板\n\n## 提交风格\n\n默认中文\n\n## Git\n\n- 不加水印\n```\nx\n```\n");
    assert!(wired(&codex, &shared) && wired(&grok, &shared));
    assert_eq!(std::fs::read_to_string(&claude).unwrap(), "@~/.agents/AGENTS.md\n");
    assert!(wired(&home.join(".claude/skills/dig"), &home.join(".agents/skills/dig")));
    // The agents' own skills join the shared folder (Claude gets its one back as a link); Codex's differing dig is set aside
    assert_eq!(std::fs::read_to_string(home.join(".agents/skills/only/SKILL.md")).unwrap(), "claude only");
    assert_eq!(std::fs::read_to_string(home.join(".agents/skills/solo/SKILL.md")).unwrap(), "codex solo");
    assert!(wired(&home.join(".claude/skills/only"), &home.join(".agents/skills/only")) && wired(&home.join(".claude/skills/solo"), &home.join(".agents/skills/solo")));
    assert!(std::fs::symlink_metadata(home.join(".codex/skills/solo")).is_err() && std::fs::symlink_metadata(home.join(".codex/skills/dig")).is_err());
    let v = cfg.view(p.clone(), agents.clone(), caps()).await;
    assert!(v.overwrite && v.takeover.is_empty() && v.plan.is_empty());
    assert!(!v.private_skills.iter().any(|x| x.scope == SharedScope::Global));

    // A link replaced by a file is wired again on the next look
    std::fs::remove_file(&codex).unwrap();
    std::fs::write(&codex, "# drift\n").unwrap();
    cfg.view(p.clone(), agents.clone(), caps()).await;
    assert!(wired(&codex, &shared));

    // The page's editor writes through to every agent; an edit started from an older text is refused
    let base = std::fs::read_to_string(&shared).unwrap();
    cfg.apply(SharedAction::SavePrompt { scope: SharedScope::Global, text: "# new\n".into(), base: base.clone() }, &p, &agents).await.unwrap();
    assert_eq!(std::fs::read_to_string(&codex).unwrap(), "# new\n");
    assert!(cfg.apply(SharedAction::SavePrompt { scope: SharedScope::Global, text: "# lost\n".into(), base }, &p, &agents).await.is_err());
    assert_eq!(std::fs::read_to_string(&shared).unwrap(), "# new\n");

    // Off: every agent gets its own file back (the first one, not the drift); the shared prompt stays
    cfg.apply(SharedAction::Overwrite { on: false, merge: vec![] }, &p, &agents).await.unwrap();
    assert_eq!(std::fs::read_to_string(&codex).unwrap(), codex_own);
    assert_eq!(std::fs::read_to_string(&claude).unwrap(), claude_own);
    assert_eq!(std::fs::read_to_string(home.join(".claude/skills/dig/SKILL.md")).unwrap(), "mine");
    for (dir, body) in [(".claude/skills/only", "claude only"), (".codex/skills/solo", "codex solo"), (".codex/skills/dig", "codex dig")] {
      assert!(!links::is_link(&home.join(dir)));
      assert_eq!(std::fs::read_to_string(home.join(dir).join("SKILL.md")).unwrap(), body);
    }
    assert!(!home.join(".agents/skills/only").exists() && !home.join(".agents/skills/solo").exists());
    assert!(std::fs::symlink_metadata(&grok).is_err());
    assert_eq!(std::fs::read_to_string(&shared).unwrap(), "# new\n");
    assert!(!cfg.view(p.clone(), agents.clone(), caps()).await.overwrite);
    assert!(!cfg.ledger.read().await.entries.iter().any(|e| e.overwrite));
  }

  #[tokio::test]
  async fn overwrite_takes_over_mixed_and_unreadable_files_and_rewired_panel_links() {
    let (_t, p, cfg, agents) = setup();
    let home = p.home.clone();
    let (claude, codex, grok) = (home.join(".claude/CLAUDE.md"), home.join(".codex/AGENTS.md"), home.join(".grok/AGENTS.md"));
    std::fs::write(home.join(".agents/AGENTS.md"), "# shared\n").unwrap();
    // Claude imports the shared prompt but adds rules of its own; Codex's file is not UTF-8
    let mixed = "@~/.agents/AGENTS.md\n\n- claude only rule\n";
    std::fs::write(&claude, mixed).unwrap();
    std::fs::write(&codex, b"\xff\xfe rules").unwrap();
    // Grok was linked by the panel, then replaced by a file of the user's own
    std::fs::write(&grok, "# shared\n").unwrap();
    cfg.apply(SharedAction::Link { picks: vec![pick(&grok, Choice::Link)], auto: false }, &p, &agents).await.unwrap();
    std::fs::remove_file(&grok).unwrap();
    std::fs::write(&grok, "# grok own\n").unwrap();

    let v = cfg.view(p.clone(), agents.clone(), caps()).await;
    let claude_take = v.takeover.iter().find(|x| Path::new(&x.path) == claude).unwrap();
    assert_eq!(claude_take.unique, "- claude only rule");
    cfg.apply(SharedAction::Overwrite { on: true, merge: vec![] }, &p, &agents).await.unwrap();
    assert_eq!(std::fs::read_to_string(&claude).unwrap(), "@~/.agents/AGENTS.md\n");
    let shared = home.join(".agents/AGENTS.md");
    assert!(wired(&codex, &shared) && wired(&grok, &shared));

    // Off: each file comes back as it was right before overwrite, the unreadable one byte for byte
    cfg.apply(SharedAction::Overwrite { on: false, merge: vec![] }, &p, &agents).await.unwrap();
    assert_eq!(std::fs::read_to_string(&claude).unwrap(), mixed);
    assert_eq!(std::fs::read(&codex).unwrap(), b"\xff\xfe rules");
    assert_eq!(std::fs::read_to_string(&grok).unwrap(), "# grok own\n");
    // Saving over a prompt that cannot be read is refused rather than taken for an empty file
    std::fs::write(home.join(".agents/AGENTS.md"), b"\xff").unwrap();
    assert!(cfg.apply(SharedAction::SavePrompt { scope: SharedScope::Global, text: "x".into(), base: String::new() }, &p, &agents).await.is_err());
  }

  #[tokio::test]
  async fn a_deleted_shared_skill_keeps_the_private_backup_on_record() {
    let (_t, p, cfg, agents) = setup();
    let home = p.home.clone();
    std::fs::create_dir_all(home.join(".claude/skills/dig")).unwrap();
    std::fs::write(home.join(".claude/skills/dig/SKILL.md"), "mine").unwrap();
    cfg.apply(SharedAction::Overwrite { on: true, merge: vec![] }, &p, &agents).await.unwrap();
    assert!(wired(&home.join(".claude/skills/dig"), &home.join(".agents/skills/dig")));
    cfg.apply(SharedAction::RemoveSkill { path: s(&home.join(".agents/skills/dig")) }, &p, &agents).await.unwrap();
    assert!(std::fs::symlink_metadata(home.join(".claude/skills/dig")).is_err());
    cfg.apply(SharedAction::Overwrite { on: false, merge: vec![] }, &p, &agents).await.unwrap();
    assert_eq!(std::fs::read_to_string(home.join(".claude/skills/dig/SKILL.md")).unwrap(), "mine");
  }

  #[test]
  fn unique_lines_keep_order_paragraphs_and_fences() {
    let base = "# A\n\n- one\n```\n";
    assert_eq!(view::unique_lines("# A\n- one\n- two\n\n@~/.agents/AGENTS.md\n\n```\ncode\n```\n", base, &[CLAUDE_GLOBAL_IMPORT.to_owned()]), "- two\n\n```\ncode\n```");
    assert_eq!(view::unique_lines("# A\n\n- one\n", base, &[]), "");
    // A fence with a language tag the shared prompt also has is kept, or the block would lose its opening
    assert_eq!(view::unique_lines("~~~sh\necho new\n~~~\n", "~~~sh\necho old\n~~~\n", &[]), "~~~sh\necho new\n~~~");
  }

  #[tokio::test]
  async fn project_claude_md_gets_an_import() {
    let (_t, p, cfg, agents) = setup();
    let root = p.root.clone().unwrap();
    std::fs::write(root.join("AGENTS.md"), "# project\n").unwrap();
    std::fs::write(root.join("CLAUDE.md"), "# claude only\n").unwrap();
    let v = cfg.view(p.clone(), agents.clone(), caps()).await;
    let project = v.prompts.iter().find(|x| x.scope == SharedScope::Project).unwrap();
    assert!(project.reach.iter().any(|r| r.agent == "claude" && r.state == ReachState::Conflict));
    cfg.apply(SharedAction::ClaudeImport, &p, &agents).await.unwrap();
    assert_eq!(std::fs::read_to_string(root.join("CLAUDE.md")).unwrap(), "@AGENTS.md\n\n# claude only\n");
    let v = cfg.view(p.clone(), agents.clone(), caps()).await;
    let project = v.prompts.iter().find(|x| x.scope == SharedScope::Project).unwrap();
    assert!(project.reach.iter().any(|r| r.agent == "claude" && r.state == ReachState::Native));
    // A project-level entry: the user-level undo leaves it alone
    cfg.apply(SharedAction::Unlink, &p, &agents).await.unwrap();
    assert_eq!(std::fs::read_to_string(root.join("CLAUDE.md")).unwrap(), "@AGENTS.md\n\n# claude only\n");
  }
}
