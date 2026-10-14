# SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later
# dxg_etw_stop_now.ps1 — DIAGNOSTIC (2026-10-08): stop the boot-time DxgKrnl session DURING a stall (before any
# teardown) so the .etl keeps its tail, and keep it under its own name (the autologger overwrites kfdxg.etl at boot).
$s = logman stop kfdxg -ets 2>&1
'ETWSTOPNOW ' + ($s -join ' ') + ' ' + (Get-Date -Format o)
# keep this boot's trace: the autologger (Append Off) would overwrite C:\kf\kfdxg.etl at the next boot
$keep = 'C:\kf\kfdxg-stall-' + (Get-Date -Format 'HHmmss') + '.etl'
Move-Item C:\kf\kfdxg.etl $keep -ErrorAction SilentlyContinue
'ETWSTOPNOW kept ' + $keep + ' ' + (Get-Item $keep -ErrorAction SilentlyContinue).Length
