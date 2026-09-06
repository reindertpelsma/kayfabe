#!/usr/bin/env bash
# ★★★★★ w384 — THE GUEST HALF: what one doorbell costs a thread INSIDE a Mode-2 guest.
#
#   usage: POST_CAPTURE_HOOK=scripts/bench/w384_hook.sh scripts/bench/boot_capture.sh <tag>
#   needs: KAYFABE_W384_BIN      — a **statically linked** kayfabe-rm-ladder (musl)
#          KAYFABE_W384_FLOOR_US — ★ THE NATIVE MEDIAN, in microseconds, measured minutes ago
#                                   by the SAME binary on THIS box. Without it there is no
#                                   grade, and this hook says so rather than inventing one.
#          GQ_TIMEOUT >= 600
#
# ## ⊘ THE ONE THING THAT MAKES OR BREAKS THIS HOOK
#
# `KAYFABE_W384_FLOOR_US` is not a convenience. The rung's gate is `native_p50 x multiple`,
# and a threshold that is not a multiple of something measured on the same hardware in the
# same hour is a number somebody made up. If the floor is absent this hook prints
# `(G) UNMEASURED_NO_FLOOR` and grades nothing — an ungraded honest run beats a graded
# invented one.
#
# ## ⊘ THE SCOPE CAVEAT, STATED HERE RATHER THAN DISCOVERED LATER
#
# ★ **The ladder builds its OWN `FERMI_VASPACE_A` inside the guest and its own CE channel.**
# What this measures is therefore the cost of a doorbell on a channel THIS PROCESS created —
# not the cost of the guest driver's own doorbells during `cuCtxCreate`, and not the LLM's.
# The claim it can support is *"a doorbell rung by an unprivileged guest process costs N"*,
# which is the quantity the async lane is moving. It is NOT *"the LLM's per-doorbell cost is
# N"*, and a reader who takes it for that has the campaign's recorded failure shape: a probe
# in the wrong address space.
#
# ⚠ **WHICH CE EXECUTOR THE BOOT RAN UNDER DECIDES WHAT THIS MEASURES.** With
# `KAYFABE_CE_EXECUTOR=local` the shell's CPU copy engine serves every CE doorbell; with
# `=host` an addressable USER-proc channel goes to the forwarding plane instead. The hook
# prints the value; a run that does not say which one it took cannot be compared to any other.
#
# ## ⊘ Traps this hook is written against, all measured in this repo
#  - **A statically linked binary, always.** A dynamic binary that fails to load reports as
#    *"the ladder found no GPU"* — a completely different finding. ASSERTED, not assumed.
#  - **Zero bytes is not "not yet".** A start marker and an explicit `W384_RC=` terminator, so
#    *"the file exists and has no terminator"* is detectable at all. The `timeout` is INSIDE
#    the guest, so `124` (the work ran out of time) and `143` (the launcher killed it) stay
#    distinguishable — they mean opposite things and arrive as the same word.
#  - **Grade on the rung's own verdict lines, anchored.** `RUNG_doorbell_latency=` and
#    `RUNGCTL_doorbell_latency=`, nothing else. A grader looking for a name nothing emits
#    graded a passing run as UNMEASURED once already.
set -uo pipefail
SELFDIR=$(cd "$(dirname "$0")" && pwd)
REPO=$(cd "$SELFDIR/../.." && pwd)
G="$SELFDIR/gssh_nv"
TAG=${1:-w384}
TGT=${CARGO_TARGET_DIR:-$REPO/target}
BIN=${KAYFABE_W384_BIN:-$TGT/x86_64-unknown-linux-musl/release/kayfabe-rm-ladder}
FLOOR=${KAYFABE_W384_FLOOR_US:-}
TMO=${KAYFABE_W384_TIMEOUT:-300}
# ★ THREE PROCESSES inside the guest, for the reason the native arm runs three: `submit_ms`
#   was measured at 9.1x across three consecutive boots of ONE build, and no number of
#   repetitions inside a process can see a between-process effect. These three share a boot,
#   so they still cannot see a per-BOOT lottery — they bound within-boot variation only, and
#   that limit is stated rather than papered over.
RUNS=${KAYFABE_W384_RUNS:-3}
OUT=/tmp/w384guest.out

die() { echo "★★★ w384 hook FAILED: $*"; echo "W384_GUEST_OUTCOME=(F) ⊘ UNMEASURED_NO_GUEST — $*"; exit 2; }

