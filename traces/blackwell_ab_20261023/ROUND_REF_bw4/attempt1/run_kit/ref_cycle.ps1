# BW4 REFERENCE: pnputil disable/enable x3 on native Windows; bounded waits; state is the verdict.
$ErrorActionPreference='Continue'
$g=@(Get-PnpDevice -PresentOnly -Class Display | Where-Object InstanceId -like 'PCI\VEN_10DE*')
if($g.Count -ne 1){ throw "expected one NVIDIA display device, actual=$($g.Count)" }
$id=$g[0].InstanceId
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
foreach($n in 1..3){
 "REF_CYCLE n=$n disable_start=$(Now)"
 pnputil /disable-device "$id" | Out-Null
 $off=WaitProblem 22 120
 "REF_CYCLE n=$n disabled=$off utc=$(Now) rc=$LASTEXITCODE"
 pnputil /enable-device "$id" | Out-Null
 $on=WaitProblem 0 120
 $c=(Get-CimInstance Win32_VideoController | Where-Object {$_.PNPDeviceID -like 'PCI\VEN_10DE*'})
 "REF_CYCLE n=$n code0=$on code=$([int]$c.ConfigManagerErrorCode) utc=$(Now)"
 Start-Sleep -Seconds 10
}
'REF_CYCLE_DONE'
