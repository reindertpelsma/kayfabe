#!/usr/bin/env bash
# ★★★★★ w309 — ISOLATE THE `CONTROL-NEVER-LANDED` DISCRIMINATOR: ONE CONFOUND PER ARM.
#
#   $1 = fresh | shared | rmplaced | nonotif   (⊘ REQUIRED, never defaulted)
#
# ## THE CONTRAST THIS RUNG EXISTS TO ISOLATE
#
# `[measured 2026-08-14, w305, vh2]` in ONE process, ONE program, ONE boot, on our emulated GPU:
#   arm 1  (a `Vas` that had ALREADY CARRIED RETIRED WORK) → 4096 bytes moved, whole four-fact bar
#   arm 4  (a FRESH `Vas`, same COPY0 class)               → control never landed, nothing moved,
#                                                            zero `Xid` on host AND guest
# ⊘ Natively BOTH work, so the asymmetry is ours. ⊘ But arm 4 differs from arm 1 in FOUR ways at
#   once, so w305 could not name which one it is.
#
# ## ★★★ THE ARM MATRIX — WHAT EACH ARM DISCRIMINATES, STATED BEFORE IT RUNS
#
#   arm        vas      ring+operand VAs   notifier   ordinal   discriminates
#   ---------  -------  -----------------  ---------  -------   -------------------------------
#   fresh      FRESH    DICTATED           PRESENT    2         ⊘ NOTHING — it is the CONTROL.
#                                                              Does w305's contrast REPRODUCE?
#   shared     SHARED   DICTATED           PRESENT    2         ★ VAS FRESHNESS, ALONE.
#   rmplaced   FRESH    RM-PLACED          PRESENT    2         ★ ADDRESS DICTATION, ALONE.
#   nonotif    FRESH    DICTATED           ABSENT     2         ★ NOTIFIER PRESENCE, ALONE.
#
# ⊘⊘ **CHANNEL ORDINAL IS HELD AT 2 ON EVERY ARM AND IS NOT SETTABLE — SAID PLAINLY RATHER THAN
#    LEFT OUT.** Arms 2/3/6 read `ce_control_placement`, which does not exist until arm 1 has
#    built a channel, so making the probe the process's FIRST channel is a different program,
#    not a flag. ⇒ THREE of the four confounds move here; the fourth does not, and no arm below
#    may be read as having tested it.
#
# ★★★ **AND NOTE WHAT ONE ARM CAN SETTLE.** If `shared` LANDS, then VAS-freshness is the
#    discriminator AND — because dictation, the notifier and the ordinal were all held at arm 4's
#    values while it landed — none of those three is a sufficient blocker. One arm, four answers.
#    If `shared` does NOT land, VAS-freshness is refuted and `rmplaced`/`nonotif` are the follow-up.
#
# ## ★★★★★ BOTH ARMS ARE NATIVE KNOWN-POSITIVES ON THIS BOX, MEASURED BEFORE ANY BOOT
#
# `[measured 2026-08-14, w309, vh2, real GA106 580.159.04, NO QEMU, rev 891cc7b9]`
#   `--ce-client-fault`            → w305: CRIT1 = FAULT-PROVOKED-ADDRESS-READ, VA-IDENTITY HOLDS
#   `--ce-client-fault-shared-vas` → w309: CRIT1 = FAULT-PROVOKED-ADDRESS-READ, VA-IDENTITY HOLDS
#                                    addr 0x0000000900000000 both planes; host `Xid 31 … CE0
#                                    HUBCLIENT_CE1 faulted @ 0x9_00000000 FAULT_PDE VIRT_READ`
# ⇒ Neither arm can fail in the guest for a reason intrinsic to the probe. **The probe is
#   correct on both arms; what differs is our device underneath it.**
#
# ## ⊘⊘⊘ WHY `shared` COULD NOT RUN IN w305 — AND WHY w305'S DIAGNOSIS OF THAT WAS WRONG
#
# w305 got `PROBE-NOT-BUILT / BadHandle(0xcafe0005)` and ruled: *"`alloc_channel_in` wants the
# RANGE and arm 1's vas handle is the SPACE; the fix is one line — pass the range."*
# ⊘ **REFUTED FROM SOURCE, w309.** `alloc_vaspace` RETURNS the range (`rm.rs:4082`); the handle
# was already the range; and `0xcafe0009` — which w305 called *"its paired range"* — is the
# **executor VAS** `map_dma_both` lazily mints (`rm.rs:4149`), a third object entirely.
# ★ The real cause: `pde_info` looked its `hVASpace` up with `companion_of`, the **REMOVING**
# accessor whose only legitimate caller is `free`. Arm 6 runs BEFORE arm 4 and ATE THE PAIRING.
# ⇒ Two symptoms, one cause, both visible in every committed R33 log and never joined:
#     (1) arm 6's first `pde_info` answered and every later one printed `Other(19270)`
#         (= `NOT_ON_THIS_RUNG`) as *"the control refused"* — it had not;
#     (2) `shared` died at `space_of(range) → None`.
# Fixed to the peeking `space_of`, whose own doc-comment already warned of exactly this.
#
# ## ★★★ PRE-REGISTERED, BEFORE THE BOOT
#
#   (A) ONE CONFOUND ISOLATED ⇒ name it, with the contrast that proves it.
#   (B) THE CONTRAST DOES NOT REPRODUCE (arm 4 lands on `fresh`, or arm 1 fails) ⇒ w305's
#       difference was boot noise. A FULL RESULT — say it plainly and the lead dies.
#   (C) CRITERION 1 MET ⇒ list exactly WHICH IDENTITY FIELDS matched: address, fault type,
#       access type, engine/client. ⊘ A COUNT CANNOT SEE A SUBSTITUTION — descriptors, never counts.
#   (D) THE ARM STILL WILL NOT BUILD ⇒ the construction failure IS the wall.
#   (E) VA IDENTITY BROKEN (guest and host name DIFFERENT addresses) ⇒ STOP AND LEAD WITH IT.
#       It outranks criterion 1: we map at identical VAs, so they MUST be equal.
#
# ⚠ `NV906F_CTRL_CMD_GET_MMU_FAULT_INFO` is read ON DEMAND, ONCE. The record is CLEARED by
#   reading it, so nothing here may fetch it eagerly or retry — a second ask answers all-zero
#   and decodes as a well-formed "fault at address 0".
set -uo pipefail
ARM="${1:-}"
case "$ARM" in
  fresh)    R33ARGS="--ce-client-fault" ;;
  shared)   R33ARGS="--ce-client-fault-shared-vas" ;;
  rmplaced) R33ARGS="--ce-client-fault-rm-placed" ;;
  nonotif)  R33ARGS="--ce-client-fault-no-notifier" ;;
  *) echo "usage: $0 fresh|shared|rmplaced|nonotif" >&2; exit 64 ;;
