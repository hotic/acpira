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

/// Packs `new` (full package and a delta from `old` as 1.0.0) and serves it with its signed manifest
fn release(dir: &Path, old: &Path, new: &Path, version: &str, broken_delta: bool) -> Release {
  let full = dir.join("full.tar.zst");
  let files = delta::pack_full(new, &full, 3).unwrap();
  let dpath = dir.join("d.delta");
  delta::make_delta(old, new, "1.0.0", version, &dpath, 3).unwrap();
  let (full_bytes, mut delta_bytes) = (std::fs::read(&full).unwrap(), std::fs::read(&dpath).unwrap());
  if broken_delta {
    // A delta from another 1.0.0: its sources do not match the installed files
    let other = dir.join("other.app");
    bundle(&other, "9.9.9");
    delta::make_delta(&other, new, "1.0.0", version, &dpath, 3).unwrap();
    delta_bytes = std::fs::read(&dpath).unwrap();
  }
  let mut files_srv = vec![("full.tar.zst".to_owned(), full_bytes.clone()), ("d.delta".to_owned(), delta_bytes.clone())];
  let srv = serve_with_manifest(&mut files_srv, |base| {
    let m = Manifest {
      schema: 1,
      version: version.into(),
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
    log: Arc::new(move |line| l.lock().push(line.to_owned())),
  };
  let u = Updater::new(env, true, move |s| {
    let mut v = w.lock();
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
  let r = release(d.path(), &installed, &new, "1.1.0", false);
  let (u, seen, lines) = updater(d.path(), installed.clone(), r.url.clone(), "1.0.0");
  u.run(true).unwrap();
  assert!(!lines.lock().iter().any(|l| l.contains("did not apply")), "{:?}", lines.lock());
  let s = u.state();
  assert_eq!((s.phase, s.version.as_deref(), s.notes.as_deref()), (Phase::Ready, Some("1.1.0"), Some("Fixes")));
  assert_eq!(*seen.lock(), [Phase::Checking, Phase::Available, Phase::Downloading, Phase::Staging, Phase::Ready]);
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
fn a_staged_update_is_checked_again_and_superseded_without_an_intermediate_install() {
  let d = tempfile::tempdir().unwrap();
  let (installed, new, newest) = (d.path().join("Applications/Acpira.app"), d.path().join("new.app"), d.path().join("newest.app"));
  bundle(&installed, "1.0.0");
  bundle(&new, "1.1.0");
  bundle(&newest, "1.2.0");
  let first = release(d.path(), &installed, &new, "1.1.0", false);
  let next = release(d.path(), &installed, &newest, "1.2.0", false);
  let (mut u, seen, _) = updater(d.path(), installed.clone(), first.url.clone(), "1.0.0");
  u.run(true).unwrap();
  let old_stage = u.staged().unwrap();
  seen.lock().clear();
  let hits = first._srv.hits.load(Ordering::SeqCst);
  u.run(true).unwrap();
  // A repeat check fetches the manifest and signature, never the same package or another ready toast.
  assert_eq!(first._srv.hits.load(Ordering::SeqCst), hits + 2);
  assert_eq!(*seen.lock(), [Phase::Ready]);
  Arc::get_mut(&mut u).unwrap().env.config.manifest_url = next.url;
  u.run(false).unwrap();
  assert_eq!((u.state().phase, u.state().version.as_deref()), (Phase::Available, Some("1.2.0")));
  assert_eq!(u.staged().unwrap().version, "1.1.0");
  assert!(old_stage.path.exists());
  u.run(true).unwrap();
  let staged = u.staged().unwrap();
  assert_eq!((u.state().phase, staged.version.as_str()), (Phase::Ready, "1.2.0"));
  assert!(!old_stage.dir.exists());
  assert_eq!(std::fs::read(installed.join("Contents/Info.plist")).unwrap(), b"<plist>1.0.0</plist>");
  delta::verify_tree(&staged.path, &delta::scan(&newest).unwrap()).unwrap();
  // A stale manifest must not replace a newer, already verified package.
  Arc::get_mut(&mut u).unwrap().env.config.manifest_url = first.url;
  u.run(true).unwrap();
  assert_eq!(u.state().version.as_deref(), Some("1.2.0"));
  assert_eq!(u.apply().unwrap(), "1.2.0");
  delta::verify_tree(&installed, &delta::scan(&newest).unwrap()).unwrap();
}

#[test]
fn a_failed_replacement_keeps_the_previously_verified_update_installable() {
  let d = tempfile::tempdir().unwrap();
  let (installed, new) = (d.path().join("Applications/Acpira.app"), d.path().join("new.app"));
  bundle(&installed, "1.0.0");
  bundle(&new, "1.1.0");
  let first = release(d.path(), &installed, &new, "1.1.0", false);
  let mut files = vec![];
  let failed = serve_with_manifest(&mut files, |base| {
    serde_json::to_vec(&Manifest {
      schema: 1,
      version: "1.2.0".into(),
      notes: "A later release".into(),
      minimum_system_version: None,
      platforms: BTreeMap::from([(
        manifest::platform_key().unwrap(),
        PlatformBuild { full: asset(base, "missing.tar.zst", b"missing"), files: vec![], deltas: BTreeMap::new() },
      )]),
    })
    .unwrap()
  });
  let (mut u, _, _) = updater(d.path(), installed.clone(), first.url.clone(), "1.0.0");
  u.run(true).unwrap();
  let ready = u.staged().unwrap();
  Arc::get_mut(&mut u).unwrap().env.config.manifest_url = format!("{}/desktop-update.json", failed.base);
  u.cycle(true);
  assert_eq!(u.state().phase, Phase::Failed);
  assert!(u.state().error.as_deref().unwrap().contains("HTTP 404"));
  assert_eq!(u.staged().unwrap().version, "1.1.0");
  delta::verify_tree(&ready.path, &delta::scan(&new).unwrap()).unwrap();
  // A later check of the same release restores the original ready state, not the failed target's notes.
  Arc::get_mut(&mut u).unwrap().env.config.manifest_url = first.url;
  u.run(true).unwrap();
  assert_eq!((u.state().phase, u.state().version.as_deref(), u.state().notes.as_deref()), (Phase::Ready, Some("1.1.0"), Some("Fixes")));
  assert!(u.state().error.is_none());
  assert_eq!(u.apply().unwrap(), "1.1.0");
  delta::verify_tree(&installed, &delta::scan(&new).unwrap()).unwrap();
  // An in-flight refresh cannot publish another update once installation has started.
  assert_eq!(u.run(true).unwrap_err(), "cancelled");
}

#[test]
fn installing_the_pending_update_cancels_an_in_flight_replacement() {
  let d = tempfile::tempdir().unwrap();
  let (installed, new, newest) = (d.path().join("Applications/Acpira.app"), d.path().join("new.app"), d.path().join("newest.app"));
  bundle(&installed, "1.0.0");
  bundle(&new, "1.1.0");
  bundle(&newest, "1.2.0");
  let first = release(d.path(), &installed, &new, "1.1.0", false);
  let next = release(d.path(), &installed, &newest, "1.2.0", false);
  let (mut u, _, _) = updater(d.path(), installed.clone(), first.url, "1.0.0");
  u.run(true).unwrap();
  let started = Arc::new(std::sync::Barrier::new(2));
  let resume = Arc::new(std::sync::Barrier::new(2));
  let (at_stage, continue_stage) = (started.clone(), resume.clone());
  let inner = Arc::get_mut(&mut u).unwrap();
  inner.env.config.manifest_url = next.url;
  let paused = AtomicBool::new(false);
  inner.on_change = Box::new(move |s| {
    if s.phase == Phase::Staging && !paused.swap(true, Ordering::SeqCst) {
      at_stage.wait();
      continue_stage.wait();
    }
  });
  let worker = u.clone();
  let thread = std::thread::spawn(move || worker.cycle(true));
  started.wait();
  // The verified old package remains usable while its replacement is being assembled.
  assert_eq!(u.apply().unwrap(), "1.1.0");
  resume.wait();
  thread.join().unwrap();
  assert_eq!(u.state().phase, Phase::Applying);
  assert!(u.staged().is_none());
  delta::verify_tree(&installed, &delta::scan(&new).unwrap()).unwrap();
}

#[test]
fn an_older_install_without_a_delta_jumps_directly_to_the_latest_full_package() {
  let d = tempfile::tempdir().unwrap();
  let (installed, new) = (d.path().join("Applications/Acpira.app"), d.path().join("new.app"));
  bundle(&installed, "0.1.0");
  bundle(&new, "1.2.0");
  let r = release(d.path(), &installed, &new, "1.2.0", false);
  let (u, _, _) = updater(d.path(), installed.clone(), r.url, "0.1.0");
  u.run(true).unwrap();
  assert_eq!(u.state().version.as_deref(), Some("1.2.0"));
  assert_eq!(u.apply().unwrap(), "1.2.0");
  delta::verify_tree(&installed, &delta::scan(&new).unwrap()).unwrap();
}

#[test]
fn a_single_exe_install_is_patched_and_renamed_into_place() {
  let d = tempfile::tempdir().unwrap();
  // The browser saved the exe under another name; the release's tree holds Acpira.exe
  let installed = d.path().join("Tools/Acpira (1).exe");
  let (old, new) = (d.path().join("old"), d.path().join("new"));
  bundle(&d.path().join("old-src"), "1.0.0");
  bundle(&d.path().join("new-src"), "1.1.0");
  for (src, tree) in [("old-src", &old), ("new-src", &new)] {
    std::fs::create_dir_all(tree).unwrap();
    std::fs::copy(d.path().join(src).join("Contents/MacOS/acpira-engine"), tree.join("Acpira.exe")).unwrap();
  }
  std::fs::create_dir_all(installed.parent().unwrap()).unwrap();
  std::fs::copy(old.join("Acpira.exe"), &installed).unwrap();
  let full = d.path().join("full.tar.zst");
  let files = delta::pack_full(&new, &full, 3).unwrap();
  let dpath = d.path().join("d.delta");
  delta::make_delta(&old, &new, "1.0.0", "1.1.0", &dpath, 3).unwrap();
  let (full_bytes, delta_bytes) = (std::fs::read(&full).unwrap(), std::fs::read(&dpath).unwrap());
  let mut served = vec![("full.tar.zst".to_owned(), full_bytes.clone()), ("d.delta".to_owned(), delta_bytes.clone())];
  let srv = serve_with_manifest(&mut served, |base| {
    let exe = PlatformBuild {
      full: asset(base, "full.tar.zst", &full_bytes),
      files: files.clone(),
      deltas: BTreeMap::from([("1.0.0".to_owned(), asset(base, "d.delta", &delta_bytes))]),
    };
    // One manifest for both platforms: this one's build is the exe, the other's a bundle it must not touch
    let other = PlatformBuild { full: asset(base, "nothing", b""), files: vec![], deltas: BTreeMap::new() };
    let m = Manifest {
      schema: 1,
      version: "1.1.0".into(),
      notes: String::new(),
      minimum_system_version: None,
      platforms: BTreeMap::from([(manifest::platform_key().unwrap(), exe), ("plan9-x86_64".to_owned(), other)]),
    };
    serde_json::to_vec(&m).unwrap()
  });
  let (u, _, lines) = updater(d.path(), installed.clone(), format!("{}/desktop-update.json", srv.base), "1.0.0");
  u.run(true).unwrap();
  assert!(!lines.lock().iter().any(|l| l.contains("did not apply")), "{:?}", lines.lock());
  let staged = u.staged().unwrap();
  assert!(staged.path.ends_with("Acpira/Acpira.exe"), "{}", staged.path.display());
  assert_eq!(u.apply().unwrap(), "1.1.0");
  assert_eq!(std::fs::read(&installed).unwrap(), std::fs::read(new.join("Acpira.exe")).unwrap());
  // Nothing left beside it here (on Windows the old exe stays until its process is gone) nor in the cache
  assert_eq!(std::fs::read_dir(installed.parent().unwrap()).unwrap().count(), 1);
  assert!(!staged.dir.exists());
}

#[test]
fn an_exe_install_refuses_a_build_that_is_not_one_exe() {
  let d = tempfile::tempdir().unwrap();
  let installed = d.path().join("Tools/Acpira.exe");
  std::fs::create_dir_all(installed.parent().unwrap()).unwrap();
  std::fs::write(&installed, b"MZ").unwrap();
  let (old, new) = (d.path().join("old.app"), d.path().join("new.app"));
  bundle(&old, "1.0.0");
  bundle(&new, "1.1.0");
  let r = release(d.path(), &old, &new, "1.1.0", false);
  let (u, _, _) = updater(d.path(), installed.clone(), r.url, "1.0.0");
  u.cycle(true);
  let s = u.state();
  assert_eq!(s.phase, Phase::Failed);
  assert!(s.error.unwrap().contains("not one Acpira.exe"));
  assert_eq!(std::fs::read(&installed).unwrap(), b"MZ");
}

#[test]
fn an_installed_copy_the_delta_does_not_fit_takes_the_full_package() {
  let d = tempfile::tempdir().unwrap();
  let (installed, new) = (d.path().join("Applications/Acpira.app"), d.path().join("build/Acpira.app"));
  bundle(&installed, "1.0.0");
  bundle(&new, "1.1.0");
  let r = release(d.path(), &installed, &new, "1.1.0", true);
  let (u, _, lines) = updater(d.path(), installed.clone(), r.url.clone(), "1.0.0");
  u.run(true).unwrap();
  assert_eq!(u.state().phase, Phase::Ready);
  assert!(
    lines.lock().iter().any(|l| l.contains("did not apply") && l.contains("is not the one 1.0.0 shipped")),
    "{:?}",
    lines.lock()
  );
  delta::verify_tree(&u.staged().unwrap().path, &delta::scan(&new).unwrap()).unwrap();
}

#[test]
fn the_current_version_a_manual_install_or_a_bad_signature_downloads_nothing() {
  let d = tempfile::tempdir().unwrap();
  let (installed, new) = (d.path().join("Applications/Acpira.app"), d.path().join("build/Acpira.app"));
  bundle(&installed, "1.0.0");
  bundle(&new, "1.1.0");
  let r = release(d.path(), &installed, &new, "1.1.0", false);
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

#[cfg(target_os = "macos")]
#[test]
fn a_bundle_moved_aside_is_in_use_while_its_program_runs() {
  let d = tempfile::tempdir().unwrap();
  let (old, aside) = (d.path().join("Acpira.app"), d.path().join("staged"));
  std::fs::create_dir_all(old.join("Contents/MacOS")).unwrap();
  let exe = old.join("Contents/MacOS/acpira-engine");
  std::fs::copy("/bin/sleep", &exe).unwrap();
  let mut child = std::process::Command::new(&exe).arg("30").spawn().unwrap();
  // The swap renames the running bundle away; its process is still found there
  std::fs::rename(&old, &aside).unwrap();
  // The kernel can take a moment to report the child's executable under its renamed path.
  let deadline = Instant::now() + Duration::from_secs(2);
  let used = loop {
    if in_use(&aside) {
      break true;
    }
    if Instant::now() >= deadline {
      break false;
    }
    std::thread::sleep(Duration::from_millis(10));
  };
  child.kill().unwrap();
  child.wait().unwrap();
  assert!(used);
  assert!(!in_use(&aside));
}
