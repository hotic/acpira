//! The @ mention index for a shell without searchFiles: a capped breadth-first walk
//! of the session's project folder, cached briefly per folder

use std::collections::VecDeque;
use std::path::Path;
use std::sync::Arc;
use std::time::{Duration, Instant};

use acpira_shared::protocol::FileHit;

use crate::acp::session::attachments::path_to_file_url;
use crate::file_rank::rank_files;

const TTL: Duration = Duration::from_secs(15);
const MAX_FILES: usize = 20_000;
// Folders kept listed at once: a project shell's windows may each show a session in a different project
const ROOTS: usize = 4;

#[derive(Default)]
pub struct NodeFiles {
  // Most recently used last
  cache: tokio::sync::Mutex<Vec<(String, Instant, Arc<Vec<FileHit>>)>>,
}

impl NodeFiles {
  pub fn new() -> Self {
    Self::default()
  }

  pub async fn search(&self, root: &str, query: &str) -> Vec<FileHit> {
    let files = self.list(root).await;
    rank_files(&files, query, 20)
  }

  async fn list(&self, root: &str) -> Arc<Vec<FileHit>> {
    // Concurrent searches while a listing is in flight share it (they queue on the mutex, then hit the cache)
    let mut cache = self.cache.lock().await;
    let hit = cache.iter().position(|(r, _, _)| r == root);
    if let Some(i) = hit {
      let entry = cache.remove(i);
      if entry.1.elapsed() < TTL {
        let files = entry.2.clone();
        cache.push(entry);
        return files;
      }
    }
    let files = Arc::new(walk(root).await);
    cache.push((root.to_owned(), Instant::now(), files.clone()));
    if cache.len() > ROOTS {
      cache.remove(0);
    }
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

#[cfg(test)]
mod tests {
  use super::*;

  #[tokio::test]
  async fn each_folder_lists_its_own_files_and_the_oldest_folder_leaves_the_cache() {
    let dir = tempfile::tempdir().unwrap();
    let roots: Vec<String> = (0..=ROOTS)
      .map(|i| {
        let root = dir.path().join(format!("p{i}"));
        std::fs::create_dir_all(&root).unwrap();
        std::fs::write(root.join(format!("only{i}.txt")), "").unwrap();
        root.to_string_lossy().into_owned()
      })
      .collect();
    let files = NodeFiles::new();
    let paths = |hits: Vec<FileHit>| hits.into_iter().map(|h| h.path).collect::<Vec<_>>();
    assert_eq!(paths(files.search(&roots[0], "only").await), ["only0.txt"]);
    assert_eq!(paths(files.search(&roots[1], "only").await), ["only1.txt"]);
    for r in &roots[2..] {
      files.search(r, "only").await;
    }
    let cached: Vec<String> = files.cache.lock().await.iter().map(|(r, _, _)| r.clone()).collect();
    assert_eq!(cached.len(), ROOTS);
    assert!(!cached.contains(&roots[0]), "the least recently used folder went first");
  }
}
