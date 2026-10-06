# Install / remove the TURZX dashboard autostart (run from an administrator PowerShell).
#
#   install:  powershell -ExecutionPolicy Bypass -File scripts\autostart.ps1
#   remove:   powershell -ExecutionPolicy Bypass -File scripts\autostart.ps1 -Uninstall
#
# Copies the release build to %LOCALAPPDATA%\TurzxDashboard (so rebuilding never hits a
# locked exe) and registers a logon task that runs it with highest privileges (PawnIO
# sensors need admin; a task avoids the UAC prompt).
param(
    [switch]$Uninstall,
    [string]$Exe = (Join-Path $PSScriptRoot "..\rust\target\release\turzx-dashboard.exe")
)
$ErrorActionPreference = "Stop"
$TaskName = "TurzxDashboard"
$InstallDir = Join-Path $env:LOCALAPPDATA "TurzxDashboard"
$Target = Join-Path $InstallDir "turzx-dashboard.exe"

$principal = [Security.Principal.WindowsPrincipal][Security.Principal.WindowsIdentity]::GetCurrent()
if (-not $principal.IsInRole([Security.Principal.WindowsBuiltInRole]::Administrator)) {
    throw "Run this from an administrator PowerShell."
}

# Stop a running dashboard so the exe can be replaced. The supervisor restarts a killed
# worker, so stop the task first and repeat until no process is left.
if (Get-ScheduledTask -TaskName $TaskName -ErrorAction SilentlyContinue) {
    Stop-ScheduledTask -TaskName $TaskName -ErrorAction SilentlyContinue
    Unregister-ScheduledTask -TaskName $TaskName -Confirm:$false
}
for ($i = 0; $i -lt 20 -and (Get-Process turzx-dashboard -ErrorAction SilentlyContinue); $i++) {
    Get-Process turzx-dashboard -ErrorAction SilentlyContinue | Stop-Process -Force -ErrorAction SilentlyContinue
    Start-Sleep -Milliseconds 300
}

if ($Uninstall) {
    Remove-Item $Target -ErrorAction SilentlyContinue
    Write-Host "Autostart removed ($TaskName). Cache and logs stay in $InstallDir."
    return
}

if (-not (Test-Path $Exe)) { throw "Release build not found: $Exe (cargo build --release in rust\)" }
New-Item -ItemType Directory -Force $InstallDir | Out-Null
Copy-Item $Exe $Target -Force

$action = New-ScheduledTaskAction -Execute $Target -WorkingDirectory $InstallDir
$trigger = New-ScheduledTaskTrigger -AtLogOn -User "$env:USERDOMAIN\$env:USERNAME"
$trigger.Delay = "PT10S"  # let USB and the network come up
$task_principal = New-ScheduledTaskPrincipal -UserId "$env:USERDOMAIN\$env:USERNAME" -LogonType Interactive -RunLevel Highest
$settings = New-ScheduledTaskSettingsSet -AllowStartIfOnBatteries -DontStopIfGoingOnBatteries `
    -ExecutionTimeLimit ([TimeSpan]::Zero) -RestartCount 3 -RestartInterval (New-TimeSpan -Minutes 1) `
    -MultipleInstances IgnoreNew
Register-ScheduledTask -TaskName $TaskName -Action $action -Trigger $trigger -Principal $task_principal `
    -Settings $settings -Description "TURZX 8.8in LCD dashboard" | Out-Null

Start-ScheduledTask -TaskName $TaskName
Start-Sleep -Seconds 3
$running = Get-Process turzx-dashboard -ErrorAction SilentlyContinue
Write-Host "Installed $Target"
Write-Host "Task '$TaskName' registered (at logon, highest privileges); running now: $([bool]$running)"
Write-Host "Log: $InstallDir\dashboard.log"