esac

REPO=${KAYFABE_REPO:-/workspace/kayfabe_w309}
export KAYFABE_REPO="$REPO"
export CARGO_TARGET_DIR=${CARGO_TARGET_DIR:-/workspace/bench/cargo-target-w309}
export KAYFABE_TAG=${KAYFABE_TAG:-w309$ARM}
export GQ_TIMEOUT=${GQ_TIMEOUT:-600}

OUT=/workspace/${KAYFABE_TAG}.log
: >"$OUT"

# ★★★ DELETE THE CLIENT FIRST: no build ⇒ no file ⇒ no run. `[ -x ]` alone would happily run a
#   STALE binary from another rung's target dir, and the whole question is what THIS build does.
export PATH=/root/.cargo/bin:$PATH
CLIENT=$CARGO_TARGET_DIR/x86_64-unknown-linux-musl/release/kayfabe-rm-ladder
rm -f "$CLIENT"
{
  echo "=== W309 START arm=$ARM $(date -Is) pid=$$ ==="
  cd "$REPO" || exit 90
  echo "=== HEAD=$(git rev-parse HEAD) ==="
  echo "=== BUILD MUSL CLIENT $(date -Is) ==="
  cargo build --release --target x86_64-unknown-linux-musl --bin kayfabe-rm-ladder
  echo "=== CLIENT BUILD RC=$? ==="
} >>"$OUT" 2>&1
[ -x "$CLIENT" ] || { echo "=== ⊘ NO CLIENT BINARY — UNMEASURED, rc 95 ===" >>"$OUT"; exit 95; }
echo "=== CLIENT md5 $(md5sum "$CLIENT" | cut -d' ' -f1) ===" >>"$OUT"

