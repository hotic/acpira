//! macOS: the gateway as a LaunchAgent, so it runs from login on without a window open. The binaries are copied under
//! `<home>/devices/bin/<version>-<digest>/` first: an extension upgrade deletes the folder the engine runs from, and a
//! LaunchAgent pointing there would fail at the next login. The copied engine is what the gateway starts its engine from
//! (an engine of its own, `shell: remote`, which reaches the windows' engines over the push bus).

use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};
use sha2::{Digest, Sha256};

/// The label for this data root: the default `~/.acpira` gets the plain one, any other root (the harness, tests) one
/// of its own, so two roots never replace each other's gateway
pub fn label(root: &Path, user_home: &Path) -> String {
  if root == user_home.join(".acpira") {
    return "com.hotic.acpira.gateway".into();
  }
  let d = Sha256::digest(root.to_string_lossy().as_bytes());
  format!("com.hotic.acpira.gateway.{}", d.iter().take(4).map(|b| format!("{b:02x}")).collect::<String>())
}

pub fn plist_path(label: &str, user_home: &Path) -> PathBuf {
  user_home.join("Library").join("LaunchAgents").join(format!("{label}.plist"))
}

/// The copied binaries' directory
pub fn bin_root(root: &Path) -> PathBuf {
  root.join("devices").join("bin")
}

/// The version an installed copy was made from (`<version>-<digest>` of the directory the engine path names)
pub fn installed_version(engine: &Path) -> Option<String> {
  let dir = engine.parent()?.file_name()?.to_str()?;
  dir.rsplit_once('-').map(|(v, _)| v.to_owned())
}

/// Whether `a` is a later version than `b` (numeric dot parts; a pre-release suffix is ignored)
pub fn newer(a: &str, b: &str) -> bool {
  let parts = |s: &str| -> Vec<u64> { s.split(['-', '+']).next().unwrap_or("").split('.').map(|p| p.parse().unwrap_or(0)).collect() };
  parts(a) > parts(b)
}

/// Copy the engine and the gateway into a directory named after the version and their contents; an identical copy is
/// reused. Answers the copies' paths
pub async fn stage(root: &Path, version: &str, engine: &Path, gateway: &Path) -> Result<(PathBuf, PathBuf)> {
  let (engine, gateway) = (engine.to_owned(), gateway.to_owned());
  let digest = tokio::task::spawn_blocking({
    let (e, g) = (engine.clone(), gateway.clone());
    move || -> Result<String> {
      let mut h = Sha256::new();
      for p in [&e, &g] {
        h.update(std::fs::read(p).with_context(|| format!("read {}", p.display()))?);
      }
      Ok(h.finalize().iter().take(6).map(|b| format!("{b:02x}")).collect())
    }
  })
  .await??;
  let dir = bin_root(root).join(format!("{version}-{digest}"));
  let names = (file_name(&engine)?, file_name(&gateway)?);
  if !dir.join(&names.0).is_file() || !dir.join(&names.1).is_file() {
    // Staged beside it and renamed into place: a half-copied directory is never the one a LaunchAgent names
    let tmp = bin_root(root).join(format!(".{version}-{digest}.{}.tmp", std::process::id()));
    let _ = tokio::fs::remove_dir_all(&tmp).await;
    tokio::fs::create_dir_all(&tmp).await?;
    tokio::fs::copy(&engine, tmp.join(&names.0)).await.with_context(|| format!("copy {}", engine.display()))?;
    tokio::fs::copy(&gateway, tmp.join(&names.1)).await.with_context(|| format!("copy {}", gateway.display()))?;
    let _ = tokio::fs::remove_dir_all(&dir).await;
    tokio::fs::rename(&tmp, &dir).await?;
  }
  Ok((dir.join(names.0), dir.join(names.1)))
}

fn file_name(p: &Path) -> Result<String> {
  Ok(p.file_name().context("binary without a file name")?.to_string_lossy().into_owned())
}

/// Remove the copies other than `keep` (older versions a previous install left)
pub async fn prune(root: &Path, keep: Option<&Path>) {
  let Ok(mut rd) = tokio::fs::read_dir(bin_root(root)).await else { return };
  while let Ok(Some(e)) = rd.next_entry().await {
    if keep != Some(e.path().as_path()) {
      let _ = tokio::fs::remove_dir_all(e.path()).await;
    }
  }
}

fn xml(s: &str) -> String {
  s.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;").replace('"', "&quot;")
}

/// The LaunchAgent: started at login, restarted when it crashes or exits with an error (not after `acpira-gateway stop`)
pub fn plist(label: &str, gateway: &Path, root: &Path) -> String {
  let log = root.join("devices").join("gateway.log");
  format!(
    r#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
  <key>Label</key>
  <string>{label}</string>
  <key>ProgramArguments</key>
  <array>
    <string>{gateway}</string>
    <string>run</string>
    <string>--home</string>
    <string>{root}</string>
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
    gateway = xml(&gateway.to_string_lossy()),
    root = xml(&root.to_string_lossy()),
    log = xml(&log.to_string_lossy()),
  )
}

fn domain() -> String {
  // SAFETY: getuid has no preconditions
  format!("gui/{}", unsafe { libc::getuid() })
}

async fn launchctl(args: &[&str]) -> Result<std::process::Output> {
  tokio::process::Command::new("/bin/launchctl").args(args).output().await.context("launchctl")
}

/// Load the plist written at `plist` (replacing a loaded one of the same label) and start it
pub async fn bootstrap(label: &str, plist: &Path) -> Result<()> {
  let _ = bootout(label).await;
  // A service still being torn down refuses a bootstrap for a moment (`5: Input/output error`)
  let mut tries = 0;
  loop {
    let out = launchctl(&["bootstrap", &domain(), &plist.to_string_lossy()]).await?;
    if out.status.success() {
      return Ok(());
    }
    tries += 1;
    if tries == 5 {
      bail!("launchctl bootstrap: {}", String::from_utf8_lossy(&out.stderr).trim());
    }
    tokio::time::sleep(std::time::Duration::from_millis(300)).await;
  }
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

/// Restart it (a relay change: the endpoint is bound with its relays)
pub async fn kickstart(label: &str) -> Result<()> {
  let out = launchctl(&["kickstart", "-k", &format!("{}/{label}", domain())]).await?;
  if !out.status.success() {
    bail!("launchctl kickstart: {}", String::from_utf8_lossy(&out.stderr).trim());
  }
  Ok(())
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn names_and_versions() {
    let home = Path::new("/Users/u");
    assert_eq!(label(Path::new("/Users/u/.acpira"), home), "com.hotic.acpira.gateway");
    assert!(label(Path::new("/Users/u/.acpira/harness"), home).starts_with("com.hotic.acpira.gateway."));
    assert_eq!(installed_version(Path::new("/r/devices/bin/0.9.3-ab12cd/acpira")).as_deref(), Some("0.9.3"));
    assert!(newer("0.10.0", "0.9.3"));
    assert!(!newer("0.9.3", "0.9.3"));
    assert!(!newer("0.9.3-dev", "0.9.3"));
    assert!(plist("l", Path::new("/a&b/acpira-gateway"), Path::new("/r")).contains("<string>/a&amp;b/acpira-gateway</string>"));
  }
}
