#!/usr/bin/env bash
# ★★★★★ w379 — THE MAPPING PLANE AS A RAW-CLIENT BATTERY, run on BARE METAL.
#
# Owner 2026-09-06: *"raw clients can remain useful, to test the allocate propagations and
# races about missing pages resulting in fault, mixing rpc with normal allocs, mean stress
# test, to have a test artifact on purely open source components easier to debug."*
#
# Four rungs, each with a positive control that must pass or its result is UNINTERPRETABLE:
#
#   RUNG_map_propagation  R2   a mapping at a DICTATED VA — does the HOST DRIVER resolve it?
#   RUNG_alias_two_vas    R1'  ONE object at TWO VAs; release through the FIRST after the
#                              SECOND is mapped. The FB-join aliasing hazard, executable.
#   RUNG_alias_unmap      R1"  unmap one alias, assert the other survives. The DISCRIMINATOR.
#   RUNG_missing_page     R3   an unmapped VA under work: CONTAINED, and NAMED.
#   RUNG_map_stress       R5   interleaved alloc/map/free over a rolling window of four live
#                              mappings. ⊘ SINGLE-CLIENT — cross-client leakage is NOT covered.
#
# ## ★★★ PRE-REGISTERED, BEFORE THE RUN — every outcome, so none reads as the favourable one
#   (A) every selected rung PASS            => the mapping plane behaves as the driver does.
#   (B) any rung FAIL with its control PASS => a real red, attributable to that rung.
#   (C) any rung NOTRUN with control FAIL   => ⊘ UNINTERPRETABLE. The rung measured its own
#                                              harness. NOT a failure value; say where it
#                                              stopped.
#   (D) no RUNG_ line at all                => ⊘ UNMEASURED. Also not a failure value.
#   (E) no binary                           => ⊘ UNMEASURED_NO_BINARY — attributable to the
#                                              BUILD. ★ This arm exists because w377 spent a
#                                              run looking for a binary named `rmladder`
#                                              while the target is `kayfabe-rm-ladder`, and
#                                              this arm is the only reason that read as a
#                                              harness fault rather than a system failure.
#
# ⊘ **GRADED ON PRINTED LINES, NEVER ON EXIT STATUS.** A timeout kills the process and its
#   status says nothing about how far it got.
# ⊘ **`--missing-page-fault` PROVOKES A REAL `Xid 31` and kills its victim channel.** That is
#   the measurement. It is last in the battery and its bystander arm is what proves the fault
#   did not take anything else with it.
# ⚠ **SCOPE — THESE ARE HOST-HARDWARE RUNGS.** The Mode-2 CPU copy-engine emulator decodes
#   `PushMethod::SemRelease` and deliberately does not act on it
#   (`kayfabe-rt/src/ceutils.rs:677-679`), so the same binary run inside a guest measures the
#   emulator's declared scope, not a defect. Do not run this as a differential and read the
#   guest half as a red.
set -uo pipefail
REPO=${KAYFABE_REPO:-/root/kayfabe}
export CARGO_TARGET_DIR=${CARGO_TARGET_DIR:-/workspace/bench/cargo-target-w297}
TAG=${KAYFABE_TAG:-w379map}
LOG=/workspace/bench/run_${TAG}_native.log
TMO=${W379_TIMEOUT:-300}
RUNGS=${W379_RUNGS:---w379}
STAMP=$(git -C "$REPO" rev-parse --short HEAD 2>/dev/null || echo unknown)

echo "=== ★★★★★ w379 MAPPING-PLANE BATTERY — NATIVE, source $STAMP  $(date -Is) ==="

# ⚠ LOCATED, never assumed. The bin target is `kayfabe-rm-ladder`; a binary named
# `rmladder` does not exist and looking for one is how a build failure reads as a GPU one.
BIN=""
for c in "$CARGO_TARGET_DIR"/release/kayfabe-rm-ladder \
         "$CARGO_TARGET_DIR"/x86_64-unknown-linux-musl/release/kayfabe-rm-ladder \
         "$REPO"/target/release/kayfabe-rm-ladder; do
  [ -x "$c" ] && BIN="$c" && break
done
if [ -z "$BIN" ]; then
  echo "W379_BIN_FOUND=no"
  echo "W379_OUTCOME=(E) ⊘ UNMEASURED_NO_BINARY — kayfabe-rm-ladder was never built"
  exit 0
fi
echo "W379_BIN_FOUND=yes ($BIN, $(stat -c %s "$BIN" 2>/dev/null) bytes)"

