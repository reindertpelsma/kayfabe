#!/usr/bin/env bash
# ★★★★★ w393 — THE BAR-MIRROR LADDER: ONE binary, THREE arms, the SAME raw client.
#
#   arm=off    bar1-passthrough=off bar2-passthrough=off   (the control: master's behaviour)
#   arm=bar1   bar1-passthrough=on  bar2-passthrough=off
#   arm=both   bar1-passthrough=on  bar2-passthrough=on
#
# Each arm is a fresh boot (`boot_capture.sh`, the WPR2 rule) with the w392d mean client as
# the POST_CAPTURE_HOOK, under the LEGACY arming set that carries the known-positive
# (`RESUME_HERE_w392_overnight.md` "HOW TO RUN IT") — so workload and arming never move
# together and only the BAR arms differ between rows.
#
# THE LEDGER, per arm, read off the artefacts and never off an exit code:
#   - the C's own rows:   BAR1-PASSTHROUGH arm= misses=,  BAR1 COUNTERS (one instant),
#                         BAR2-PASSTHROUGH arm= misses=
#   - the archive's rows: BAR-MIRROR bar1/bar2 AT END OF RUN (fills, distinct_pages,
#                         distinct_frames), BAR-MIRROR MECHANISM (slots, revalidate, quiesce,
#                         refused=[…]), and the attach banners (ARMED / OFF / NOT BUILT)
#   - the archive audit:  "translated read(s) / write(s)" = bar1_reads / bar1_writes
#   - the client:         W392D_OUTCOME, THREADS, MEAN_FALSIFIER; host Xid count
#
# ⚠ `misses=0` under an armed arm with `bar1 touches=0` is NOTHING TOUCHED, not passthrough.
# ⚠ A boot whose archive prints `BAR-MIRROR … NOT BUILT` is the census-only build; its
#   misses equal its touches by construction and it grades NOTHING about the mirror.
set -uo pipefail
SRC_DIR="$(cd "$(dirname "$0")" && pwd)"
BENCH=${BENCH_DIR:-/workspace/bench}
ARMS=${ARMS:-off bar1 both}
PREFIX=${PREFIX:-w393m}
say(){ printf '[w393_bar_boots] %s\n' "$*"; }

# The legacy arming set — pinned here so the ladder cannot drift from the known-positive.
export KAYFABE_ISOLATES=real KAYFABE_GUEST_RAM=memfd NVKVM_RAM_BACKEND=memfd \
       KAYFABE_FB_JOIN=shared KAYFABE_GUEST_RING=ring KAYFABE_GR_ROUTE=passthrough \
       KAYFABE_OPERAND_JOIN=join KAYFABE_PT_SWEEP=on KAYFABE_VAS_PUBLISH=drain \
       KAYFABE_PT_WITNESS_EXEC=on BOOT_TIMEOUT=${BOOT_TIMEOUT:-180}
export POST_CAPTURE_HOOK="$SRC_DIR/w392d_mean_hook.sh"
export UVM_BIN=${UVM_BIN:-}

echo "=== w393 BAR-MIRROR LADDER $(date -Is) arms=[$ARMS] prefix=$PREFIX ==="
echo "qemu: $(strings "$BENCH/qemu-build/qemu-system-x86_64" 2>/dev/null | grep -o 'kayfabe-rev:[0-9a-f]*' | sort -u | tr '\n' ' ')"