export KAYFABE_R33_BIN="$CLIENT"
export KAYFABE_R33_ARGS="$R33ARGS"
export POST_CAPTURE_HOOK="$REPO/scripts/bench/r33_hook_ce_client.sh"

rm -f /workspace/bench/qemu-build/qemu-system-x86_64
"$REPO/scripts/bench/w290p_run.sh" "${W298_ARM:-drain}" >>"$OUT" 2>&1
BRC=$?

Q=/workspace/bench/run_${KAYFABE_TAG}_qemu.log
P=/workspace/bench/run_${KAYFABE_TAG}_probe.log
D=/workspace/bench/run_${KAYFABE_TAG}_hostdmesg.log

{
echo ""
echo "================================================================================"
echo "=== ★★★★★ W309 GRADING — ONE CONFOUND PER ARM, CRITERION 1 BY IDENTITY  arm=$ARM inner_rc=$BRC  $(date -Is)"
echo "================================================================================"
echo ""
echo "=== ⊘ PRECONDITION — the artefacts exist (zero bytes is not 'not yet')"
echo "    qemu log  = [$(stat -c%s "$Q" 2>/dev/null || echo MISSING)] bytes"
echo "    probe log = [$(stat -c%s "$P" 2>/dev/null || echo MISSING)] bytes"
echo "    guest terminator ^R33_RC = [$(grep -oE '^R33_RC=[0-9]+' "$P" 2>/dev/null | tail -1)]"
echo "    ⊘ 143 = the job was KILLED; 124 = the LAUNCHER expired while the job ran. Opposite."
echo ""
echo "=== ⚠ AN ARM YOU SET IS NOT AN ARM IN FORCE — read the arming from the CLIENT'S OWN emissions."
echo "    ⊘ w305 recovered this by grepping for 'ce-client-fault' and printed [] on BOTH boots,"
echo "      because nothing echoed the client's args. Two independent lines now, and they must agree:"
echo "    (1) what the process was ASKED to do — its own argv, echoed before any flag was parsed:"
grep -a 'RMLADDER ARGV' "$P" 2>/dev/null | fold -w 200 | sed 's/^/      /'
echo "    (2) what the probe ACTUALLY RAN — built from the values passed to it, not from flags:"
grep -a 'R33 arm 4 CONFIG' "$P" 2>/dev/null | fold -w 200 | sed 's/^/      /'
echo "    the harness INTENDED = [$ARM ⇒ $R33ARGS]"
echo "    ⊘ If (1) and (2) disagree with each other or with the line above, this boot measured a"
echo "      DIFFERENT experiment than the one it is filed under, and every verdict below is void."
echo "    arm 4's SPACE line, verbatim:"
grep -a 'R33 arm 4 SPACE' "$P" 2>/dev/null | fold -w 200 | sed 's/^/      /'
echo ""
echo "=== ★★★★★ THE CONTRAST THIS RUNG IS ABOUT — arm 1 vs arm 4, ONE PROCESS, ONE BOOT."
echo "===     ⊘ arm 1 is the IN-BOOT KNOWN-POSITIVE. If arm 1 FAILS, arm 4 failing says NOTHING"
echo "===       about any confound and this boot is outcome (B)-adjacent noise, not a result."
echo "--- arm 1 (the VAS that carries the work):"
grep -aE 'R33 arm 1 (COPY|OPERANDS)' "$P" 2>/dev/null | fold -w 200 | sed 's/^/      /'
echo "--- arm 4 (the probe):"
grep -aE 'R33 arm 4 (FAULTED|control|RESOLVED|AMBIGUOUS)' "$P" 2>/dev/null | fold -w 200 | sed 's/^/      /'
A1=$(grep -ac '★     R33 arm 1 COPY' "$P" 2>/dev/null)
echo "    arm 1 met the whole four-fact bar = [$( [ "${A1:-0}" -ge 1 ] && echo YES || echo NO ⊘ THE IN-BOOT KNOWN-POSITIVE DID NOT FIRE )]"
echo ""
echo "=== ★★★★★ THE GATE — CRIT1 STATE. Every VA-IDENTITY number below is VACUOUS unless"
echo "===     this reads FAULT-PROVOKED-ADDRESS-READ."
grep -a 'R33 CRIT1 STATE' "$P" 2>/dev/null | fold -w 200 | sed 's/^/      /'
CST=$(grep -oE 'CRIT1 STATE     = [A-Z-]+' "$P" 2>/dev/null | tail -1 | awk '{print $NF}')
echo "    parsed CRIT1 = [${CST:-⊘ NO CRIT1 LINE AT ALL — the client never printed it. UNMEASURED}]"
echo ""
echo "=== ★★★★★ THE GUEST'S OWN DESCRIPTOR (planes A + D, read IN THE GUEST PROCESS)"
echo "--- plane D, WHERE — GET_MMU_FAULT_INFO, relayed one-to-one, read ONCE, on demand:"
grep -a 'R33 arm 5 WHERE' "$P" 2>/dev/null | fold -w 200 | sed 's/^/      /'
echo "--- plane A, the error notifier (code + engine; it has NO address field):"
grep -a 'R33 arm 5 NOTIFIER' "$P" 2>/dev/null | fold -w 200 | sed 's/^/      /'
echo "--- the negative control on the SAME 16 bytes, BEFORE the fault:"
grep -a 'R33 arm 5 CONTROL' "$P" 2>/dev/null | fold -w 200 | sed 's/^/      /'
echo "--- arm 4's own verdict:"
grep -aE 'R33 arm 4 (FAULTED|control|RESOLVED|AMBIGUOUS)' "$P" 2>/dev/null | fold -w 200 | sed 's/^/      /'
echo ""
echo "=== ★★★★★ THE HOST'S DESCRIPTOR (host dmesg, watermarked to THIS boot)"
if [ -e "$D" ]; then
  echo "    host dmesg delta exists, $(stat -c%s "$D") bytes"
  grep -aE 'Xid' "$D" 2>/dev/null | sed 's/^/      /'
  echo "      Xid count      = [$(grep -c Xid "$D")]"
  echo "      ⊘ A COUNT CANNOT SEE A SUBSTITUTION — the fields below are the measurement."
  echo "      host address   = [$(grep -oE 'faulted @ 0x[0-9a-f_]+' "$D" 2>/dev/null | sort -u | tr '\n' ' ')]"
  echo "      host faultType = [$(grep -oE 'type FAULT_[A-Z0-9_]+' "$D" 2>/dev/null | sort -u | tr '\n' ' ')]"
  echo "      host access    = [$(grep -oE 'ACCESS_TYPE_[A-Z_]+' "$D" 2>/dev/null | sort -u | tr '\n' ' ')]"
  echo "      host engine    = [$(grep -oE 'ENGINE [A-Z0-9_]+' "$D" 2>/dev/null | sort -u | tr '\n' ' ')]"
  echo "      host hubclient = [$(grep -oE 'HUBCLIENT_[A-Z0-9_]+' "$D" 2>/dev/null | sort -u | tr '\n' ' ')]"
  echo "      host channel   = [$(grep -oE 'channel 0x[0-9a-f]+' "$D" 2>/dev/null | sort -u | tr '\n' ' ')]"
else
  echo "    ⊘ NO HOST DMESG FILE AT ALL — UNMEASURED. ⊘ NOT 'no fault'."
fi
echo ""
echo "=== ★★★★★ THE JOIN, BY IDENTITY — field by field, BOTH SIDES PRINTED EVEN WHEN THEY AGREE"
echo "--- the NATIVE reference, measured 2026-08-14 on this same box with this same program:"
echo "      native  address   = 0x0000000900000000   (both planes: in-process AND host dmesg)"
echo "      native  faultType = FAULT_PDE (0x0)"
echo "      native  access    = ACCESS_TYPE_VIRT_READ"
echo "      native  engine    = ENGINE CE0 / HUBCLIENT_CE1"
GADDR=$(grep -oE 'GET_MMU_FAULT_INFO addr=0x[0-9a-f]+' "$P" 2>/dev/null | tail -1 | grep -oE '0x[0-9a-f]+')
GTYPE=$(grep -oE 'faultString="[A-Z0-9_]+"' "$P" 2>/dev/null | tail -1)
HADDR=$(grep -oE 'faulted @ 0x[0-9a-f_]+' "$D" 2>/dev/null | tail -1 | grep -oE '0x[0-9a-f_]+' | tr -d '_')
echo "--- THIS BOOT:"
echo "      guest   address   = [${GADDR:-⊘ UNMEASURED — plane D did not answer}]"
echo "      guest   faultType = [${GTYPE:-⊘ UNMEASURED}]"
echo "      host    address   = [${HADDR:-⊘ UNMEASURED — no host Xid on this boot}]"
echo "      the probe ASKED for = 0x900000000 (UNMAPPED_VA, dictated and asserted at compile time)"
if [ -n "${GADDR:-}" ] && [ -n "${HADDR:-}" ]; then
  # ⊘ Normalised to plain hex on both sides before comparing: the host log writes `0x9_00000000`
  #   and the client writes `0x0000000900000000`, and a string compare of those says DIFFERENT
  #   for two identical numbers. Compared as NUMBERS, printed as both.
  GN=$((GADDR)); HN=$((HADDR))
  printf '      normalised: guest=%#x host=%#x\n' "$GN" "$HN"
  if [ "$GN" -eq "$HN" ]; then
    echo "      ★★★★★ (a) ADDRESS IDENTITY HOLDS — the guest's fault address EQUALS the host's."
  else
    echo "      ⊘⊘ (b) ADDRESS IDENTITY BROKEN — guest and host name DIFFERENT addresses."
    echo "         ⚠ We map guest ranges at IDENTICAL host VAs, so these MUST be equal."
    echo "           THIS IS THE HEADLINE: VA identity is what the whole port rests on."
  fi
else
  echo "      ⊘ THE JOIN IS VACUOUS ON THIS BOOT — one side is unmeasured. NOT a disagreement."
fi
echo ""
echo "=== ★ THE VERDICT, in the pre-registered vocabulary"
case "${CST:-}" in
  FAULT-PROVOKED-ADDRESS-READ)
      echo "    ★★★★★ THE PROBE LANDED IN THE GUEST. Read the join above for (C) vs (E)."
      case "$ARM" in
        fresh)    echo "      ⊘⊘ ON THE **CONTROL** ARM ⇒ pre-registered outcome (B): w305's"
                  echo "         arm-1-vs-arm-4 contrast DID NOT REPRODUCE and was boot noise."
                  echo "         Say it plainly; the VAS lead dies here." ;;
        shared)   echo "      ★★★★★ (A) ONE CONFOUND ISOLATED = **VAS FRESHNESS**. Everything else"
                  echo "         (dictated addresses, notifier present, channel ordinal 2) was held"
                  echo "         at arm 4's values and it landed ⇒ none of those three is a"
                  echo "         sufficient blocker, and a VAS that has carried retired work is." ;;
        rmplaced) echo "      ★★★★★ (A) ONE CONFOUND ISOLATED = **ADDRESS DICTATION**." ;;
        nonotif)  echo "      ★★★★★ (A) ONE CONFOUND ISOLATED = **NOTIFIER PRESENCE**."
                  echo "         ⊘ Plane A is structurally absent on this arm, so criterion 1 can"
                  echo "           only be part-measured here: plane D carries the address, and the"
                  echo "           notifier's code/engine are UNMEASURED, not quiet." ;;
      esac ;;
  FAULT-PROVOKED-ADDRESS-SILENT)
      echo "    the fault WAS provoked and GET_MMU_FAULT_INFO gave no address ⇒ this run carries"
      echo "    the fault's CODE and not its ADDRESS. Criterion 1 is PART-MEASURED: compare the"
      echo "    notifier's code/engine against the host Xid, and say the address is UNMEASURED." ;;
  CONTROL-NEVER-LANDED)
      echo "    CONTROL-NEVER-LANDED on arm=$ARM. The positive control did not land, so the"
      echo "    deliberate fault was never issued and every zero here is VACUOUS."
      echo "    ⊘ This arm's confound is therefore NOT the discriminator — read it against the"
      echo "      arm-1 contrast above and against the OTHER arms, never on its own."
      case "$ARM" in
        fresh)    echo "      ⇒ expected: this is the CONTROL. It reproduces w305." ;;
        shared)   echo "      ⇒ ★★★ VAS FRESHNESS IS REFUTED as the discriminator: a VAS that had"
                  echo "        already carried retired work did NOT rescue the probe. Run rmplaced"
                  echo "        and nonotif." ;;
        rmplaced) echo "      ⇒ ADDRESS DICTATION is refuted as the discriminator." ;;
        nonotif)  echo "      ⇒ NOTIFIER PRESENCE is refuted as the discriminator." ;;
      esac ;;
  PROBE-NOT-BUILT)
      echo "    (d) THE PROBE COULD NOT BE CONSTRUCTED — the construction failure IS the wall."
      echo "    The refusing ioctl, by name, from the client's own in-band census:" ;;
  ARM-NOT-SELECTED)
      echo "    ⊘ THE FAULT ARM WAS NEVER REQUESTED — the harness passed the wrong args."
      echo "      This is a HARNESS defect, not a result." ;;
  *)  echo "    ⊘ NO CRIT1 STATE PARSED — UNMEASURED. Read the client's raw output below." ;;
