//! macOS: remote access's LaunchAgent, which starts the machine's engine (and with it the gateway) at login without a
//! window open, from the service's copy (stage.rs); and the `gui/<uid>` jobs of engines started over SSH.

use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use anyhow::{Context, Result, bail};

/// The label for this data root: the default `~/.acpira` gets the plain one, any other root (the harness, tests) one
/// of its own, so two roots never replace each other's. The name is from when it ran a standalone gateway
pub fn label(root: &Path, user_home: &Path) -> String {
  super::service::label("com.hotic.acpira.gateway", ".", root, user_home)
}

pub fn plist_path(label: &str, user_home: &Path) -> PathBuf {
  user_home.join("Library").join("LaunchAgents").join(format!("{label}.plist"))
}

fn xml(s: &str) -> String {
  s.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;").replace('"', "&quot;").replace('\'', "&apos;")
}

/// The LaunchAgent label of the peer engine for `version` (`remote/helper/streams.rs`)
pub fn peer_label(version: &str) -> String {
  let clean: String = version.chars().map(|c| if c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-') { c } else { '-' }).collect();
  format!("com.hotic.acpira.peer.{clean}")
}

/// What a job runs: `exe` itself, or the launcher starting it (machine_engine/launcher.rs), which keeps the engine's
/// privacy grants under one program whatever build it runs
fn program(launcher: Option<&Path>, exe: &Path) -> String {
  let mut parts = vec![];
  if let Some(l) = launcher {
    parts.push(xml(&l.to_string_lossy()));
    parts.push(crate::machine_engine::launcher::COMMAND.to_owned());
  }
  parts.push(xml(&exe.to_string_lossy()));
  parts.iter().map(|p| format!("<string>{p}</string>")).collect::<Vec<_>>().join("\n    ")
}

/// The login job's program: the service copy itself, under `supervise` when engines start under a launcher, which
/// it then picks each time (machine_engine/launcher.rs)
fn supervised(launcher: Option<&Path>, exe: &Path) -> String {
  let mut parts = vec![format!("<string>{}</string>", xml(&exe.to_string_lossy()))];
  if launcher.is_some() {
    parts.push(format!("<string>{}</string>", crate::machine_engine::launcher::SUPERVISE));
  }
  parts.join("\n    ")
}

/// A job started from the desktop app's bundle names the app, so Login Items lists it under Acpira.app instead of a
/// bare program
fn associated(launcher: Option<&Path>) -> String {
  if !launcher.is_some_and(crate::machine_engine::launcher::in_app) {
    return String::new();
  }
  format!(
    "\n  <key>AssociatedBundleIdentifiers</key>\n  <array>\n    <string>{}</string>\n  </array>",
    crate::machine_engine::launcher::APP_ID
  )
}

/// The LaunchAgent plist for a peer engine on macOS: runs in the user's `Aqua` login session (`gui/<uid>`), where the
/// login keychain is unlocked, without `KeepAlive` so the engine still exits on its idle grace (`cli.rs`)
pub fn peer_plist(label: &str, launcher: Option<&Path>, exe: &Path, socket: &Path, home: &Path, log: &Path) -> String {
  format!(
    r#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
  <key>Label</key>
  <string>{label}</string>{associated}
  <key>LimitLoadToSessionType</key>
  <string>Aqua</string>
  <key>ProgramArguments</key>
  <array>
    {program}
    <string>serve</string>
    <string>--socket</string>
    <string>{socket}</string>
    <string>--home</string>
    <string>{home}</string>
  </array>
  <key>RunAtLoad</key>
  <true/>
  <key>StandardOutPath</key>
  <string>{log}</string>
  <key>StandardErrorPath</key>
  <string>{log}</string>
</dict>
</plist>
"#,
    label = xml(label),
    program = program(launcher, exe),
    associated = associated(launcher),
    socket = xml(&socket.to_string_lossy()),
    home = xml(&home.to_string_lossy()),
    log = xml(&log.to_string_lossy()),
  )
}

#[cfg(unix)]
fn passwd_home() -> Option<PathBuf> {
  // SAFETY: geteuid has no preconditions; getpwuid_r writes into caller-owned buffers and sets `result` on success
  unsafe {
    let uid = libc::geteuid();
    let mut pwd: libc::passwd = std::mem::zeroed();
    let mut buf = vec![0 as libc::c_char; 4096];
    let mut result: *mut libc::passwd = std::ptr::null_mut();
    if libc::getpwuid_r(uid, &mut pwd, buf.as_mut_ptr(), buf.len(), &mut result) != 0 || result.is_null() || pwd.pw_dir.is_null() {
      return None;
    }
    let dir = std::ffi::CStr::from_ptr(pwd.pw_dir).to_str().ok()?.trim();
    (!dir.is_empty()).then(|| PathBuf::from(dir))
  }
}

/// Whether `home` is this OS user's real `~/.acpira` with no test or custom environment overrides: tests (`remote_peer`)
/// and custom homes must stay in their own process tree and never load a job into the host's `gui/<uid>` launchd domain
pub fn is_default_user_home(home: &Path) -> bool {
  #[cfg(unix)]
  {
    if ["ACPIRA_HOME", "ACPIRA_REMOTE_ROOT_MAP", "ACPIRA_REMOTE_MACHINE_ID"].iter().any(|k| std::env::var_os(k).is_some()) {
      return false;
    }
    passwd_home().is_some_and(|u| home == u.join(".acpira"))
  }
  #[cfg(not(unix))]
  {
    let _ = home;
    false
  }
}

/// Whether the user has an active macOS GUI (`Aqua`) login session (`gui/<uid>`)
pub async fn gui_available() -> bool {
  if !cfg!(target_os = "macos") {
    return false;
  }
  launchctl(&["print", &domain()]).await.is_ok_and(|o| o.status.success())
}

/// Whether a peer engine for `home` can run in the user's macOS GUI (`gui/<uid>`) login session
pub async fn can_use_gui(home: &Path) -> bool {
  cfg!(target_os = "macos") && is_default_user_home(home) && gui_available().await
}

/// Start (or kickstart after an idle exit) the peer engine for `version` in `gui/<uid>`. The plist lives under
/// `<home>/run/` rather than `~/Library/LaunchAgents/` so it neither triggers a Login Items notification nor starts
/// every old version at login
pub async fn start_peer(version: &str, exe: &Path, socket: &Path, home: &Path, log: &Path) -> Result<()> {
  start_serve(&peer_label(version), &format!("peer-{version}"), exe, socket, home, log).await
}

const ENGINE_LABEL: &str = "com.hotic.acpira.engine";

/// The machine's one engine (`machine_engine`), started from an SSH login on a Mac whose user is logged into the
/// desktop: in `gui/<uid>` like a peer engine, so it reads the login keychain. One label per landed build (`build`
/// names its folder): replacing one job's plist means a `bootout`, which would end an older engine still finishing its
/// turns after it handed the socket over. Jobs of other builds that no longer run are unloaded here. Returns the label
pub async fn start_machine_engine(build: &str, exe: &Path, socket: &Path, home: &Path, log: &Path) -> Result<String> {
  let label = format!("{ENGINE_LABEL}.{build}");
  let stem = format!("engine-{build}");
  let run = home.join("run");
  if let Ok(entries) = std::fs::read_dir(&run) {
    for e in entries.flatten() {
      let name = e.file_name().to_string_lossy().into_owned();
      let Some(other) = name.strip_prefix("engine-").and_then(|n| n.strip_suffix(".plist")) else { continue };
      if other == build {
        continue;
      }
      let other_label = format!("{ENGINE_LABEL}.{other}");
      if !running(&other_label).await {
        let _ = bootout(&other_label).await;
        let _ = std::fs::remove_file(e.path());
      }
    }
  }
  start_serve(&label, &stem, exe, socket, home, log).await?;
  Ok(label)
}

/// Whether the job has a process right now (loaded alone is not enough: a job without KeepAlive stays loaded after it
/// exits)
pub async fn running(label: &str) -> bool {
  launchctl(&["print", &format!("{}/{label}", domain())])
    .await
    .is_ok_and(|o| o.status.success() && String::from_utf8_lossy(&o.stdout).lines().any(|l| l.trim_start().starts_with("pid = ")))
}

/// `acpira serve` as a job in `gui/<uid>`; `stem` names its plist and lock under `<home>/run/`
async fn start_serve(label: &str, stem: &str, exe: &Path, socket: &Path, home: &Path, log: &Path) -> Result<()> {
  for p in [exe, socket, home, log] {
    if p.to_string_lossy().bytes().any(|b| b < 0x20) {
      bail!("path contains control characters: {}", p.display());
    }
  }
  if !gui_available().await {
    bail!("no GUI login session ({})", domain());
  }
  let run = home.join("run");
  let plist_path = run.join(format!("{stem}.plist"));
  let lock_path = run.join(format!("{stem}.plist.lock"));
  let launcher = crate::machine_engine::launcher::ensure(home, exe);
  let text = peer_plist(label, launcher.as_deref(), exe, socket, home, log);
  crate::store::file_lock::with_file_lock(&lock_path, || async {
    let same = tokio::fs::read_to_string(&plist_path).await.is_ok_and(|t| t == text);
    if !same {
      crate::store::file_lock::write_atomic(&plist_path, text.as_bytes(), Some(0o600)).await?;
    }
    if loaded(label).await {
      if !same {
        unload(label).await?;
        return load(label, &plist_path).await;
      }
      return kickstart(label, false).await;
    }
    load(label, &plist_path).await
  })
  .await
}

/// The LaunchAgent: started at login, restarted when it crashes or exits with an error (not after an idle exit, a
/// handover to a newer build or `serve` finding the engine running already). `serve` moves into its landed copy at once
/// (`cli::run`) and writes its log itself; launchd's capture covers what comes before
pub fn plist(label: &str, launcher: Option<&Path>, exe: &Path, root: &Path) -> String {
  let log = super::service::log_path(root);
  let args: String = super::service::args(root).iter().map(|a| format!("\n    <string>{}</string>", xml(a))).collect();
  format!(
    r#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
  <key>Label</key>
  <string>{label}</string>{associated}
  <key>ProgramArguments</key>
  <array>
    {program}{args}
  </array>
  <key>RunAtLoad</key>
  <true/>
  <key>KeepAlive</key>
  <dict>
    <key>SuccessfulExit</key>
    <false/>
  </dict>
  <key>ThrottleInterval</key>
  <integer>10</integer>
  <key>StandardOutPath</key>
  <string>{log}</string>
  <key>StandardErrorPath</key>
  <string>{log}</string>
</dict>
</plist>
"#,
    label = xml(label),
    program = supervised(launcher, exe),
    associated = associated(launcher),
    log = xml(&log.to_string_lossy()),
  )
}

/// Write `text` as the label's plist and have launchd run it, leaving a job whose engine runs as it is (its process may
/// be the very engine serving every window; the new text applies from the next login)
pub async fn install(label: &str, user_home: &Path, text: &str) -> Result<()> {
  let path = plist_path(label, user_home);
  if let Some(dir) = path.parent() {
    tokio::fs::create_dir_all(dir).await?;
  }
  let changed = !tokio::fs::read_to_string(&path).await.is_ok_and(|t| t == text);
  if changed {
    crate::store::file_lock::write_atomic(&path, text.as_bytes(), Some(0o644)).await?;
  }
  if !loaded(label).await {
    return load(label, &path).await;
  }
  if running(label).await {
    return Ok(());
  }
  if changed {
    unload(label).await?;
    return load(label, &path).await;
  }
  kickstart(label, false).await
}

/// Remove the label's plist so nothing starts it at login; a job without a process is unloaded too, one whose engine
/// runs stays loaded until logout (ending it would end every session it runs)
pub async fn uninstall(label: &str, user_home: &Path) -> Result<()> {
  if !running(label).await {
    unload(label).await?;
  }
  let _ = tokio::fs::remove_file(plist_path(label, user_home)).await;
  Ok(())
}

fn domain() -> String {
  // SAFETY: getuid has no preconditions
  #[cfg(unix)]
  let uid = unsafe { libc::getuid() };
  // launchd exists only on macOS; other platforms never reach launchctl, this only keeps them compiling
  #[cfg(not(unix))]
  let uid = 0;
  format!("gui/{uid}")
}

async fn launchctl(args: &[&str]) -> Result<std::process::Output> {
  tokio::process::Command::new("/bin/launchctl").args(args).output().await.context("launchctl")
}

/// How long launchd may take to let go of a job after `bootout`: it waits for the process to exit, up to the job's
/// `ExitTimeOut` (20 s by default) before a SIGKILL
const GONE_WAIT: Duration = Duration::from_secs(25);

/// Whether launchd has the label loaded, running or not
pub async fn loaded(label: &str) -> bool {
  launchctl(&["print", &format!("{}/{label}", domain())]).await.is_ok_and(|o| o.status.success())
}

/// Stop it, unload it, and wait until launchd has let go of it: `bootout` answers while the job is still listed
/// (`state = SIGTERMed`), and a bootstrap in that window fails with `5: Input/output error`
pub async fn unload(label: &str) -> Result<()> {
  bootout(label).await?;
  let deadline = Instant::now() + GONE_WAIT;
  while loaded(label).await {
    if Instant::now() >= deadline {
      bail!("launchd did not let go of {label}");
    }
    tokio::time::sleep(Duration::from_millis(100)).await;
  }
  Ok(())
}

/// Load the plist at `plist` and start it; the label must not be loaded (`unload` first to replace it)
pub async fn load(label: &str, plist: &Path) -> Result<()> {
  let out = launchctl(&["bootstrap", &domain(), &plist.to_string_lossy()]).await?;
  // 5 also answers a label already loaded: another engine sharing the data root loaded it meanwhile
  if out.status.success() || loaded(label).await {
    return Ok(());
  }
  let err = String::from_utf8_lossy(&out.stderr);
  let mut msg = format!("launchctl bootstrap: {}", err.lines().next().unwrap_or("").trim());
  if out.status.code() == Some(5) {
    // Neither loaded nor being torn down: launchd answers 5 for a job switched off too
    msg.push_str(" (is Acpira switched off under System Settings > General > Login Items?)");
  }
  bail!(msg)
}

/// Stop it and unload it; not loaded is not an error
pub async fn bootout(label: &str) -> Result<()> {
  let out = launchctl(&["bootout", &format!("{}/{label}", domain())]).await?;
  // 3: no such process; 113: not loaded (`Could not find service`)
  if !out.status.success() && !matches!(out.status.code(), Some(3) | Some(113)) {
    bail!("launchctl bootout: {}", String::from_utf8_lossy(&out.stderr).trim());
  }
  Ok(())
}

/// Start it; with `kill`, stop a running one first
pub async fn kickstart(label: &str, kill: bool) -> Result<()> {
  let target = format!("{}/{label}", domain());
  let args: &[&str] = if kill { &["kickstart", "-k", &target] } else { &["kickstart", &target] };
  let out = launchctl(args).await?;
  // 37 (Operation already in progress): already running when `kill` is false
  if !out.status.success() && (kill || out.status.code() != Some(37)) {
    bail!("launchctl kickstart: {}", String::from_utf8_lossy(&out.stderr).trim());
  }
  Ok(())
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn names() {
    let home = Path::new("/Users/u");
    assert_eq!(label(Path::new("/Users/u/.acpira"), home), "com.hotic.acpira.gateway");
    assert!(label(Path::new("/Users/u/.acpira/harness"), home).starts_with("com.hotic.acpira.gateway."));
    let p = plist("l", None, Path::new("/a&b/acpira"), Path::new("/r"));
    assert!(p.contains("<string>/a&amp;b/acpira</string>\n    <string>serve</string>"), "{p}");
    // The log path is joined natively, so Windows (where this file's text is also tested) puts backslashes in it
    let log = super::super::service::log_path(Path::new("/r"));
    assert!(p.contains("<string>--service</string>") && p.contains(&format!("<string>{}</string>", log.display())), "{p}");
    assert_eq!(peer_label("2.0.5"), "com.hotic.acpira.peer.2.0.5");
    let peer = peer_plist("com.hotic.acpira.peer.2.0.5", None, Path::new("/a'b/acpira"), Path::new("/r/peer.sock"), Path::new("/r"), Path::new("/r/peer.log"));
    assert!(peer.contains("<string>Aqua</string>") && peer.contains("/a&apos;b/acpira") && !peer.contains("KeepAlive"));
    // With engines under a launcher, the job runs the service copy under `supervise`, which picks the launcher
    let launched = plist("l", Some(Path::new("/r/engine/launcher/Acpira Engine")), Path::new("/s/acpira"), Path::new("/r"));
    assert!(launched.contains("<string>/s/acpira</string>\n    <string>supervise</string>\n    <string>serve</string>"), "{launched}");
    assert!(!launched.contains("AssociatedBundleIdentifiers"), "{launched}");
    // Started from the desktop app's bundle, the job names the app
    let from_app = plist("l", Some(Path::new("/Applications/Acpira.app/Contents/MacOS/acpira-engine")), Path::new("/s/acpira"), Path::new("/r"));
    assert!(from_app.contains("<key>AssociatedBundleIdentifiers</key>\n  <array>\n    <string>cc.hotic.acpira.desktop</string>"), "{from_app}");
    assert!(!is_default_user_home(Path::new("/tmp/not-real-user-home/.acpira")));
  }
}