echo "=== ★★★★★ w384 — THE DOORBELL-LATENCY RUNG, IN THE GUEST  tag=$TAG  $(date -Is) ==="
echo "W384_CE_EXECUTOR=[${KAYFABE_CE_EXECUTOR:-⊘unset ⇒ local ⇒ the SHELL CPU copy engine serves every CE doorbell}]"
echo "W384_ISOLATES=[${KAYFABE_ISOLATES:-⊘unset}]  W384_GR_ROUTE=[${KAYFABE_GR_ROUTE:-⊘unset}]"
echo "W384_NATIVE_FLOOR_US=[${FLOOR:-⊘NONE}]"
[ -x "$G" ]   || die "no gssh_nv at $G"
[ -f "$BIN" ] || { echo "W384_GUEST_OUTCOME=(E) ⊘ UNMEASURED_NO_BINARY — no binary at $BIN"; exit 2; }

echo "--- the binary under test (⊘ the native arm MUST carry the same md5):"
printf '    %-72s %9s bytes\n' "$BIN" "$(stat -c %s "$BIN")"
echo "    W384_BIN_MD5=$(md5sum < "$BIN" | cut -d' ' -f1)"
case "$(file -b "$BIN")" in
  *static*) echo "    ★ STATIC — it will run in the guest's userland" ;;
  *) die "the binary is NOT statically linked; a load failure would report as 'no GPU'" ;;
esac

echo "=== guest preconditions (⊘ each is a DIFFERENT failure from 'the rung did not work') ==="
$G 'echo "GUEST_UNAME=$(uname -r)"; echo "GUEST_NVIDIA_DEVS=[$(ls /dev/nvidia* 2>/dev/null | tr "\n" " ")]"; echo "GUEST_NVRM_LOADED=$(lsmod | grep -c "^nvidia ")"; echo "GUEST_NPROC=$(nproc)"' \
  || die "the guest did not answer ssh"

echo "=== push the binary into the guest ==="
$G "cat > /tmp/kayfabe-rm-ladder && chmod +x /tmp/kayfabe-rm-ladder" < "$BIN" || die "could not push the binary"
$G 'echo "GUEST_MD5=$(md5sum < /tmp/kayfabe-rm-ladder | cut -d" " -f1)"'

# ⊘ NO FLOOR ⇒ NO GRADE. The rung is still RUN — the distribution is worth having either way
#   — but it is run WITHOUT `--doorbell-latency-native-us`, so it reports itself as a
#   CALIBRATION rather than silently grading against a threshold nobody measured.
if [ -n "$FLOOR" ]; then
  ARGS="--doorbell-latency --doorbell-latency-native-us $FLOOR"
else
  echo "⊘⊘ NO NATIVE FLOOR WAS PASSED IN. The rung will run and print its distribution, but"
  echo "   it will report DBL_ROLE=CALIBRATION and this hook will grade nothing."
  ARGS="--doorbell-latency"
fi
echo "W384_GUEST_ARGS=$ARGS"

echo "=== run it x$RUNS, under its OWN deadline, with a START marker and an RC terminator ==="
$G "echo STARTED \$(date -Is) > /tmp/w384.started; : > $OUT; for i in \$(seq 1 $RUNS); do echo \"--- guest run \$i/$RUNS ---\" >> $OUT; timeout $TMO sudo /tmp/kayfabe-rm-ladder $ARGS >> $OUT 2>&1; echo W384_RC=\$? >> $OUT; done"
LOCAL=/tmp/w384guest_${TAG}.out
$G "cat $OUT" > "$LOCAL" 2>/dev/null
echo "--- the rung's own output, verbatim ---"
cat "$LOCAL"
echo "--- end of the rung's output ---"

