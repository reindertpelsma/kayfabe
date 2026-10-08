# SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later
# session_probe.ps1 -- read-only (2026-10-08): the interactive side of a Windows guest. Which sessions and
# display-owning processes exist (LogonUI, dwm, winlogon, explorer), the System log's display-related
# events since boot, and the PnP state of every display-class device. One line per fact: `SES <what> <value>`.
$ErrorActionPreference = 'Continue'
"SES uptime_s " + [int]((Get-Date) - (Get-CimInstance Win32_OperatingSystem).LastBootUpTime).TotalSeconds
try { (quser 2>&1) | ForEach-Object { "SES quser $_" } } catch { "SES quser exception" }
Get-Process -Name LogonUI, dwm, winlogon, explorer, csrss -ErrorAction SilentlyContinue |
  ForEach-Object { "SES proc $($_.Name) pid=$($_.Id) session=$($_.SessionId)" }
$boot = (Get-CimInstance Win32_OperatingSystem).LastBootUpTime
Get-WinEvent -FilterHashtable @{LogName='System'; StartTime=$boot} -ErrorAction SilentlyContinue |
  Where-Object { $_.ProviderName -match 'nvlddmkm|Display|dxgkrnl|DxgKrnl|BasicDisplay|Kernel-PnP|Winlogon' -or $_.Id -in 4101,1001,41 } |
  Select-Object -First 40 |
  ForEach-Object { "SES evt $($_.TimeCreated.ToString('HH:mm:ss')) $($_.ProviderName) id=$($_.Id) lvl=$($_.LevelDisplayName) :: " + (($_.Message -replace '\s+',' ') -replace '^(.{0,220}).*','$1') }
Get-PnpDevice -Class Display -ErrorAction SilentlyContinue | ForEach-Object {
  $p = Get-PnpDeviceProperty -InstanceId $_.InstanceId -KeyName DEVPKEY_Device_ProblemStatus -ErrorAction SilentlyContinue
  "SES display '$($_.FriendlyName)' Status=$($_.Status) Problem=$($_.Problem) ProblemStatus=$('{0:x}' -f $p.Data) Id=$($_.InstanceId)"
}
Get-CimInstance Win32_VideoController | ForEach-Object { "SES vc '$($_.Name)' AdapterRAM=$($_.AdapterRAM) Mode=$($_.CurrentHorizontalResolution)x$($_.CurrentVerticalResolution)@$($_.CurrentRefreshRate) DriverVersion=$($_.DriverVersion)" }
