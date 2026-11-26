//! Diff texts kept out of the record. An edit's diff carries the whole file before and after, so a long session that
//! kept editing one big file wrote the same file dozens of times into its record (one record ran to 36 MB, 30 of them
//! diff texts). A record write moves texts of `MIN_LEN` or more into the session's blob folder (zstd, named by the
//! text's hash, so an unchanged text is one blob however often it repeats) and the record names them
//! (`DiffSource::old_blob` / `new_blob`). The texts are read back only when a page asks for them.
//!
//! Older engines read such a source as `omitted` (the shape the Storage tab's slimming writes): they show the diff from
//! its lines and offer no whole-file view, and a record they rewrite drops the names

use std::collections::HashSet;
use std::path::Path;
use std::sync::Arc;

use anyhow::{Result, anyhow};
use acpira_shared::transcript::{DiffSource, TextBlobSink};

use super::blob_pool;
use super::transcript_store::blob_name;

/// Both texts together below this many bytes stay inline: a blob costs a file and a disk block of its own
pub const MIN_LEN: usize = 8 * 1024;

const EXT: &str = ".zst";
/// zstd's default: fast enough for a write path, and source text still shrinks several times over
const LEVEL: i32 = 3;

/// The blob name a text is stored under: its own hash, so equal texts (an edit's new text and the next edit's old one)
/// share one blob
pub fn name_of(text: &str) -> String {
  blob_name(EXT, text.as_bytes())
}

/// Whether a blob name is one of these texts' (by its extension; images and attachments never end so)
pub fn is_text_blob(name: &str) -> bool {
  name.ends_with(EXT)
}

/// The sink a record write of a session collects texts with; `known` names the blobs its folder `dir` already holds.
/// A known name is checked against the folder too: the Storage tab may have cleaned a blob no record named any more,
/// and the same text written again must not leave the record naming a missing blob
pub fn sink(known: Arc<parking_lot::Mutex<HashSet<String>>>, dir: std::path::PathBuf) -> TextBlobSink {
  let known = move |n: &str| {
    let mut k = known.lock();
    if !k.contains(n) {
      return false;
    }
    if dir.join(n).is_file() {
      return true;
    }
    k.remove(n);
    false
  };
  TextBlobSink { min_len: MIN_LEN, name: Box::new(name_of), known: Box::new(known), out: vec![] }
}

/// Compress and store texts a record write collected, in the session's blob folder `dir`. Blocking
pub fn store(pool: &Path, dir: &Path, texts: &[(String, String)]) -> Result<()> {
  if texts.is_empty() {
    return Ok(());
  }
  std::fs::create_dir_all(dir)?;
  for (name, text) in texts {
    let bytes = zstd::bulk::compress(text.as_bytes(), LEVEL)?;
    blob_pool::place(pool, dir, name, &bytes)?;
  }
  Ok(())
}

/// A text blob read back
pub fn decode(bytes: &[u8]) -> Result<String> {
  let raw = zstd::stream::decode_all(bytes)?;
  String::from_utf8(raw).map_err(|e| anyhow!("diff text is not UTF-8: {e}"))
}

/// `src` with its texts inline, reading its blobs through `read` (name → bytes); None when a blob cannot be read. A
/// source already inline comes back as it is
pub async fn resolve<F, Fut>(src: DiffSource, read: F) -> Option<DiffSource>
where
  F: Fn(String) -> Fut,
  Fut: std::future::Future<Output = Result<Vec<u8>>>,
{
  if !src.in_blobs() {
    return Some(src);
  }
  let text = async |name: &Option<String>| -> Option<String> {
    match name {
      Some(n) => decode(&read(n.clone()).await.ok()?).ok(),
      None => Some(String::new()),
    }
  };
  let old = text(&src.old_blob).await?;
  let new = text(&src.new_blob).await?;
  Some(DiffSource::whole(src.path, old, new))
}

#[cfg(test)]
mod tests {
  use super::*;

  #[tokio::test]
  async fn texts_round_trip_through_their_blobs() {
    let root = tempfile::tempdir().unwrap();
    let dir = root.path().join("s");
    let text = "fn main() {}\n".repeat(1000);
    let name = name_of(&text);
    store(&root.path().join(blob_pool::POOL_DIR), &dir, &[(name.clone(), text.clone())]).unwrap();
    assert!(std::fs::metadata(dir.join(&name)).unwrap().len() < text.len() as u64 / 10, "stored compressed");
    let src = DiffSource { path: "/a.rs".into(), omitted: true, new_blob: Some(name), ..Default::default() };
    let read = |n: String| {
      let p = dir.join(n);
      async move { Ok(std::fs::read(p)?) }
    };
    let back = resolve(src.clone(), read).await.unwrap();
    assert_eq!((back.old_text.as_str(), back.new_text.as_str(), back.omitted, back.in_blobs()), ("", text.as_str(), false, false));
    // A blob gone missing gives no source rather than a wrong one
    let gone = DiffSource { old_blob: Some("0000000000000000.zst".into()), ..src };
    assert!(resolve(gone, read).await.is_none());
  }
}
