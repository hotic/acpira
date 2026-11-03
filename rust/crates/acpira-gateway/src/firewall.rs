//! Windows Firewall and the LAN path. The first time a program listens, Windows asks whether to let it in. A prompt
//! that is dismissed or never seen (the gateway starts from a login task, often behind other windows) leaves inbound
//! block rules for the program, and block rules win over allow rules; a prompt answered with its defaults ticks private
//! networks only, while Windows files a new Wi-Fi as public. Either way a device on the same network cannot reach the
//! gateway, the code pairing (LAN only) fails, and a QR pairing has the relay alone. Outbound traffic, the relay
//! included, is never affected.
//!
//! `blocked` reads the rules that apply to the networks this computer is on now; `allow` replaces the program's inbound
//! rules with one allow rule for every network type, elevated through UAC. Both read rules by the gateway's own path,
//! which stays the same across upgrades (devices/stage.rs). Other systems have no such gate here.

/// Whether Windows Firewall turns away devices on the current network; false when it cannot tell
pub async fn blocked() -> bool {
  imp::blocked().await
}

/// Let devices on any network in: the program's inbound rules replaced by one allow rule, after a UAC prompt. Fails
/// when the prompt is declined
pub async fn allow() -> anyhow::Result<()> {
  imp::allow().await
}

#[cfg(windows)]
mod imp {
  use anyhow::{Context, Result, bail};
  use base64::Engine;

  /// No console window for the PowerShell children (the gateway runs without one)
  const CREATE_NO_WINDOW: u32 = 0x0800_0000;

  // The rules naming this program, as they apply to the current network types (HNetCfg.FwPolicy2 reads without
  // elevation and quickly): a block rule, or no allow rule at all, turns devices away. A network type the firewall is
  // off for lets everything in. Profiles: 1 domain, 2 private, 4 public; protocol 17 UDP, 256 any; action 0 block
  const CHECK: &str = r#"
$fw = New-Object -ComObject HNetCfg.FwPolicy2
$cur = $fw.CurrentProfileTypes
$on = $false
foreach ($b in 1, 2, 4) { if (($cur -band $b) -and $fw.FirewallEnabled($b)) { $on = $true } }
if (-not $on) { 'ok'; exit 0 }
$allow = $false; $block = $false
foreach ($r in $fw.Rules) {
  if ($r.Direction -ne 1 -or -not $r.Enabled -or -not $r.ApplicationName) { continue }
  if ([Environment]::ExpandEnvironmentVariables($r.ApplicationName) -ne $p) { continue }
  if (($r.Profiles -band $cur) -eq 0) { continue }
  if ($r.Protocol -ne 17 -and $r.Protocol -ne 256) { continue }
  if ($r.Action -eq 0) { $block = $true } else { $allow = $true }
}
if ($block -or -not $allow) { 'blocked' } else { 'ok' }
"#;

  // Run elevated: every inbound rule for this program goes (the prompt's block rules among them), then one allow rule
  // for every network type. UDP only: iroh speaks QUIC
  const ALLOW: &str = r#"
$ErrorActionPreference = 'Stop'
Get-NetFirewallApplicationFilter | Where-Object { $_.Program -and [Environment]::ExpandEnvironmentVariables($_.Program) -eq $p } |
  Get-NetFirewallRule | Where-Object Direction -eq Inbound | Remove-NetFirewallRule
New-NetFirewallRule -DisplayName 'Acpira' -Description 'Lets paired phones on this network reach Acpira remote access' `
  -Direction Inbound -Program $p -Action Allow -Profile Any -Protocol UDP | Out-Null
"#;

  /// A script with `$p` set to this program's path, as PowerShell's -EncodedCommand (UTF-16LE, base64): no quoting
  fn encoded(script: &str) -> Result<String> {
    let exe = std::env::current_exe().context("the gateway's own path")?;
    let path = exe.to_string_lossy().replace('\'', "''");
    let full = format!("$p = '{path}'\n{script}");
    let bytes: Vec<u8> = full.encode_utf16().flat_map(u16::to_le_bytes).collect();
    Ok(base64::engine::general_purpose::STANDARD.encode(bytes))
  }

  async fn powershell(args: &[&str]) -> Result<std::process::Output> {
    let mut cmd = tokio::process::Command::new("powershell.exe");
    cmd.args(["-NoProfile", "-NonInteractive", "-ExecutionPolicy", "Bypass"]).args(args).stdin(std::process::Stdio::null());
    cmd.creation_flags(CREATE_NO_WINDOW);
    Ok(cmd.output().await?)
  }

  pub async fn blocked() -> bool {
    let Ok(script) = encoded(CHECK) else { return false };
    match powershell(&["-EncodedCommand", &script]).await {
      Ok(o) => String::from_utf8_lossy(&o.stdout).trim() == "blocked",
      Err(_) => false,
    }
  }

  pub async fn allow() -> Result<()> {
    let script = encoded(ALLOW)?;
    // Start-Process -Verb RunAs raises the UAC prompt and throws when it is declined; the elevated child's exit code
    // says whether the rules were written
    let outer = format!(
      "$x = Start-Process -FilePath powershell.exe -Verb RunAs -Wait -PassThru -WindowStyle Hidden \
       -ArgumentList '-NoProfile','-NonInteractive','-ExecutionPolicy','Bypass','-EncodedCommand','{script}'; exit $x.ExitCode"
    );
    let o = powershell(&["-Command", &outer]).await?;
    if !o.status.success() {
      let why = String::from_utf8_lossy(&o.stderr);
      bail!("{}", why.lines().find(|l| !l.trim().is_empty()).unwrap_or("declined").trim());
    }
    Ok(())
  }
}

#[cfg(not(windows))]
mod imp {
  pub async fn blocked() -> bool {
    false
  }

  pub async fn allow() -> anyhow::Result<()> {
    Ok(())
  }
}
