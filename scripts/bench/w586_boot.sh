#!/usr/bin/env bash
# ★★★★★ w586 — THE FIRST GRADE SINCE w583, AND IT ASKS THREE QUESTIONS AT ONCE.
#
# Four commits landed unmeasured. Each was a real defect, and all four are the SAME defect
# wearing different clothes — the store and the file were two different memories:
#
#   w584  `fresh_page` handed the arena a FRAME NUMBER where it wanted a BYTE ADDRESS, so
#         every store page since w569 was refused and fell back to the heap.
#   w585  the store then answered a frame it had no page for with ZEROS, and zero-filled each
#         arena page at creation — ignoring what the file held, then erasing it.
#   w585  `device_reset` cleared a page map that no longer holds the bytes (cross-life leak).
#   w586  the store's own census had no reader, which is why none of the above was visible.
#
# ## ★★ PRE-REGISTERED OUTCOMES — written before the boot, so none reads as the good one
#
#   Q1 THE CLIENT. `W392D_GUEST_OUTCOME=(P)` with `THREADS 8 of 8`. ⊘ Anything else and Q2/Q3 are
#      facts about a failed boot, not about the surface. Graded FIRST for that reason.
#
#   Q2 PRAMIN, now UN-PARKED (w586). `window[SERVED r=0 w=0]` in `BAR0-READS`.
#      ⚠ w582 already reached r=0/w=0 and the guest still failed — so **r=0/w=0 alone is not
#      the pass**. The pass is r=0/w=0 *together with* (P). That pairing is the whole point:
#      zero traps proves the slot INTERCEPTS, never that it shows the same bytes.
#
#   Q3 BAR1/BAR2, fable's prediction, and this one is free. `HEAP-PAGE` in the BAR-MIRROR
#      refusal list should be LARGE in any boot since w569 (every store page was on the heap,
#      so no memslot could be installed and both BARs kept trapping) and should now be **0**.
#      ⇒ If it is 0 here, w584 was a BAR1/BAR2 fix as well as a PRAMIN one, and nobody knew.
#
#   (E) no `W392D_GUEST_OUTCOME` line at all => UNMEASURED. Say where it stopped. Not a failure value.
set -uo pipefail
SRC_DIR="$(cd "$(dirname "$0")" && pwd)"
BENCH=${BENCH_DIR:-/workspace/bench}
tag=${PREFIX:-w586a}

export KAYFABE_ISOLATES=real KAYFABE_GUEST_RAM=memfd NVKVM_RAM_BACKEND=memfd \
       KAYFABE_FB_JOIN=shared KAYFABE_GUEST_RING=ring KAYFABE_GR_ROUTE=passthrough \
       KAYFABE_CE_EXECUTOR=host \
       NVKVM_RAM_MB=${NVKVM_RAM_MB:-16384} BOOT_TIMEOUT=${BOOT_TIMEOUT:-180}
export POST_CAPTURE_HOOK="$SRC_DIR/w392d_mean_hook.sh"

echo "=== w586 BOOT $(date -Is) tag=$tag ==="

# ⚠ VERIFY THE BINARY BY CONTENT, never by a stamp. `[measured]` this bench served a binary
# built from a four-week-old revision for weeks while every stamp said HEAD. A string this
# build introduces is the only check that cannot be inherited from an older one.
Q_BIN="$BENCH/qemu-build/qemu-system-x86_64"
echo "qemu rev: $(strings "$Q_BIN" 2>/dev/null | grep -o 'kayfabe-rev:[0-9a-f]*' | sort -u | tr '\n' ' ')"
# ⊘ `grep -c` on its own line, never piped into `grep -q`: a pipe that closes early returns
# 141 under `pipefail` and MANUFACTURES a failure when the string is present (measured w418).
n_hot=$(strings "$Q_BIN" 2>/dev/null | grep -c 'BAR0-READ-HOTSPOTS')
n_cen=$(strings "$Q_BIN" 2>/dev/null | grep -c 'store_read_refused')
echo "W586-CONTENT: hotspots=$n_hot store_census=$n_cen (either 0 ⇒ this is an OLDER binary — STOP, do not grade)"
if [ "$n_hot" -eq 0 ] || [ "$n_cen" -eq 0 ]; then
  echo "⊘ REFUSING TO GRADE: the binary predates w586. Rebuild, then re-run."
  exit 4
