//! A session's blobs (`<dir>/<id>/`): saving, copying, reading and adopting them, plus finished exports

use std::path::PathBuf;

use anyhow::{Result, anyhow};
use tokio::fs;

use acpira_shared::transcript::DiffSource;

use super::{TranscriptStore, blob_name, confined, is_blob_name, is_session_id};
use crate::store::blob_pool;
use crate::store::diff_texts;
use crate::store::file_lock::write_atomic;
use crate::i18n::tp;

impl TranscriptStore {
  pub async fn save_blob(&self, session_id: &str, ext: &str, bytes: &[u8]) -> Result<(String, PathBuf)> {
    let legal_ext = ext.strip_prefix('.').is_some_and(|e| !e.is_empty() && e.chars().all(|c| c.is_ascii_alphanumeric() || c == '_'));
    let illegal = || anyhow!(tp("host.blobIllegal", &[("path", &format!("{session_id}/*{ext}"))]));
    if !is_session_id(session_id) || !legal_ext {
      return Err(illegal());
    }
    let name = blob_name(ext, bytes);
    fs::create_dir_all(self.dir.join(session_id)).await?;
    let dir = confined(&self.dir, session_id).await.ok_or_else(illegal)?;
    // Shared with every session holding the same content (`blob_pool`); a file already there is never rewritten
    let (pool, owned, file) = (self.dir.join(blob_pool::POOL_DIR), bytes.to_vec(), name.clone());
    let path = tokio::task::spawn_blocking(move || blob_pool::place(&pool, &dir, &file, &owned)).await??;
    self.stub_stale.lock().insert(session_id.to_owned());
    Ok((name, path))
  }

  /// Copy blob `name` of session `from` into session `to` under the same name (one file on disk, `blob_pool`)
  pub async fn copy_blob(&self, from: &str, to: &str, name: &str) -> Result<()> {
    let bytes = self.read_blob(from, name).await?;
    self.put_blob(to, name, bytes).await
  }

  /// Store `bytes` as blob `name` of session `id`; the caller vouches that the name fits the content (`save_blob` names
  /// by the bytes, a diff text blob by its text, `diff_texts::name_of`)
  pub async fn put_blob(&self, id: &str, name: &str, bytes: Vec<u8>) -> Result<()> {
    let illegal = || anyhow!(tp("host.blobIllegal", &[("path", &format!("{id}/{name}"))]));
    if !is_session_id(id) || !is_blob_name(name) {
      return Err(illegal());
    }
    fs::create_dir_all(self.dir.join(id)).await?;
    let dir = confined(&self.dir, id).await.ok_or_else(illegal)?;
    let (pool, file) = (self.dir.join(blob_pool::POOL_DIR), name.to_owned());
    tokio::task::spawn_blocking(move || blob_pool::place(&pool, &dir, &file, &bytes)).await??;
    self.stub_stale.lock().insert(id.to_owned());
    Ok(())
  }

  /// `src` with its texts read back from the session's blobs when the record keeps them there (`diff_texts`)
  pub async fn resolve_diff(&self, session_id: &str, src: DiffSource) -> Option<DiffSource> {
    diff_texts::resolve(src, |name| async move { self.read_blob(session_id, &name).await }).await
  }

  pub fn blob_path(&self, session_id: &str, name: &str) -> Option<PathBuf> {
    if !is_session_id(session_id) || !is_blob_name(name) {
      return None;
    }
    Some(self.dir.join(session_id).join(name))
  }

  pub async fn read_blob(&self, session_id: &str, name: &str) -> Result<Vec<u8>> {
    let illegal = || anyhow!(tp("host.blobIllegal", &[("path", &format!("{session_id}/{name}"))]));
    if !is_session_id(session_id) || !is_blob_name(name) {
      return Err(illegal());
    }
    let dir = confined(&self.dir, session_id).await.ok_or_else(illegal)?;
    let file = confined(&dir, name).await.ok_or_else(illegal)?;
    Ok(fs::read(file).await?)
  }

  /// Blob `name` of session `id` opened for reading, with its length
  pub async fn open_blob(&self, session_id: &str, name: &str) -> Result<(fs::File, u64)> {
    let illegal = || anyhow!(tp("host.blobIllegal", &[("path", &format!("{session_id}/{name}"))]));
    if !is_session_id(session_id) || !is_blob_name(name) {
      return Err(illegal());
    }
    let dir = confined(&self.dir, session_id).await.ok_or_else(illegal)?;
    let file = confined(&dir, name).await.ok_or_else(illegal)?;
    let f = fs::File::open(file).await?;
    let len = f.metadata().await?.len();
    Ok((f, len))
  }

  /// Where a blob still coming over (`session_manager/hosts.rs`) is written: inside the sessions folder, so it moves in
  /// with a rename, and under a dot name no reader takes for a session
  pub fn partial_dir(&self) -> PathBuf {
    self.dir.join(".partial")
  }

  /// Move the finished file `src` in as blob `name` of session `id` (`blob_pool::adopt`); the caller checked that the
  /// name fits the content
  pub async fn adopt_blob(&self, id: &str, name: &str, src: PathBuf) -> Result<PathBuf> {
    let illegal = || anyhow!(tp("host.blobIllegal", &[("path", &format!("{id}/{name}"))]));
    if !is_session_id(id) || !is_blob_name(name) {
      return Err(illegal());
    }
    fs::create_dir_all(self.dir.join(id)).await?;
    let dir = confined(&self.dir, id).await.ok_or_else(illegal)?;
    let (pool, file) = (self.dir.join(blob_pool::POOL_DIR), name.to_owned());
    let path = tokio::task::spawn_blocking(move || blob_pool::adopt(&pool, &dir, &file, &src)).await??;
    self.stub_stale.lock().insert(id.to_owned());
    Ok(path)
  }

  /// A finished export lands next to the sessions dir (~/.acpira/exports)
  pub async fn write_export(&self, name: &str, content: &str) -> Result<PathBuf> {
    let illegal = || anyhow!(tp("host.blobIllegal", &[("path", name)]));
    if name.is_empty() || name.contains('/') || name.contains('\\') || name.contains("..") {
      return Err(illegal());
    }
    let dir = self.dir.parent().unwrap_or(&self.dir).join("exports");
    fs::create_dir_all(&dir).await?;
    let path = confined(&dir, name).await.ok_or_else(illegal)?;
    write_atomic(&path, content.as_bytes(), None).await?;
    // canonicalize() hands back `\\?\C:\…` on Windows; the path is shown to the user and opened by the IDE, so use the
    // ordinary drive / UNC spelling
    Ok(crate::platform::paths::for_cli(path))
  }
}
