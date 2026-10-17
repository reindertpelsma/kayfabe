$ErrorActionPreference='Stop';$ProgressPreference='SilentlyContinue'
Set-Location C:\ProgramData\KayfabeSweep
if(@(Get-Process cup2,cup3,cup8,kf_dxprobe,kfwc_collect -ErrorAction SilentlyContinue).Count) { throw 'Owned native workloads still active' }
if((Get-Service KayfabeWinCapture).Status -ne 'Stopped') { throw 'Old measurement service must be stopped' }
& .\install.ps1 -Action Remove
if((Get-FileHash caps581\kfwc.sys).Hash.ToLower() -ne '371744be56f471aca3a5e882bf08afc03525cc008525ef7cf44a41d81306e2af') { throw 'Caps module pin mismatch' }
if((Get-FileHash caps581\kfwc_collect.exe).Hash.ToLower() -ne '766dd94a088decfbd76db18b1ad4c18bd819bfa4461797e9dfbfaae755449afd') { throw 'Caps reader pin mismatch' }
try {
 & .\install.ps1 -Action Install -Build C:\ProgramData\KayfabeSweep\caps581
 & .\caps581\kfwc_collect.exe --display-caps
 $code=$LASTEXITCODE
 Write-Output "NATIVE_CAPS_READER_EXIT=$code"
 if($code) { throw 'Matching native caps read failed' }
 Write-Output 'NATIVE_CAPS58151_EXIT=0'
} finally { & .\install.ps1 -Action Stop }
& .\install.ps1 -Action Remove
Write-Output 'NATIVE_CAPS_SERVICE_REMOVED=1'
