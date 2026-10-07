#!/usr/bin/env bash
# ★★★★★ w735 — RUN THE 30-ARM RAW-CLIENT SUITE IN THE GUEST, AND CONTAIN THE CASCADE.
#
#   usage: bash scripts/bench/w735_suite_run.sh [tag]      (run ON the bench box)
#
# ## Why this run exists
#
# `SINGLE_STORE_PLAN.md` §7's deletions — the fake framebuffer, the per-leaf join, the
# demand-fill mirror — are licensed by **the full 30-arm guest suite** and by nothing smaller
# (`THE_CONSTRAINTS.md` §w724g: *"the gate for unwiring is the GUEST suite"*). ⊘ Every
# *"raw client (P)"* this campaign has recorded is therefore a **narrower** check than the
# licence needs: one arm, not thirty.
#
# `[measured w734t]` the guest suite returned `PASS=2 FAIL=2 SKIP_CASCADE=26`. **26 of 30 arms
# reported nothing**, because one arm left the device unopenable and every arm after it died at
# `R1 openat(nvidia0)`. A suite that cannot report on 26 of its own arms licenses nothing.
#
# ## ⊘ PRE-REGISTERED — written before the boot, so no number reads as the good one
#
#   Q1 DOES THE SUITE REPORT ON ITSELF AT ALL?  `SUITE_ARMS=30` and
#      `PASS+FAIL+TIMEOUT+UNMEASURED == 30`, with a `SUITE_RC=` terminator.
#      ⊘ `SUITE_STARTED` with no `SUITE_RC=` is the suite DYING, which is a third state and
#      not "still running" (`[measured 2026-08-10]`, the zero-byte-output trap).
#
#   Q2 ⊘⊘ **IS THE CASCADE CONTAINED?**  `SUITE_UNMEASURED` is the number to read, and the
#      gate is **`SUITE_UNMEASURED == 0`** — every arm reached its own subject. ⚠ It is NOT
#      `SUITE_FAIL == 0`: a suite of 30 real verdicts with failures in it is the deliverable;
#      a green suite with 26 arms that never ran is not.
#      ★ `SUITE_RECOVERIES` vs `SUITE_RECOVERED` says WHERE the wedged state lives:
#        equal        ⇒ a guest-side driver reload puts the device back ⇒ the leak is in the
#                       guest's RM and the suite can run to completion around it;
#        recovered=0  ⇒ the wedge SURVIVES `modprobe -r nvidia` ⇒ the state is OURS (the
#                       emulated device / the host isolates), which is a product defect and is
#                       the more interesting answer.
#
#   Q3 THE THREE KNOWN FAILURES, and whether they are still the same three.
#      `[measured w734t]` `--concurrency` and `--engines` FAIL at `R10 isolate`; a
#      `--gpu-info-sweep` TIMEOUT was reported in an earlier session and did NOT reproduce in
#      w734t. ⚠ If the list has changed, **that is the finding** — the tree has moved a great
#      deal since, and re-fixing a list nobody re-measured is how a campaign chases a ghost.
#      ★ The R10 line now carries `kind=` and the spawn's own sentence, so "the isolate did not
#      start" stops being a guess about RM.
#
#   Q4 THE UID. `[measured w734t]` the hook ran the ladder as `ubuntu` while the 30/30 host
#      reference ran as root — so the delta it printed was not host-vs-guest. This run is root
#      on both sides. `SUITE_UID_NOT_ROOT=1` in the output means it is not, and every arm below
#      it is a different experiment.
#
# ## Traps encoded inline
# - ★★ `pgrep -x qemu-system-x86_64` can NEVER match (/proc/PID/comm truncates at 15).
# - ★★ the kill goes on a line of ITS OWN (nvkvm-pv 2026-08-17).
# - ★ `grep -c` on its own line, never piped into `grep -q` (SIGPIPE + pipefail = 141).
set -uo pipefail
SRC_DIR="$(cd "$(dirname "$0")" && pwd)"
REPO=${KAYFABE_REPO:-/root/kayfabe}
BENCH=${BENCH_DIR:-/workspace/bench}
TAG=${1:-w735a}
cd "$REPO" || { echo "⊘ no repo at $REPO"; exit 2; }

echo "=== W735 SUITE RUN $(date -Is) tag=$TAG ==="
echo "TREE_REV=$(git rev-parse HEAD)"

pkill -f '[q]emu-system-x86'
sleep 3

# ★★★ BUILD, HERE, because `single_store_e6_boot.sh` REFUSES a binary whose embedded
# `kayfabe-rev:` differs from the tree's HEAD — and rightly: `[measured]` the bench silently
# served a binary built from `862c7c2` for weeks and results attributed to HEAD were not HEAD's.
# ⊘ `cuda-scratchpad` is on because e6 exports `KAYFABE_SCRATCHPAD_CUDA=on` unconditionally and
# `[measured w734t]` — the run this one is compared against — had it. The feature costs nothing
# to build: `kayfabe-cuda` `dlopen`s libcuda at RUN time and links nothing.
# ⊘ `W735_SKIP_BUILD=1` for the 2nd..Nth boot of a BATCHED run: the tree has not changed between
# them, and `single_store_e6_boot.sh` still refuses a binary whose rev is not HEAD's, so the
# guard that matters is not skipped — only the ~1 min relink is.
export KAYFABE_SHIM_FEATURES="${KAYFABE_SHIM_FEATURES:-host-isolates cuda-scratchpad}"
if [ "${W735_SKIP_BUILD:-0}" = "1" ]; then
  echo "== W735_SKIP_BUILD=1 — reusing the binaries from the previous boot of this run"
