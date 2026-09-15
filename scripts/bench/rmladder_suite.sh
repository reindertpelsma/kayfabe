#!/usr/bin/env bash
# ★★★★★ THE RAW CLIENT AS A SUITE — because a test nobody runs is not a test.
#
# > Owner, 2026-09-14: *"can you ensure everything of the raw client is run, a test that exists
# > but nobody runs is not a useful test"* … *"tests are suites for a reason, not probes"*.
#
# ## ⊘⊘ What this replaces, and the number that makes the case
#
# `kayfabe-rm-ladder` has **30 arms**. Before this file:
#   - the graded GUEST boot (`w392d_mean_hook.sh`) ran **one** (`--uvm-mean`);
#   - `run_full_suite.sh` — the file whose own header calls itself *"THE ONE WAY TO RUN
#     EVERYTHING"* — ran **two** (`--engines`, `--concurrency`).
#
# So ~28 arms existed and were exercised by nobody, including `--ce-client`: the CPU-writes-
# vidmem-through-MMIO → CE-DMA-copies-it → CPU-reads-it-back round trip. It passes on bare metal
# and had never been run in a guest, which is precisely where it can fail.
#
# ## ★ Design: one PROCESS per arm, not one process with 30 flags
#
# Each arm allocates, submits and tears down its own RM state. Running them in one process would
# let arm N's leak or wedge decide arm N+1's verdict, and a crash would take the remaining arms
# with it — turning 30 measurements into one. ⊘ Separate processes also mean a hang is attributed
# to the arm that hung, which is the whole point of a per-arm timeout.
#
# ## ★★★★★ w735 — AND SEPARATE PROCESSES ARE NOT ENOUGH, BECAUSE THE WEDGE IS IN THE DEVICE
#
# `[measured w717, and again w734t]` one arm made the device unopenable and the arms after it
# all died at `RM bring-up failed at R1 openat(nvidia<gpu>)`. w717b taught this file to *detect*
# that and print `SKIP/cascade` instead of manufacturing 25 defects — which was right, and which
# still ends with **26 of 30 arms reporting nothing**. A suite that cannot report on 26 of its
# own arms is not a licence for anything, and `SINGLE_STORE_PLAN.md` §7's deletions are licensed
# by **this suite**, not by one workload.
#
# ⇒ Detection is not containment. This file now **RECOVERS and RETRIES**:
#
#   1. an arm whose log says `openat(nvidia` measured NOTHING — it never reached its subject;
#   2. so the device is put back (`$RMLADDER_RECOVER`, default = reload the guest's NVIDIA
#      modules) and the arm is run **again**, once;
#   3. the retry's verdict is the arm's verdict. If the retry ALSO cannot open the node, the arm
#      is `UNMEASURED` — ⊘ **which is a worse outcome than FAIL, not a better one**, and it makes
#      `SUITE_RC` non-zero exactly as a failure does.
#
# ⚠ A TIMEOUT is recovered from too, and for a different reason: `[measured w424]` the wedge is
# what a *previous* process left behind, so an arm that hung is the most likely thing to have
# left it. Recovering after a TIMEOUT does not change that arm's verdict — it is still TIMEOUT —
# it protects the arms after it.
#
# ⊘⊘ **RECOVERY IS INSTRUMENTED, NOT ASSUMED.** `SUITE_RECOVERIES=` counts attempts and
# `SUITE_RECOVERED=` counts the ones after which the device opened. If those two differ, the
# wedge survives a driver reload — which is a **finding about where the leaked state lives**
# (the guest's RM, or ours) and is printed as one.
#
# ## ★★ The outcomes, pre-registered so none reads as the favourable one
#
#   PASS       exit 0.
#   FAIL       exit non-zero — the arm ran and judged itself failed. ★ THE DIAGNOSTIC OUTCOME.
#   TIMEOUT    exceeded ARM_TIMEOUT. ⊘ NOT a FAIL: a hang and a refusal have different causes,
#              and collapsing them loses the distinction this campaign has paid for repeatedly.
#   UNMEASURED the device would not open, before AND after recovery. The arm never reached its
#              subject, so this is not its verdict — it is the absence of one. ⊘ Counted
#              against the suite, never toward it.
#   ⊘ There is no SKIP. w717b's `SKIP/cascade` was the only producer of one and it is now
#     UNMEASURED; no arm declares a precondition of its own today. A field nothing can set
#     prints a reassuring zero forever.
#
# ⚠ An arm's own internal verdict is NOT re-derived here. This file reports what the arm said
# about itself; it does not grade the GPU. Read the arm's own output for that.
#
# ## ⊘ THE OPEN-ORDINAL PROBE IS A SEPARATE RUN, ON PURPOSE
#
# `[measured w424]` guest device opens #1–#3 pass and **#4 fails**, permanently, because RM's
# own CeUtils scrubber cannot initialise its copy engine on the Nth `RmInitAdapter`. Every arm
# is a device open, so that wall bounds the whole suite. `RMLADDER_OPEN_PROBE=<n>` measures where
# it is — and it **consumes opens**, so it runs INSTEAD of the suite and never beside it. An
# instrument that spends the resource it is measuring cannot share a run with the measurement.
#
#   usage: rmladder_suite.sh <path-to-kayfabe-rm-ladder> [gpu] [arm-timeout-seconds]
set -uo pipefail
BIN=${1:?usage: rmladder_suite.sh <binary> [gpu] [timeout]}
GPU=${2:-0}
ARM_TIMEOUT=${3:-${RMLADDER_ARM_TIMEOUT:-120}}
OUT=${RMLADDER_SUITE_LOGDIR:-/tmp/rmladder_suite}
mkdir -p "$OUT"

