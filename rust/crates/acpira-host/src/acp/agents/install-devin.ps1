# Run in a child scope: the vendor installer changes preferences and local variables.
& {
  $ErrorActionPreference = "Stop";
  if (-not $env:LOCALAPPDATA) { throw "LOCALAPPDATA is not set" };

  # Fetch before moving the entrypoint, so an offline retry leaves it untouched.
  $installer = Invoke-RestMethod https://cli.devin.ai/install.ps1;
  $entry = Join-Path $env:LOCALAPPDATA "devin\cli\bin\devin.exe";
  $backup = $null;
  if (Test-Path -LiteralPath $entry) {
    # Windows permits renaming a running executable, but not overwriting it.
    # A unique sibling preserves the old process and stays on the same volume.
    $backup = $entry + ".acpira-" + [Guid]::NewGuid().ToString("N") + ".old";
    Move-Item -LiteralPath $entry -Destination $backup -ErrorAction Stop;
  };
  try {
    # Isolate vendor variables (notably EntryExe / ErrorActionPreference).
    # Native exit codes are global; a scriptblock invocation itself can report success.
    $global:LASTEXITCODE = 0;
    & ([ScriptBlock]::Create($installer));
    if ($global:LASTEXITCODE -ne 0) { throw "Devin setup failed (exit $global:LASTEXITCODE); see the installer output above" };
  } catch {
    if ($backup -and (Test-Path -LiteralPath $backup)) {
      if (-not (Test-Path -LiteralPath $entry)) {
        Move-Item -LiteralPath $backup -Destination $entry -ErrorAction Stop;
      } else {
        Write-Host "Installation failed; previous executable retained at: $backup";
      };
    };
    throw;
  };
  if ($backup) {
    # A mapped image may stay locked until its existing sessions exit.
    # Cleanup must not turn a successful installation into an error.
    Remove-Item -LiteralPath $backup -Force -ErrorAction SilentlyContinue;
    if (Test-Path -LiteralPath $backup) {
      Write-Host "Previous executable retained until Devin exits: $backup";
    };
  };
}
