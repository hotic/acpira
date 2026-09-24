//! The @ mention index for a shell without searchFiles (mirror of src/host/nodeFiles.ts): a capped breadth-first walk
//! of the workspace folder, cached briefly

use std::collections::VecDeque;
use std::path::Path;
use std::sync::Arc;
use std::time::{Duration, Instant};

use acpira_shared::protocol::FileHit;

use crate::acp::attachments::path_to_file_url;
use crate::file_rank::rank_files;

const TTL: Duration = Duration::from_secs(15);
const MAX_FILES: usize = 20_000;

pub struct NodeFiles {
  root: Arc<dyn Fn() -> String + Send + Sync>,
  cache: tokio::sync::Mutex<Option<(String, Instant, Arc<Vec<FileHit>>)>>,
}

impl NodeFiles {
  pub fn new(root: Arc<dyn Fn() -> String + Send + Sync>) -> Self {
    NodeFiles { root, cache: tokio::sync::Mutex::new(None) }
  }

  pub async fn search(&self, query: &str) -> Vec<FileHit> {
    let files = self.list().await;
    rank_files(&files, query, 20)
  }

  async fn list(&self) -> Arc<Vec<FileHit>> {
    let root = (self.root)();
    // Concurrent searches while a listing is in flight share it (they queue on the mutex, then hit the cache)
    let mut cache = self.cache.lock().await;
    if let Some((r, at, files)) = cache.as_ref()
      && *r == root
      && at.elapsed() < TTL
    {
      return files.clone();
    }
    let files = Arc::new(walk(&root).await);
    *cache = Some((root, Instant::now(), files.clone()));
    files
  }
}

async fn walk(root: &str) -> Vec<FileHit> {
  let mut files = vec![];
  let mut queue = VecDeque::from([root.to_owned()]);
  while let Some(dir) = queue.pop_front() {
    if files.len() >= MAX_FILES {
      break;
    }
    let Ok(mut rd) = tokio::fs::read_dir(&dir).await else { continue };
    let mut entries = vec![];
    while let Ok(Some(e)) = rd.next_entry().await {
      entries.push(e);
    }
    entries.sort_by_key(|e| e.file_name());
    for e in entries {
      if files.len() >= MAX_FILES {
        break;
      }
      let name = e.file_name().to_string_lossy().into_owned();
      let abs = Path::new(&dir).join(&name);
      let Ok(ft) = e.file_type().await else { continue };
      if ft.is_dir() {
        if name != "node_modules" && name != ".git" {
          queue.push_back(abs.to_string_lossy().into_owned());
        }
      } else if ft.is_file() {
        let rel = abs.strip_prefix(root).map(|p| p.to_string_lossy().replace('\\', "/")).unwrap_or_default();
        files.push(FileHit { uri: path_to_file_url(&abs.to_string_lossy()), path: rel });
      }
    }
  }
  files.sort_by(|a, b| {
    a.path.split('/').count().cmp(&b.path.split('/').count()).then_with(|| crate::inventory::locale_compare(&a.path, &b.path))
  });
  files
}
