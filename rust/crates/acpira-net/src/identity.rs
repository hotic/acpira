//! A device's identity: one Ed25519 key per install, whose public half is its iroh `EndpointId`. The TLS handshake proves
//! it, so a paired peer is recognised by the key alone and an address is only a hint.

use std::path::Path;

use anyhow::{Context, Result, bail};
pub use iroh::{EndpointId, SecretKey};

/// The key stored at `path` (32 raw bytes), created on first use. Unix: the file is written tmp + rename with mode 600;
/// a key another process wrote meanwhile wins, so two starts never end up with two identities
pub fn load_or_create(path: &Path) -> Result<SecretKey> {
  if let Some(key) = read(path)? {
    return Ok(key);
  }
  if let Some(dir) = path.parent() {
    std::fs::create_dir_all(dir).with_context(|| format!("create {}", dir.display()))?;
  }
  let key = SecretKey::generate();
  let tmp = path.with_extension(format!("tmp-{}", std::process::id()));
  write_private(&tmp, &key.to_bytes())?;
  // hard_link fails when the target exists: the first writer's key stays
  let linked = std::fs::hard_link(&tmp, path);
  let _ = std::fs::remove_file(&tmp);
  match linked {
    Ok(()) => Ok(key),
    Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => read(path)?.context("identity vanished"),
    Err(e) => Err(e).with_context(|| format!("write {}", path.display())),
  }
}

fn read(path: &Path) -> Result<Option<SecretKey>> {
  match std::fs::read(path) {
    Ok(bytes) => {
      let Ok(arr) = <[u8; 32]>::try_from(bytes.as_slice()) else { bail!("{} is not a key", path.display()) };
      Ok(Some(SecretKey::from_bytes(&arr)))
    }
    Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
    Err(e) => Err(e).with_context(|| format!("read {}", path.display())),
  }
}

fn write_private(path: &Path, bytes: &[u8]) -> Result<()> {
  use std::io::Write;
  let mut o = std::fs::OpenOptions::new();
  o.write(true).create_new(true);
  #[cfg(unix)]
  {
    use std::os::unix::fs::OpenOptionsExt;
    o.mode(0o600);
  }
  let mut f = o.open(path).with_context(|| format!("create {}", path.display()))?;
  f.write_all(bytes)?;
  f.sync_all()?;
  Ok(())
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn a_key_is_created_once_and_kept() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("devices").join("identity.key");
    let a = load_or_create(&path).unwrap();
    let b = load_or_create(&path).unwrap();
    assert_eq!(a.public(), b.public());
    #[cfg(unix)]
    {
      use std::os::unix::fs::PermissionsExt;
      assert_eq!(std::fs::metadata(&path).unwrap().permissions().mode() & 0o777, 0o600);
    }
    std::fs::write(&path, b"short").unwrap();
    assert!(load_or_create(&path).is_err());
  }
}
