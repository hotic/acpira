//! A helper build for a host's platform that this machine does not carry, taken from the platform's VSIX of the engine's
//! own version (`acpira-<version>-<target>.vsix`, which holds the bare binary at `extension/bin/acpira`). Since 2.0.6 the
//! VSIX sits on the shared `packages` Release and the version's Release lists its digest in `SHA256SUMS`; from 1.11.0 to
//! 2.0.5 both the VSIX and `vsix.sha256` sat on the version's Release. Builds land in
//! `<builds>/<version>/<platform>/acpira`; a fetch for one version removes the other versions' folders. The host itself
//! still never downloads anything (docs/dev/remote.md)

use std::path::{Path, PathBuf};

use anyhow::{Context, Result, anyhow, bail};

use crate::acp::agents::native_release::{download_progress, extract_zip};

/// Where releases are published; `<base>/<tag>/<asset>` is an asset's URL
pub const RELEASES: &str = "https://github.com/hotic/acpira/releases/download";

/// The prerelease holding every version's files nobody downloads by hand (`scripts/release/packages-release.mjs`)
pub const PACKAGES_TAG: &str = "packages";

/// The VSIX that carries `platform`'s binary: `dist/sidecar` names (`mac-arm64`, `linux-x86_64`) map to VS Code targets
/// (`darwin-arm64`, `linux-x64`) as in `scripts/release/sidecar-targets.mjs`
pub fn asset_name(version: &str, platform: &str) -> String {
  let target = platform.replacen("mac-", "darwin-", 1).replacen("windows-", "win32-", 1).replace("x86_64", "x64");
  format!("acpira-{version}-{target}.vsix")
}

/// A fetched build of `version` for `platform`, when one is in place
pub fn installed(builds: &Path, version: &str, platform: &str) -> Option<PathBuf> {
  let p = builds.join(version).join(platform).join("acpira");
  p.is_file().then_some(p)
}

/// The digest a `sha256sum` listing gives `name` (`<hex>  <name>`, a `*` before binary-mode names)
pub fn digest_in_sums(sums: &str, name: &str) -> Option<String> {
  sums.lines().find_map(|line| {
    let (hex, file) = line.trim().split_once(char::is_whitespace)?;
    (file.trim().trim_start_matches('*') == name && hex.len() == 64 && hex.bytes().all(|b| b.is_ascii_hexdigit())).then(|| hex.to_ascii_lowercase())
  })
}

/// Download, check and unpack the build; blocking. `progress` gets (bytes so far, archive size)
pub fn fetch(base: &str, builds: &Path, version: &str, platform: &str, progress: &mut dyn FnMut(u64, Option<u64>)) -> Result<PathBuf> {
  if !valid_part(version) || !valid_part(platform) {
    bail!("not a release name: {version} / {platform}");
  }
  std::fs::create_dir_all(builds).with_context(|| format!("create {}", builds.display()))?;
  let stage = builds.join(format!(".stage-{}-{}", std::process::id(), nanos()));
  std::fs::create_dir(&stage).with_context(|| format!("create {}", stage.display()))?;
  let out = fetch_into(base, builds, &stage, version, platform, progress);
  let _ = std::fs::remove_dir_all(&stage);
  let path = out?;
  prune(builds, version);
  Ok(path)
}

fn fetch_into(base: &str, builds: &Path, stage: &Path, version: &str, platform: &str, progress: &mut dyn FnMut(u64, Option<u64>)) -> Result<PathBuf> {
  let base = base.trim_end_matches('/');
  let release = format!("{base}/v{version}");
  let name = asset_name(version, platform);
  // The current layout first, then the 1.11.0 - 2.0.5 one (whose SHA256SUMS lists only the IntelliJ zips)
  let layouts = [("SHA256SUMS", format!("{base}/{PACKAGES_TAG}")), ("vsix.sha256", release.clone())];
  let (mut found, mut listed, mut miss) = (None, false, None);
  for (sums_name, dir) in layouts {
    let sums_path = stage.join(sums_name);
    if let Err(e) = download_progress(&format!("{release}/{sums_name}"), &sums_path, &mut |_, _| {}) {
      miss.get_or_insert(e);
      continue;
    }
    // A listing came: the Release exists, so not finding the VSIX means it has no build for this platform
    listed = true;
    if let Some(digest) = digest_in_sums(&std::fs::read_to_string(&sums_path)?, &name) {
      found = Some((sums_name, dir, digest));
      break;
    }
  }
  let Some((sums_name, dir, expected)) = found else {
    return Err(match miss {
      Some(e) if !listed => anyhow!("no published Acpira {version} release to take a {platform} build from ({e:#})"),
      _ => anyhow!("the Acpira {version} release has no {platform} build ({name})"),
    });
  };
  let archive = stage.join(&name);
  let got = download_progress(&format!("{dir}/{name}"), &archive, progress)?;
  if !got.eq_ignore_ascii_case(&expected) {
    bail!("SHA-256 mismatch for {name}: {sums_name} lists {expected}, the download is {got}");
  }
  let pkg = stage.join("pkg");
  extract_zip(&archive, &pkg)?;
  let inner = pkg.join("extension").join("bin").join("acpira");
  if !inner.is_file() {
    bail!("{name} holds no extension/bin/acpira");
  }
  #[cfg(unix)]
  {
    use std::os::unix::fs::PermissionsExt;
    std::fs::set_permissions(&inner, std::fs::Permissions::from_mode(0o755))?;
  }
  let dir = builds.join(version).join(platform);
  std::fs::create_dir_all(&dir).with_context(|| format!("create {}", dir.display()))?;
  let target = dir.join("acpira");
  std::fs::rename(&inner, &target).with_context(|| format!("move into {}", target.display()))?;
  Ok(target)
}

