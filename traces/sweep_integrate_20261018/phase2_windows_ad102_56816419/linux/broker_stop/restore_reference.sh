#!/bin/bash
# Proposed only: requires coordinator approval after the recorded prep refusal.
set -euo pipefail
cd /root/kayfabe
[ "$(git rev-parse HEAD)" = 0c87acda802b18a610739e75f2b50049a9247067 ]
[ -z "$(pgrep -x qemu-system-x86 || true)" ]
[ "$(date -u +%s)" -lt "$(date -u -d '2026-10-20T03:30:00Z' +%s)" ]
[ ! -e /root/sweep-nvpv-reference ]
git clone --filter=blob:none --no-checkout https://github.com/reindertpelsma/nvkvm-pv.git /root/sweep-nvpv-reference
git -C /root/sweep-nvpv-reference cat-file -e 'badf2d7^{commit}'
git -C /root/sweep-nvpv-reference rev-parse 'badf2d7^{commit}' >/root/prov/sweep_broker_reference_revision.txt
export NVPV_DIR=/root/sweep-nvpv-reference
bash /root/sweep_desktop595_prep.sh
bash /root/sweep_desktop595_run.sh
bash /root/sweep_desktop580_prep.sh
bash /root/sweep_desktop580_run.sh
