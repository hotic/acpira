//! The updater: the state the settings page, the toast and the menu show, and the check → download → stage cycle
//! behind it. `Idle → Checking → (UpToDate | Available) → Downloading → Staging → Ready → Applying`; a failure lands in
//! `Failed` and the next cycle (or a manual check) starts over, resuming a partial download. Blocking work runs on a
//! thread of its own; `on_change` gets every new state there

use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use serde::Serialize;

use super::delta;
use super::fetch::{self, Policy};
use super::manifest::{self, MAX_MANIFEST};
use super::stage;
use super::swap::{self, ManualReason};

/// A delta larger than this share of the full package is not worth it: the full package is taken instead
const DELTA_MAX_SHARE: f64 = 0.6;
/// How often download progress is pushed at most
const PROGRESS_EVERY: Duration = Duration::from_millis(250);

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum Phase {
  Idle,
  Checking,
  UpToDate,
  Available,
  Downloading,
  Staging,
  Ready,
  Applying,
  Failed,
}

/// What the page shows (`UpdateState` in packages/shared/protocol.ts)
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct UpdateState {
  pub phase: Phase,
  /// This build's version
  pub current: String,
  /// The newer version found (Available and after)
  #[serde(skip_serializing_if = "Option::is_none")]
  pub version: Option<String>,
  #[serde(skip_serializing_if = "Option::is_none")]
  pub notes: Option<String>,
  /// Download progress in bytes
  #[serde(skip_serializing_if = "Option::is_none")]
  pub done: Option<u64>,
  #[serde(skip_serializing_if = "Option::is_none")]
  pub total: Option<u64>,
  #[serde(skip_serializing_if = "Option::is_none")]
  pub error: Option<String>,
  /// Set when this install only links to the release page
  #[serde(skip_serializing_if = "Option::is_none")]
  pub manual: Option<ManualReason>,
  /// When the last check finished (ms since the epoch)
  #[serde(skip_serializing_if = "Option::is_none")]
  pub checked_at: Option<i64>,
  /// Download and stage new versions on their own
  pub auto: bool,
  pub release_url: String,
}

pub struct Config {
  pub manifest_url: String,
  pub keys: Vec<[u8; 32]>,
  pub policy: Policy,
  /// The code requirement a staged bundle must meet; None skips codesign (unit tests)
  pub requirement: Option<String>,
  /// Gatekeeper's assessment of the staged bundle (notarization)
  pub notarized: bool,
  pub release_url: String,
}

/// Everything the updater needs from the shell
pub struct Env {
  pub config: Config,
  /// The installed `.app` (None: not running from a bundle)
  pub installed: Option<PathBuf>,
  /// The updater's own cache folder (downloads, and staged bundles when on the install's volume)
  pub cache: PathBuf,
  pub current: String,
  pub os_version: Option<String>,
  /// Known before any check (the install's location, its signature, the build's configuration)
  pub manual: Option<ManualReason>,
  /// The proxy in effect when a cycle starts
  pub proxy: Box<dyn Fn() -> Option<String> + Send + Sync>,
  pub log: Arc<dyn Fn(&str) + Send + Sync>,
}

/// A bundle ready to take the installed one's place
#[derive(Debug, Clone)]
pub struct Staged {
  pub version: String,
  pub path: PathBuf,
  /// The folder it was assembled in, removed after the swap
  pub dir: PathBuf,
}

type Listener = Box<dyn Fn(&UpdateState) + Send + Sync>;

pub struct Updater {
  env: Env,
  state: Mutex<UpdateState>,
  running: AtomicBool,
  cancel: AtomicBool,
  staged: Mutex<Option<Staged>>,
  on_change: Listener,
}

fn now_ms() -> i64 {
  std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_millis() as i64).unwrap_or(0)
}

impl Updater {
  pub fn new(env: Env, auto: bool, on_change: impl Fn(&UpdateState) + Send + Sync + 'static) -> Arc<Updater> {
    let manual = if env.config.keys.is_empty() { Some(ManualReason::Unconfigured) } else { env.manual };
    let state = UpdateState {
      phase: Phase::Idle,
      current: env.current.clone(),
      version: None,
      notes: None,
      done: None,
      total: None,
      error: None,
      manual,
      checked_at: None,
      auto,
      release_url: env.config.release_url.clone(),
    };
    Arc::new(Updater {
      env,
      state: Mutex::new(state),
      running: AtomicBool::new(false),
      cancel: AtomicBool::new(false),
      staged: Mutex::new(None),
      on_change: Box::new(on_change),
    })
  }

