#!/bin/bash
# Owner 2026-10-24 decision 2: after a FATAL stop only, one bounded standard-VGA collection
# boot (no kf3 device, no GPU work) of the stopped Windows arm's disk, then pack every arm's
# logs into the private capture. Started by the operator; never starts a product boot.
set -uo pipefail
umask 077
exec >/root/prov/fatal_collect.log 2>&1
echo "FATAL_COLLECT_SH_START $(date -u -Is)"
[ -e /root/prov/XID_STOP ] || { echo FATAL_COLLECT_REFUSED_NO_MARKER; exit 2; }
rc=0
timeout 1500 python3 -I /root/sweep2/fatal_collect.py || rc=$?
bash /root/sweep2/pack_box.sh || echo PACK_BOX_FAILED
echo "FATAL_COLLECT_SH_EXIT rc=$rc $(date -u -Is)"