else
QEMU_SRC=${KAYFABE_QEMU_SRC:-$BENCH/qemu-10.2.4}
QEMU_BUILD=${KAYFABE_QEMU_BUILD:-$BENCH/qemu-build}
echo "== building the shim: features=$KAYFABE_SHIM_FEATURES src=$QEMU_SRC"
bash scripts/build_qom_shim.sh "$QEMU_SRC" "$QEMU_BUILD" > "$BENCH/w735_build.log" 2>&1
rc=$?
echo "SHIM_RC=$rc"
# ⊘ The status is captured on its own line and BRANCHED ON, never piped: `a pipe eats the exit
# status` and the boot would then proceed on the PREVIOUS binary.
if [ "$rc" -ne 0 ]; then echo "⊘ BUILD FAILED:"; tail -40 "$BENCH/w735_build.log"; exit 3; fi

# ★ And the GUEST-side ladder, which is a separate musl binary and is what the suite actually
# runs. ⊘ A stale one here is invisible: the hook finds *a* binary, pushes it, and every arm
# reports on a tree nobody is testing.
( . "$HOME/.cargo/env" 2>/dev/null; cd "$REPO"   && cargo build --release --target x86_64-unknown-linux-musl --bin kayfabe-rm-ladder )   > "$BENCH/w735_ladder_build.log" 2>&1
lrc=$?
echo "LADDER_RC=$lrc"
if [ "$lrc" -ne 0 ]; then echo "⊘ LADDER BUILD FAILED:"; tail -30 "$BENCH/w735_ladder_build.log"; exit 3; fi
L="$REPO/target/x86_64-unknown-linux-musl/release/kayfabe-rm-ladder"
# ⊘ CONTENT, not a stamp: this run's whole R10 change is a string, so grep for it. A binary
# without it would report the OLD sentence and the reader would attribute the guess to RM again.
n_r10=$(strings "$L" 2>/dev/null | grep -c 'it did not start: kind=')
echo "LADDER-CONTENT: r10_refusal_strings=$n_r10 (0 ⇒ the ladder predates w735 — STOP)"
if [ "$n_r10" -eq 0 ]; then echo "⊘ stale guest ladder"; exit 4; fi
fi

export POST_CAPTURE_HOOK="$SRC_DIR/rmladder_suite_hook.sh"
# ⊘ 90 s is `[measured w734t]`'s value, kept UNCHANGED on purpose. Shortening it would make a
# legitimately slow arm (`--concurrent-fuzz`, `--map-stress`) time out for a reason that is not
# the one under test, and a run that changes two things at once cannot attribute either.
export RMLADDER_ARM_TIMEOUT=${RMLADDER_ARM_TIMEOUT:-90}
export RMLADDER_RECOVER=${RMLADDER_RECOVER:-modprobe}
export RMLADDER_SUDO=${RMLADDER_SUDO:-1}
# ⊘ SHADOW=off: this run is about the guest's RM plane, and the walk shadow needs the CUDA
# scratchpad image. Arming a variable a run does not measure is how two boots come to differ in
# something neither log names.
PREFIX="$TAG" SHADOW=off bash "$SRC_DIR/single_store_e6_boot.sh" 2>&1 | tee "$BENCH/w735_run.log"

S="$BENCH/run_${TAG}_suite.out"
D="$BENCH/run_${TAG}_probe.log"

echo
echo "=== ★★★★★ W735 — THE LEDGER ==="
echo "--- Q1 DID THE SUITE REPORT ON ITSELF? ---"
n_start=$(grep -ac 'SUITE_STARTED=' "$S" 2>/dev/null)
n_end=$(grep -ac 'SUITE_RC=' "$S" 2>/dev/null)
echo "W735-STARTED=${n_start:-0} W735-TERMINATED=${n_end:-0}  (1/0 ⇒ the SUITE died mid-run)"
grep -a 'SUITE_ARMS=' "$S" 2>/dev/null | tail -1 | cut -c1-200

echo "--- Q2 IS THE CASCADE CONTAINED? (the gate is UNMEASURED=0) ---"
grep -ao 'SUITE_UNMEASURED=[0-9]*' "$S" 2>/dev/null | tail -1
grep -a 'SUITE_RECOVERIES=' "$S" 2>/dev/null | tail -1 | cut -c1-240
grep -a 'SUITE_UNMEASURED_ARMS' "$S" 2>/dev/null | tail -1 | cut -c1-300

echo "--- Q3 THE FAILING ARMS, and the R10 sentence that used to be a guess ---"
grep -a 'SUITE_NOT_PASSING' "$S" 2>/dev/null | tail -1 | cut -c1-300
grep -a 'R10 isolate' "$S" 2>/dev/null | head -4 | cut -c1-220

echo "--- Q4 THE UID ---"
grep -a 'SUITE_STARTED=' "$S" 2>/dev/null | tail -1 | cut -c1-200
echo "W735-NOT-ROOT=$(grep -ac 'SUITE_UID_NOT_ROOT=1' "$S" 2>/dev/null)  (1 ⇒ NOT the host reference's experiment)"

echo "--- ★ every arm's row, verbatim ---"
grep -aE '^\s*--[a-z]' "$S" 2>/dev/null | cut -c1-160

echo "--- ★ host Xid, if any ---"
grep -a 'HOST_DMESG_XID' "$BENCH/w735_run.log" 2>/dev/null | tail -2 | cut -c1-200
echo "=== W735 END $(date -Is) ==="
