#!/usr/bin/env bash
# ★★★★★ w381 — THE MAPPING-PLANE BATTERY AS A **DIFFERENTIAL**: the SAME binary, on bare
# metal and inside the Mode-2 guest, graded as a pair.
#
# ## WHY THIS EXISTS — the w379 battery could not run in a guest AT ALL
#
# Every w379 rung proved a VA live with `HostRmBackend::submit_release_at`, which emits the
# host-FIFO semaphore run. The Mode-2 CPU copy-engine emulator decodes that as
# `PushMethod::SemRelease` and **deliberately does not act on it**
# (`crates/kayfabe-rt/src/ceutils.rs`, the `else` arm of the `CeLaunchDma` `let`; the same
# decision restated in `release_targets_of`). ⇒ inside a guest EVERY w379 rung reports its
# own CONTROL as failed and prints `NOTRUN`. The battery was a native-only control.
#
# `--probe-launch-dma` swaps the primitive for one the emulator serves end to end
# (`run_submission` -> `execute_ours_spans` MOVES THE BYTES -> `write_resolved_completion`),
# and `--w381` selects the whole battery on it in ONE flag — so the two halves of the
# differential cannot drift apart by being assembled out of six flags each.
#
# ## ★★★ THE DIFFERENTIAL IS THE DELIVERABLE
#
#   A GUEST-ONLY RED CANNOT DISTINGUISH "we are broken" FROM "the probe is wrong."
#
# So this script runs ONE arm and prints a machine-readable block; the pairing is done by
# running it twice (`W381_ARM=native`, then `W381_ARM=guest`) and diffing the two blocks.
# Every rung's verdict line is `RUNG_<name>=PASS|FAIL|NOTRUN` and its control's is
# `RUNGCTL_<name>=`; ⊘ this grader `sed`s **exactly** those, because w377 printed prose while
# its grader looked for a name nothing emitted and a 3/3 run graded as UNMEASURED.
#
# ## ★★★ PRE-REGISTERED, BEFORE THE RUN — every outcome, so none reads as the favourable one
#   (A) every selected rung PASS            => the arm behaves as the driver does.
#   (B) any rung FAIL with its control PASS => a real red, attributable to that rung.
#   (C) any rung NOTRUN with control FAIL   => ⊘ UNINTERPRETABLE. The rung measured its own
#                                              harness. NOT a failure value.
#   (D) no RUNG_ line at all                => ⊘ UNMEASURED. Also not a failure value.
#   (E) no binary                           => ⊘ UNMEASURED_NO_BINARY — attributable to the
#                                              BUILD, not to the GPU.
#   (F) guest unreachable                   => ⊘ UNMEASURED_NO_GUEST — attributable to the
#                                              BOOT. ★ Distinct from (D): "the guest never
#                                              answered" and "the guest answered nothing" are
#                                              different facts and must not share a verdict.
#
# ⊘ **GRADED ON PRINTED LINES, NEVER ON EXIT STATUS.** A timeout kills the process and its
#   status says nothing about how far it got.
# ⊘ **`dmesg -C` IS NOT CALLED HERE.** The w379 script clears the host ring buffer, which
#   destroys a concurrent lane's evidence on a shared box. A WATERMARK is taken instead and
#   only the lines past it are read.
# ⚠ **`--missing-page-fault` PROVOKES A REAL `Xid 31`** and kills its victim channel. That is
#   the measurement. Its bystander arm is what proves the fault took nothing else with it.
set -uo pipefail

ARM=${W381_ARM:-native}
REPO=${KAYFABE_REPO:-/root/kayfabe}
export CARGO_TARGET_DIR=${CARGO_TARGET_DIR:-/workspace/bench/cargo-target-w381}
TAG=${KAYFABE_TAG:-w381}
LOG=/workspace/bench/run_${TAG}_${ARM}.log
TMO=${W381_TIMEOUT:-420}
RUNGS=${W381_RUNGS:---w381}
GUEST=${W381_GUEST:-ubuntu@192.168.77.2}
STAMP=$(git -C "$REPO" rev-parse --short HEAD 2>/dev/null || echo unknown)

