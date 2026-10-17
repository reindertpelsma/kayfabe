#!/usr/bin/env bash
set -euo pipefail
exec >/root/prov/gb206phase_gpu_wave.log 2>&1
trap 'rc=$?; echo "GPU_WAVE_EXIT rc=$rc $(date -u -Is)"' EXIT
echo "GPU_WAVE_START $(date -u -Is)"
grep -q '^V3_GATES_SUMMARY pass=9 fail=0 gate10=PASS' /root/prov/gb206phase_gates.log
grep -q '^GB206PHASE_LINUX_EXIT rc=0 ' /root/prov/gb206phase_linux.log
set +e
bash /root/gb206phase_raw.sh
raw_rc=$?
set -e
echo "RAW_WAVE_COMPLETE rc=$raw_rc $(date -u -Is)"
# A measured raw failure does not erase the separately authorized display test.
# The next lane still refuses if any QEMU/unclean owned process remains.
test -z "$(pgrep -x qemu-system-x86 || true)"
bash /root/gb206phase_windows.sh baseline
echo "DISPLAY_BASELINE_DIAGNOSTICS_COMPLETE $(date -u -Is)"
exit "$raw_rc"
