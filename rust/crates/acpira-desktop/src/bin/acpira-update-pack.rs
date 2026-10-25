//! The desktop update's release side (docs/releasing.md "Desktop app"): packs a signed, notarized `Acpira.app` as the
//! full package, makes a delta from each earlier version's full package, and writes the manifest with its Ed25519
//! signature. Every delta is applied back onto its base and checked against the new bundle before it is listed.
//!
//!   acpira-update-pack keygen
//!   acpira-update-pack pack --app Acpira.app --version 1.11.0 --base-url https://github.com/hotic/acpira/releases/download/v1.11.0 \
//!     --out dist/desktop [--prev 1.10.8=Acpira-1.10.8-darwin-aarch64.app.tar.zst]… [--notes-file notes.md] [--min-os 26.0] \
//!     [--platform darwin-aarch64] [--level 19] [--key-env DESKTOP_UPDATE_KEY | --unsigned]

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use acpira_desktop::update::delta;
use acpira_desktop::update::manifest::{self, Asset, Manifest, PlatformBuild};

fn main() {
  let args: Vec<String> = std::env::args().skip(1).collect();
  let result = match args.first().map(String::as_str) {
    Some("keygen") => keygen(),
    Some("pack") => pack(&args[1..]),
    _ => Err("usage: acpira-update-pack keygen | pack --app <Acpira.app> --version <v> --base-url <url> --out <dir> [options]".into()),
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
    platform: manifest::platform_key().unwrap_or_default(),
    level: 19,
    key: None,
  };
  let mut key_env = "DESKTOP_UPDATE_KEY".to_owned();
  let mut unsigned = false;
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
      "--key-env" => key_env = value()?,
      "--unsigned" => unsigned = true,
      other => return Err(format!("unknown option {other}")),
    }
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
  if !unsigned {
    let raw = std::env::var(&key_env).map_err(|_| format!("{key_env} is not set (pass --unsigned for a dry run)"))?;
    o.key = Some(manifest::key_of_hex(&raw).ok_or_else(|| format!("{key_env} is not a 32-byte hex seed"))?);
  }
  Ok(o)
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

fn pack_in(o: &Opts, work: &Path) -> Result<(), String> {
  let full_name = format!("Acpira-{}-{}.app.tar.zst", o.version, o.platform);
  let full_path = o.out.join(&full_name);
  eprintln!("packing {} → {full_name}", o.app.display());
  let files = delta::pack_full(&o.app, &full_path, o.level)?;
  let full = asset_of(&full_path, format!("{}/{full_name}", o.base_url))?;
  // The package must unpack to exactly the bundle it came from
  let check = work.join("full.app");
  delta::unpack_full(&full_path, &check)?;
  delta::verify_tree(&check, &files)?;
  eprintln!("  full package {} ({} files)", mb(full.size), files.len());
  let mut deltas = BTreeMap::new();
  for (from, base_pkg) in &o.prev {
    if !manifest::is_newer(&o.version, from) {
      return Err(format!("--prev {from} is not older than {}", o.version));
    }
    let base = work.join(format!("base-{from}.app"));
    delta::unpack_full(base_pkg, &base)?;
    let name = format!("Acpira-{from}-to-{}-{}.delta", o.version, o.platform);
    let path = o.out.join(&name);
    let stats = delta::make_delta(&base, &o.app, from, &o.version, &path, o.level)?;
    // Applied back onto its base, it must rebuild the new bundle exactly
    let rebuilt = work.join(format!("rebuilt-{from}.app"));
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
  let mut bytes = serde_json::to_vec_pretty(&m).map_err(|e| e.to_string())?;
  bytes.push(b'\n');
  let manifest_path = o.out.join("desktop-update.json");
  std::fs::write(&manifest_path, &bytes).map_err(|e| format!("{}: {e}", manifest_path.display()))?;
  match &o.key {
    Some(seed) => {
      let sig = manifest::sign(&bytes, seed)?;
      // Verified with the key's own public half before it is published
      manifest::verify(&bytes, sig.as_bytes(), &[manifest::public_key(seed)?])?;
      std::fs::write(o.out.join("desktop-update.json.sig"), format!("{sig}\n")).map_err(|e| e.to_string())?;
      eprintln!("  manifest signed by {}", manifest::hex(&manifest::public_key(seed)?));
    }
    None => eprintln!("  manifest left unsigned (--unsigned)"),
  }
  Ok(())
}