  pub fn state(&self) -> UpdateState {
    self.state.lock().unwrap().clone()
  }

  fn set(&self, f: impl FnOnce(&mut UpdateState)) {
    let snap = {
      let mut s = self.state.lock().unwrap();
      f(&mut s);
      s.clone()
    };
    (self.on_change)(&snap);
  }

  fn log(&self, line: &str) {
    (self.env.log)(&format!("update: {line}"));
  }

  /// Whether checks can run at all (an unconfigured build never asks the network)
  pub fn enabled(&self) -> bool {
    self.state().manual != Some(ManualReason::Unconfigured)
  }

  /// The installed bundle this updater replaces
  pub fn installed(&self) -> Option<&std::path::Path> {
    self.env.installed.as_deref()
  }

  pub fn staged(&self) -> Option<Staged> {
    self.staged.lock().unwrap().clone()
  }

  /// The automatic download switch; turned on while a version waits, that version is fetched now
  pub fn set_auto(self: &Arc<Self>, on: bool) {
    self.set(|s| s.auto = on);
    if on && self.state().phase == Phase::Available {
      self.check(true);
    }
  }

  /// A check on a thread of its own, unless one is running; `download` fetches and stages a newer version, otherwise it
  /// is only reported
  pub fn check(self: &Arc<Self>, download: bool) {
    if !self.enabled() || self.running.swap(true, Ordering::SeqCst) {
      return;
    }
    let me = self.clone();
    std::thread::Builder::new().name("update".into()).spawn(move || me.cycle(download)).map(drop).unwrap_or_else(|e| {
      self.running.store(false, Ordering::SeqCst);
      self.log(&format!("no thread: {e}"));
    });
  }

  /// One cycle on the calling thread; `running` is already held
  fn cycle(&self, download: bool) {
    if let Err(e) = self.run(download) {
      self.log(&e);
      if e != "cancelled" {
        self.set(|s| {
          s.phase = Phase::Failed;
          s.error = Some(e);
          s.done = None;
          s.total = None;
        });
      }
    }
    self.running.store(false, Ordering::SeqCst);
  }

  /// A check right away, then one every `every`, each downloading when the switch is on; the first waits `first` so
  /// the launch is not slowed
  pub fn schedule(self: &Arc<Self>, first: Duration, every: Duration) {
    if !self.enabled() {
      return;
    }
    let me = Arc::downgrade(self);
    let _ = std::thread::Builder::new().name("update-schedule".into()).spawn(move || {
      std::thread::sleep(first);
      loop {
        let Some(u) = me.upgrade() else { return };
        if !u.running.swap(true, Ordering::SeqCst) {
          let auto = u.state().auto;
          u.cycle(auto);
        }
        drop(u);
        std::thread::sleep(every);
      }
    });
  }

  /// Stops a download between chunks (quitting); the partial file stays for the next launch
  pub fn stop(&self) {
    self.cancel.store(true, Ordering::SeqCst);
  }

