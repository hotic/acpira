# Offline regression tests; Windows additionally exercises a running executable.
param([string]$InstallerPath = (Join-Path $PSScriptRoot "../rust/crates/acpira-host/src/acp/agents/install-devin.ps1"))
$ErrorActionPreference = "Stop"
$source = (Get-Content -LiteralPath $InstallerPath | ForEach-Object { $_.Trim() } | Where-Object { $_ -and -not $_.StartsWith("#") }) -join " "
$install = [ScriptBlock]::Create($source)
$root = Join-Path ([IO.Path]::GetTempPath()) ("acpira-devin-test-" + [Guid]::NewGuid().ToString("N"))
$previousLocalAppData = $env:LOCALAPPDATA
$vendorSuccess = @'
$entry = "vendor-local-variable"
New-Item -ItemType Directory -Path (Split-Path $script:entry) -Force | Out-Null
Copy-Item -LiteralPath $script:versionExe -Destination $script:entry -Force
'@

function Assert($condition, $message) {
  if (-not $condition) { throw $message }
}
function Invoke-RestMethod($Uri) {
  Assert ($Uri -eq "https://cli.devin.ai/install.ps1") "Unexpected installer URL"
  if ($script:offline) { throw "offline fixture" }
  return $script:vendor
}
function New-Case($name, [bool]$existing = $true) {
  $env:LOCALAPPDATA = Join-Path $root $name
  $script:entry = Join-Path $env:LOCALAPPDATA "devin/cli/bin/devin.exe"
  New-Item -ItemType Directory -Path (Split-Path $script:entry) -Force | Out-Null
  $script:versionExe = Join-Path $env:LOCALAPPDATA "version.exe"
  [IO.File]::WriteAllText($script:versionExe, "new")
  if ($existing) { [IO.File]::WriteAllText($script:entry, "old") }
  $script:vendor = $vendorSuccess
  $script:offline = $false
}
function Backups {
  @(Get-ChildItem -LiteralPath (Split-Path $script:entry) -Filter "devin.exe.acpira-*.old")
}
function Expect-Failure($pattern) {
  $failure = $null
  try { & $install } catch { $failure = $_.Exception.Message }
  Assert ($failure -and $failure -match $pattern) "Expected failure matching $pattern, got $failure"
}

try {
  New-Case "fresh install" $false
  & $install
  Assert ([IO.File]::ReadAllText($script:entry) -eq "new") "Fresh install failed"

  # Keep this file ASCII so Windows PowerShell 5.1 does not need a UTF-8 BOM.
  New-Case ("existing O'Brien [" + [char]0x7528 + [char]0x6237 + "]")
  & $install
  Assert ([IO.File]::ReadAllText($script:entry) -eq "new") "Existing entry was not replaced"
  Assert (@(Backups).Count -eq 0) "Unlocked backup was not cleaned up"
  & $install
  Assert ([IO.File]::ReadAllText($script:entry) -eq "new") "Repeat install failed"

  New-Case "offline"
  $script:offline = $true
  Expect-Failure "offline fixture"
  Assert ([IO.File]::ReadAllText($script:entry) -eq "old") "Download failure changed the entry"
  Assert (@(Backups).Count -eq 0) "Download failure moved the entry"

  New-Case "failed before copy"
  $script:vendor = 'throw "installation fixture"'
  Expect-Failure "installation fixture"
  Assert ([IO.File]::ReadAllText($script:entry) -eq "old") "Missing entry was not restored"
  Assert (@(Backups).Count -eq 0) "Restoration left a redundant backup"

  New-Case "failed after copy"
  $script:vendor = $vendorSuccess + '; throw "setup fixture"'
  Expect-Failure "setup fixture"
  Assert ([IO.File]::ReadAllText($script:entry) -eq "new") "New entry was unexpectedly overwritten"
  Assert (@(Backups).Count -eq 1) "Failure discarded the recovery copy"
  Assert ([IO.File]::ReadAllText(@(Backups)[0].FullName) -eq "old") "Recovery copy was changed"

  New-Case "native setup failure"
  $script:vendor = $vendorSuccess + '; $native = [Diagnostics.Process]::GetCurrentProcess().MainModule.FileName; & $native -NoProfile -Command "exit 23"'
  Expect-Failure "Devin setup failed"
  Assert (@(Backups).Count -eq 1) "Native setup failure discarded the recovery copy"

  New-Case "rename denied"
  & {
    function Move-Item { throw "rename denied fixture" }
    Expect-Failure "rename denied fixture"
  }
  Assert ([IO.File]::ReadAllText($script:entry) -eq "old") "Installer ran after a failed rename"

  if ($env:OS -eq "Windows_NT") {
    New-Case "running executable"
    Copy-Item -LiteralPath (Join-Path $env:SystemRoot "System32/ping.exe") -Destination $script:entry -Force
    $start = New-Object System.Diagnostics.ProcessStartInfo
    $start.FileName = $script:entry
    $start.Arguments = "-t 127.0.0.1"
    $start.UseShellExecute = $false
    $start.CreateNoWindow = $true
    $start.RedirectStandardOutput = $true
    $process = [Diagnostics.Process]::Start($start)
    try {
      Assert (-not $process.HasExited) "Executable fixture exited early"
      $locked = $false
      try { Copy-Item -LiteralPath $script:versionExe -Destination $script:entry -Force } catch { $locked = $true }
      Assert $locked "Original Copy-Item did not reproduce the sharing violation"
      & $install
      Assert ([IO.File]::ReadAllText($script:entry) -eq "new") "Running entry could not be replaced"
      Assert (-not $process.HasExited) "Installation stopped the existing process"
    } finally {
      if (-not $process.HasExited) { $process.Kill(); $process.WaitForExit() }
      $process.Dispose()
    }
    Write-Host "PASS: Windows running executable survives replacement"
  } else {
    Write-Host "SKIP: Windows running executable (requires Windows)"
  }
  Write-Host "PASS: Devin installer offline regressions"
} finally {
  $env:LOCALAPPDATA = $previousLocalAppData
  Remove-Item -LiteralPath $root -Recurse -Force
}
