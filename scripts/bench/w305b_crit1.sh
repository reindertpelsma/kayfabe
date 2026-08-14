#!/usr/bin/env bash
# ★★★★★ w305 ITEM B — CRITERION 1: DOES THE GUEST OBSERVE THE SAME FAULT, BY IDENTITY?
#
#   $1 = fresh | shared   (⊘ REQUIRED, never defaulted — a defaulted arm makes an evidence
#                          run and its own control indistinguishable at the call site)
#     fresh  : `--ce-client-fault`             — arm 4 in a THIRD, freshly allocated VAS.
#                                                BYTE-IDENTICAL to every committed run.
#     shared : `--ce-client-fault-shared-vas`  — arm 4 in arm 1's ALREADY-WORKING VAS.
#
# ## ★★★★★ THE NATIVE KNOWN-POSITIVE IS ALREADY IN HAND, AND IT REFUTES THE BRIEFED DIAGNOSIS
#
# `[measured 2026-08-14, w305, vh2, real GA106 580.159.04, NO QEMU]` the SAME program, same
# `--ce-client-fault`, same THIRD freshly-allocated VAS, run natively:
#
#   ★ R33 CRIT1 STATE = FAULT-PROVOKED-ADDRESS-READ | VA-IDENTITY MEASURED = yes
#   ★ R33 arm 5 WHERE = GET_MMU_FAULT_INFO addr=0x0000000900000000 faultType=0x0
#                       faultString="FAULT_PDE" | VA-IDENTITY HOLDS
#   host dmesg        = Xid 31 … channel 0x00000005 … MMU Fault: ENGINE CE0 HUBCLIENT_CE1
#                       faulted @ 0x9_00000000. Fault is of type FAULT_PDE ACCESS_TYPE_VIRT_READ
#
# ⇒ **A third, freshly-allocated VAS is NOT the blocker**: on real hardware that exact
#   arrangement provokes the fault, reads its address, and the VA identity holds. The probe is
#   CORRECT. ⊘ Therefore `CONTROL-NEVER-LANDED` in the guest is OUR device failing to carry
#   that channel — not the probe's VAS choice, which is what §2 of
#   `road_to_v1_after_cup2.md` ruled. See `rmladder.rs` at the arm-4 block for why the §2 fix
#   is ALSO a no-op in source: the operands were always in the ringing channel's VAS.
#
# ## ★★★ PRE-REGISTERED, BEFORE THE BOOT
#
#   (a) SAME FAULT BY IDENTITY  ⇒ criterion 1 MET. Report exactly WHICH FIELDS matched:
#       address, fault type, access type, engine/client. ⊘ A count of faults is not identity
#       — this rung compares DESCRIPTORS and prints both sides even when they agree.
#   (b) THE GUEST SEES *A* FAULT BUT A DIFFERENT ONE ⇒ print BOTH descriptors side by side.
#       ⊘ Do not average, do not pick. A difference in the ADDRESS is the headline, because
#       we map at identical VAs and the two MUST be equal.
#   (c) STILL `CONTROL-NEVER-LANDED` ⇒ the VAS was never the blocker and §2's diagnosis is
#       wrong. ★ Say so plainly; the native arm above is the known-positive that makes this
#       claim a measurement rather than a shrug.
#   (d) THE PROBE WILL NOT BUILD (`PROBE-NOT-BUILT`, e.g. the old `Other(0x80000016)`) ⇒ the
#       construction failure IS the wall. Report the refusing ioctl by name from the census.
#
# ⚠ `NV906F_CTRL_CMD_GET_MMU_FAULT_INFO` is read ON DEMAND, ONCE. The record is cleared by
#   reading it, so nothing here may fetch it eagerly or retry — a second ask answers all-zero
#   and decodes as a well-formed "fault at address 0".
set -uo pipefail
ARM="${1:-}"
case "$ARM" in
  fresh)  R33ARGS="--ce-client-fault" ;;
  shared) R33ARGS="--ce-client-fault-shared-vas" ;;
  *) echo "usage: $0 fresh|shared" >&2; exit 64 ;;
esac

REPO=${KAYFABE_REPO:-/workspace/kayfabe_w305}
export KAYFABE_REPO="$REPO"
export CARGO_TARGET_DIR=${CARGO_TARGET_DIR:-/workspace/bench/cargo-target-w305}
export KAYFABE_TAG=${KAYFABE_TAG:-w305b$ARM}
export GQ_TIMEOUT=${GQ_TIMEOUT:-600}

OUT=/workspace/${KAYFABE_TAG}.log
: >"$OUT"

# ★★★ DELETE THE CLIENT FIRST: no build ⇒ no file ⇒ no run. `[ -x ]` alone would happily run a
#   STALE binary from another rung's target dir, and the whole question is what THIS build does.
export PATH=/root/.cargo/bin:$PATH
CLIENT=$CARGO_TARGET_DIR/x86_64-unknown-linux-musl/release/kayfabe-rm-ladder
rm -f "$CLIENT"
{
  echo "=== W305B START arm=$ARM $(date -Is) pid=$$ ==="
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
echo "=== ★★★★★ W305 ITEM B GRADING — CRITERION 1 BY IDENTITY  arm=$ARM inner_rc=$BRC  $(date -Is)"
echo "================================================================================"
echo ""
echo "=== ⊘ PRECONDITION — the artefacts exist (zero bytes is not 'not yet')"
echo "    qemu log  = [$(stat -c%s "$Q" 2>/dev/null || echo MISSING)] bytes"
echo "    probe log = [$(stat -c%s "$P" 2>/dev/null || echo MISSING)] bytes"
echo "    guest terminator ^R33_RC = [$(grep -oE '^R33_RC=[0-9]+' "$P" 2>/dev/null | tail -1)]"
echo "    ⊘ 143 = the job was KILLED; 124 = the LAUNCHER expired while the job ran. Opposite."
echo "    the ARM ACTUALLY PASSED to the client = [$(grep -oE 'ce-client-fault(-shared-vas)?' "$P" 2>/dev/null | sort -u | tr '\n' ' ')]"
echo "    arm 4's SPACE line, verbatim:"
grep -a 'R33 arm 4 SPACE' "$P" 2>/dev/null | fold -w 200 | sed 's/^/      /'
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
      echo "    the address plane ANSWERED — read the join above for (a) vs (b)." ;;
  FAULT-PROVOKED-ADDRESS-SILENT)
      echo "    the fault WAS provoked and GET_MMU_FAULT_INFO gave no address ⇒ this run carries"
      echo "    the fault's CODE and not its ADDRESS. Criterion 1 is PART-MEASURED: compare the"
      echo "    notifier's code/engine against the host Xid, and say the address is UNMEASURED." ;;
  CONTROL-NEVER-LANDED)
      echo "    ★★★★★ (c) STILL CONTROL-NEVER-LANDED. The positive control did not land, so the"
      echo "    deliberate fault was never issued and every zero here is VACUOUS."
      echo "    ⇒ §2 of road_to_v1_after_cup2.md is WRONG: the same probe, same fresh VAS, works"
      echo "      NATIVELY on this box (see the header). The blocker is our device carrying the"
      echo "      channel, NOT the probe's VAS choice." ;;
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
echo "    w305b grading lines = [$(grep -c 'W305 ITEM B GRADING' "$OUT" 2>/dev/null)]  (MUST be >= 1)"
echo "=== W305B EXIT rc=$BRC arm=$ARM at $(date -Is) ==="
} >>"$OUT" 2>&1

exit "$BRC"