# ★★★ A START MARKER, because `[measured 2026-08-10]` an empty output file read as "still in
# flight" three times. The terminator is `SUITE_RC=` at the bottom; anything between this line
# and that one is a run that DIED, which is a state of its own and not "not yet".
printf 'SUITE_STARTED=%s bin=%s gpu=%s arm_timeout=%s uid=%s\n' \
       "$(date -u +%FT%TZ)" "$BIN" "$GPU" "$ARM_TIMEOUT" "$(id -u)"

[ -x "$BIN" ] || { echo "SUITE_RC=2 ⊘ no binary at $BIN — UNMEASURED, not a failure"; exit 2; }

# ★★★★★ **THE SUITE MUST RUN AS ROOT, AND IT SAYS SO RATHER THAN DEGRADING.**
#
# `[measured w734t]` the guest hook ran this as `ubuntu` while the 30/30 host reference and the
# graded `--uvm-mean` boot both ran under `sudo`. That is not the same experiment: the ladder's
# own R16 comment records that *"`kayfabe-rm-ladder` runs as root, so every mapping above took
# `RmValidateMmapRequest`'s `osIsAdministrator()` fast path"*, and the isolate spawn takes
# `clone(CLONE_NEWUSER|CLONE_NEWPID|…)`. ⊘ Neither is a thing to discover from a red arm.
# ⚠ Reported, not enforced: refusing here would turn a comparable-but-unprivileged run into no
# run at all, and the uid is on the `SUITE_STARTED` line for anyone reading a result.
if [ "$(id -u)" -ne 0 ]; then
  echo "⚠ SUITE_UID_NOT_ROOT=1 — the host 30/30 reference ran as root; arms that map, spawn an"
  echo "  isolate or open a namespace may fail for a PRIVILEGE reason and say something else."
fi

# ⊘ The arm list is DERIVED from the source (flags whose handler sets a `want_* = true`), not
# hand-maintained. A hand list silently rots the moment someone adds an arm — which is the exact
# failure this file exists to end.
# ⊘ Overridable so a single arm can be given a CLEAN run.
if [ -n "${RMLADDER_ARMS:-}" ]; then ARMS="$RMLADDER_ARMS"; else
ARMS="--concurrency --timer --engines --doorbell-census --gpu-info-sweep --bus-info-sweep
--gpga-reserve-probe --atomics-probe --pce-mask-probe --dictated-ring --dictated-ring-negative
--late-map-race --blockage-coverage --uvm-invalidate --uvm-mean --alias-two-vas
--alias-unmap-observe --map-propagation --missing-page-fault --map-stress --rpc-mixed-allocs
--cross-client-leak --concurrent-fuzz --defer-liveness --guest-ram-pin --guest-ring-channel
--executor-vas --ce-client --ce-client-guest-ram --bar1-crossing"
fi

