# B1n DIAGNOSTIC arm on_diag_pnp (owner 2026-10-24): the single 580.88 enable, then ONE
# pnputil disable/enable once Code 0 is observed (second enable; this arm only). Bounded waits;
# the device state is the verdict, not a command rc. A marker forbids any replay.
$ErrorActionPreference='Stop'
$g=@(Get-PnpDevice -PresentOnly -Class Display | Where-Object InstanceId -like 'PCI\VEN_10DE*')
if($g.Count -ne 1 -or [int]$g[0].Problem -ne 22) {
 throw 'Require one disabled device before the cycle'
}
$id=$g[0].InstanceId
$v=(Get-PnpDeviceProperty -InstanceId $id -KeyName DEVPKEY_Device_DriverVersion).Data
if($v -ne '32.0.15.8088') { throw 'Actual 580.88 required' }
$marker='C:\kf\enable580_cycle.started'
if(Test-Path $marker) { throw 'An earlier cycle exists; inspect outcome, never replay' }
[DateTime]::UtcNow.ToString('o') | Set-Content $marker
function Now { [DateTime]::UtcNow.ToString('o') }
function WaitProblem([int]$want, [int]$seconds) {
 $deadline=(Get-Date).AddSeconds($seconds)
 do {
  $p=Get-PnpDevice -InstanceId $id
  if([int]$p.Problem -eq $want -and ($want -ne 0 -or $p.Status -eq 'OK')) { return $true }
  Start-Sleep -Milliseconds 500
 } until ((Get-Date) -gt $deadline)
 return $false
}
Write-Output "ENABLE580_START actual=$v UTC=$(Now) label=DIAGNOSTIC enable=1of2"
pnputil /enable-device "$id"
Write-Output "ENABLE580_PNPUTIL_RC=$LASTEXITCODE UTC=$(Now)"
$first=WaitProblem 0 120
Write-Output "CYCLE_CODE0_FIRST=$first UTC=$(Now)"
if($first) {
 Write-Output "CYCLE_DISABLE_START UTC=$(Now)"
 pnputil /disable-device "$id"
 Write-Output "CYCLE_DISABLE_RC=$LASTEXITCODE UTC=$(Now)"
 $off=WaitProblem 22 120
 Write-Output "CYCLE_DISABLED=$off UTC=$(Now)"
 if($off) {
  Write-Output "CYCLE_ENABLE2_START UTC=$(Now) enable=2of2"
  pnputil /enable-device "$id"
  Write-Output "CYCLE_ENABLE2_RC=$LASTEXITCODE UTC=$(Now)"
  $second=WaitProblem 0 120
  Write-Output "CYCLE_CODE0_SECOND=$second UTC=$(Now)"
 }
}
Get-CimInstance Win32_VideoController |
 Select-Object Name,DriverVersion,ConfigManagerErrorCode | ConvertTo-Json -Compress
Get-PnpDevice -InstanceId $id | Select-Object FriendlyName,Status,Problem | ConvertTo-Json -Compress
Write-Output "ENABLE580_OBSERVED UTC=$(Now); device status is verdict, not this command rc"
