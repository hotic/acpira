//! SSH remote workspaces, mirroring packages/shared/remote.ts. A folder on an SSH host is written `ssh://<alias>/<absolute
//! path>` in every field that holds a folder (a session's cwd, the project list, records); the alias is the host's name
//! in ~/.ssh/config (or a typed `user@host:port`) and is the host's identity. Parsing and formatting live here and
//! nowhere else, so two spellings of one folder never become two projects. Both sides run test/fixtures/remote-folders.json

use serde::{Deserialize, Serialize};

pub const REMOTE_SCHEME: &str = "ssh://";

/// A folder on an SSH host, normalized: `path` is absolute, without empty, `.` or trailing segments
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct RemoteFolder {
  pub alias: String,
  pub path: String,
}

impl RemoteFolder {
  /// The canonical `ssh://` form
  pub fn uri(&self) -> String {
    format_remote_folder(&self.alias, &self.path).unwrap_or_default()
  }
}

/// Whether a folder string names a remote folder at all (well formed or not): local paths never start with the scheme
pub fn is_remote_folder(folder: &str) -> bool {
  folder.starts_with(REMOTE_SCHEME)
}

/// A host alias as ssh takes it on its command line: no whitespace or control characters, nothing that ends the
/// authority (`/ ? # \`), and no leading `-`, which ssh would read as an option
pub fn is_alias(alias: &str) -> bool {
  !alias.is_empty()
    && !alias.starts_with('-')
    && alias.chars().all(|c| !c.is_whitespace() && !c.is_control() && !matches!(c, '/' | '?' | '#' | '\\'))
}

/// An absolute POSIX path normalized: empty and `.` segments dropped, no trailing slash. `..` is refused rather than
/// resolved (a symlink makes `a/..` something else on the host), as is any control character
pub fn normalize_remote_path(path: &str) -> Option<String> {
  if !path.starts_with('/') || path.chars().any(char::is_control) {
    return None;
  }
  let mut parts = Vec::new();
  for seg in path.split('/') {
    match seg {
      "" | "." => {}
      ".." => return None,
      s => parts.push(s),
    }
  }
  Some(format!("/{}", parts.join("/")))
}

/// `ssh://<alias>/<path>` split and normalized; None for anything else, including a local path
pub fn parse_remote_folder(folder: &str) -> Option<RemoteFolder> {
  let rest = folder.strip_prefix(REMOTE_SCHEME)?;
  let slash = rest.find('/')?;
  let (alias, path) = rest.split_at(slash);
  if !is_alias(alias) {
    return None;
  }
  Some(RemoteFolder { alias: alias.to_owned(), path: normalize_remote_path(path)? })
}

/// The canonical form of a folder on a host; None when the alias or the path is not acceptable
pub fn format_remote_folder(alias: &str, path: &str) -> Option<String> {
  if !is_alias(alias) {
    return None;
  }
  Some(format!("{REMOTE_SCHEME}{alias}{}", normalize_remote_path(path)?))
}

/// How a host's sessions run by default: `local` keeps the agent loop and the logins here and runs only the tools on the
/// host (through `acpira remote`); `remote` drives the host's own engine
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub enum RunMode {
  #[default]
  Local,
  Remote,
}

/// A connection's state as the settings page and the column show it
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub enum ConnectionState {
  #[default]
  Off,
  Connecting,
  Connected,
  Error,
}

/// The step a connection attempt is on, shown in place on the host's page
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ConnectStep {
  /// ssh itself: resolving, the host key, authentication
  Ssh,
  /// Platform, installed helper version, machine id
  Probe,
  /// Uploading the helper binary
  Upload,
  /// Starting the long-lived helper and waiting for its handshake
  Start,
}

