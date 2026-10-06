# Build the release exe and install it (developer shortcut for `turzx-dashboard install`).
#   powershell -ExecutionPolicy Bypass -File scripts\deploy.ps1
$ErrorActionPreference = "Stop"
$rust = Join-Path $PSScriptRoot "..\rust"
Push-Location $rust
try { cargo build --release } finally { Pop-Location }
$exe = Join-Path $rust "target\release\turzx-dashboard.exe"
$p = Start-Process $exe -ArgumentList "install" -Wait -PassThru
exit $p.ExitCode