for arm in $ARMS; do
  case "$arm" in
    off)  extra="" ;;
    bar1) extra="bar1-passthrough=on" ;;
    both) extra="bar1-passthrough=on,bar2-passthrough=on" ;;
    bar2) extra="bar2-passthrough=on" ;;
    *) say "unknown arm $arm"; exit 2 ;;
  esac
  tag="${PREFIX}_${arm}"
  say "=== ARM=$arm  NVKVM_DEV_EXTRA='$extra'  tag=$tag  $(date -Is) ==="
  if pgrep -x qemu-system-x86 >/dev/null 2>&1; then say "⊘ a QEMU is running; refusing"; exit 3; fi
  NVKVM_DEV_EXTRA="$extra" bash "$SRC_DIR/boot_capture.sh" "$tag" > "$BENCH/run_${tag}_driver.log" 2>&1
  rc=$?
  say "boot_capture rc=$rc (⊘ rc=5 is the evidence-persist check; read the artefacts)"
  Q="$BENCH/run_${tag}_qemu.log"
  # ⊘ The hook's output is appended to the PROBE log by boot_capture.sh, not to its stdout.
  D="$BENCH/run_${tag}_probe.log"
  echo "--- LEDGER arm=$arm tag=$tag ---"
  echo "[C]      $(grep -a 'BAR1-PASSTHROUGH arm=' "$Q" | grep -a misses= | tail -1 | sed 's/.*nvkvm: //' | cut -c1-90)"
  echo "[C]      $(grep -a 'BAR1 COUNTERS' "$Q" | tail -1 | sed 's/.*nvkvm: //')"
  echo "[C]      $(grep -a 'BAR2-PASSTHROUGH arm=' "$Q" | grep -a misses= | tail -1 | sed 's/.*nvkvm: //' | cut -c1-90)"
  echo "[C]      $(grep -a 'BAR1 access log:' "$Q" | tail -1 | sed 's/.*nvkvm: *//' | cut -c1-100)"
  echo "[C]      reports printed: $(grep -ac 'BAR1 COUNTERS' "$Q")"
  grep -a 'BAR-MIRROR bar[12]: \(ARMED\|OFF\|⊘\)' "$Q" | sed 's/.*kayfabe: /[archive] /' | cut -c1-110
  grep -a 'BAR-MIRROR.*NOT BUILT\|BAR-MIRROR.*NOT ARMED' "$Q" | head -2 | sed 's/.*kayfabe: /[archive] /' | cut -c1-110
  grep -a 'BAR-MIRROR bar[12] AT END OF RUN' "$Q" | sed 's/.*kayfabe: /[archive] /' | cut -c1-140
  grep -a 'BAR-MIRROR MECHANISM AT END OF RUN' "$Q" | sed 's/.*kayfabe: /[archive] /'
  echo "[archive] revalidate lines printed: $(grep -ac 'BAR-MIRROR REVALIDATE' "$Q")  fill refusals printed: $(grep -ac 'FILL REFUSED' "$Q")"
  echo "[audit]  $(grep -a 'translated read(s)' "$Q" | tail -1 | sed 's/.*nvkvm: //' | cut -c1-160)"
  echo "[audit]  slots: $(grep -a 'kernel slots live=' "$Q" | tail -1 | sed 's/.*nvkvm: //' | cut -c1-120)"
  echo "[xid]    host Xid lines this boot: $(grep -ac 'Xid' "$BENCH/run_${tag}_hostdmesg.log" 2>/dev/null)"
  echo "[client] $(grep -a 'W392D_OUTCOME=' "$D" | tail -1 | sed 's/^ *//' | cut -c1-80)"
  echo "[client] $(grep -a 'THREADS ' "$D" | tail -1 | sed 's/^ *//' | cut -c1-80)"
  echo "[client] $(grep -a 'MEAN_FALSIFIER' "$D" | tail -1 | sed 's/^ *//' | cut -c1-80)"
  echo "[client] $(grep -a 'W392D_GUEST_OUTCOME' "$D" | tail -1 | sed 's/^ *//' | cut -c1-100)"
  echo "[boot]   arming banners: $(grep -ao 'GUEST-RING arm=[a-z]*\|FB-JOIN arm=[a-z]*\|GR-ROUTE arm=[a-z]*\|VAS-PUBLISH arm=[a-z]*' "$Q" | sort -u | tr '\n' ' ')"
  echo "--- END LEDGER arm=$arm ---"
done
echo "=== w393 BAR-MIRROR LADDER DONE $(date -Is) ==="
