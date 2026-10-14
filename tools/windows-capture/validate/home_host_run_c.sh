#!/usr/bin/env bash
# Trusted host. kfwc: one boot, observer on, scan-only capture across a restart + back-off reset test (phase C).
set -uo pipefail
W=/var/lib/kf-windows-20261005; REV=${REV:-ec45aad9}; N=${N:-3401}
B=$W/kayfabe-win-6fafcc6e/scripts/bench/windows/windows_broker_claude_inst2.sh
RUN=$W/boundary-kayfabe-$N; D=$W/claude-agent-kfwc; OUT=$D/out-$N; PY=/data/claude-agent-kfwc.py
rm -rf "$OUT"; mkdir -p -m 0700 "$OUT"
echo "KFWC_C_START $(date -Is) rev=$REV run=$N"
if pgrep -f 'qemu-system-x86_64' >/dev/null 2>&1; then echo "a QEMU is running; abort"; exit 12; fi
export KF_LOCKED=1 WIN_GSP_OBSERVER=0 WIN_FLAGS= PROD_EXTRA_FLAGS= WIN_MAX_SECONDS=0 WIN_FB_MB=4096 WIN_DROP="KF3_TIMER_MAP" WIN_NET_RESTRICT=on WIN_RAM_MB=8192 WIN_TRACE=0
while IFS= read -r name; do unset "$name"; done < <(compgen -e | sed -n '/^KF3_/p')
export KF3_REV=$REV
export WIN_CPU="host,-vmx,hv_relaxed,hv_vapic,hv_spinlocks=0x1fff,hv_vpindex,hv_runtime,hv_time,hv_synic,hv_stimer,hv_tlbflush,hv_ipi,hv_frequencies"
export WIN_QEMU_EXTRA="-global kf3-gpu.x-gsp-observer=$RUN/gsp.jsonl -global kf3-gpu.x-gsp-observer-seconds=3600"
case "$(cat /sys/kernel/iommu_groups/11/type)" in identity) ;; *) bash $W/pti-iommu_nogdm.sh identity || { echo IOMMU-SWITCH-FAILED; exit 5; };; esac
trap 'bash $W/pti-iommu_nogdm.sh DMA-FQ >/dev/null 2>&1 || true; echo "KFWC_C_EXIT $(date -Is)"' EXIT
rm -rf "$RUN"
WIN_REUSE=0 bash "$B" run "$N" > "$OUT/launch.log" 2>&1
python3 "$PY" C "$RUN" "$OUT"; echo "PHASE_C_RC=$?"
cp "$RUN/qemu.log" "$OUT/qemu-boot.log" 2>/dev/null
QPID=$(sed -n 's/^QPID=//p' "$W/windows-broker-run$N.state")
WIN_REUSE=0 bash "$B" stop "$N" > "$OUT/stop.log" 2>&1
if [ -n "$QPID" ] && kill -0 "$QPID" 2>/dev/null; then kill "$QPID" 2>/dev/null; sleep 4; kill -9 "$QPID" 2>/dev/null; echo "killed own qemu $QPID"; fi
ls -la "$RUN/gsp.jsonl" 2>&1
rm -f "$RUN/windows.qcow2"
echo "KFWC_C_DONE $(date -Is)"
