//! Cross-process mutex around a shared file (mirror of src/host/store/fileLock.ts, same on-disk protocol so TS windows
//! and Rust sidecars arbitrate with each other): `<file>.lock` is created with O_EXCL and carries `pid\ntoken`; a sibling
//! `<file>.lock.<token>` is the generation stale waiters rename, so a check made against a dead holder can never move a
//! lock a live host just created. Callers in this process queue per path, so the lock file only arbitrates between processes

use std::collections::HashMap;
use std::future::Future;
use std::io::ErrorKind;
use std::path::{Path, PathBuf};
use std::sync::{Arc, LazyLock};
use std::time::{Duration, Instant, SystemTime};

use anyhow::{Result, anyhow};
use tokio::fs;
use tokio::io::AsyncWriteExt;

use crate::util::random_hex;

// A holder that died leaves its lock behind; past this age another host takes it over
const STALE: Duration = Duration::from_secs(10);
// Give up on a lock that stays held this long
const WAIT: Duration = Duration::from_secs(5);

static CHAINS: LazyLock<parking_lot::Mutex<HashMap<PathBuf, Arc<tokio::sync::Mutex<()>>>>> = LazyLock::new(Default::default);

pub async fn with_file_lock<T, F, Fut>(file: &Path, f: F) -> Result<T>
where
  F: FnOnce() -> Fut,
  Fut: Future<Output = Result<T>>,
{
  let chain = CHAINS.lock().entry(file.to_path_buf()).or_default().clone();
  let guard = chain.lock().await;
  let out = locked(file, f).await;
  drop(guard);
  // Drop the per-path entry once nobody else holds a reference to it
  let mut chains = CHAINS.lock();
  if chains.get(file).is_some_and(|c| Arc::strong_count(c) <= 2) {
    chains.remove(file);
  }
  drop(chain);
  out
}

fn with_suffix(p: &Path, suffix: &str) -> PathBuf {
  let mut s = p.as_os_str().to_owned();
  s.push(suffix);
  PathBuf::from(s)
}

async fn locked<T, F, Fut>(file: &Path, f: F) -> Result<T>
where
  F: FnOnce() -> Fut,
  Fut: Future<Output = Result<T>>,
{
  let lock = with_suffix(file, ".lock");
  let deadline = Instant::now() + WAIT;
  let token = random_hex(16);
  let gen_path = with_suffix(&lock, &format!(".{token}"));
  let body = format!("{}\n{token}", std::process::id());
  let acquired: Result<()> = async {
    loop {
      fs::write(&gen_path, &body).await?;
      match fs::OpenOptions::new().write(true).create_new(true).open(&lock).await {
        Ok(mut h) => {
          h.write_all(body.as_bytes()).await?;
          h.flush().await?;
          return Ok(());
        }
        Err(e) => {
          let _ = fs::remove_file(&gen_path).await;
          if e.kind() != ErrorKind::AlreadyExists {
            return Err(e.into());
          }
          if claim_stale(&lock).await {
            continue;
          }
          if Instant::now() > deadline {
            return Err(anyhow!("{} is locked by another host ({})", file.display(), lock.display()));
          }
          if age(&lock).await.is_some() {
            let jitter = 5 + (random_hex(1).as_bytes()[0] as u64 % 20);
            tokio::time::sleep(Duration::from_millis(jitter)).await;
          }
        }
      }
    }
  }
  .await;
  if let Err(e) = acquired {
    release(&lock, &token).await;
    return Err(e);
  }
  let out = f().await;
  release(&lock, &token).await;
  out
}

async fn release(lock: &Path, token: &str) {
  let body = fs::read_to_string(lock).await.unwrap_or_default();
  if parse_lock(&body).1.as_deref() == Some(token) {
    let _ = fs::remove_file(lock).await;
  }
  let _ = fs::remove_file(with_suffix(lock, &format!(".{token}"))).await;
}

fn parse_lock(body: &str) -> (Option<i64>, Option<String>) {
  let mut lines = body.split('\n').map(|l| l.strip_suffix('\r').unwrap_or(l));
  let pid = lines.next().and_then(|l| l.trim().parse::<i64>().ok());
  let token = lines.next().map(str::trim).filter(|t| t.len() >= 16 && t.chars().all(|c| c.is_ascii_hexdigit())).map(str::to_owned);
  (pid, token)
}

async fn age(p: &Path) -> Option<Duration> {
  let m = fs::metadata(p).await.ok()?.modified().ok()?;
  Some(SystemTime::now().duration_since(m).unwrap_or_default())
}

