//! Running the system `ssh` for a host: the fixed options every connection gets, the remote command quoted for whatever
//! login shell the host has, and ssh's stderr mapped to a failure the page can offer a way out of (docs/dev/remote.md
//! "Errors under BatchMode")

use std::path::PathBuf;

use acpira_shared::remote::ConnectFailure;
use tokio::process::Command;

/// The ssh binary: `ACPIRA_SSH` (a fake for manual runs), else `ssh` on PATH
pub fn ssh_program() -> PathBuf {
  std::env::var_os("ACPIRA_SSH").map(PathBuf::from).unwrap_or_else(|| PathBuf::from("ssh"))
}

/// Where a host is reached: an alias from ~/.ssh/config, or a typed `user@host:port`
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Target {
  Alias(String),
  Typed { user: Option<String>, host: String, port: Option<u16> },
}

impl Target {
  /// A typed address when the alias looks like one (`user@host`, `host:port`, `[v6]:port`), else the alias itself
  pub fn of(alias: &str) -> Target {
    parse_typed(alias).unwrap_or_else(|| Target::Alias(alias.to_owned()))
  }

  /// The arguments that name the destination, ending with `--` and the destination
  fn args(&self) -> Vec<String> {
    match self {
      Target::Alias(a) => vec!["--".into(), a.clone()],
      Target::Typed { user, host, port } => {
        let mut v = vec![];
        if let Some(p) = port {
          v.push("-p".into());
          v.push(p.to_string());
        }
        v.push("--".into());
        v.push(match user {
          Some(u) => format!("{u}@{host}"),
          None => host.clone(),
        });
        v
      }
    }
  }
}

/// A host as one ssh program reaches it (the engine's tests run a fake per test, so the program is not process-wide)
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Host {
  pub program: PathBuf,
  pub target: Target,
}

impl Host {
  pub fn new(program: PathBuf, alias: &str) -> Host {
    Host { program, target: Target::of(alias) }
  }

  /// `ssh` with the options every helper connection gets: no prompts (BatchMode: a host key or a passphrase is settled
  /// in a terminal instead), keepalives, no agent / X11 forwarding and none of the config's port forwards, `-T` (no
  /// pty). `verbose` adds `-v` so a locked key can be told from a refused one. Dropping the command's child kills ssh
  pub fn command(&self, script: &str, verbose: bool) -> Command {
    let mut cmd = Command::new(&self.program);
    cmd.args(["-T", "-o", "BatchMode=yes", "-o", "ConnectTimeout=15", "-o", "ServerAliveInterval=15", "-o", "ServerAliveCountMax=3"]);
    cmd.args(["-o", "ForwardAgent=no", "-o", "ForwardX11=no", "-o", "ClearAllForwardings=yes", "-o", "PermitLocalCommand=no"]);
    if verbose {
      cmd.arg("-v");
    }
    cmd.args(self.target.args()).arg(remote_command(script)).kill_on_drop(true);
    cmd
  }

  /// `ssh -G` for the target: the resolved configuration, without connecting
  pub fn resolve_command(&self) -> Command {
    let mut cmd = Command::new(&self.program);
    cmd.arg("-G").args(self.target.args()).kill_on_drop(true);
    cmd
  }

  /// The arguments of an interactive `ssh` to the host, for a terminal on this machine
  pub fn terminal_args(&self) -> Vec<String> {
    self.target.args()
  }
}

/// `user@host:port` → its parts; None when the text has neither a user nor a port (then it is an alias)
pub fn parse_typed(text: &str) -> Option<Target> {
  let (user, rest) = match text.rsplit_once('@') {
    Some((u, r)) if !u.is_empty() => (Some(u.to_owned()), r),
    Some(_) => return None,
    None => (None, text),
  };
  let (host, port) = if let Some(v6) = rest.strip_prefix('[') {
    let (h, after) = v6.split_once(']')?;
    let port = match after.strip_prefix(':') {
      Some(p) => Some(p.parse::<u16>().ok()?),
      None if after.is_empty() => None,
      None => return None,
    };
    (h.to_owned(), port)
  } else {
    match rest.rsplit_once(':') {
      Some((h, p)) if !h.contains(':') => (h.to_owned(), Some(p.parse::<u16>().ok()?)),
      Some(_) => return None,
      None => (rest.to_owned(), None),
    }
  };
  if host.is_empty() || host.starts_with('-') || (user.is_none() && port.is_none()) {
    return None;
  }
  Some(Target::Typed { user, host, port })
}