# ⊘ The FIXED denominator. Seven names, written once, so the census and the grader can never
# disagree about how many rungs there are — and so a rung deleted in one place goes red here
# instead of shrinking the total silently.
RUNG_NAMES=(map_propagation alias_two_vas alias_unmap missing_page map_stress rpc_mixed cross_client)
N_RUNGS=${#RUNG_NAMES[@]}

echo "=== ★★★★★ w381 GUEST-SERVABLE BATTERY — arm=$ARM  source $STAMP  $(date -Is) ==="
echo "W381_ARM=$ARM"
echo "W381_RUNGS_ASKED=$RUNGS"

# ⚠ LOCATED, never assumed. The bin target is `kayfabe-rm-ladder`; a binary named `rmladder`
# does not exist and looking for one is how a build failure reads as a GPU one.
BIN=""
for c in "$CARGO_TARGET_DIR"/x86_64-unknown-linux-musl/release/kayfabe-rm-ladder \
         "$CARGO_TARGET_DIR"/release/kayfabe-rm-ladder \
         "$REPO"/target/x86_64-unknown-linux-musl/release/kayfabe-rm-ladder \
         "$REPO"/target/release/kayfabe-rm-ladder; do
  [ -x "$c" ] && BIN="$c" && break
done
if [ -z "$BIN" ]; then
  echo "W381_BIN_FOUND=no"
  echo "W381_OUTCOME=(E) ⊘ UNMEASURED_NO_BINARY — kayfabe-rm-ladder was never built"
  exit 0
fi
echo "W381_BIN_FOUND=yes ($BIN, $(stat -c %s "$BIN" 2>/dev/null) bytes)"

: > "$LOG"
echo "W381_START=$(date -Is) arm=$ARM rungs=$RUNGS" | tee -a "$LOG"
# ⊘ A WATERMARK, not a clear: see the header. `dmesg -C` on a shared box deletes somebody
# else's evidence, and this script must be safe to run beside another lane.
DMESG_MARK=$(sudo dmesg 2>/dev/null | wc -l)
echo "W381_DMESG_WATERMARK=$DMESG_MARK lines" | tee -a "$LOG"

case "$ARM" in
  native)
    # ⊘ NOT piped into anything: a pipe makes `$?` the pager's, and this repo has already had
    #   a harness print success over a workspace that never compiled.
    timeout "$TMO" "$BIN" $RUNGS >>"$LOG" 2>&1
    RC=$?
    ;;
  guest)
    # ★ The binary is STATIC musl on purpose — it is pushed into a guest whose libc, whose
    #   kernel and whose driver build are all somebody else's.
    SSHO="-o StrictHostKeyChecking=no -o UserKnownHostsFile=/dev/null -o LogLevel=ERROR -o ConnectTimeout=15"
    if ! timeout 40 ssh $SSHO "$GUEST" true 2>/dev/null; then
      echo "W381_GUEST_REACHABLE=no ($GUEST)" | tee -a "$LOG"
      echo "W381_OUTCOME=(F) ⊘ UNMEASURED_NO_GUEST — the guest never answered ssh. NOT a"
      echo "    failure value, and NOT the same as (D): attributable to the BOOT."
      exit 0
    fi
    echo "W381_GUEST_REACHABLE=yes ($GUEST)" | tee -a "$LOG"
    timeout 120 scp $SSHO -q "$BIN" "$GUEST":/tmp/kayfabe-rm-ladder || {
      echo "W381_OUTCOME=(F) ⊘ UNMEASURED_NO_GUEST — the binary could not be copied in"
      exit 0
    }
    # ⚠ `sudo` in the guest: the rungs open `/dev/nvidiactl` and `/dev/nvidia0`, and the
    #   euid is printed by every rung so a permission failure is never mistaken for a GPU one.
    timeout "$TMO" ssh $SSHO "$GUEST" \
      "chmod +x /tmp/kayfabe-rm-ladder && sudo /tmp/kayfabe-rm-ladder $RUNGS" >>"$LOG" 2>&1
    RC=$?
    # ★ The GUEST's own kernel log is where its driver's Xids are. The host's ring buffer
    #   does not carry them, and reading only the host's is the w377-class mistake of
    #   grepping the file that looks like it should have the evidence.
    echo "--- guest dmesg (NVRM/Xid) after the battery ---" | tee -a "$LOG"
    timeout 60 ssh $SSHO "$GUEST" "sudo dmesg | grep -iE 'xid|nvrm' | tail -25" >>"$LOG" 2>&1
    ;;
  *)
    echo "W381_OUTCOME=⊘ W381_ARM=$ARM is not an arm I know (native|guest)"; exit 2 ;;
esac
echo "W381_TERMINATOR=reached inner_rc=$RC" | tee -a "$LOG"

echo "--- host dmesg PAST THE WATERMARK ---" | tee -a "$LOG"
sudo dmesg 2>/dev/null | tail -n "+$((DMESG_MARK+1))" | grep -iE "xid|nvrm" | tail -20 | tee -a "$LOG"