async fn claim_stale(lock: &Path) -> bool {
  let body = fs::read_to_string(lock).await.unwrap_or_default();
  if body.is_empty() {
    return false;
  }
  let (pid, token) = parse_lock(&body);
  match age(lock).await {
    Some(a) if a > STALE => {}
    _ => return false,
  }
  if pid_alive(pid) {
    return false;
  }
  let tag = format!("{}-{}", std::process::id(), random_hex(4));
  if let Some(token) = token {
    let gen_path = with_suffix(lock, &format!(".{token}"));
    let dead = with_suffix(&gen_path, &format!(".dead-{tag}"));
    // Unique source: a second waiter renaming the same generation loses
    if fs::rename(&gen_path, &dead).await.is_err() {
      return false;
    }
    let still = fs::read_to_string(lock).await.unwrap_or_default();
    if parse_lock(&still).1.as_deref() == Some(token.as_str()) {
      let _ = fs::remove_file(lock).await;
    }
    let _ = fs::remove_file(&dead).await;
    return true;
  }
  // Legacy lock (pid only): keep the moved bytes only when they are still the observation we checked
  let claim = with_suffix(lock, &format!(".stale-{tag}"));
  if fs::rename(lock, &claim).await.is_err() {
    return false;
  }
  let moved = fs::read_to_string(&claim).await.unwrap_or_default();
  if moved == body {
    let _ = fs::remove_file(&claim).await;
    return true;
  }
  let _ = fs::rename(&claim, lock).await;
  false
}

#[cfg(unix)]
fn pid_alive(pid: Option<i64>) -> bool {
  let Some(pid) = pid.filter(|p| *p > 0 && *p <= i32::MAX as i64) else { return false };
  // SAFETY: signal 0 only checks for existence and permission
  let r = unsafe { libc::kill(pid as libc::pid_t, 0) };
  r == 0 || std::io::Error::last_os_error().raw_os_error() == Some(libc::EPERM)
}

#[cfg(not(unix))]
fn pid_alive(pid: Option<i64>) -> bool {
  // Without a portable liveness probe a stale-by-age lock is taken over; holders keep it for milliseconds
  let _ = pid;
  false
}

/// tmp + rename so a reader in another host never sees a half-written file; mode applies to the temp file and travels with the rename
pub async fn write_atomic(path: &Path, data: &[u8], mode: Option<u32>) -> Result<()> {
  let tmp = with_suffix(path, &format!(".{}.tmp", std::process::id()));
  let mut opts = fs::OpenOptions::new();
  opts.write(true).create(true).truncate(true);
  #[cfg(unix)]
  if let Some(m) = mode {
    opts.mode(m);
  }
  #[cfg(not(unix))]
  let _ = mode;
  let mut f = opts.open(&tmp).await?;
  f.write_all(data).await?;
  f.flush().await?;
  drop(f);
  fs::rename(&tmp, path).await?;
  Ok(())
}

#[cfg(test)]
mod tests {
  use super::*;
  use std::sync::atomic::{AtomicUsize, Ordering};

  #[tokio::test]
  async fn serializes_callers_and_cleans_up() {
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("x.json");
    let inside = Arc::new(AtomicUsize::new(0));
    let mut tasks = vec![];
    for _ in 0..8 {
      let (file, inside) = (file.clone(), inside.clone());
      tasks.push(tokio::spawn(async move {
        with_file_lock(&file, || async {
          assert_eq!(inside.fetch_add(1, Ordering::SeqCst), 0);
          tokio::time::sleep(Duration::from_millis(2)).await;
          inside.fetch_sub(1, Ordering::SeqCst);
          Ok(())
        })
        .await
        .unwrap();
      }));
    }
    for t in tasks {
      t.await.unwrap();
    }
    let left: Vec<_> = std::fs::read_dir(dir.path()).unwrap().collect();
    assert!(left.is_empty(), "lock files left behind");
  }

  #[tokio::test]
  async fn takes_over_a_dead_holders_lock() {
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("y.json");
    let lock = with_suffix(&file, ".lock");
    std::fs::write(&lock, "999999\n0123456789abcdef0123456789abcdef").unwrap();
    std::fs::write(with_suffix(&lock, ".0123456789abcdef0123456789abcdef"), "999999").unwrap();
    let old = SystemTime::now() - Duration::from_secs(60);
    std::fs::File::options().write(true).open(&lock).unwrap().set_modified(old).unwrap();
    let v = with_file_lock(&file, || async { Ok(7) }).await.unwrap();
    assert_eq!(v, 7);
    assert!(!lock.exists());
  }
}
