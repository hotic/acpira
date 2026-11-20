//! Installed versions of known non-adapter CLIs, without opening an ACP session.

use std::path::Path;
use std::process::Stdio;
use std::time::Duration;

use tokio::io::{AsyncRead, AsyncReadExt};

use super::{launch, registry::{AgentDef, SELF_AGENT_ID}};

const TIMEOUT: Duration = Duration::from_secs(3);
const OUTPUT_LIMIT: u64 = 8192;

pub(crate) async fn installed_version(binary: &str, def: &AgentDef) -> Option<String> {
  if def.id == SELF_AGENT_ID && std::env::current_exe().ok().as_deref() == Some(Path::new(binary)) {
    return Some(acpira_shared::version::VERSION.to_owned());
  }
  let prefix = def.version_prefix?;
  let mut cmd = launch::command(binary, &["--version".into()]);
  cmd.current_dir(crate::store::data_dir::home_dir()).stdin(Stdio::null()).stdout(Stdio::piped()).stderr(Stdio::piped()).kill_on_drop(true);
  cmd.envs(def.env.iter().flatten());
  #[cfg(unix)]
  cmd.process_group(0);
  #[cfg(windows)]
  let (mut child, _job) = crate::platform::windows_process::spawn(&mut cmd).await.ok()?;
  #[cfg(not(windows))]
  let mut child = cmd.spawn().ok()?;
  #[cfg(unix)]
  let mut group = launch::ProbeGroup(child.id());
  let stdout = child.stdout.take()?;
  let stderr = child.stderr.take()?;
  tokio::time::timeout(TIMEOUT, async {
    // Drain both pipes before reaping the group leader, in case a wrapper hands them to a child.
    let (stdout, stderr) = tokio::try_join!(read_output(stdout), read_output(stderr)).ok()?;
    let status = child.wait().await.ok()?;
    #[cfg(unix)]
    {
      group.0 = None;
    }
    if !status.success() {
      return None;
    }
    parse_version(&String::from_utf8_lossy(&stdout), prefix)
      .or_else(|| parse_version(&String::from_utf8_lossy(&stderr), prefix))
  })
  .await
  .ok()?
}

async fn read_output(pipe: impl AsyncRead + Unpin) -> std::io::Result<Vec<u8>> {
  let mut bytes = Vec::new();
  pipe.take(OUTPUT_LIMIT + 1).read_to_end(&mut bytes).await?;
  if bytes.len() as u64 > OUTPUT_LIMIT {
    return Err(std::io::Error::other("CLI version output exceeded its limit"));
  }
  Ok(bytes)
}

fn parse_version(output: &str, prefix: &str) -> Option<String> {
  output.lines().find_map(|line| {
    let line = line.trim();
    let candidate = match line.split_at_checked(prefix.len()) {
      Some((name, rest)) if name.eq_ignore_ascii_case(prefix) && rest.starts_with(char::is_whitespace) => rest.split_whitespace().next()?,
      _ => line,
    };
    semver::Version::parse(candidate.strip_prefix('v').unwrap_or(candidate)).ok().map(|version| version.to_string())
  })
}

#[cfg(test)]
mod tests {
  use super::parse_version;

  #[test]
  fn cli_version_formats_ignore_build_banners_and_unrelated_component_versions() {
    for (label, output, expected) in [
      ("agy", "1.5.13\n", Some("1.5.13")),
      ("devin", "devin 3000.14.30821 (a09cd7f323ee)\n", Some("3000.14.30821")),
      ("grok", "grok 1.0.77 (50ca579fa996) [stable]\n", Some("1.0.77")),
      ("Hermes Agent", "Hermes Agent v0.22.5+1115.g930e5ed (2026.9.24)\nPython: 3.14.9", Some("0.22.5+1115.g930e5ed")),
      ("Hermes Agent", "Python: 3.14.9\nOpenAI SDK: 2.54.0", None),
      ("kimi", "Warning: update available to 2.2.0", None),
    ] {
      assert_eq!(parse_version(output, label).as_deref(), expected);
    }
  }
}
