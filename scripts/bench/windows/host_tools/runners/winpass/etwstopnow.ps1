$s = logman stop kfdxg -ets 2>&1
'ETWSTOPNOW ' + ($s -join ' ') + ' ' + (Get-Date -Format o)
# keep this boot's trace: the autologger (Append Off) would overwrite C:\kf\kfdxg.etl at the next boot
$keep = 'C:\kf\kfdxg-stall-' + (Get-Date -Format 'HHmmss') + '.etl'
Move-Item C:\kf\kfdxg.etl $keep -ErrorAction SilentlyContinue
'ETWSTOPNOW kept ' + $keep + ' ' + (Get-Item $keep -ErrorAction SilentlyContinue).Length