/// Why an attempt failed, so the page offers the matching way out
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ConnectFailure {
  /// The host key is unknown or changed: connect once in a terminal
  HostKey,
  /// The key is accepted but passphrase-locked and not in an agent: unlock it in a terminal
  KeyLocked,
  /// Authentication refused
  Denied,
  /// Unreachable, refused, timed out
  Network,
  /// The alias now leads to another machine than the one first connected to
  MachineChanged,
  /// No helper build for the host's platform
  Unsupported,
  /// No ssh on this machine
  NoSsh,
  Other,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ConnectionError {
  pub kind: ConnectFailure,
  pub step: ConnectStep,
  pub message: String,
}

/// One host as the pages see it (HostMsg `connections`)
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct ConnectionView {
  pub alias: String,
  pub mode: RunMode,
  #[serde(default, skip_serializing_if = "std::ops::Not::not")]
  pub disabled: bool,
  pub state: ConnectionState,
  /// While connecting: the step, and the newest line of what it does (an upload's progress)
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub step: Option<ConnectStep>,
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub detail: Option<String>,
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub error: Option<ConnectionError>,
  /// `user@host:port` as `ssh -G` resolves the alias
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub address: Option<String>,
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub identity_file: Option<String>,
  /// The helper's last measured round trip
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub rtt_ms: Option<u32>,
  /// The helper version installed on the host, once known
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub remote_version: Option<String>,
  /// The host's folders in the project list, as `ssh://` forms
  #[serde(default)]
  pub projects: Vec<String>,
}

/// A host ~/.ssh/config names, for the add dialog
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SshHost {
  pub alias: String,
  /// `user@host:port` when known without running ssh
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub address: Option<String>,
}

/// One entry of a remote directory listing
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RemoteDirEntry {
  pub name: String,
  pub dir: bool,
  /// A folder holding `.git`
  #[serde(default, skip_serializing_if = "std::ops::Not::not")]
  pub git: bool,
}

/// What the settings page asks of a host (WebviewMsg `connectionOp`); every op answers with a `connections` push
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "op", rename_all = "camelCase", rename_all_fields = "camelCase")]
pub enum ConnectionOp {
  /// Remember the host and connect to it
  Add { alias: String, mode: RunMode },
  /// Forget the host here; nothing on the host is touched
  Remove { alias: String },
  SetMode { alias: String, mode: RunMode },
  SetDisabled { alias: String, disabled: bool },
  /// `accept_new_machine`: the alias now leads to another machine and the user confirmed it is the right one
  Connect { alias: String, #[serde(default)] accept_new_machine: bool },
  Disconnect { alias: String },
  /// `ssh <alias>` in a terminal on this machine, to confirm a host key or unlock a key
  OpenTerminal { alias: String },
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn parses_exactly_like_the_typescript_mirror() {
    let fixture: serde_json::Value = serde_json::from_str(include_str!("../../../../test/fixtures/remote-folders.json")).unwrap();
    for case in fixture.as_array().unwrap() {
      let input = case["input"].as_str().unwrap();
      let parsed = parse_remote_folder(input);
      assert_eq!(parsed.as_ref().map(|f| f.alias.as_str()), case["alias"].as_str(), "alias of {input:?}");
      assert_eq!(parsed.as_ref().map(|f| f.path.as_str()), case["path"].as_str(), "path of {input:?}");
      assert_eq!(parsed.as_ref().map(RemoteFolder::uri).as_deref(), case["canonical"].as_str(), "canonical of {input:?}");
      if let Some(f) = parsed {
        assert_eq!(parse_remote_folder(&f.uri()), Some(f.clone()), "{input:?} round-trips");
      }
    }
  }

  #[test]
  fn local_paths_are_never_remote() {
    assert!(!is_remote_folder("/ssh:/x"));
    assert!(!is_remote_folder("ssh:/x"));
    assert!(is_remote_folder("ssh://gen1/x"));
    assert_eq!(format_remote_folder("-x", "/a"), None);
    assert_eq!(format_remote_folder("gen1", "relative"), None);
  }
}
