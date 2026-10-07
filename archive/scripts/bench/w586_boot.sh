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
# ⊘⊘⊘ **OVERRIDABLE — it was an UNCONDITIONAL export and that silently discarded the caller's
# hook (w669).** `PREFIX=w669a POST_CAPTURE_HOOK=.../cup3_hook.sh bash w586_boot.sh` ran the MEAN
# hook: this line overwrote the caller's value, the boot graded green, and the ladder rung the
# caller asked for was never executed. ⚠ It is only detectable by noticing the FIELD NAMES in the
# output belong to a different hook — nothing refuses, and a reader looking for a result sees a
# complete, healthy ledger for the wrong workload.
#
# ★ The default stays the mean hook, because that is what this script is for. `:-` makes it a
# default rather than a decree.
export POST_CAPTURE_HOOK="${POST_CAPTURE_HOOK:-$SRC_DIR/w392d_mean_hook.sh}"
# ⊘ Say which one ran, in the boot's own log. A hook is the whole payload of a boot and it was
# nowhere in the output.
echo "POST_CAPTURE_HOOK=$POST_CAPTURE_HOOK"

# ★★★★★ **THE BINARY MUST BE THE TREE YOU THINK YOU ARE TESTING (w672).**
#
# ⊘⊘⊘ Four times in one session a run was graded for a change that was not in it:
#   1. `KAYFABE_SYNC3` absent from the binary — the rebuild had REFUSED (`cargo` not on PATH)
#      and the boot happily used the previous one.
#   2. `POST_CAPTURE_HOOK` overwritten by this script, so the mean hook ran instead of cup3.
#   3. `PUBQUEUE by_kind` missing — this script does not rebuild, and nothing said so.
#   4. The content-check for (3) matched `by_kind={}` in an UNRELATED census line and reported
#      the change present. ⚠ A substring is not a witness.
#
# ★ All four have ONE detectable signature: **the binary's revision is not the tree's HEAD.**
# That comparison is cheap, exact, and cannot be fooled by a substring — so it is made here, and
# it REFUSES rather than warns. A boot that grades the wrong binary is worse than no boot: it
# produces a number that looks like evidence.
#
# ⊘ `KAYFABE_ALLOW_STALE_BINARY=1` is the deliberate escape hatch — re-running an OLD revision
# on purpose is a legitimate thing to do (it is how a control arm gets measured), but it must be
# stated rather than defaulted into.
# THE EXTRACTION IS `grep -ao`, NOT AN ANCHORED `sed` - and the first version of this
# guard was VACUOUS because of it.
#
# The stamp is embedded MID-STRING in the binary: `strings` yields
# `Executorkayfabe-rev:<40 hex>mid > len`, never a line that IS the stamp. An anchored
# `^kayfabe-rev:...$` matched nothing, BIN_REV came out EMPTY, and the `-n` test below then
# SKIPPED the comparison - so the guard written to catch a stale binary could not fire on any
# boot at all.
#
# Caught only because the rebuild printed an empty `binary rev` beside a real `tree HEAD`.
# A guard that CANNOT fire and a guard that PASSES look identical - which is why every check
# in this file is a refusal that carries its own evidence.
# `grep -ao` is how boot_capture.sh and assert_boot_evidence.sh already read this stamp: one
# extraction, not three.
BIN_REV=$(strings "$BENCH/qemu-build/qemu-system-x86_64" 2>/dev/null \
          | grep -ao 'kayfabe-rev:[0-9a-f]\{40\}' | head -1 | cut -d: -f2)
TREE_REV=$(git -C "${KAYFABE_REPO:-/root/kayfabe}" rev-parse HEAD 2>/dev/null)
if [ -n "$BIN_REV" ] && [ -n "$TREE_REV" ] && [ "$BIN_REV" != "$TREE_REV" ]; then
  echo "⊘⊘⊘ STALE BINARY — refusing to grade a run that does not contain the tree."
  echo "    binary : $BIN_REV"
  echo "    tree   : $TREE_REV"
  echo "    Rebuild:  export PATH=\$HOME/.cargo/bin:\$PATH"
  echo "              KAYFABE_SHIM_FEATURES=host-isolates bash scripts/build_qom_shim.sh \\"
  echo "                  $BENCH/qemu-10.2.4 $BENCH/qemu-build"
  echo "    ⚠ The rebuild can REFUSE (cargo off PATH) and leave the old binary in place —"
  echo "      check the binary's mtime, not the rebuild's exit status."
  echo "    To measure an older revision ON PURPOSE: KAYFABE_ALLOW_STALE_BINARY=1"
  [ "${KAYFABE_ALLOW_STALE_BINARY:-0}" = "1" ] || exit 3
  echo "    ⊘ KAYFABE_ALLOW_STALE_BINARY=1 — proceeding with the stale binary, as asked."
fi
echo "BINARY_REV=${BIN_REV:-UNKNOWN} TREE_REV=${TREE_REV:-UNKNOWN}"

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
# ★★★★★ w712 - REFUSED DOORBELLS, BESIDE THE GRADE THAT IGNORES THEM.
#
# ⊘⊘ The owner, 2026-09-14, on an LLM boot reported as a PASS:
#   "`doorbells: 22671 arrived, 22517 served, 8 REFUSED` ... I think refused should be 0 in our
#    test runs right"
#
# Yes. A refused doorbell is a submission that NEVER REACHED THE GPU. And this script - the one
# that decides whether a boot graded - printed the arrived/served/refused line NOWHERE, so eight
# dropped submissions rode along inside a green and the by-name breakdown was never read.
#
# ⚠ A grade that cannot see a dropped submission is not a grade of the data plane. Printed
# UNCONDITIONALLY, including when refused=0, because "nothing was refused" and "nobody looked"
# are the same silence - the rule this tree keeps relearning.
echo "--- ★ REFUSED DOORBELLS — a refusal is a submission that never reached the GPU ---"
grep -ao "doorbells: [0-9]* arrived, [0-9]* served, [0-9]* REFUSED[^;]*" "${BENCH}/run_${tag}_qemu.log" 2>/dev/null | tail -1
grep -ao "DOORBELL-REFUSALS[^|]\{0,180\}" "${BENCH}/run_${tag}_qemu.log" 2>/dev/null | tail -1
grep -ao "GPFIFO-STRANDED-CENSUS[^⊘✔]\{0,90\}" "${BENCH}/run_${tag}_qemu.log" 2>/dev/null | tail -1
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