esac
echo ""
echo "=== ★★★ `failed=0` IS NOT 'NOTHING REFUSED' — the client's OWN in-band census"
echo "===     RM writes status INTO THE PARAMETER STRUCT while ioctl(2) returns 0."
grep -a 'IN-BAND VERDICT' "$P" 2>/dev/null | sed 's/^/      /'
grep -a 'IN-BAND CAL' "$P" 2>/dev/null | fold -w 200 | sed 's/^/      /'
echo "      ⊘ if the CAL line is absent the reader is UNCALIBRATED and a zero-refusal report"
echo "        is a blind spot rather than a measurement."
echo "--- every census row that carries a non-ok status:"
grep -aE '^ +[0-9]+: nr +[0-9]+ ' "$P" 2>/dev/null | grep -v 'ok *RM ok' | head -20 | sed 's/^/      /'
echo ""
echo "=== ★ THE CLIENT'S FAIL LINES, verbatim"
grep -a '^FAIL ' "$P" 2>/dev/null | fold -w 200 | sed 's/^/      /'
echo ""
echo "=== ★★ HARNESS SELF-CHECK"
echo "    w309 grading lines = [$(grep -c 'W305 ITEM B GRADING' "$OUT" 2>/dev/null)]  (MUST be >= 1)"
echo "=== W309 EXIT rc=$BRC arm=$ARM at $(date -Is) ==="
} >>"$OUT" 2>&1

exit "$BRC"
