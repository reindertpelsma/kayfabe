# SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later
# kf_cleanup.ps1 [-Id APP] -- after an app: remove its scheduled task, kill anything still running from the
# app trees or an Edge instance with a kf profile, so the next app starts from the same state.
param([string]$Id = '')
$ErrorActionPreference = 'Continue'
if ($Id) { try { Unregister-ScheduledTask -TaskName "kfapp_$Id" -Confirm:$false -ErrorAction SilentlyContinue } catch { } }
$n = 0
foreach ($p in @(Get-CimInstance Win32_Process -ErrorAction SilentlyContinue)) {
    $cl = [string]$p.CommandLine; $ex = [string]$p.ExecutablePath
    $mine = ($ex -like 'C:\kfapps\*') -or ($ex -match '^[A-Z]:\\(py|pkg)\\') -or ($cl -match 'user-data-dir=C:\\kf\\')
    if ($mine -and $p.ProcessId -ne $PID) { & taskkill.exe /PID $p.ProcessId /T /F 2>&1 | Out-Null; $n++ }
}
Write-Output "KFCLEAN killed=$n"
exit 0