fi

# ⚠ `pgrep -x qemu-system-x86_64` can NEVER match: /proc/PID/comm truncates to 15 chars.
if pgrep -x qemu-system-x86 >/dev/null 2>&1; then echo "⊘ a QEMU is running; refusing"; exit 3; fi

bash "$SRC_DIR/boot_capture.sh" "$tag" > "$BENCH/run_${tag}_driver.log" 2>&1
echo "boot_capture rc=$?"

Q="$BENCH/run_${tag}_qemu.log"; D="$BENCH/run_${tag}_probe.log"
echo "--- Q1 THE CLIENT (graded first; everything below is uninterpretable without it) ---"
echo "[client] $(grep -a 'W392D_GUEST_OUTCOME=' "$D" 2>/dev/null | tail -1 | sed 's/^ *//' | cut -c1-90)"
echo "[client] $(grep -a 'THREADS ' "$D" 2>/dev/null | tail -1 | sed 's/^ *//' | cut -c1-80)"
echo "[client] $(grep -a 'MEAN_FALSIFIER' "$D" 2>/dev/null | tail -1 | sed 's/^ *//' | cut -c1-80)"
echo "--- Q2 THE BAR0 READ SURFACE ---"
# ⊘ The QEMU log puts every census on ONE 120 KB line, so `grep | fold` shows the line's
# START and not the field asked for. `grep -o` extracts the field itself.
grep -ao 'BAR0-READS total=[^|]*|[^|]*|[^|]*|[^|]*|[^|]*|[^|]*' "$Q" 2>/dev/null | tail -1 | tr '|' '\n' 
echo "--- ★ WHERE THE REMAINING READS ARE (w586's new instrument) ---"
grep -ao 'BAR0-READ-HOTSPOTS[^⊘]*' "$Q" 2>/dev/null | tail -1
echo "--- ★ PRAMIN's slot: did it FOLLOW the guest's 42 window moves? (w587) ---"
grep -ao 'PRAMIN-SLOT AT [^⊘]*' "$Q" 2>/dev/null | tail -1
grep -ao 'PRAMIN-WINDOW [^⊘]*' "$Q" 2>/dev/null | tail -2
echo "--- Q3 BAR1/BAR2 + THE STORE CENSUS (fable's prediction: HEAP-PAGE was large, must be 0) ---"
grep -a 'BAR-MIRROR' "$Q" 2>/dev/null | tail -3 | fold -w 150 | head -12
echo "--- trap latency, goal 6's standing number ---"
grep -ao 'TRAPWITNESS[^|]*' "$Q" 2>/dev/null | tail -1
grep -ao 'SLOW-SITES[^⊘]*' "$Q" 2>/dev/null | tail -1
# ★ w592 — the discriminator the owner's rule needs: descheduled, or our own work?
grep -ao 'TRAP-CPU[^⇒]*⇒[^—]*' "$Q" 2>/dev/null | tail -1
echo "--- ★ the GSP submit path, which is what a hang shows up in ---"
grep -a 'kayfabe: GSP-SUBMIT' "$Q" 2>/dev/null | tail -1 | cut -c1-400
grep -ao 'QUEUE coalesce[^|]*' "$Q" 2>/dev/null | tail -1
echo "--- ★ the guest OWN first failure (NOT the last line: a re-boot attempt masks it) ---"
grep -a 'NVRM' "$BENCH/run_${tag}_dmesg.log" 2>/dev/null | head -12 | cut -c1-170
echo "=== w586 END $(date -Is) ==="