/// Remove the folders of every other version and any stage a crashed fetch left
fn prune(builds: &Path, keep: &str) {
  let Ok(rd) = std::fs::read_dir(builds) else { return };
  for e in rd.flatten() {
    let name = e.file_name().to_string_lossy().into_owned();
    if name != keep && (name.starts_with(".stage-") || valid_part(&name)) {
      let _ = std::fs::remove_dir_all(e.path());
    }
  }
}

/// A version or platform name as a path segment: letters, digits, `.`, `-`, `_`, `+`, not starting with `.`
fn valid_part(s: &str) -> bool {
  !s.is_empty() && !s.starts_with('.') && s.bytes().all(|b| b.is_ascii_alphanumeric() || matches!(b, b'.' | b'-' | b'_' | b'+'))
}

fn nanos() -> u128 {
  std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_nanos()).unwrap_or(0)
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn sums_lines_name_their_asset() {
    let sums = "935fb453c15315d0245af00c33f55c3150f2885b16b0ba1c225c9812e0399958  acpira-1.11.0-linux-arm64.zip\n\
                A0DFBD61092180C9813EFF72E410F37CE549AC983C5F0D2458B2C541DCA69D22 *acpira-1.11.0-linux-x86_64.zip\n";
    assert_eq!(digest_in_sums(sums, "acpira-1.11.0-linux-arm64.zip").as_deref(), Some("935fb453c15315d0245af00c33f55c3150f2885b16b0ba1c225c9812e0399958"));
    assert_eq!(digest_in_sums(sums, "acpira-1.11.0-linux-x86_64.zip").as_deref(), Some("a0dfbd61092180c9813eff72e410f37ce549ac983c5f0d2458b2c541dca69d22"));
    assert_eq!(digest_in_sums(sums, "acpira-1.11.0-linux-x86.zip"), None);
    assert_eq!(digest_in_sums("nothex  acpira-1.11.0-linux-arm64.zip", "acpira-1.11.0-linux-arm64.zip"), None);
  }

  #[test]
  fn names_stay_one_path_segment() {
    assert_eq!(asset_name("1.11.0", "linux-x86_64"), "acpira-1.11.0-linux-x64.vsix");
    assert_eq!(asset_name("1.11.0", "mac-arm64"), "acpira-1.11.0-darwin-arm64.vsix");
    assert_eq!(asset_name("1.11.0", "linux-arm64"), "acpira-1.11.0-linux-arm64.vsix");
    for bad in ["", ".", "..", "../x", "a/b", ".stage-1"] {
      assert!(!valid_part(bad), "{bad}");
    }
    assert!(valid_part("1.11.0-beta+3"));
  }

  #[test]
  fn a_fetch_keeps_only_its_own_version() {
    let dir = tempfile::tempdir().unwrap();
    for v in ["1.10.0", "1.11.0", ".stage-1-2"] {
      std::fs::create_dir_all(dir.path().join(v).join("linux-x86_64")).unwrap();
    }
    std::fs::write(dir.path().join("1.11.0/linux-x86_64/acpira"), b"x").unwrap();
    prune(dir.path(), "1.11.0");
    let left: Vec<_> = std::fs::read_dir(dir.path()).unwrap().flatten().map(|e| e.file_name().to_string_lossy().into_owned()).collect();
    assert_eq!(left, ["1.11.0"]);
    assert!(installed(dir.path(), "1.11.0", "linux-x86_64").is_some());
    assert!(installed(dir.path(), "1.11.0", "linux-arm64").is_none());
  }

  /// The real Release (network, about 40 MB): `cargo test -p acpira-host --lib remote::fetch -- --ignored`
  #[test]
  #[ignore]
  fn fetches_a_published_release() {
    let dir = tempfile::tempdir().unwrap();
    let path = fetch(RELEASES, dir.path(), "1.11.0", "linux-x86_64", &mut |_, _| {}).unwrap();
    assert!(std::fs::metadata(&path).unwrap().len() > 1_000_000);
    assert_eq!(std::fs::read(&path).unwrap()[..4], *b"\x7fELF");
  }
}
