//! The update manifest (`desktop-update.json` on the latest GitHub release) and its detached Ed25519 signature
//! (`desktop-update.json.sig`, the 64 signature bytes in hex). The signature covers the manifest's raw bytes, so the
//! digests of every package it names, and through the delta's digest the delta's own `from` list, are ours. It is
//! independent of the Apple code signature, which the staged bundle is checked against separately (`stage.rs`)

use std::cmp::Ordering;
use std::collections::BTreeMap;

use ring::signature::{ED25519, Ed25519KeyPair, KeyPair, UnparsedPublicKey};
use serde::{Deserialize, Serialize};

/// The manifest format this build reads
pub const SCHEMA: u32 = 1;
/// A manifest or signature larger than this is refused before parsing
pub const MAX_MANIFEST: usize = 4 << 20;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Manifest {
  pub schema: u32,
  pub version: String,
  #[serde(default, skip_serializing_if = "String::is_empty")]
  pub notes: String,
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub minimum_system_version: Option<String>,
  /// By platform key (`darwin-aarch64`, the ACP Registry's names); one build per platform
  pub platforms: BTreeMap<String, PlatformBuild>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PlatformBuild {
  /// The whole bundle as a zstd-compressed tar (`delta::pack_full`)
  pub full: Asset,
  /// Every file of the signed, notarized bundle: what a staged bundle must match exactly
  pub files: Vec<FileEntry>,
  /// Deltas by the version they start from
  #[serde(default)]
  pub deltas: BTreeMap<String, Asset>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Asset {
  pub url: String,
  pub size: u64,
  pub sha256: String,
}

/// One file of a bundle, by its path relative to the bundle root (`/`-separated). A symlink carries its target in
/// `link` and no size or digest
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FileEntry {
  pub path: String,
  #[serde(default)]
  pub size: u64,
  #[serde(default, skip_serializing_if = "String::is_empty")]
  pub sha256: String,
  /// Unix permission bits (0o755 = 493); 0 for a symlink
  #[serde(default)]
  pub mode: u32,
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub link: Option<String>,
}

/// Parses the manifest only once one of `keys` verifies `sig_hex` over its exact bytes
pub fn verify(bytes: &[u8], sig: &[u8], keys: &[[u8; 32]]) -> Result<Manifest, String> {
  if bytes.len() > MAX_MANIFEST || sig.len() > 1024 {
    return Err("the update manifest is too large".into());
  }
  let sig = unhex(std::str::from_utf8(sig).map_err(|_| "the manifest signature is not text")?.trim())
    .filter(|s| s.len() == 64)
    .ok_or("the manifest signature is not 64 bytes of hex")?;
  if keys.is_empty() {
    return Err("this build has no update key".into());
  }
  if !keys.iter().any(|k| UnparsedPublicKey::new(&ED25519, k).verify(bytes, &sig).is_ok()) {
    return Err("the update manifest's signature does not verify".into());
  }
  let m: Manifest = serde_json::from_slice(bytes).map_err(|e| format!("the update manifest does not parse: {e}"))?;
  if m.schema != SCHEMA {
    return Err(format!("update manifest schema {} (this build reads {SCHEMA})", m.schema));
  }
  if parse_version(&m.version).is_none() {
    return Err(format!("update manifest version {:?} is not a plain version", m.version));
  }
  Ok(m)
}

/// The detached signature (hex) of `bytes` with the 32-byte seed `seed`; the release workflow's pack tool signs with it
pub fn sign(bytes: &[u8], seed: &[u8; 32]) -> Result<String, String> {
  let kp = Ed25519KeyPair::from_seed_unchecked(seed).map_err(|e| format!("bad signing key: {e}"))?;
  Ok(hex(kp.sign(bytes).as_ref()))
}

/// The public key that verifies what `seed` signs
pub fn public_key(seed: &[u8; 32]) -> Result<[u8; 32], String> {
  let kp = Ed25519KeyPair::from_seed_unchecked(seed).map_err(|e| format!("bad signing key: {e}"))?;
  kp.public_key().as_ref().try_into().map_err(|_| "unexpected public key length".to_owned())
}

/// A 32-byte key from 64 hex digits
pub fn key_of_hex(s: &str) -> Option<[u8; 32]> {
  unhex(s.trim())?.try_into().ok()
}

/// `1.10.8` as numbers: one to four dot-separated components, digits only (a version is also a directory name)
pub fn parse_version(v: &str) -> Option<Vec<u64>> {
  let parts: Vec<u64> = v
    .split('.')
    .map(|p| if !p.is_empty() && p.len() <= 9 && p.bytes().all(|b| b.is_ascii_digit()) { p.parse().ok() } else { None })
    .collect::<Option<_>>()?;
  (1..=4).contains(&parts.len()).then_some(parts)
}

/// Numeric comparison with missing components as 0 (`1.10` = `1.10.0`); an unparsable side sorts lowest
pub fn compare_versions(a: &str, b: &str) -> Ordering {
  match (parse_version(a), parse_version(b)) {
    (Some(x), Some(y)) => {
      let n = x.len().max(y.len());
      let pad = |v: &Vec<u64>| (0..n).map(|i| v.get(i).copied().unwrap_or(0)).collect::<Vec<_>>();
      pad(&x).cmp(&pad(&y))
    }
    (Some(_), None) => Ordering::Greater,
    (None, Some(_)) => Ordering::Less,
    (None, None) => Ordering::Equal,
  }
}

pub fn is_newer(candidate: &str, current: &str) -> bool {
  compare_versions(candidate, current) == Ordering::Greater
}

/// Whether the system version `os` (`26.1`) meets the manifest's minimum; an unknown system version is let through
/// (the bundle's own LSMinimumSystemVersion still applies)
pub fn os_supports(minimum: Option<&str>, os: Option<&str>) -> bool {
  match (minimum, os) {
    (Some(min), Some(os)) => compare_versions(os, min) != Ordering::Less,
    _ => true,
  }
}

/// The ACP Registry's platform key for this build (`darwin-aarch64`, `linux-x86_64`, …)
pub fn platform_key() -> Option<String> {
  let os = match std::env::consts::OS {
    "macos" => "darwin",
    "linux" => "linux",
    "windows" => "windows",
    _ => return None,
  };
  let arch = match std::env::consts::ARCH {
    "aarch64" => "aarch64",
    "x86_64" => "x86_64",
    _ => return None,
  };
  Some(format!("{os}-{arch}"))
}

pub fn hex(bytes: &[u8]) -> String {
  bytes.iter().map(|b| format!("{b:02x}")).collect()
}

pub fn unhex(s: &str) -> Option<Vec<u8>> {
  if !s.len().is_multiple_of(2) {
    return None;
  }
  (0..s.len()).step_by(2).map(|i| u8::from_str_radix(s.get(i..i + 2)?, 16).ok()).collect()
}

#[cfg(test)]
mod tests {
  use super::*;

  const SEED: [u8; 32] = [7; 32];
  const OTHER: [u8; 32] = [9; 32];

  fn manifest(version: &str) -> Vec<u8> {
    serde_json::to_vec(&serde_json::json!({
      "schema": 1, "version": version, "notes": "n", "minimumSystemVersion": "26.0",
      "platforms": { "darwin-aarch64": {
        "full": { "url": "https://github.com/x", "size": 3, "sha256": "ab" },
        "files": [{ "path": "Contents/MacOS/Acpira", "size": 3, "sha256": "ab", "mode": 493 }],
        "deltas": { "1.10.8": { "url": "https://github.com/y", "size": 1, "sha256": "cd" } }
      } }
    }))
    .unwrap()
  }

  #[test]
  fn a_manifest_parses_only_under_a_good_signature_from_a_known_key() {
    let bytes = manifest("1.11.0");
    let sig = sign(&bytes, &SEED).unwrap();
    let keys = [public_key(&OTHER).unwrap(), public_key(&SEED).unwrap()];
    // Either of two built-in keys verifies (rotation)
    let m = verify(&bytes, sig.as_bytes(), &keys).unwrap();
    assert_eq!(m.version, "1.11.0");
    assert_eq!(m.platforms["darwin-aarch64"].files[0].mode, 0o755);
    assert_eq!(m.platforms["darwin-aarch64"].deltas["1.10.8"].size, 1);
    // A changed byte, a wrong key, a truncated or garbled signature, no key at all
    let mut tampered = bytes.clone();
    let at = tampered.windows(6).position(|w| w == b"1.11.0").unwrap();
    tampered[at + 3] = b'2';
    assert!(verify(&tampered, sig.as_bytes(), &keys).unwrap_err().contains("does not verify"));
    assert!(verify(&bytes, sig.as_bytes(), &[public_key(&OTHER).unwrap()]).unwrap_err().contains("does not verify"));
    assert!(verify(&bytes, &sig.as_bytes()[..100], &keys).is_err());
    assert!(verify(&bytes, b"zz", &keys).is_err());
    assert!(verify(&bytes, sig.as_bytes(), &[]).unwrap_err().contains("no update key"));
    // Whitespace around the hex (a trailing newline) is fine
    assert!(verify(&bytes, format!("{sig}\n").as_bytes(), &keys).is_ok());
  }

  #[test]
  fn a_signed_manifest_must_still_be_a_manifest_this_build_reads() {
    let keys = [public_key(&SEED).unwrap()];
    let bad_version = manifest("1.11.0-beta");
    assert!(verify(&bad_version, sign(&bad_version, &SEED).unwrap().as_bytes(), &keys).unwrap_err().contains("plain version"));
    let future = serde_json::to_vec(&serde_json::json!({ "schema": 2, "version": "9.0.0", "platforms": {} })).unwrap();
    assert!(verify(&future, sign(&future, &SEED).unwrap().as_bytes(), &keys).unwrap_err().contains("schema 2"));
  }

  #[test]
  fn versions_compare_numerically() {
    assert!(is_newer("1.10.9", "1.10.8"));
    assert!(is_newer("1.11.0", "1.10.10"));
    assert!(is_newer("2", "1.99.99"));
    assert!(!is_newer("1.10.8", "1.10.8"));
    assert!(!is_newer("1.10", "1.10.0"));
    assert!(!is_newer("1.9.10", "1.10.0"));
    assert!(!is_newer("garbage", "1.0.0"));
    assert!(is_newer("1.0.0", "garbage"));
    for bad in ["", "1..2", "1.2.3.4.5", "v1.2", "1.2-rc1", "../1", "1.2 "] {
      assert!(parse_version(bad).is_none(), "{bad}");
    }
  }

  #[test]
  fn the_minimum_system_version_gates_only_a_known_system() {
    assert!(os_supports(Some("26.0"), Some("26.0.1")));
    assert!(os_supports(Some("26.0"), Some("27")));
    assert!(!os_supports(Some("26.0"), Some("15.7")));
    assert!(os_supports(Some("26.0"), None));
    assert!(os_supports(None, Some("15.0")));
  }

  #[test]
  fn hex_round_trips_and_rejects_odd_or_foreign_digits() {
    assert_eq!(unhex(&hex(&[0, 15, 255])).unwrap(), vec![0, 15, 255]);
    assert!(unhex("abc").is_none());
    assert!(unhex("zz").is_none());
    assert_eq!(key_of_hex(&hex(&[1; 32])), Some([1; 32]));
    assert_eq!(key_of_hex(&hex(&[1; 31])), None);
  }
}
