//! The macOS default keychain, driven through /usr/bin/security (the tool Claude Code reads and writes its login with).
//! Keychain unlock state belongs to a security session: a remote workspace over SSH gets its own session in which the
//! login keychain stays locked while the desktop session has it unlocked, and every read fails with
//! errSecInteractionNotAllowed (exit 36) instead of prompting. `security unlock-keychain` run from a terminal of that
//! same session (the IDE server's integrated terminal descends from the same sshd session as the sidecar) unlocks it for
//! the sidecar and the CLIs it spawns. Seen 2026-09-27 on macOS 26 with Claude Code 2.1.224

use std::time::Duration;

const SECURITY: &str = "/usr/bin/security";
/// errSecInteractionNotAllowed: the keychain is locked and this session has no UI to unlock it
const INTERACTION_NOT_ALLOWED: i32 = 36;

fn command(args: &[&str]) -> tokio::process::Command {
  let mut cmd = tokio::process::Command::new(SECURITY);
  cmd.args(args).stdin(std::process::Stdio::null()).stderr(std::process::Stdio::null()).kill_on_drop(true);
  cmd
}

/// Path of the default keychain (`security default-keychain` prints it quoted and indented)
async fn default_keychain() -> Option<String> {
  let out = tokio::time::timeout(Duration::from_secs(10), command(&["default-keychain"]).output()).await.ok()?.ok()?;
  let path = String::from_utf8_lossy(&out.stdout).trim().trim_matches('"').to_owned();
  (out.status.success() && !path.is_empty()).then_some(path)
}

/// Whether the default keychain is locked with no way to unlock it from this session. `show-keychain-info` needs the
/// keychain unlocked and reveals no secret; it gets the path explicitly because without one it reports on `<NULL>`.
/// Any other outcome (unlocked, no keychain, no tool) counts as not locked
pub async fn default_keychain_locked() -> bool {
  let Some(path) = default_keychain().await else { return false };
  let status = command(&["show-keychain-info", &path]).stdout(std::process::Stdio::null()).status();
  matches!(tokio::time::timeout(Duration::from_secs(10), status).await, Ok(Ok(s)) if s.code() == Some(INTERACTION_NOT_ALLOWED))
}

/// The terminal command that unlocks the default keychain; `security` asks for the password on the terminal itself
pub fn unlock_command() -> (String, Vec<String>) {
  (SECURITY.to_owned(), vec!["unlock-keychain".to_owned()])
}
