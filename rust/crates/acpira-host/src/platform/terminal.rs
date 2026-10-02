//! One Windows terminal launch for both IDE shells. PowerShell source travels as UTF-16LE base64;
//! native argv bypasses PowerShell 5.1's lossy native-command marshalling entirely.

use std::collections::BTreeMap;

use base64::Engine;

use super::command::{Os, quote_windows_argument, spawn_spec};

fn literal(s: &str) -> String {
  format!("'{}'", s.replace('\'', "''"))
}

pub fn windows_launch(command: &str, args: &[String], env: Option<&BTreeMap<String, Option<String>>>) -> (String, Vec<String>) {
  let mut script = String::from("$ErrorActionPreference = 'Stop'; ");
  for (key, value) in env.into_iter().flatten() {
    match value {
      Some(value) => script.push_str(&format!("[Environment]::SetEnvironmentVariable({}, {}, 'Process'); ", literal(key), literal(value))),
      // PowerShell binds $null to an empty .NET string; the Env provider actually removes the variable.
      None => script.push_str(&format!("Remove-Item -LiteralPath {} -ErrorAction SilentlyContinue; ", literal(&format!("Env:{key}")))),
    }
  }
  let installer = (command.eq_ignore_ascii_case("powershell") || command.eq_ignore_ascii_case("powershell.exe"))
    && args.len() == 2
    && args[0].eq_ignore_ascii_case("-Command");
  if installer {
    // Installer commands already contain PowerShell source, including call operators and nested quotes.
    script.push_str(&args[1]);
  } else {
    let spec = spawn_spec(command, args, Os::Windows, std::env::var("ComSpec").ok().as_deref());
    let arguments =
      if spec.verbatim { spec.args.join(" ") } else { spec.args.iter().map(|a| quote_windows_argument(a)).collect::<Vec<_>>().join(" ") };
    script.push_str(&format!(
      "$acpiraStart = New-Object System.Diagnostics.ProcessStartInfo; \
       $acpiraStart.FileName = {}; $acpiraStart.Arguments = {}; \
       $acpiraStart.UseShellExecute = $false; $acpiraStart.WorkingDirectory = (Get-Location).ProviderPath; \
       $acpiraChild = [System.Diagnostics.Process]::Start($acpiraStart); \
       $acpiraChild.WaitForExit(); $global:LASTEXITCODE = $acpiraChild.ExitCode; \
       $acpiraChild.Dispose(); if ($global:LASTEXITCODE -ne 0) {{ throw ('Command exited with code ' + $global:LASTEXITCODE) }}",
      literal(&spec.command),
      literal(&arguments),
    ));
  }
  let bytes: Vec<u8> = script.encode_utf16().flat_map(u16::to_le_bytes).collect();
  (
    "powershell.exe".into(),
    vec![
      "-NoLogo".into(),
      "-NoProfile".into(),
      "-NoExit".into(),
      "-EncodedCommand".into(),
      base64::engine::general_purpose::STANDARD.encode(bytes),
    ],
  )
}

#[cfg(test)]
mod tests {
  use super::*;

  fn source(args: &[String]) -> String {
    let bytes = base64::engine::general_purpose::STANDARD.decode(args.last().unwrap()).unwrap();
    String::from_utf16(&bytes.chunks_exact(2).map(|b| u16::from_le_bytes([b[0], b[1]])).collect::<Vec<_>>()).unwrap()
  }

  fn powershell() -> Option<String> {
    std::env::var("ACPIRA_TEST_POWERSHELL").ok().or_else(|| cfg!(windows).then(|| "powershell.exe".into()))
  }

