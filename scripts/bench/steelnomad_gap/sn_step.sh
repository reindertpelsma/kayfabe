#!/usr/bin/env bash
# SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later
# sn_step.sh FX FY OUT.png: click at window-relative fractions in the 3DMark window (interactive session), wait 3 s, screenshot back to OUT.png (960 px wide).
set -euo pipefail
cat > /tmp/sn_step.ps1 <<PS
& C:\\kf\\sn\\sn_it.ps1 -ScriptArgs "-Action click -FX $1 -FY $2" -Wait 40
Start-Sleep 3
& C:\\kf\\sn\\sn_it.ps1 -ScriptArgs "-Action shot" -Wait 30
& C:\\kf\\sn\\sn_upload.ps1 -File C:\\kf\\sn\\shot.png -Name sngap_step
PS
"$(dirname "$0")/guest_run.sh" /tmp/sn_step.ps1 150 | grep -E "CLICKED|UPLOAD"
ssh ${SN_HOST:?set SN_HOST=root@<trusted-host>} "python3 -c \"
from PIL import Image
im=Image.open('/data/kfagent/r/sngap_step.txt').convert('RGB'); im.resize((960,540)).save('/tmp/sngap_step.png')\""
scp -q ${SN_HOST:?set SN_HOST=root@<trusted-host>}:/tmp/sngap_step.png "$3"
