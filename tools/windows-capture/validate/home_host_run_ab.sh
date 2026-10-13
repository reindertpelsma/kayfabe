#!/usr/bin/env bash
# Trusted host. kfwc validation under kayfabe: LTSC baseline overlay, two boots (A: testsigning + files; B: observer on, capture).
# Caller: flock /tmp/kf-bisect.lock bash this.sh
set -uo pipefail
W=/var/lib/kf-windows-20261005; REV=${REV:-bf510bdf}; N=${N:-3400}
B=$W/kayfabe-win-6fafcc6e/scripts/bench/windows/windows_broker_claude_inst2.sh
RUN=$W/boundary-kayfabe-$N; D=$W/claude-agent-kfwc; OUT=$D/out-$N; PY=/data/claude-agent-kfwc.py
mkdir -p -m 0700 "$OUT"
echo "KFWC_VALIDATE_START $(date -Is) rev=$REV run=$N"
if pgrep -x qemu-system-x86 >/dev/null 2>&1 || pgrep -f 'qemu-system-x86_64' >/dev/null 2>&1; then echo "a QEMU is running; abort"; exit 12; fi
export KF_LOCKED=1 WIN_GSP_OBSERVER=0 WIN_FLAGS= PROD_EXTRA_FLAGS= WIN_MAX_SECONDS=0 WIN_FB_MB=4096 WIN_DROP="KF3_TIMER_MAP" WIN_NET_RESTRICT=on WIN_RAM_MB=8192 WIN_TRACE=0
while IFS= read -r name; do unset "$name"; done < <(compgen -e | sed -n '/^KF3_/p')
export KF3_REV=$REV
export WIN_CPU="host,-vmx,hv_relaxed,hv_vapic,hv_spinlocks=0x1fff,hv_vpindex,hv_runtime,hv_time,hv_synic,hv_stimer,hv_tlbflush,hv_ipi,hv_frequencies"
case "$(cat /sys/kernel/iommu_groups/11/type)" in identity) ;; *) bash $W/pti-iommu_nogdm.sh identity || { echo IOMMU-SWITCH-FAILED; exit 5; };; esac
trap 'bash $W/pti-iommu_nogdm.sh DMA-FQ >/dev/null 2>&1 || true; echo "KFWC_VALIDATE_EXIT $(date -Is)"' EXIT
stopvm() {
    QPID=$(sed -n 's/^QPID=//p' "$W/windows-broker-run$N.state")
    WIN_REUSE=0 bash "$B" stop "$N" > "$OUT/stop.$1.log" 2>&1
    if [ -n "$QPID" ] && kill -0 "$QPID" 2>/dev/null; then kill "$QPID" 2>/dev/null; sleep 4; kill -9 "$QPID" 2>/dev/null; echo "killed own qemu $QPID" ; fi
}
rm -rf "$RUN"; mkdir -p -m 0700 "$RUN"
qemu-img create -q -f qcow2 -F qcow2 -b $W/baseline/windows.qcow2 "$RUN/windows.qcow2"; cp $W/baseline/OVMF_VARS.fd "$RUN/OVMF_VARS.fd"
cp "$RUN/OVMF_VARS.fd" "$OUT/OVMF_VARS.before.fd"
# ---- boot A
WIN_REUSE=0 bash "$B" run "$N" > "$RUN/launch.log" 2>&1
python3 "$PY" A "$RUN" "$OUT"; echo "PHASE_A_RC=$?"
stopvm A
sleep 5
# ---- boot B (the same disk in a new QEMU, observer on)
export WIN_QEMU_EXTRA="-global kf3-gpu.x-gsp-observer=$RUN/gsp.jsonl -global kf3-gpu.x-gsp-observer-seconds=3600"
WIN_REUSE=1 bash "$B" run "$N" > "$RUN/launch-b.log" 2>&1
python3 "$PY" B "$RUN" "$OUT"; echo "PHASE_B_RC=$?"
cp "$RUN/OVMF_VARS.fd" "$OUT/OVMF_VARS.after.fd" 2>/dev/null
cp "$RUN/qemu.log" "$OUT/qemu-boot-b.log" 2>/dev/null
stopvm B
ls -la "$RUN/gsp.jsonl" "$OUT" 2>&1 | head -30
rm -f "$RUN/windows.qcow2"
echo "KFWC_VALIDATE_DONE $(date -Is)"
