$l = Get-Content C:\kfdbg\analyze.log -ErrorAction SilentlyContinue
$on = $false
foreach ($x in $l) {
  if ($x -match '^KF_RAW') { $on = $true }
  if ($on -and $x -notmatch 'NatVis|^\s*$') { $x.Substring(0, [Math]::Min(200, $x.Length)) }
}