  /// The blocking cycle: manifest, then (when asked and possible) download, assemble and verify
  pub fn run(&self, download: bool) -> Result<(), String> {
    if self.staged().is_some() {
      // One update per run of the app: the staged version waits for the restart
      return Ok(());
    }
    let cfg = &self.env.config;
    self.set(|s| {
      s.phase = Phase::Checking;
      s.error = None;
    });
    let agent = fetch::agent((self.env.proxy)().as_deref());
    let bytes = fetch::get_small(&agent, &cfg.policy, &cfg.manifest_url, MAX_MANIFEST)?;
    let sig = fetch::get_small(&agent, &cfg.policy, &format!("{}.sig", cfg.manifest_url), 1024)?;
    let m = manifest::verify(&bytes, &sig, &cfg.keys)?;
    let build = manifest::platform_key().and_then(|k| m.platforms.get(&k).cloned());
    let checked = now_ms();
    let Some(build) = build.filter(|_| manifest::is_newer(&m.version, &self.env.current)) else {
      self.log(&format!("{} is current (latest {})", self.env.current, m.version));
      self.set(|s| {
        s.phase = Phase::UpToDate;
        s.version = None;
        s.notes = None;
        s.checked_at = Some(checked);
      });
      return Ok(());
    };
    let manual = self.env.manual.or_else(|| {
      (!manifest::os_supports(m.minimum_system_version.as_deref(), self.env.os_version.as_deref())).then_some(ManualReason::System)
    });
    let version = m.version.clone();
    self.log(&format!("{version} is available"));
    self.set(|s| {
      s.phase = Phase::Available;
      s.version = Some(version.clone());
      s.notes = (!m.notes.is_empty()).then(|| m.notes.clone());
      s.manual = manual;
      s.checked_at = Some(checked);
    });
    if manual.is_some() || !download {
      return Ok(());
    }
    let installed = self.env.installed.clone().ok_or("not running from an app bundle")?;
    let dir = stage::staging_dir(&installed, &self.env.cache, &version);
    self.prune(&version);
    std::fs::create_dir_all(&dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    let app = dir.join("Acpira.app");
    remove_any(&app)?;
    let mut built = false;
    let delta = build.deltas.get(&self.env.current).filter(|d| d.size as f64 <= build.full.size as f64 * DELTA_MAX_SHARE);
    if let Some(asset) = delta {
      let file = self.env.cache.join(&version).join("update.delta");
      self.fetch(&agent, asset, &file)?;
      self.set(|s| s.phase = Phase::Staging);
      match delta::apply_delta(&file, &installed, &app).and_then(|_| delta::verify_tree(&app, &build.files)) {
        Ok(()) => built = true,
        Err(e) => {
          self.log(&format!("the delta did not apply ({e}); taking the full package"));
          remove_any(&app)?;
        }
      }
      let _ = std::fs::remove_file(&file);
    }
    if !built {
      let file = self.env.cache.join(&version).join("update.tar.zst");
      self.fetch(&agent, &build.full, &file)?;
      self.set(|s| s.phase = Phase::Staging);
      let unpacked = delta::unpack_full(&file, &app).and_then(|_| delta::verify_tree(&app, &build.files));
      if let Err(e) = unpacked {
        // A verified download that unpacks wrong is not worth keeping either
        let _ = std::fs::remove_file(&file);
        let _ = remove_any(&app);
        return Err(e);
      }
      let _ = std::fs::remove_file(&file);
    }
    if let Some(req) = &cfg.requirement {
      stage::verify_signature(&app, req).inspect_err(|_| _ = remove_any(&app))?;
    }
    if cfg.notarized {
      stage::verify_notarized(&app).inspect_err(|_| _ = remove_any(&app))?;
    }
    self.log(&format!("{version} is staged at {}", app.display()));
    *self.staged.lock().unwrap() = Some(Staged { version, path: app, dir });
    self.set(|s| {
      s.phase = Phase::Ready;
      s.done = None;
      s.total = None;
    });
    Ok(())
  }

  fn fetch(&self, agent: &ureq::Agent, asset: &manifest::Asset, to: &std::path::Path) -> Result<(), String> {
    if let Some(dir) = to.parent() {
      std::fs::create_dir_all(dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    }
    self.cancel.store(false, Ordering::SeqCst);
    let mut last: Option<Instant> = None;
    let mut progress = |done: u64, total: u64| {
      if last.is_some_and(|t| t.elapsed() < PROGRESS_EVERY) && done < total {
        return;
      }
      last = Some(Instant::now());
      self.set(|s| {
        s.phase = Phase::Downloading;
        s.done = Some(done);
        s.total = Some(total);
      });
    };
    fetch::download(agent, &self.env.config.policy, asset, to, &mut progress, &self.cancel)
  }

  /// Earlier versions' downloads and staged bundles go; this version's partial download stays for the resume
  fn prune(&self, keep: &str) {
    if let Ok(entries) = std::fs::read_dir(&self.env.cache) {
      for e in entries.flatten() {
        if e.file_name().to_str() != Some(keep) {
          let _ = remove_any(&e.path());
        }
      }
    }
    if let Some(parent) = self.env.installed.as_ref().and_then(|i| i.parent())
      && let Ok(entries) = std::fs::read_dir(parent)
    {
      let own = format!(".Acpira-{keep}.staging");
      for e in entries.flatten() {
        let name = e.file_name().to_string_lossy().into_owned();
        if name.starts_with(".Acpira-") && name.ends_with(".staging") && name != own {
          let _ = remove_any(&e.path());
        }
      }
    }
  }

  /// Puts the staged bundle in the installed one's place (at quit, or before a restart) and deletes the old one.
  /// Returns the version now installed
  pub fn apply(&self) -> Result<String, String> {
    let staged = self.staged.lock().unwrap().take().ok_or("no update is ready")?;
    let installed = self.env.installed.clone().ok_or("not running from an app bundle")?;
    self.set(|s| s.phase = Phase::Applying);
    if let Err(e) = swap::swap(&staged.path, &installed) {
      let msg = format!("could not replace {}: {e}", installed.display());
      self.log(&msg);
      *self.staged.lock().unwrap() = Some(staged);
      self.set(|s| {
        s.phase = Phase::Failed;
        s.error = Some(msg.clone());
      });
      return Err(msg);
    }
    // The old bundle now sits where the new one was assembled
    let _ = remove_any(&staged.dir);
    self.log(&format!("installed {}", staged.version));
    Ok(staged.version)
  }
}

fn remove_any(p: &std::path::Path) -> Result<(), String> {
  match std::fs::symlink_metadata(p) {
    Ok(m) if m.is_dir() => std::fs::remove_dir_all(p),
    Ok(_) => std::fs::remove_file(p),
    Err(_) => Ok(()),
  }
  .map_err(|e| format!("{}: {e}", p.display()))
}

#[cfg(test)]
mod tests {
  use super::*;
  use crate::update::fetch::tests::local;
  use crate::update::manifest::{Asset, Manifest, PlatformBuild, hex, public_key, sign};
  use sha2::{Digest, Sha256};
  use std::collections::BTreeMap;
  use std::path::Path;

  const SEED: [u8; 32] = [3; 32];

  fn put(root: &Path, rel: &str, bytes: &[u8]) {
    let p = root.join(rel);
    std::fs::create_dir_all(p.parent().unwrap()).unwrap();
    std::fs::write(p, bytes).unwrap();
  }

  fn bundle(root: &Path, version: &str) {
    // Incompressible, like a binary: only a patch against the installed copy makes it small
    let mut x = 88172645463325252u64;
    let mut engine: Vec<u8> = (0..120_000)
      .map(|_| {
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        (x >> 24) as u8
      })
      .collect();
    engine[500..500 + version.len()].copy_from_slice(version.as_bytes());
    put(root, "Contents/Info.plist", format!("<plist>{version}</plist>").as_bytes());
    put(root, "Contents/MacOS/acpira-engine", &engine);
    put(root, "Contents/Resources/webview/main.js", b"console.log(1)");
  }

  fn asset(base: &str, name: &str, body: &[u8]) -> Asset {
    Asset { url: format!("{base}/redirect/{name}"), size: body.len() as u64, sha256: hex(&Sha256::digest(body)) }
  }

  struct Release {
    _srv: crate::update::fetch::tests::Server,
    url: String,
  }

  /// Packs `new` as version 1.1.0 (full package and a delta from `old` as 1.0.0) and serves it with its signed manifest
  fn release(dir: &Path, old: &Path, new: &Path, broken_delta: bool) -> Release {
    let full = dir.join("full.tar.zst");
    let files = delta::pack_full(new, &full, 3).unwrap();
    let dpath = dir.join("d.delta");
    delta::make_delta(old, new, "1.0.0", "1.1.0", &dpath, 3).unwrap();
    let (full_bytes, mut delta_bytes) = (std::fs::read(&full).unwrap(), std::fs::read(&dpath).unwrap());
    if broken_delta {
      // A delta from another 1.0.0: its sources do not match the installed files
      let other = dir.join("other.app");
      bundle(&other, "9.9.9");
      delta::make_delta(&other, new, "1.0.0", "1.1.0", &dpath, 3).unwrap();
      delta_bytes = std::fs::read(&dpath).unwrap();
    }
    let mut files_srv = vec![("full.tar.zst".to_owned(), full_bytes.clone()), ("d.delta".to_owned(), delta_bytes.clone())];
    let srv = serve_with_manifest(&mut files_srv, |base| {
      let m = Manifest {
        schema: 1,
        version: "1.1.0".into(),
        notes: "Fixes".into(),
        minimum_system_version: None,
        platforms: BTreeMap::from([(
          manifest::platform_key().unwrap(),
          PlatformBuild {
            full: asset(base, "full.tar.zst", &full_bytes),
            files: files.clone(),
            deltas: BTreeMap::from([("1.0.0".to_owned(), asset(base, "d.delta", &delta_bytes))]),
          },
        )]),
      };
      serde_json::to_vec(&m).unwrap()
    });
    let url = format!("{}/desktop-update.json", srv.base);
    Release { _srv: srv, url }
  }

  /// The packages plus a signed manifest naming them on the server's own address
  fn serve_with_manifest(
    files: &mut Vec<(String, Vec<u8>)>,
    manifest: impl FnOnce(&str) -> Vec<u8>,
  ) -> crate::update::fetch::tests::Server {
    let mut files = std::mem::take(files);
    crate::update::fetch::tests::serve_built(false, move |base| {
      let bytes = manifest(base);
      let sig = sign(&bytes, &SEED).unwrap();
      files.push(("desktop-update.json".into(), bytes));
      files.push(("desktop-update.json.sig".into(), sig.into_bytes()));
      files
    })
  }

  type Seen = Arc<Mutex<Vec<Phase>>>;
  type Lines = Arc<Mutex<Vec<String>>>;

  fn updater(dir: &Path, installed: PathBuf, url: String, current: &str) -> (Arc<Updater>, Seen, Lines) {
    let seen = Arc::new(Mutex::new(vec![]));
    let w = seen.clone();
    let lines: Lines = Arc::new(Mutex::new(vec![]));
    let l = lines.clone();
    let env = Env {
      config: Config {
        manifest_url: url,
        keys: vec![public_key(&SEED).unwrap()],
        policy: local(),
        requirement: None,
        notarized: false,
        release_url: "https://github.com/hotic/acpira/releases/latest".into(),
      },
      installed: Some(installed),
      cache: dir.join("cache/updates"),
      current: current.into(),
      os_version: Some("26.0".into()),
      manual: None,
      proxy: Box::new(|| None),
      log: Arc::new(move |line| l.lock().unwrap().push(line.to_owned())),
    };
    let u = Updater::new(env, true, move |s| {
      let mut v = w.lock().unwrap();
      if v.last() != Some(&s.phase) {
        v.push(s.phase);
      }
    });
    (u, seen, lines)
  }

  #[test]
  fn a_newer_release_is_patched_in_staged_and_swapped_into_place() {
    let d = tempfile::tempdir().unwrap();
    let (installed, new) = (d.path().join("Applications/Acpira.app"), d.path().join("build/Acpira.app"));
    bundle(&installed, "1.0.0");
    bundle(&new, "1.1.0");
    let r = release(d.path(), &installed, &new, false);
    let (u, seen, lines) = updater(d.path(), installed.clone(), r.url.clone(), "1.0.0");
    u.run(true).unwrap();
    assert!(!lines.lock().unwrap().iter().any(|l| l.contains("did not apply")), "{:?}", lines.lock().unwrap());
    let s = u.state();
    assert_eq!((s.phase, s.version.as_deref(), s.notes.as_deref()), (Phase::Ready, Some("1.1.0"), Some("Fixes")));
    assert_eq!(*seen.lock().unwrap(), [Phase::Checking, Phase::Available, Phase::Downloading, Phase::Staging, Phase::Ready]);
    let staged = u.staged().unwrap();
    delta::verify_tree(&staged.path, &delta::scan(&new).unwrap()).unwrap();
    // Nothing of the download is left beside the staged bundle
    assert_eq!(std::fs::read_dir(&staged.dir).unwrap().count(), 1);
    assert_eq!(u.apply().unwrap(), "1.1.0");
    delta::verify_tree(&installed, &delta::scan(&new).unwrap()).unwrap();
    assert!(!staged.dir.exists());
    assert!(u.apply().is_err());
  }

  #[test]
  fn an_installed_copy_the_delta_does_not_fit_takes_the_full_package() {
    let d = tempfile::tempdir().unwrap();
    let (installed, new) = (d.path().join("Applications/Acpira.app"), d.path().join("build/Acpira.app"));
    bundle(&installed, "1.0.0");
    bundle(&new, "1.1.0");
    let r = release(d.path(), &installed, &new, true);
    let (u, _, lines) = updater(d.path(), installed.clone(), r.url.clone(), "1.0.0");
    u.run(true).unwrap();
    assert_eq!(u.state().phase, Phase::Ready);
    assert!(
      lines.lock().unwrap().iter().any(|l| l.contains("did not apply") && l.contains("is not the one 1.0.0 shipped")),
      "{:?}",
      lines.lock().unwrap()
    );
    delta::verify_tree(&u.staged().unwrap().path, &delta::scan(&new).unwrap()).unwrap();
  }

  #[test]
  fn the_current_version_a_manual_install_or_a_bad_signature_downloads_nothing() {
    let d = tempfile::tempdir().unwrap();
    let (installed, new) = (d.path().join("Applications/Acpira.app"), d.path().join("build/Acpira.app"));
    bundle(&installed, "1.0.0");
    bundle(&new, "1.1.0");
    let r = release(d.path(), &installed, &new, false);
    let (u, _, _) = updater(d.path(), installed.clone(), r.url.clone(), "1.1.0");
    u.run(true).unwrap();
    assert_eq!(u.state().phase, Phase::UpToDate);
    assert!(u.state().checked_at.is_some());
    // Auto off: reported, not fetched
    let (u, _, _) = updater(d.path(), installed.clone(), r.url.clone(), "1.0.0");
    u.run(false).unwrap();
    assert_eq!((u.state().phase, u.state().version.as_deref()), (Phase::Available, Some("1.1.0")));
    assert!(!d.path().join("cache/updates/1.1.0").exists());
    // A translocated or read-only install: reported with the reason
    let (mut u, _, _) = updater(d.path(), installed.clone(), r.url.clone(), "1.0.0");
    Arc::get_mut(&mut u).unwrap().env.manual = Some(ManualReason::Translocated);
    u.run(true).unwrap();
    assert_eq!((u.state().phase, u.state().manual), (Phase::Available, Some(ManualReason::Translocated)));
    // A manifest signed with another key fails the cycle and says so
    let (mut u, _, _) = updater(d.path(), installed.clone(), r.url.clone(), "1.0.0");
    Arc::get_mut(&mut u).unwrap().env.config.keys = vec![public_key(&[4; 32]).unwrap()];
    u.cycle(true);
    let s = u.state();
    assert_eq!(s.phase, Phase::Failed);
    assert!(s.error.unwrap().contains("signature"));
    // No key at all: never checks
    let (mut u, _, _) = updater(d.path(), installed, r.url, "1.0.0");
    Arc::get_mut(&mut u).unwrap().env.config.keys.clear();
    let u = Updater::new(Arc::try_unwrap(u).ok().unwrap().env, true, |_| {});
    assert_eq!(u.state().manual, Some(ManualReason::Unconfigured));
    assert!(!u.enabled());
  }

  #[test]
  fn the_state_serializes_as_the_page_reads_it() {
    let d = tempfile::tempdir().unwrap();
    let (u, _, _) = updater(d.path(), d.path().join("A.app"), "http://127.0.0.1:1/x".into(), "1.10.8");
    assert_eq!(
      serde_json::to_value(u.state()).unwrap(),
      serde_json::json!({ "phase": "idle", "current": "1.10.8", "auto": true, "releaseUrl": "https://github.com/hotic/acpira/releases/latest" })
    );
    let mut s = u.state();
    s.phase = Phase::Downloading;
    s.done = Some(5);
    s.total = Some(10);
    s.manual = Some(ManualReason::NotWritable);
    let v = serde_json::to_value(s).unwrap();
    assert_eq!((v["phase"].as_str(), v["done"].as_u64(), v["manual"].as_str()), (Some("downloading"), Some(5), Some("notWritable")));
  }
}