  #[test]
  fn installer_source_is_not_passed_through_a_second_command_parser() {
    for input in [
      r"& 'c:\Users\Spark\.vscode\extensions\hotic.acpira-1.8.0-win32-x64\bin\acpira.exe' install-agent antigravity",
      r#"& 'C:\Users\O''Brien 用户\acpira.exe' install-agent antigravity; Write-Output "done""#,
      "irm https://x.ai/cli/install.ps1 | iex",
      "npm install -g --include=optional @agentclientprotocol/codex-acp@1.13.0",
      "Write-Output \"a $variable with quotes\"",
    ] {
      let (command, args) = windows_launch("powershell", &["-Command".into(), input.into()], None);
      assert_eq!(command, "powershell.exe");
      assert_eq!(&args[..4], ["-NoLogo", "-NoProfile", "-NoExit", "-EncodedCommand"]);
      assert!(source(&args).ends_with(input));
      assert!(!source(&args).contains("'powershell'"));
    }
  }

  #[test]
  fn installer_call_operator_handles_spaces_apostrophes_and_unicode() {
    let Some(shell) = powershell() else { return };
    let root = tempfile::tempdir().unwrap();
    let fixture = root.path().join("O'Brien 用户.ps1");
    let receipt = root.path().join("receipt.json");
    std::fs::write(&fixture, "[IO.File]::WriteAllText($env:ACPIRA_TERMINAL_RESULT, (ConvertTo-Json -InputObject @($args)))").unwrap();
    let input = format!("& {} install-agent antigravity", literal(fixture.to_str().unwrap()));
    let (_, args) = windows_launch(
      "powershell",
      &["-Command".into(), input],
      Some(&BTreeMap::from([("ACPIRA_TERMINAL_RESULT".into(), Some(receipt.to_string_lossy().into_owned()))])),
    );
    let output = std::process::Command::new(shell).args(args.iter().filter(|a| *a != "-NoExit")).output().unwrap();
    assert!(output.status.success(), "{}", String::from_utf8_lossy(&output.stderr));
    let text = std::fs::read_to_string(receipt).unwrap();
    assert_eq!(serde_json::from_str::<Vec<String>>(text.trim_start_matches('\u{feff}')).unwrap(), ["install-agent", "antigravity"]);
  }

  #[test]
  fn quoted_executable_without_call_operator_reproduces_reported_parser_error() {
    let Some(shell) = powershell() else { return };
    let old = r"'C:\Users\Spark\AppData\Local\devin\cli\bin\devin.exe' auth login";
    let output = std::process::Command::new(shell).args(["-NoProfile", "-Command", old]).output().unwrap();
    assert!(!output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("UnexpectedToken") || stderr.contains("Unexpected token 'auth'"), "{stderr}");
  }

  #[test]
  fn native_arguments_and_account_environment_survive_legacy_powershell() {
    let Some(shell) = powershell() else { return };
    let root = tempfile::tempdir().unwrap();
    let fixture = root.path().join("receipt 用户 O'Brien.cjs");
    let receipt = root.path().join("receipt.json");
    std::fs::write(&fixture, "require('node:fs').writeFileSync(process.env.ACPIRA_TERMINAL_RESULT, JSON.stringify({args:process.argv.slice(2), home:process.env.CODEX_HOME, backend:process.env.ACP_BACKEND ?? null}));").unwrap();
    let args: Vec<String> =
      ["", "auth", "login", "two words", "中文", "embedded\"quote", "trailing \\", "& %PATH% !x! ^"].map(String::from).to_vec();
    let mut native_args = vec![fixture.to_string_lossy().into_owned()];
    native_args.extend(args.clone());
    let home = root.path().join("isolated 用户 account").to_string_lossy().into_owned();
    let env = BTreeMap::from([
      ("ACPIRA_TERMINAL_RESULT".into(), Some(receipt.to_string_lossy().into_owned())),
      ("CODEX_HOME".into(), Some(home.clone())),
      ("ACP_BACKEND".into(), None),
    ]);
    let (_, encoded) = windows_launch("node", &native_args, Some(&env));
    // PowerShell 7 on macOS can exercise the 5.1 argument binder as well. ProcessStartInfo bypasses both.
    let script = format!("$PSNativeCommandArgumentPassing = 'Legacy'; {}", source(&encoded));
    let output =
      std::process::Command::new(&shell).args(["-NoProfile", "-Command", &script]).env("ACP_BACKEND", "must-be-removed").output().unwrap();
    assert!(output.status.success(), "{}", String::from_utf8_lossy(&output.stderr));
    let result: serde_json::Value = serde_json::from_slice(&std::fs::read(&receipt).unwrap()).unwrap();
    assert_eq!(result, serde_json::json!({ "args": args, "home": home, "backend": null }));
  }