/// A POSIX shell word: single-quoted, with `'` written as `'\''`
pub fn sh_quote(s: &str) -> String {
  if !s.is_empty() && s.chars().all(|c| c.is_ascii_alphanumeric() || "-_./=:@%+,".contains(c)) {
    return s.to_owned();
  }
  format!("'{}'", s.replace('\'', r"'\''"))
}

/// The remote command for `script`: the host's login shell runs `/bin/sh -c '<script>'`, which bash, zsh, fish and
/// csh all read the same way, so the script itself is always POSIX sh
pub fn remote_command(script: &str) -> String {
  format!("/bin/sh -c {}", sh_quote(script))
}

/// What ssh's stderr says went wrong, with the line worth showing
pub fn classify(stderr: &str) -> (ConnectFailure, String) {
  let lines: Vec<&str> = stderr.lines().map(str::trim).filter(|l| !l.is_empty()).collect();
  let shown = lines.iter().rev().find(|l| !l.starts_with("debug") && !l.starts_with("OpenSSH_") && !l.starts_with('@')).copied().unwrap_or("").to_owned();
  let has = |needle: &str| lines.iter().any(|l| l.contains(needle));
  let kind = if has("Host key verification failed") || has("REMOTE HOST IDENTIFICATION HAS CHANGED") || has("No ED25519 host key is known") {
    ConnectFailure::HostKey
  } else if has("Permission denied") {
    // A key the server accepted but ssh could not use: passphrase-locked and not in an agent
    if has("Server accepts key") { ConnectFailure::KeyLocked } else { ConnectFailure::Denied }
  } else if has("Connection refused")
    || has("Operation timed out")
    || has("Connection timed out")
    || has("No route to host")
    || has("Could not resolve hostname")
    || has("Network is unreachable")
    || has("Connection closed by")
    || has("Connection reset")
  {
    ConnectFailure::Network
  } else {
    ConnectFailure::Other
  };
  (kind, shown)
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn typed_addresses_and_aliases() {
    assert_eq!(Target::of("asgard-gen1"), Target::Alias("asgard-gen1".into()));
    assert_eq!(Target::of("root@10.0.0.5:2222"), Target::Typed { user: Some("root".into()), host: "10.0.0.5".into(), port: Some(2222) });
    assert_eq!(Target::of("ci@[fd00::3]:22"), Target::Typed { user: Some("ci".into()), host: "fd00::3".into(), port: Some(22) });
    assert_eq!(Target::of("box:2200"), Target::Typed { user: None, host: "box".into(), port: Some(2200) });
    // Not a port: stays an alias, as does anything ssh could take for an option
    assert_eq!(Target::of("box:http"), Target::Alias("box:http".into()));
    assert_eq!(Target::of("u@-oProxyCommand=x"), Target::Alias("u@-oProxyCommand=x".into()));
    assert_eq!(Target::Typed { user: Some("u".into()), host: "h".into(), port: Some(2) }.args(), vec!["-p", "2", "--", "u@h"]);
  }

  // The quoting is for the SSH host's shell; Windows has no /bin/sh to check it with
  #[cfg(unix)]
  #[test]
  fn quoting_survives_any_shell_text() {
    assert_eq!(sh_quote("/home/u/x"), "/home/u/x");
    assert_eq!(sh_quote("a b"), "'a b'");
    assert_eq!(sh_quote("it's"), r"'it'\''s'");
    let out = std::process::Command::new("/bin/sh").arg("-c").arg(format!("printf %s {}", sh_quote("$HOME `x` 'q' \"d\" \\ ;"))).output().unwrap();
    assert_eq!(String::from_utf8_lossy(&out.stdout), "$HOME `x` 'q' \"d\" \\ ;");
  }

  #[test]
  fn classifies_what_ssh_says() {
    assert_eq!(classify("Host key verification failed.\n").0, ConnectFailure::HostKey);
    assert_eq!(classify("@@@@\n@    WARNING: REMOTE HOST IDENTIFICATION HAS CHANGED!     @\nHost key verification failed.").0, ConnectFailure::HostKey);
    assert_eq!(classify("hotic@1.2.3.4: Permission denied (publickey).").0, ConnectFailure::Denied);
    let locked = classify("debug1: Server accepts key: /k ED25519 SHA256:x explicit\nspark@127.0.0.1: Permission denied (publickey).");
    assert_eq!(locked, (ConnectFailure::KeyLocked, "spark@127.0.0.1: Permission denied (publickey).".into()));
    assert_eq!(classify("ssh: connect to host h port 22: Operation timed out").0, ConnectFailure::Network);
    assert_eq!(classify("Connection closed by 198.18.0.13 port 22").0, ConnectFailure::Network);
    assert_eq!(classify("something else").0, ConnectFailure::Other);
  }
}
