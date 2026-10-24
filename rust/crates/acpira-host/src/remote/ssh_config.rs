//! The hosts ~/.ssh/config names, for the add dialog: every `Host` alias that is not a pattern, through `Include`d files,
//! with the `HostName` / `User` / `Port` written in its own block as a first guess at the address. The real resolution
//! (Match blocks, `Host *` defaults, ProxyJump) is `ssh -G`'s job when the host is added or connected

use std::path::{Path, PathBuf};

use acpira_shared::remote::{SshHost, is_alias};

/// Include depth ssh itself allows
const MAX_DEPTH: usize = 16;

/// The aliases of `~/.ssh/config` in file order, each once
pub fn list_hosts(home: &Path) -> Vec<SshHost> {
  let ssh_dir = home.join(".ssh");
  let mut out: Vec<SshHost> = vec![];
  read_file(&ssh_dir.join("config"), &ssh_dir, home, 0, &mut out);
  out
}

#[derive(Default)]
struct Block {
  aliases: Vec<String>,
  host_name: Option<String>,
  user: Option<String>,
  port: Option<String>,
}

fn read_file(path: &Path, ssh_dir: &Path, home: &Path, depth: usize, out: &mut Vec<SshHost>) {
  if depth > MAX_DEPTH {
    return;
  }
  let Ok(text) = std::fs::read_to_string(path) else { return };
  let mut block: Option<Block> = None;
  for raw in text.lines() {
    let line = raw.trim();
    if line.is_empty() || line.starts_with('#') {
      continue;
    }
    let (key, value) = split_keyword(line);
    match key.to_ascii_lowercase().as_str() {
      "host" => {
        flush(block.take(), out);
        let aliases = words(value).into_iter().filter(|a| !a.contains(['*', '?', '!']) && is_alias(a)).collect();
        block = Some(Block { aliases, ..Block::default() });
      }
      // A Match block's options are conditional; its hosts are not aliases
      "match" => {
        flush(block.take(), out);
      }
      "include" => {
        for pattern in words(value) {
          for file in expand_include(&pattern, ssh_dir, home) {
            read_file(&file, ssh_dir, home, depth + 1, out);
          }
        }
      }
      k => {
        if let Some(b) = block.as_mut() {
          let first = || words(value).into_iter().next();
          match k {
            "hostname" if b.host_name.is_none() => b.host_name = first(),
            "user" if b.user.is_none() => b.user = first(),
            "port" if b.port.is_none() => b.port = first(),
            _ => {}
          }
        }
      }
    }
  }
  flush(block, out);
}

fn flush(block: Option<Block>, out: &mut Vec<SshHost>) {
  let Some(b) = block else { return };
  for alias in b.aliases {
    if out.iter().any(|h| h.alias == alias) {
      continue;
    }
    let host = b.host_name.clone().unwrap_or_else(|| alias.clone());
    let address = Some(format_address(b.user.as_deref(), &host, b.port.as_deref()));
    out.push(SshHost { alias, address });
  }
}

/// `user@host:port`, leaving out what is unknown or the default port
pub fn format_address(user: Option<&str>, host: &str, port: Option<&str>) -> String {
  let host = if host.contains(':') && !host.starts_with('[') { format!("[{host}]") } else { host.to_owned() };
  let mut s = String::new();
  if let Some(u) = user.filter(|u| !u.is_empty()) {
    s.push_str(u);
    s.push('@');
  }
  s.push_str(&host);
  if let Some(p) = port.filter(|p| !p.is_empty() && *p != "22") {
    s.push(':');
    s.push_str(p);
  }
  s
}

/// `Keyword value` or `Keyword=value`
fn split_keyword(line: &str) -> (&str, &str) {
  let end = line.find(|c: char| c.is_whitespace() || c == '=').unwrap_or(line.len());
  let (key, rest) = line.split_at(end);
  let rest = rest.trim_start();
  let rest = rest.strip_prefix('=').unwrap_or(rest).trim();
  (key, rest)
}

/// Whitespace-separated words, double quotes grouping
fn words(value: &str) -> Vec<String> {
  let mut out = vec![];
  let mut cur = String::new();
  let mut quoted = false;
  for c in value.chars() {
    match c {
      '"' => quoted = !quoted,
      c if c.is_whitespace() && !quoted => {
        if !cur.is_empty() {
          out.push(std::mem::take(&mut cur));
        }
      }
      c => cur.push(c),
    }
  }
  if !cur.is_empty() {
    out.push(cur);
  }
  out
}