# ★★ Put the device back. Default = reload the guest's NVIDIA modules, which is the only lever a
# guest-side script has over `RmInitAdapter`'s accumulated state.
# ⊘ Stragglers are killed FIRST and by the bracket trick, on their own line: a `pkill -f` whose
# pattern appears later on the same command line matches the shell running it
# (`nvkvm-pv 2026-08-17`), and then everything after the kill silently never runs.
# ⊘ `RMLADDER_RECOVER=none` disables it, so a run can MEASURE the cascade rather than contain it
# — the control this file needs to claim containment at all.
recover() {
  if [ "${RMLADDER_RECOVER:-modprobe}" = "none" ]; then return 1; fi
  if [ -n "${RMLADDER_RECOVER_CMD:-}" ]; then
    $SUDO sh -c "$RMLADDER_RECOVER_CMD" >>"$OUT/recover.log" 2>&1
    return $?
  fi
  pkill -f '[k]ayfabe-rm-ladder' >/dev/null 2>&1
  pkill -f '[k]ayfabe-isolate'   >/dev/null 2>&1
  sleep 1
  {
    echo "--- recover $(date -u +%FT%TZ) ---"
    $SUDO modprobe -r nvidia_uvm nvidia_drm nvidia_modeset nvidia 2>&1
    echo "rmmod rc=$?"
    $SUDO modprobe nvidia_uvm 2>&1
    echo "modprobe rc=$?"
    # ★★★ AND RE-CREATE THE NODES. `[boot_capture.sh:50]` *"`modprobe` does not run
    # `RmInitAdapter`"* — it registers the PCI driver, and `/dev/nvidia0` does not exist until
    # something makes it. Without this the retry's `openat` fails with ENOENT and the log still
    # says `openat(nvidia<gpu>)`, so a recovery that worked would read as one that did not.
    # ⊘ `nvidia-modprobe -c 0 -u` and NOT `nvidia-smi`: it mknods and loads, it does not OPEN
    # the device, so the instrument does not spend the resource (`RmInitAdapter` cycles) whose
    # exhaustion is the thing being recovered from.
    $SUDO nvidia-modprobe -c 0 -u 2>&1
    echo "nvidia-modprobe rc=$?"
    ls -l /dev/nvidia* 2>&1
    $SUDO lsmod 2>/dev/null | grep -E '^nvidia' 2>&1
  } >>"$OUT/recover.log" 2>&1
  sleep 2
}
SUDO=""
[ "$(id -u)" -ne 0 ] && command -v sudo >/dev/null 2>&1 && SUDO="sudo -n"

# ⊘ `dmesg` at the MOMENT of the failure, because it cannot be taken later: `[measured w424]`
# the whole `ce_utils.c:304` chain existed only in the ring buffer of the failing open, and by
# the next boot the wedge is gone with the guest. `errno 5` carries none of it.
grab_dmesg() {
  $SUDO dmesg 2>/dev/null | grep -a NVRM | tail -25 > "$OUT/$1.nvrm" 2>/dev/null
}

# ★★★★★ THE OPEN-ORDINAL PROBE — runs INSTEAD of the suite (see the header).
if [ -n "${RMLADDER_OPEN_PROBE:-}" ]; then
  n=${RMLADDER_OPEN_PROBE}
  echo "=== OPEN-ORDINAL PROBE — $n consecutive device opens, nothing else ==="
  wall=0
  for i in $(seq 1 "$n"); do
    timeout 30 "$BIN" --gpu "$GPU" --timer > "$OUT/open$i.out" 2>&1
    rc=$?
    ok=$(grep -ac 'openat(nvidia' "$OUT/open$i.out" 2>/dev/null)
    printf 'OPEN %-3s rc=%-4s unopenable=%s\n' "$i" "$rc" "$ok"
    if [ "$ok" -gt 0 ] && [ "$wall" -eq 0 ]; then wall=$i; grab_dmesg "open$i"; fi
  done
  echo "OPEN_ORDINAL_WALL=$wall  (0 ⇒ no wall within $n opens)"
  echo "SUITE_RC=0 (probe only — no arms were run)"
  exit 0
fi

printf '=== RMLADDER SUITE — %s arms, gpu %s, %ss each, recover=%s ===\n' \
       "$(echo $ARMS | wc -w)" "$GPU" "$ARM_TIMEOUT" "${RMLADDER_RECOVER:-modprobe}"
printf '%-28s %-12s %s\n' "ARM" "VERDICT" "last line"
n_pass=0; n_fail=0; n_to=0; n_unmeas=0; n_rec=0; n_recok=0
failed=""; unmeasured=""

# One arm, one process. Echoes nothing; the caller reads `rc` and `$log`.
run_arm() {  # $1 = arm, $2 = log path
  timeout -k 5 "$ARM_TIMEOUT" "$BIN" --gpu "$GPU" "$1" > "$2" 2>&1
}
# Did this run even reach its subject? ⊘ `openat(nvidia` is R1 — before the arm's own code.
# ⚠ **THE EXIT CODE IS PART OF THE PREDICATE, and dropping it was a real bug in this rewrite's
# first draft.** An arm may legitimately PRINT that string while passing — `--cross-client-leak`
# opens a second client, and a rung that reports a *refusal* it expected quotes the same rung
# name. w717b's original had the `rc != 0` conjunct; it is not decoration.
# ⇒ `unopenable` means **failed AND never got past R1**, never "the string appears".
unopenable() { [ "${2:-1}" -ne 0 ] && [ "$(grep -ac 'openat(nvidia' "$1" 2>/dev/null)" -gt 0 ]; }

