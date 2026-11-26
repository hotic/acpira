//! Task-local wire identity for the request currently being handled by an engine.

tokio::task_local! {
  /// The wire whose view message is being handled: platform requests and localized strings produced meanwhile
  /// target that wire's capabilities and language.
  pub static ORIGIN: u64;
}

/// The wire behind the current task, when there is one.
pub fn origin() -> Option<u64> {
  ORIGIN.try_with(|o| *o).ok()
}
