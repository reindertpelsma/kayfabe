#!/usr/bin/env bash
# SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later
# guest_put.sh LOCAL_FILE GUEST_PATH: copy a (small) file into the guest through the kfagent diag channel.
set -euo pipefail
f=$1; g=$2; b=$(base64 -w0 "$f")
ssh ${SN_HOST:?set SN_HOST=root@<trusted-host>} "cd /data && python3 kfagent_server.py run \"New-Item -ItemType Directory -Force (Split-Path '$g') | Out-Null; [IO.File]::WriteAllBytes('$g',[Convert]::FromBase64String('$b')); 'put ' + (Get-Item '$g').Length\" 60"