sudo dmesg -C >/dev/null 2>&1 || true
: > "$LOG"
echo "W379_START=$(date -Is) rungs=$RUNGS" | tee -a "$LOG"
# ⊘ NOT piped into anything: a pipe makes `$?` the pager's, and this repo has already had a
#   harness print success over a workspace that never compiled.
timeout "$TMO" "$BIN" $RUNGS >>"$LOG" 2>&1
RC=$?
echo "W379_TERMINATOR=reached inner_rc=$RC" | tee -a "$LOG"
echo "--- host dmesg after the battery ---" | tee -a "$LOG"
sudo dmesg 2>&1 | grep -iE "xid|nvrm" | tail -20 | tee -a "$LOG"

grep -aE '^(info|ok|★|⊘|⚠|\?\?|FAIL|ALIAS_MARK|FAULT_MARK|RUNG)' "$LOG" | sed 's/^/    /'

echo ""
echo "================================================================================"
echo "=== ★★★★★ W379 GRADING — source $STAMP  inner_rc=$RC  $(date -Is)"
echo "================================================================================"

# ★ The grader reads EXACTLY the lines the rungs print. w377's rung printed prose while its
#   grader `sed`-ed for `RACEMAP_ARM_A=`, so a passing native run graded as UNMEASURED.
fail=0; notrun=0; pass=0; seen=0
for r in map_propagation alias_two_vas alias_unmap missing_page map_stress; do
  v=$(sed -n "s/^RUNG_${r}=//p" "$LOG" | tail -1)
  c=$(sed -n "s/^RUNGCTL_${r}=//p" "$LOG" | tail -1)
  printf '    %-18s RUNG=%-8s CONTROL=%s\n' "$r" "${v:-NONE}" "${c:-NONE}"
  [ -n "$v" ] && seen=$((seen+1))
  case "$v" in
    PASS)   pass=$((pass+1)) ;;
    FAIL)   fail=$((fail+1)) ;;
    NOTRUN) notrun=$((notrun+1)) ;;
  esac
done
# ⊘ EXACT COUNTS over a FIXED denominator — a capped list is not a census, and 4 is the
#   whole vocabulary rather than a sample of it.
echo "    counts: pass=$pass fail=$fail notrun=$notrun  of 5 rungs, $seen verdict lines seen"

echo ""
echo "=== ★★★★★ THE VERDICT — pre-registered, stated once"
if [ "$seen" -eq 0 ]; then
  echo "    W379_OUTCOME=(D) ⊘ UNMEASURED — not one RUNG_ line. NOT a failure value."
elif [ "$fail" -gt 0 ]; then
  echo "    W379_OUTCOME=(B) $fail rung(s) FAILED with their controls passing — a real red."
elif [ "$notrun" -gt 0 ] && [ "$pass" -eq 0 ]; then
  echo "    W379_OUTCOME=(C) ⊘ UNINTERPRETABLE — every selected rung's control failed."
else
  echo "    W379_OUTCOME=(A) $pass of 5 rungs PASS, $notrun not selected, 0 red."
  echo "        ⊘ On BARE METAL this is the CONTROL, not the milestone: it says the driver"
  echo "        permits what the guest does. It says nothing about our emulated path."
fi
echo "--- ★★ HARNESS SELF-CHECK — assert THIS block's own inputs exist ---"
echo "    log bytes         = [$(wc -c < "$LOG" 2>/dev/null)]"
echo "    RUNG_ lines       = [$(grep -ac '^RUNG_' "$LOG" 2>/dev/null)]  (MUST be 5)"
echo "    RUNGCTL_ lines    = [$(grep -ac '^RUNGCTL_' "$LOG" 2>/dev/null)]"
# ⊘ `grep -c Xid` over the whole log counts the RUNG'"'"'S OWN PROSE too — the R3 pass line
# contains the string `Xid 31`. Count the KERNEL'"'"'s format (`Xid (PCI:`) instead, so the
# number means what the label says. [caught w379: the naive count read 2 and the label said
# "exactly one", which is a self-check that would have hidden a second real fault.]
echo "    kernel Xid records = [$(grep -ac 'Xid (PCI:' "$LOG" 2>/dev/null)]  (R3 provokes exactly 1)"
echo "    ⊘ zero bytes is not 'not yet'; it is a state that needs its own check."
echo "=== W379 EXIT rc=$RC at $(date -Is) ==="
