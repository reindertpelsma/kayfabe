# itask.ps1 -Script X.ps1 [-ScriptArgs "..."] [-WaitS 20] : run a script in the interactive session of `vast` (scheduled task, /IT) and wait.
param([Parameter(Mandatory)][string]$Script, [string]$ScriptArgs = '', [int]$WaitS = 20)
schtasks /Delete /TN kfi /F 2>$null | Out-Null
$tr = "powershell.exe -NoProfile -WindowStyle Hidden -ExecutionPolicy Bypass -File $Script $ScriptArgs"
schtasks /Create /TN kfi /TR $tr /SC ONCE /ST 23:59 /RU vast /IT /RL HIGHEST /F | Out-Null
schtasks /Run /TN kfi | Out-Null
Start-Sleep $WaitS
schtasks /Query /TN kfi /FO LIST | Select-String 'Status|Last Result'
