//! The desktop update's release side (docs/releasing.md "Desktop app"): packs a signed, notarized `Acpira.app` (or
//! Windows' `Acpira.exe`, a tree of that one file) as the full package, makes a delta from each earlier version's full
//! package, and writes the manifest with its Ed25519 signature. Every delta is applied back onto its base and checked
//! against the new build before it is listed. Each platform is packed where it is built (`--unsigned`), and `merge`
//! joins their manifests into the release's one and signs it.
//!
//!   acpira-update-pack keygen
//!   acpira-update-pack pack --app Acpira.app --version 1.11.0 --base-url https://github.com/hotic/acpira/releases/download/v1.11.0 \
//!     --out dist/desktop [--prev 1.10.8=Acpira-1.10.8-darwin-aarch64.app.tar.zst]… [--notes-file notes.md] [--min-os 26.0] \
//!     [--platform darwin-aarch64] [--level 19] [--key-env DESKTOP_UPDATE_KEY | --unsigned]
//!   acpira-update-pack pack --app Acpira.exe … (the platform defaults to windows-x86_64; packages are `….exe.tar.zst`)
//!   acpira-update-pack merge --out dist/desktop mac/desktop-update.json win/desktop-update.json [--key-env … | --unsigned]

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use acpira_desktop::update::delta;
use acpira_desktop::update::manifest::{self, Asset, Manifest, PlatformBuild};
use acpira_desktop::update::swap::EXE_NAME;

/// The platform a Windows exe is packed for unless `--platform` says otherwise
const EXE_PLATFORM: &str = "windows-x86_64";

fn main() {
  let args: Vec<String> = std::env::args().skip(1).collect();
  let result = match args.first().map(String::as_str) {
    Some("keygen") => keygen(),
    Some("pack") => pack(&args[1..]),
    Some("merge") => merge(&args[1..]),
    _ => Err(
      "usage: acpira-update-pack keygen | pack --app <Acpira.app | Acpira.exe> --version <v> --base-url <url> --out <dir> [options] \
       | merge --out <dir> <desktop-update.json>… [--key-env <var> | --unsigned]"
        .into(),
    ),
  };
  if let Err(e) = result {
    eprintln!("acpira-update-pack: {e}");
    std::process::exit(1);
  }
}

/// A new signing key: the seed (secret) on stdout, so `> file` captures only it; the public key on stderr
fn keygen() -> Result<(), String> {
  let mut seed = [0u8; 32];
  ring::rand::SecureRandom::fill(&ring::rand::SystemRandom::new(), &mut seed).map_err(|_| "no system randomness")?;
  let public = manifest::public_key(&seed)?;
  eprintln!("public key (rust/crates/acpira-desktop/src/update/mod.rs PUBLIC_KEYS): {}", manifest::hex(&public));
  eprintln!("the line on stdout is the secret seed: store it as the DESKTOP_UPDATE_KEY secret, never in the repository");
  println!("{}", manifest::hex(&seed));
  Ok(())
}

struct Opts {
  /// The `.app` folder, or the exe file (a one-file tree)
  app: PathBuf,
  version: String,
  base_url: String,
  out: PathBuf,
  prev: Vec<(String, PathBuf)>,
  notes: String,
  min_os: Option<String>,
  platform: String,
  level: i32,
  key: Option<[u8; 32]>,
}

fn parse(args: &[String]) -> Result<Opts, String> {
  let mut o = Opts {
    app: PathBuf::new(),
    version: String::new(),
    base_url: String::new(),
    out: PathBuf::new(),
    prev: vec![],
    notes: String::new(),
    min_os: None,
    platform: String::new(),
    level: 19,
    key: None,
  };
  let mut key = KeyOpts::default();
  let mut it = args.iter();
  while let Some(a) = it.next() {
    let mut value = || it.next().cloned().ok_or_else(|| format!("{a} needs a value"));
    match a.as_str() {
      "--app" => o.app = value()?.into(),
      "--version" => o.version = value()?,
      "--base-url" => o.base_url = value()?.trim_end_matches('/').to_owned(),
      "--out" => o.out = value()?.into(),
      "--prev" => {
        let v = value()?;
        let (ver, file) = v.split_once('=').ok_or("--prev takes <version>=<full package>")?;
        o.prev.push((ver.to_owned(), file.into()));
      }
      "--notes-file" => o.notes = std::fs::read_to_string(value()?).map_err(|e| format!("--notes-file: {e}"))?.trim().to_owned(),
      "--min-os" => o.min_os = Some(value()?),
      "--platform" => o.platform = value()?,
      "--level" => o.level = value()?.parse().map_err(|_| "--level takes a number")?,
      "--key-env" => key.env = Some(value()?),
      "--unsigned" => key.unsigned = true,
      other => return Err(format!("unknown option {other}")),
    }
  }
  if o.platform.is_empty() {
    o.platform = if o.app.is_file() { EXE_PLATFORM.to_owned() } else { manifest::platform_key().unwrap_or_default() };
  }
  if o.app.as_os_str().is_empty() || o.version.is_empty() || o.base_url.is_empty() || o.out.as_os_str().is_empty() {
    return Err("--app, --version, --base-url and --out are required".into());
  }
  if manifest::parse_version(&o.version).is_none() || o.prev.iter().any(|(v, _)| manifest::parse_version(v).is_none()) {
    return Err("versions must be plain (1.11.0)".into());
  }
  if o.platform.is_empty() {
    return Err("--platform is required on this system".into());
  }
  o.key = key.seed()?;
  Ok(o)
}

