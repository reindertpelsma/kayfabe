# SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later
# sn_upload.ps1 -File C:\kf\sn\x.zip -Name NAME: POST a guest file to the host kfagent broker (10.0.2.2:8765 -> /data/kfagent/r/NAME.txt).
param([string]$File, [string]$Name)
$r = Invoke-WebRequest -UseBasicParsing -Uri "http://10.0.2.2:8765/result/$Name" -Method Post -InFile $File -TimeoutSec 300
"UPLOAD $Name " + (Get-Item $File).Length + " status " + $r.StatusCode
