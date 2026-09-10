#!/usr/bin/env bash
# ★★★★★ w401 — DOES THE WRITER-SIDE PUBLICATION TRIGGER BRING `P3 rpc-bind` BACK?
#
# `P3 rpc-bind` went red at commit af41c717, which deleted leg 8 — the doorbell publication
# trigger. Leg 8 re-published every VAS on EVERY doorbell, so it was a POLL, and the rung was
# riding on it rather than on a trigger. Underneath it the only real trigger latched inside the
# `GPU_PROMOTE_CTX` handler, which fires FOUR TIMES in a boot.
#
# The replacement asks `AddressTable::generation` — the writer-side fact — per VAS, so any bind
# by any path arms a publication. This boot asks whether that is enough.
#
# ⚠ ONE ARM, current defaults: no BAR passthrough, sweep ON. `KAYFABE_VAS_PUBLISH` is NOT
#   exported: leg 8 is gone, and the worker sets `VasPublishArm::Publish` on its own jobs. A run
#   that re-exported it would be measuring a knob whose path no longer exists — the exact shape
#   the leg-8 commit message calls out.
#
# GRADE, off the artefacts and never off an exit code:
#   PASS  = `P3 rpc-bind` is not red AND the client's W392D_OUTCOME=(P) AND THREADS 4 of 4
#   The publication census must ALSO show the trigger firing more than 4 times, or a green P3
#   is somebody else's doing and this change is unproven.
set -uo pipefail
SRC_DIR="$(cd "$(dirname "$0")" && pwd)"
BENCH=${BENCH_DIR:-/workspace/bench}
tag=${PREFIX:-w401a}

export KAYFABE_ISOLATES=real KAYFABE_GUEST_RAM=memfd NVKVM_RAM_BACKEND=memfd \
       KAYFABE_FB_JOIN=shared KAYFABE_GUEST_RING=ring KAYFABE_GR_ROUTE=passthrough \
       KAYFABE_OPERAND_JOIN=join KAYFABE_PT_SWEEP=on \
       KAYFABE_PT_WITNESS_EXEC=on KAYFABE_CE_EXECUTOR=host \
       NVKVM_RAM_MB=${NVKVM_RAM_MB:-16384} BOOT_TIMEOUT=${BOOT_TIMEOUT:-180}
export POST_CAPTURE_HOOK="$SRC_DIR/w392d_mean_hook.sh"

echo "=== w401 TRIGGER BOOT $(date -Is) tag=$tag ==="
# ⚠ VERIFY THE BINARY BY CONTENT. The box's HEAD has lied before; a stamp is a claim and the
# string table is the thing that will actually run.
echo "qemu rev: $(strings "$BENCH/qemu-build/qemu-system-x86_64" 2>/dev/null | grep -o 'kayfabe-rev:[0-9a-f]*' | sort -u | tr '\n' ' ')"
echo "trigger present in binary: $(strings "$BENCH/qemu-build/qemu-system-x86_64" 2>/dev/null | grep -c 'RPCBIND-PUBLISH')"

if pgrep -x qemu-system-x86 >/dev/null 2>&1; then echo "⊘ a QEMU is running; refusing"; exit 3; fi
bash "$SRC_DIR/boot_capture.sh" "$tag" > "$BENCH/run_${tag}_driver.log" 2>&1
echo "boot_capture rc=$? (⊘ rc=5 is the evidence-persist check; read the artefacts)"

Q="$BENCH/run_${tag}_qemu.log"; D="$BENCH/run_${tag}_probe.log"
echo "--- LEDGER tag=$tag ---"
echo "[client] $(grep -a 'W392D_OUTCOME=' "$D" | tail -1 | sed 's/^ *//' | cut -c1-90)"
echo "[client] $(grep -a 'THREADS ' "$D" | tail -1 | sed 's/^ *//' | cut -c1-80)"
echo "[client] $(grep -a 'MEAN_FALSIFIER' "$D" | tail -1 | sed 's/^ *//' | cut -c1-80)"
echo "--- THE LADDER (P1..Pn), which is what this change is graded on ---"
grep -aE '^\s+P[0-9]+ ' "$D" | sed 's/^ */[ladder] /' | cut -c1-120
echo "--- THE TRIGGER ITSELF ---"
echo "[trig]   RPCBIND-PUBLISH lines: $(grep -ac 'RPCBIND-PUBLISH' "$Q")"
grep -a 'RPCBIND-PUBLISH' "$Q" | head -8 | sed 's/.*kayfabe: /[trig] /' | cut -c1-150
echo "[trig]   MMUINVAL-PUBLISH lines: $(grep -ac 'MMUINVAL-PUBLISH' "$Q")"
echo "[trig]   PROMOTE-BOUND lines:    $(grep -ac 'PROMOTE-BOUND' "$Q")"
echo "[xid]    host Xid lines: $(grep -ac 'Xid' "$BENCH/run_${tag}_hostdmesg.log" 2>/dev/null)"
echo "[boot]   arming banners: $(grep -ao 'GUEST-RING arm=[a-z]*\|FB-JOIN arm=[a-z]*\|GR-ROUTE arm=[a-z]*\|PT-SWEEP arm=[a-z]*' "$Q" | sort -u | tr '\n' ' ')"
echo "--- END LEDGER tag=$tag ---"
