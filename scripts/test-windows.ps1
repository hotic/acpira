# Offline compatibility checks; no real agents, credentials, logins or model calls.
$ErrorActionPreference = 'Stop'
Push-Location (Split-Path $PSScriptRoot -Parent)
try {
    & powershell.exe -NoLogo -NoProfile -ExecutionPolicy Bypass -File test/devin-install.test.ps1
    if ($LASTEXITCODE -ne 0) { throw 'Windows installer regressions failed' }
    & node --experimental-strip-types --test test/windowsTerminal.test.mjs
    if ($LASTEXITCODE -ne 0) { throw 'Windows terminal regressions failed' }
    Push-Location rust
    try {
        & cargo test --workspace --lib --locked
        if ($LASTEXITCODE -ne 0) { throw 'Windows unit regressions failed' }
        & cargo test -p acpira-host --test windows_launch --test windows_compat --locked
        if ($LASTEXITCODE -ne 0) { throw 'Windows process regressions failed' }
        & cargo clippy --workspace --all-targets --all-features --locked -- -D warnings
        if ($LASTEXITCODE -ne 0) { throw 'Windows target checks failed' }
    } finally { Pop-Location }
} finally { Pop-Location }