/// `--key-env <var>` (the signing seed's variable, DESKTOP_UPDATE_KEY by default) or `--unsigned`
#[derive(Default)]
struct KeyOpts {
  env: Option<String>,
  unsigned: bool,
}

impl KeyOpts {
  fn seed(&self) -> Result<Option<[u8; 32]>, String> {
    if self.unsigned {
      return Ok(None);
    }
    let var = self.env.as_deref().unwrap_or("DESKTOP_UPDATE_KEY");
    let raw = std::env::var(var).map_err(|_| format!("{var} is not set (pass --unsigned for a dry run)"))?;
    manifest::key_of_hex(&raw).map(Some).ok_or_else(|| format!("{var} is not a 32-byte hex seed"))
  }
}

fn asset_of(path: &Path, url: String) -> Result<Asset, String> {
  let size = std::fs::metadata(path).map_err(|e| format!("{}: {e}", path.display()))?.len();
  let sha256 = delta::sha256_file(path).map_err(|e| format!("{}: {e}", path.display()))?;
  Ok(Asset { url, size, sha256 })
}

fn mb(n: u64) -> String {
  format!("{:.2} MB", n as f64 / 1e6)
}

fn pack(args: &[String]) -> Result<(), String> {
  let o = parse(args)?;
  std::fs::create_dir_all(&o.out).map_err(|e| format!("{}: {e}", o.out.display()))?;
  let work = o.out.join(format!(".work-{}", std::process::id()));
  let _ = std::fs::remove_dir_all(&work);
  std::fs::create_dir_all(&work).map_err(|e| format!("{}: {e}", work.display()))?;
  let result = pack_in(&o, &work);
  let _ = std::fs::remove_dir_all(&work);
  result
}

/// An exe as the one-file tree the packages describe: `<work>/tree/Acpira.exe`, whatever the input is called, with the
/// permissions Windows reports for every file (0o644), so a manifest packed anywhere matches what the client scans
fn exe_tree(exe: &Path, work: &Path) -> Result<PathBuf, String> {
  let tree = work.join("tree");
  std::fs::create_dir_all(&tree).map_err(|e| format!("{}: {e}", tree.display()))?;
  let to = tree.join(EXE_NAME);
  std::fs::copy(exe, &to).map_err(|e| format!("{}: {e}", exe.display()))?;
  #[cfg(unix)]
  {
    use std::os::unix::fs::PermissionsExt;
    std::fs::set_permissions(&to, std::fs::Permissions::from_mode(0o644)).map_err(|e| format!("{}: {e}", to.display()))?;
  }
  Ok(tree)
}