for arm in $ARMS; do
  name=${arm#--}
  log="$OUT/$name.out"
  run_arm "$arm" "$log"; rc=$?
  note=""

  # ★★★ CONTAINMENT. Two triggers, one action, and the RETRY is what turns a cascade back into
  # a measurement. ⊘ The retry runs at most once per arm: a second would make a flaky arm
  # indistinguishable from a recovered one.
  if unopenable "$log" "$rc" || [ $rc -eq 124 ]; then
    grab_dmesg "$name"
    if recover; then
      # ⊘ Counted only when recovery was ATTEMPTED — `RMLADDER_RECOVER=none` returns non-zero
      # without touching anything, and a counter that ticked for it would report a containment
      # that is switched off.
      n_rec=$((n_rec+1))
      if unopenable "$log" "$rc"; then
        run_arm "$arm" "$log.retry"; rc2=$?
        if unopenable "$log.retry" "$rc2"; then
          note=" (recovery did NOT reopen the device)"
        else
          n_recok=$((n_recok+1)); mv -f "$log.retry" "$log"; rc=$rc2; note=" (after recovery)"
        fi
      else
        # It TIMED OUT with the node openable — recovery is for the arms AFTER this one, and
        # this arm keeps its own verdict. ⊘ Whether it worked is unknown until the NEXT arm
        # opens the node, so this does not tick `SUITE_RECOVERED`.
        note=" (recovered for the next arm)"
      fi
    else
      note=" (no recovery attempted: RMLADDER_RECOVER=${RMLADDER_RECOVER:-modprobe})"
    fi
  fi

  last=$(grep -vE '^\s*$' "$log" 2>/dev/null | tail -1 | cut -c1-64)
  if unopenable "$log" "$rc"; then
    v=UNMEASURED; n_unmeas=$((n_unmeas+1)); unmeasured="$unmeasured $arm"
  else
  case $rc in
    0)   v=PASS;    n_pass=$((n_pass+1));;
    124) v=TIMEOUT; n_to=$((n_to+1));   failed="$failed $arm(timeout)";;
    *)   v="FAIL($rc)"; n_fail=$((n_fail+1)); failed="$failed $arm";;
  esac
  fi
  printf '%-28s %-12s %s%s\n' "$arm" "$v" "$last" "$note"
done

echo ""
# ⊘ There is no `SUITE_SKIP` any more. w717b's `SKIP/cascade` was the only thing that ever
# produced one, and it has become `UNMEASURED` — a name that does not read as benign. No arm
# currently declares a precondition of its own, so a `SKIP=0` field would be structurally zero
# and would report "nothing was skipped" from a counter nothing can increment.
echo "SUITE_ARMS=$(echo $ARMS | wc -w) SUITE_PASS=$n_pass SUITE_FAIL=$n_fail SUITE_TIMEOUT=$n_to SUITE_UNMEASURED=$n_unmeas"
# ⊘⊘ The two recovery numbers are printed TOGETHER and always. `attempts > succeeded` says the
# device does not come back from a guest-side driver reload — i.e. the leaked state is NOT in
# the guest's RM. That is a finding about kayfabe, and it is invisible if only one is printed.
echo "SUITE_RECOVERIES=$n_rec SUITE_RECOVERED=$n_recok  (attempts > recovered ⇒ the wedge SURVIVES a guest driver reload ⇒ the leaked state is ours, not the guest RM's)"
[ -n "$unmeasured" ] && echo "SUITE_UNMEASURED_ARMS (never reached their subject — the absence of a verdict, not a verdict):$unmeasured"
[ -n "$failed" ] && echo "SUITE_NOT_PASSING:$failed"
echo "SUITE_LOGDIR=$OUT"
# ⊘ Non-zero if anything did not pass, so a caller cannot record a green by ignoring the body.
# ⚠ UNMEASURED counts here. A suite that reported nothing for an arm has not licensed anything,
# and an exit code that forgave it would be the quietest possible way to buy a pass.
if [ $n_fail -gt 0 ] || [ $n_to -gt 0 ] || [ $n_unmeas -gt 0 ]; then echo "SUITE_RC=1"; exit 1; fi
echo "SUITE_RC=0"
