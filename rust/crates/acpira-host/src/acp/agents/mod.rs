pub mod adapter_info;
pub mod cli_latest;
pub mod config;
pub mod installer;
pub use crate::acp::launch;
pub mod lock_holder;
pub use crate::acp::login_path;
pub mod node_runtime;
pub mod npm_latest;
pub mod pool;
pub mod registry;
pub mod spawn;
pub(crate) mod version;

/// Async purge hook for `TranscriptStore` that removes the built-in agent's log, folder, and scratch directory once
/// the last session record driving `acp` is permanently gone.
pub fn builtin_purge_hook(
  home: std::path::PathBuf,
  log: crate::store::transcript_store::LogFn,
) -> crate::store::transcript_store::PurgeHook {
  std::sync::Arc::new(move |removed: String, acp: String| {
    let home = home.clone();
    let log = log.clone();
    Box::pin(async move {
      let root = acpira_agent::acp::sessions_root(&home);
      if let Some(log_file) = crate::store::transcript_store::confined(&root, &format!("{acp}.jsonl")).await {
        let data = home.clone();
        if let Ok(Err(e)) = tokio::task::spawn_blocking(move || acpira_agent::scratch::remove_for_log(&log_file, &data)).await {
          log(&format!("session {removed}: scratch folder not removed ({e})"));
        }
      }
      for path in acpira_agent::acp::session_paths(&home, &acp) {
        let Some(name) = path.file_name().map(|n| n.to_string_lossy().into_owned()) else { continue };
        let Some(target) = crate::store::transcript_store::confined(&root, &name).await else { continue };
        let result = if target.is_dir() { tokio::fs::remove_dir_all(&target).await } else { tokio::fs::remove_file(&target).await };
        if let Err(e) = result.or_else(|e| if e.kind() == std::io::ErrorKind::NotFound { Ok(()) } else { Err(e) }) {
          log(&format!("session {removed}: built-in agent log {} not removed ({e})", target.display()));
        }
      }
    })
  })
}