fn pack_in(o: &Opts, work: &Path) -> Result<(), String> {
  let (root, kind) = if o.app.is_file() { (exe_tree(&o.app, work)?, "exe") } else { (o.app.clone(), "app") };
  let full_name = format!("Acpira-{}-{}.{kind}.tar.zst", o.version, o.platform);
  let full_path = o.out.join(&full_name);
  eprintln!("packing {} → {full_name}", o.app.display());
  let files = delta::pack_full(&root, &full_path, o.level)?;
  let full = asset_of(&full_path, format!("{}/{full_name}", o.base_url))?;
  // The package must unpack to exactly the bundle it came from
  let check = work.join("full");
  delta::unpack_full(&full_path, &check)?;
  delta::verify_tree(&check, &files)?;
  eprintln!("  full package {} ({} files)", mb(full.size), files.len());
  let mut deltas = BTreeMap::new();
  for (from, base_pkg) in &o.prev {
    if !manifest::is_newer(&o.version, from) {
      return Err(format!("--prev {from} is not older than {}", o.version));
    }
    let base = work.join(format!("base-{from}"));
    delta::unpack_full(base_pkg, &base)?;
    let name = format!("Acpira-{from}-to-{}-{}.delta", o.version, o.platform);
    let path = o.out.join(&name);
    let stats = delta::make_delta(&base, &root, from, &o.version, &path, o.level)?;
    // Applied back onto its base, it must rebuild the new bundle exactly
    let rebuilt = work.join(format!("rebuilt-{from}"));
    delta::apply_delta(&path, &base, &rebuilt)?;
    delta::verify_tree(&rebuilt, &files)?;
    let a = asset_of(&path, format!("{}/{name}", o.base_url))?;
    let share = a.size as f64 / full.size as f64;
    eprintln!(
      "  delta from {from}: {} ({:.0}% of full; keep {}, patch {}, add {}, link {})",
      mb(a.size),
      share * 100.0,
      stats.keep,
      stats.patch,
      stats.add,
      stats.link
    );
    if share > 0.6 {
      // The app takes the full package past this share anyway; not worth an asset
      eprintln!("  dropped: larger than 60% of the full package");
      let _ = std::fs::remove_file(&path);
      continue;
    }
    deltas.insert(from.clone(), a);
  }
  let m = Manifest {
    schema: manifest::SCHEMA,
    version: o.version.clone(),
    notes: o.notes.clone(),
    minimum_system_version: o.min_os.clone(),
    platforms: BTreeMap::from([(o.platform.clone(), PlatformBuild { full, files, deltas })]),
  };
  write_manifest(&o.out, &m, o.key.as_ref())
}

/// `desktop-update.json` in `out`, and its signature beside it when there is a key
fn write_manifest(out: &Path, m: &Manifest, key: Option<&[u8; 32]>) -> Result<(), String> {
  let mut bytes = serde_json::to_vec_pretty(m).map_err(|e| e.to_string())?;
  bytes.push(b'\n');
  let manifest_path = out.join("desktop-update.json");
  std::fs::write(&manifest_path, &bytes).map_err(|e| format!("{}: {e}", manifest_path.display()))?;
  let sig_path = out.join("desktop-update.json.sig");
  match key {
    Some(seed) => {
      let sig = manifest::sign(&bytes, seed)?;
      // Verified with the key's own public half before it is published
      manifest::verify(&bytes, sig.as_bytes(), &[manifest::public_key(seed)?])?;
      std::fs::write(&sig_path, format!("{sig}\n")).map_err(|e| e.to_string())?;
      eprintln!("  manifest signed by {}", manifest::hex(&manifest::public_key(seed)?));
    }
    None => {
      // A signature of earlier bytes must not sit beside the new manifest
      let _ = std::fs::remove_file(&sig_path);
      eprintln!("  manifest left unsigned (--unsigned)");
    }
  }
  Ok(())
}

/// Joins the per-platform manifests of one release (`Manifest::merge`) into `<out>/desktop-update.json`, signed
fn merge(args: &[String]) -> Result<(), String> {
  let mut out = PathBuf::new();
  let mut inputs: Vec<PathBuf> = vec![];
  let mut key = KeyOpts::default();
  let mut it = args.iter();
  while let Some(a) = it.next() {
    let mut value = || it.next().cloned().ok_or_else(|| format!("{a} needs a value"));
    match a.as_str() {
      "--out" => out = value()?.into(),
      "--key-env" => key.env = Some(value()?),
      "--unsigned" => key.unsigned = true,
      other if other.starts_with("--") => return Err(format!("unknown option {other}")),
      file => inputs.push(file.into()),
    }
  }
  if out.as_os_str().is_empty() || inputs.is_empty() {
    return Err("merge needs --out <dir> and at least one manifest".into());
  }
  let seed = key.seed()?;
  let mut merged: Option<Manifest> = None;
  for file in &inputs {
    let bytes = std::fs::read(file).map_err(|e| format!("{}: {e}", file.display()))?;
    let m: Manifest = serde_json::from_slice(&bytes).map_err(|e| format!("{}: {e}", file.display()))?;
    if m.schema != manifest::SCHEMA || manifest::parse_version(&m.version).is_none() {
      return Err(format!("{}: not a schema {} manifest with a plain version", file.display(), manifest::SCHEMA));
    }
    eprintln!("{}: {} for {}", file.display(), m.version, m.platforms.keys().cloned().collect::<Vec<_>>().join(", "));
    match &mut merged {
      None => merged = Some(m),
      Some(into) => into.merge(m).map_err(|e| format!("{}: {e}", file.display()))?,
    }
  }
  std::fs::create_dir_all(&out).map_err(|e| format!("{}: {e}", out.display()))?;
  write_manifest(&out, &merged.expect("at least one input"), seed.as_ref())
}

