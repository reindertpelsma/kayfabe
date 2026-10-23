#!/usr/bin/env bash
# Missing tier-2 GB205 data only; all other audit-covered groups are subtracted.
set -euo pipefail
if [ "$1" != GB205 ]; then
 echo AUDIT_TIER2_GROUPS_ALREADY_COVERED
 exit 0
fi
printf 'subdevice\t0x20808162\t1\n' >/tmp/sweep-board-gate.tsv
chmod 0644 /tmp/sweep-board-gate.tsv
runuser -u nobody -- env KF_PROBE_I_KNOW=1 KF_PROBE_EXTRA_ONLY=1 KF_PROBE_HEAD=1 \
 /tmp/sweep-ctl-probe /tmp/sweep-board-gate.tsv >/root/prov/audit_gb205_8162.log 2>&1
runuser -u nobody -- env KF_PROBE_I_KNOW=1 KF_PROBE_EXTRA_ONLY=1 KF_PROBE_ZBC=1 \
 /tmp/sweep-ctl-probe >/root/prov/audit_gb205_zbc.log 2>&1