  #[cfg(windows)]
  #[test]
  fn builtin_terminal_logins_preserve_native_and_npm_batch_arguments_and_account_environment() {
    let root = tempfile::Builder::new().prefix("acpira-auth O'Brien 用户 ").tempdir().unwrap();
    let fixture = root.path().join("fixture.cjs");
    let executable = root.path().join("agent.exe");
    let shim = root.path().join("agent.cmd");
    let receipt = root.path().join("receipt.json");
    let node = std::process::Command::new("node").args(["-p", "process.execPath"]).output().unwrap();
    assert!(node.status.success());
    std::fs::copy(String::from_utf8(node.stdout).unwrap().trim(), &executable).unwrap();
    std::fs::write(
      &fixture,
      "require('node:fs').writeFileSync(process.env.ACPIRA_TERMINAL_RESULT, JSON.stringify({args:process.argv.slice(2), home:process.env.CODEX_HOME, backend:process.env.ACP_BACKEND ?? null}));",
    )
    .unwrap();
    std::fs::write(&shim, "@echo off\r\n\"%~dp0agent.exe\" \"%~dp0fixture.cjs\" %*\r\n").unwrap();
    let home = root.path().join("isolated account").to_string_lossy().into_owned();
    let env = BTreeMap::from([
      ("ACPIRA_TERMINAL_RESULT".into(), Some(receipt.to_string_lossy().into_owned())),
      ("CODEX_HOME".into(), Some(home.clone())),
      ("ACP_BACKEND".into(), None),
    ]);
    let cases: &[(&str, &[&str])] = &[
      ("Devin", &["auth", "login"]),
      ("Codex", &["cli", "login"]),
      ("Claude", &["--cli", "auth", "login", "--claudeai"]),
      ("Claude Console", &["--cli", "auth", "login", "--console"]),
      ("Grok", &["login"]),
      ("Kimi", &[]),
      ("OpenCode", &["auth", "login"]),
      ("DSH", &["web"]),
      ("Pi", &[]),
      ("literal arguments", &["", "a&b", "a^b", "quote\"here", "%ACPIRA_LITERAL_TEST%", "!bang!", "end \\", "(group)"]),
    ];
    for (name, expected) in cases {
      for command in [&executable, &shim] {
        if receipt.exists() {
          std::fs::remove_file(&receipt).unwrap();
        }
        let mut args: Vec<String> = expected.iter().map(|s| (*s).to_owned()).collect();
        if command == &executable {
          args.insert(0, fixture.to_string_lossy().into_owned());
        }
        let (_, launch) = windows_launch(command.to_str().unwrap(), &args, Some(&env));
        let output = std::process::Command::new("powershell.exe")
          .args(launch.iter().filter(|a| *a != "-NoExit"))
          .env("ACP_BACKEND", "must-be-removed")
          .env("ACPIRA_LITERAL_TEST", "must-not-expand")
          .env("bang", "must-not-expand")
          .output()
          .unwrap();
        assert!(output.status.success(), "{name} {}: {}", command.display(), String::from_utf8_lossy(&output.stderr));
        let result: serde_json::Value = serde_json::from_slice(&std::fs::read(&receipt).unwrap()).unwrap();
        assert_eq!(result, serde_json::json!({ "args": expected, "home": home, "backend": null }), "{name} {}", command.display());
      }
    }
  }
}