#[cfg(test)]
mod tests {
  use super::*;

  fn args(list: &[&str]) -> Vec<String> {
    list.iter().map(|s| (*s).to_owned()).collect()
  }

  fn s(p: &Path) -> &str {
    p.to_str().unwrap()
  }

  /// A file standing in for an exe: incompressible bytes with the version stamped in, as two builds differ
  fn exe(path: &Path, version: &str) {
    let mut x = 0x9e37_79b9_7f4a_7c15u64;
    let mut bytes: Vec<u8> = (0..150_000)
      .map(|_| {
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        (x >> 24) as u8
      })
      .collect();
    bytes[4000..4000 + version.len()].copy_from_slice(version.as_bytes());
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(path, bytes).unwrap();
  }

  #[test]
  fn an_exe_packs_as_a_one_file_tree_and_merges_with_the_mac_manifest() {
    let d = tempfile::tempdir().unwrap();
    let (v1, v2) = (d.path().join("build1/acpira-desktop-tauri.exe"), d.path().join("build2/Acpira.exe"));
    exe(&v1, "1.10.0");
    exe(&v2, "1.11.0");
    let base = "https://github.com/hotic/acpira/releases/download";
    let (prev, win) = (d.path().join("prev"), d.path().join("win"));
    pack(&args(&["--app", s(&v1), "--version", "1.10.0", "--base-url", base, "--out", s(&prev), "--level", "3", "--unsigned"])).unwrap();
    let old_pkg = prev.join("Acpira-1.10.0-windows-x86_64.exe.tar.zst");
    let prev_arg = format!("1.10.0={}", s(&old_pkg));
    pack(&args(&[
      "--app", s(&v2), "--version", "1.11.0", "--base-url", base, "--out", s(&win), "--level", "3", "--prev", &prev_arg, "--unsigned",
    ]))
    .unwrap();
    let m: Manifest = serde_json::from_slice(&std::fs::read(win.join("desktop-update.json")).unwrap()).unwrap();
    let build = &m.platforms["windows-x86_64"];
    // One file named Acpira.exe whatever the input was called, the mode Windows reports
    assert_eq!(build.files.len(), 1);
    assert_eq!((build.files[0].path.as_str(), build.files[0].mode), (EXE_NAME, 0o644));
    assert!(build.full.url.ends_with("/Acpira-1.11.0-windows-x86_64.exe.tar.zst"));
    assert!(build.deltas["1.10.0"].size < build.full.size / 4, "{:?}", build.deltas);
    assert!(!win.join("desktop-update.json.sig").exists());
    // The full package unpacks to the tree the client verifies
    let unpacked = d.path().join("unpacked");
    delta::unpack_full(&win.join("Acpira-1.11.0-windows-x86_64.exe.tar.zst"), &unpacked).unwrap();
    delta::verify_tree(&unpacked, &build.files).unwrap();

    // The macOS runner's manifest of the same release, then both merged into the one the release carries
    let mac = d.path().join("mac");
    std::fs::create_dir_all(&mac).unwrap();
    let mut mac_m = m.clone();
    mac_m.platforms = BTreeMap::from([("darwin-aarch64".to_owned(), build.clone())]);
    mac_m.notes = "Fixes".into();
    std::fs::write(mac.join("desktop-update.json"), serde_json::to_vec(&mac_m).unwrap()).unwrap();
    let out = d.path().join("out");
    let parts = [mac.join("desktop-update.json"), win.join("desktop-update.json")];
    merge(&args(&["--out", s(&out), s(&parts[0]), s(&parts[1]), "--unsigned"])).unwrap();
    let merged: Manifest = serde_json::from_slice(&std::fs::read(out.join("desktop-update.json")).unwrap()).unwrap();
    assert_eq!(merged.platforms.keys().collect::<Vec<_>>(), ["darwin-aarch64", "windows-x86_64"]);
    assert_eq!(merged.notes, "Fixes");
    // A platform twice is an error, not a silent overwrite
    assert!(merge(&args(&["--out", s(&out), s(&parts[1]), s(&parts[1]), "--unsigned"])).unwrap_err().contains("in both"));
  }
}
