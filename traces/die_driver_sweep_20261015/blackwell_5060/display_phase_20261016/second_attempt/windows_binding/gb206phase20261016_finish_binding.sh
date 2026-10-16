#!/usr/bin/env bash
set -euo pipefail
exec >/root/prov/gb206phase_finish_binding.log 2>&1
trap 'rc=$?; echo "FINISH_BINDING_EXIT rc=$rc $(date -u -Is)"' EXIT
echo "FINISH_BINDING_START $(date -u -Is)"
grep -q '^GUEST_PROGRAM_EXIT gb206phase20261016_set_gsp.ps1 code=0' /root/prov/gb206phase_bind_flag.log
test ! -e /root/win/codex-gb206phase/bound580.qcow2
bash /root/gb206phase_stop_windows.sh /root/win/codex-gb206phase f8485f6d /root/kayfabe
mv /root/win/codex-gb206phase/windows.qcow2 /root/win/codex-gb206phase/bound580.qcow2
cp /root/win/codex-gb206phase/OVMF_VARS.fd /root/win/codex-gb206phase/bound580-VARS.fd
echo "STOPPED_BOUND_PARENT_READY $(date -u -Is)"
bash /root/gb206phase_windows.sh ready