echo ""
echo "=== ★★★★★ THE GUEST GRADE — anchored on the rung's own verdict lines ==="
grep -a '^DBL_CFG\|^DBL_ROLE\|^DBL_WITNESS\|^DBL_DIST\|^DBL_NATIVE_P50_US\|^DBL_GATE_US\|^DBL_MEASURED_P50_US\|^DBL_REP_MEDIANS\|^DBL_CALIBRATION' "$LOCAL" 2>/dev/null | sed 's/^/    /'
pass=$(grep -ac '^RUNG_doorbell_latency=PASS' "$LOCAL")
fail=$(grep -ac '^RUNG_doorbell_latency=FAIL' "$LOCAL")
notrun=$(grep -ac '^RUNG_doorbell_latency=NOTRUN' "$LOCAL")
seen=$(grep -ac '^RUNG_doorbell_latency=' "$LOCAL")
ctlfail=$(grep -ac '^RUNGCTL_doorbell_latency=FAIL' "$LOCAL")
echo "    W384_GUEST_ROW runs=$RUNS seen=$seen pass=$pass fail=$fail notrun=$notrun ctl_failed=$ctlfail"
echo "    W384_GUEST_RC=[$(grep -aoE '^W384_RC=[0-9]+' "$LOCAL" 2>/dev/null | tr '\n' ' ')]"
echo "    ⊘ 124 = the work ran out of time INSIDE the guest; 143 = something killed it; an"
echo "      ABSENT line = no terminator was written at all. Three different facts."

echo ""
echo "=== ★★★★★ THE GUEST VERDICT — pre-registered, stated once"
if [ "$seen" -eq 0 ]; then
  echo "    W384_GUEST_OUTCOME=(D) ⊘ UNMEASURED — not one RUNG_doorbell_latency line. NOT a failure value."
elif [ "$notrun" -gt 0 ] && [ "$pass" -eq 0 ] && [ "$fail" -eq 0 ]; then
  # ⊘ `NOTRUN` covers TWO different ways of having nothing to say and the rung prints which:
  #   the positive control did not pass (`RUNGCTL_...=FAIL`), or it did and the loop produced
  #   no gradeable distribution (`R6 SAMPLES`). Neither is a failure value; both are (D).
  echo "    W384_GUEST_OUTCOME=(D) ⊘ UNINTERPRETABLE — $notrun of $seen runs printed NOTRUN"
  echo "        (control_failed_runs=$ctlfail). Either the positive control did not pass — so"
  echo "        every number is over submissions that carried no work — or too few samples fit"
  echo "        inside the wall budget to grade and the minimum was not determinate. NOT a red."
elif [ -z "$FLOOR" ]; then
  echo "    W384_GUEST_OUTCOME=(G) ⊘ UNMEASURED_NO_FLOOR — the distribution is real and the"
  echo "        verdict is not: nothing measured the bare-metal floor this must be a multiple of."
elif [ "$fail" -gt 0 ] && [ "$pass" -eq 0 ]; then
  echo "    W384_GUEST_OUTCOME=(A) ★★★★★ RED, all $fail of $seen runs — the intended shape."
  echo "        ⚠ Compare against the NATIVE row before attributing it: a guest-only red"
  echo "        cannot distinguish \"we are broken\" from \"the probe is wrong\"."
elif [ "$pass" -gt 0 ] && [ "$fail" -eq 0 ]; then
  echo "    W384_GUEST_OUTCOME=(B) ⚠⚠ GREEN, all $pass of $seen runs — A FINDING, NOT A GREEN."
  echo "        It would mean the 60-71 ms inline publication is NOT on the raw client's"
  echo "        doorbell path and the LLM's cost has been mis-attributed. Read \`arm=freshmap\`"
  echo "        against \`arm=submit\` above before concluding anything, and DO NOT tune the"
  echo "        rung until it goes red."
else
  echo "    W384_GUEST_OUTCOME=(H) ⊘ SPLIT — $pass pass, $fail fail over $seen runs. ★ That is"
  echo "        itself the result: the cost is BIMODAL within one boot, and a single-run gate"
  echo "        would report whichever side it happened to land on. Read DBL_REP_MEDIANS."
fi

echo ""
echo "=== the guest driver's own word across the run (⊘ the HOST ring buffer does not carry it) ==="
$G 'sudo dmesg 2>/dev/null | grep -iE "nvrm|xid" | tail -15 | sed "s/^/    /"'
echo "=== ★★ HOOK SELF-CHECK — assert this block's own inputs exist ==="
echo "    guest output bytes = [$(wc -c < "$LOCAL" 2>/dev/null)]"
echo "    DBL_DIST lines     = [$(grep -ac '^DBL_DIST' "$LOCAL" 2>/dev/null)]  (⊘ zero means the"
echo "                          rung never reached its loop, whatever else printed)"
echo "    ⊘ zero bytes is not 'not yet'; it is a state that needs its own check."
echo "=== w384 guest hook done $(date -Is) ==="