grep -aE '^(info|ok|★|⊘|⚠|\?\?|FAIL|ALIAS_MARK|FAULT_MARK|RUNG|W381)' "$LOG" | sed 's/^/    /'

# ⊘⊘ THE GRADING BLOCK IS `tee`d INTO THE ARM'S OWN LOG, AND THAT IS NOT COSMETIC.
#
# It printed to stdout only in the first version, so `w381_differential.sh` — which builds its
# table by grepping `W381_TABLE_ROW` out of each arm's log FILE — found nothing and printed
# `⊘ NO TABLE ROW` for all three arms over three runs that had all passed. ★ Caught by
# dry-running the table block against real logs before shipping it, which is the only reason
# it is not the w377 defect again: a grader looking for a line its producer never wrote.
{
echo ""
echo "================================================================================"
echo "=== ★★★★★ W381 GRADING — arm=$ARM  source $STAMP  inner_rc=$RC  $(date -Is)"
echo "================================================================================"

PROBE=$(sed -n 's/^W381_PROBE=\([^ ]*\).*/\1/p' "$LOG" | tail -1)
echo "    W381_PROBE_USED=${PROBE:-NONE}"
if [ -z "$PROBE" ]; then
  echo "    ⚠ NO W381_PROBE LINE. A battery whose liveness primitive is not on its own log"
  echo "      cannot be compared to the other half of its differential."
fi

fail=0; notrun=0; pass=0; seen=0
for r in "${RUNG_NAMES[@]}"; do
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
# ⊘ EXACT COUNTS over a FIXED denominator — a capped list is not a census.
echo "    counts: pass=$pass fail=$fail notrun=$notrun  of $N_RUNGS rungs, $seen verdict lines seen"
echo "    W381_TABLE_ROW arm=$ARM probe=${PROBE:-NONE} pass=$pass fail=$fail notrun=$notrun seen=$seen of=$N_RUNGS"

echo ""
echo "=== ★★★★★ THE VERDICT — pre-registered, stated once"
if [ "$seen" -eq 0 ]; then
  echo "    W381_OUTCOME=(D) ⊘ UNMEASURED — not one RUNG_ line. NOT a failure value."
elif [ "$fail" -gt 0 ]; then
  echo "    W381_OUTCOME=(B) $fail rung(s) FAILED with their controls passing — a real red."
elif [ "$notrun" -gt 0 ] && [ "$pass" -eq 0 ]; then
  echo "    W381_OUTCOME=(C) ⊘ UNINTERPRETABLE — every selected rung's control failed."
  if [ "$PROBE" = "sem-release" ] && [ "$ARM" = "guest" ]; then
    echo "        ★ AND THIS IS THE EXPECTED ANSWER FOR THIS ARM: the emulator does not act"
    echo "        on \`SemRelease\`. Re-run with \`--probe-launch-dma\`; a red here is the"
    echo "        emulator's DECLARED SCOPE and not a defect."
  fi
else
  echo "    W381_OUTCOME=(A) $pass of $N_RUNGS rungs PASS, $notrun not selected, 0 red."
  if [ "$ARM" = "native" ]; then
    echo "        ⊘ On BARE METAL this is the CONTROL, not the milestone: it says the driver"
    echo "        permits what the guest does. It says nothing about our emulated path."
  else
    echo "        ★★★ In the GUEST this is the result the native arm exists to be compared to."
  fi
fi
echo "--- ★★ HARNESS SELF-CHECK — assert THIS block's own inputs exist ---"
echo "    log bytes          = [$(wc -c < "$LOG" 2>/dev/null)]"
echo "    RUNG_ lines        = [$(grep -ac '^RUNG_' "$LOG" 2>/dev/null)]  (MUST be $N_RUNGS)"
echo "    RUNGCTL_ lines     = [$(grep -ac '^RUNGCTL_' "$LOG" 2>/dev/null)]"
# ⊘ `grep -c Xid` over the whole log counts the RUNG'S OWN PROSE too — the R3 pass line
# contains the string `Xid 31`. Count the KERNEL's format (`Xid (PCI:`) instead, so the
# number means what the label says.
echo "    kernel Xid records = [$(grep -ac 'Xid (PCI:' "$LOG" 2>/dev/null)]  (R3 provokes exactly 1)"
echo "    ⊘ zero bytes is not 'not yet'; it is a state that needs its own check."
echo "=== W381 EXIT rc=$RC arm=$ARM at $(date -Is) ==="
} 2>&1 | tee -a "$LOG"
