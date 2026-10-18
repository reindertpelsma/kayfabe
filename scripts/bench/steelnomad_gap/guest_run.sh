#!/usr/bin/env bash
# SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later
# guest_run.sh FILE.ps1 [TIMEOUT]: run a PowerShell file in the guest (SYSTEM) via the kfagent diag channel and print its output.
set -euo pipefail
f=$1; t=${2:-90}
scp -q "$f" ${SN_HOST:?set SN_HOST=root@<trusted-host>}:/tmp/sn_run.ps1
ssh ${SN_HOST:?set SN_HOST=root@<trusted-host>} "cd /data && python3 kfagent_server.py run \"\$(cat /tmp/sn_run.ps1)\" $t"
