$l = Get-Content C:\kfdbg\analyze.log -ErrorAction SilentlyContinue
"lines " + $l.Count
$i = 0
$on = $false
foreach ($x in $l) {
  if ($x -match 'KF_TDR_CONTEXT$') { $on = $true }
  if ($on -or $x -match 'BUGCHECK_P|STACK_TEXT|dxgkrnl!|nvlddmkm|FAILURE_BUCKET|Symbol .* not found|KF_DONE') {
    $i++
    if ($i -le 220) { $x.Substring(0, [Math]::Min(170, $x.Length)) }
  }
  if ($x -match 'KF_TDR_CONTEXT_END') { $on = $false }
}
