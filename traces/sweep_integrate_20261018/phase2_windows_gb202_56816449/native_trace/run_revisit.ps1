$ErrorActionPreference='Stop'
Set-Location 'C:\ProgramData\KayfabeSweepNative'
$rc=1
try { & .\revisit.ps1; if($?) { $rc=0 } } catch { $_ | Out-String | Write-Output } finally { try { & .\install.ps1 -Action Stop } catch { $_ | Out-String | Write-Output; $rc=1 } }
"FINAL_NATIVE_TRACE_EXIT rc=$rc $([DateTime]::UtcNow.ToString('o'))"
exit $rc