/// An Include argument as ssh reads it: `~` is the home, a relative path is under ~/.ssh, `*` / `?` glob the last
/// segment (enough for `config.d/*` and the tools that write `Include ~/.orbstack/ssh/config`)
fn expand_include(pattern: &str, ssh_dir: &Path, home: &Path) -> Vec<PathBuf> {
  let path = if let Some(rest) = pattern.strip_prefix("~/") {
    home.join(rest)
  } else if Path::new(pattern).is_absolute() {
    PathBuf::from(pattern)
  } else {
    ssh_dir.join(pattern)
  };
  let name = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
  if !name.contains(['*', '?']) {
    return vec![path];
  }
  let Some(dir) = path.parent() else { return vec![] };
  let Ok(rd) = std::fs::read_dir(dir) else { return vec![] };
  let mut files: Vec<PathBuf> = rd.flatten().filter(|e| glob_match(&name, &e.file_name().to_string_lossy())).map(|e| e.path()).collect();
  files.sort();
  files
}

fn glob_match(pattern: &str, name: &str) -> bool {
  fn go(p: &[char], n: &[char]) -> bool {
    match (p.first(), n.first()) {
      (None, None) => true,
      (Some('*'), _) => go(&p[1..], n) || (!n.is_empty() && go(p, &n[1..])),
      (Some('?'), Some(_)) => go(&p[1..], &n[1..]),
      (Some(a), Some(b)) if a == b => go(&p[1..], &n[1..]),
      _ => false,
    }
  }
  go(&pattern.chars().collect::<Vec<_>>(), &name.chars().collect::<Vec<_>>())
}

/// What `ssh -G <alias>` resolved: the address the page shows and the first identity file
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Resolved {
  pub user: Option<String>,
  pub host: String,
  pub port: Option<String>,
  pub identity_file: Option<String>,
}

impl Resolved {
  pub fn address(&self) -> String {
    format_address(self.user.as_deref(), &self.host, self.port.as_deref())
  }
}

/// Parse `ssh -G` output (`key value` per line, keys lowercase)
pub fn parse_ssh_g(text: &str, home: &Path) -> Resolved {
  let mut r = Resolved::default();
  for line in text.lines() {
    let Some((k, v)) = line.split_once(' ') else { continue };
    match k {
      "hostname" => r.host = v.to_owned(),
      "user" => r.user = Some(v.to_owned()),
      "port" => r.port = Some(v.to_owned()),
      "identityfile" if r.identity_file.is_none() => {
        // ssh -G lists every default identity; the first one that exists is the one worth showing
        let full = v.strip_prefix("~/").map(|rest| home.join(rest)).unwrap_or_else(|| PathBuf::from(v));
        if full.exists() {
          r.identity_file = Some(v.to_owned());
        }
      }
      _ => {}
    }
  }
  r
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn lists_concrete_aliases_through_includes() {
    let home = tempfile::tempdir().unwrap();
    let ssh = home.path().join(".ssh");
    std::fs::create_dir_all(ssh.join("config.d")).unwrap();
    std::fs::write(
      ssh.join("config"),
      "Include config.d/*\nInclude ~/.other/ssh_config\n\nHost asgard-gen1\n  HostName 61.164.246.74\n  User hotic\n  Port 29654\n\nHost quark quark-n1 *.internal\n  HostName=n1.quark.dev\n  User ubuntu\n\nMatch originalhost asgard-pc exec \"x\"\n  User spark\n\nHost asgard-pc\n  IdentitiesOnly yes\n\nHost *\n  ServerAliveInterval 30\n",
    )
    .unwrap();
    std::fs::write(ssh.join("config.d/10-work"), "Host bifrost\n  HostName bifrost.asgard.sh\n  Port 22\n").unwrap();
    std::fs::create_dir_all(home.path().join(".other")).unwrap();
    std::fs::write(home.path().join(".other/ssh_config"), "Host orb\n  HostName 127.0.0.1\n  Port 32222\n  User default\n").unwrap();
    let hosts = list_hosts(home.path());
    let got: Vec<(&str, Option<&str>)> = hosts.iter().map(|h| (h.alias.as_str(), h.address.as_deref())).collect();
    assert_eq!(
      got,
      vec![
        ("bifrost", Some("bifrost.asgard.sh")),
        ("orb", Some("default@127.0.0.1:32222")),
        ("asgard-gen1", Some("hotic@61.164.246.74:29654")),
        ("quark", Some("ubuntu@n1.quark.dev")),
        ("quark-n1", Some("ubuntu@n1.quark.dev")),
        ("asgard-pc", Some("asgard-pc")),
      ]
    );
  }

  #[test]
  fn reads_ssh_g() {
    let home = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(home.path().join(".ssh")).unwrap();
    std::fs::write(home.path().join(".ssh/id_ed25519"), "k").unwrap();
    let text = "user hotic\nhostname 61.164.246.74\nport 29654\nidentityfile ~/.ssh/id_rsa\nidentityfile ~/.ssh/id_ed25519\n";
    let r = parse_ssh_g(text, home.path());
    assert_eq!(r.address(), "hotic@61.164.246.74:29654");
    assert_eq!(r.identity_file.as_deref(), Some("~/.ssh/id_ed25519"));
    assert_eq!(format_address(None, "fd00::3", Some("22")), "[fd00::3]");
  }
}
