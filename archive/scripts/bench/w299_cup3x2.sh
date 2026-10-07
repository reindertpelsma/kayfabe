#!/usr/bin/env bash
# ★★★★★ w299 — TWO CONCURRENT CUDA PROCESSES ON THE COMPUTE PLANE.
#
#   $1 = concurrent | staggered   (⊘ REQUIRED — never defaulted. A defaulted arm makes an
#                                  evidence run and its own control indistinguishable at the
#                                  call site, which is the shape w290p already names.)
#
# `cup3` crossed at `^CUP3_VAL=43` (w297, master `91f8b34b`) — FIRST COMPUTE, ONE process, ONE
# context. This rung asks the next question and no other: **does it survive a second concurrent
# process?** That is the `#14` shape from the C era (two CUDA apps hang at `cuCtxCreate`),
# explicitly deferred to this Rust rewrite, never tested at the compute plane.
#
# ## ⊘ THE ARMING DOES NOT MOVE — byte for byte w297's, which is byte for byte w294's
#
# `w290p_run.sh drain` supplies all eleven. Nothing is added, nothing relaxed further. The ONLY
# variables are the PROCESS COUNT and the START OFFSET. Changing arming and process count
# together would make any outcome unattributable.
#
# ## ★★★ PRE-REGISTERED, BEFORE THE BOOT — every outcome, so none reads as the favourable one
#
#   (A) BOTH `43` ⇒ multi-process compute works FOR THIS SHAPE. ⊘ Two identical short
#       workloads is ONE SHAPE — it is NOT a multi-tenancy claim. Report every relaxation.
#   (B) one `43`, one hang/timeout ⇒ name WHICH, and name THE WAIT. ⊘ This must NOT read as
#       "mostly working".
#   (C) BOTH hang ⇒ the second process broke the first. STRONGER than (B), not weaker.
#   (D) the second process fails at ALLOCATION / `cuCtxCreate` rather than at compute ⇒ the
#       C's `#14` IS REPRODUCING. Say so and name the refusal.
#   (E) `SystemDataPlane` or another named refusal blocks the setup ⇒ report and STOP. It is
#       an open owner ruling, not this rung's to route around.
#   (F) cup3 does not pass single-process on this box ⇒ THE BOX, not the code. Run
#       `w297_cup3.sh` first; if it does not print 43, everything after it is unattributable.
#
# ★★ (B), (C) and (D) are ENTIRELY HONEST OUTCOMES and this is the first look.
#    ⊘ DO NOT ITERATE TOWARD A GREEN.
#
# ## ★★★★★ THE BQL SHARPENING — what the beacon is for
#
# The doorbell path runs UNDER THE QEMU BQL (`crates/kayfabe-qemu-raw/src/shim.rs:4877`,
# `:6146`, `:6046`), and the kernel-CE completion runs synchronously inline off the doorbell
# (`kayfabe-abi/src/eventnotify.rs:191-193`). ⇒ blocking there stalls **every vCPU and QEMU's
# main loop**, not just the ringing vCPU. So the predicted symptom is NOT "B is slow while A
# runs" — it is **both stopping together and the guest freezing**. `cup3x2_hook.sh` therefore
# runs a GPU-free beacon in the guest; a GAP in it is a GLOBAL freeze (BQL), a TICKING beacon
# beside a stalled process is a PER-PROCESS wait. ⊘ A bare timeout cannot separate these.
set -uo pipefail
MODE="${1:-}"
case "$MODE" in
  concurrent|staggered) ;;
  # ⊘ `solo` is NOT an arm of the question — it is the BEACON CONTROL. The concurrent arm
  #   measured a 1.221 s beacon gap, and a gap has no meaning without a one-process baseline
  #   taken through the SAME instrument. Same hook, same arming, ONE cup3.
  solo) ;;
  *) echo "usage: $0 concurrent|staggered|solo" >&2; exit 64 ;;
esac

REPO=${KAYFABE_REPO:-/workspace/kayfabe_w299}
export KAYFABE_REPO="$REPO"
export CARGO_TARGET_DIR=${CARGO_TARGET_DIR:-/workspace/bench/cargo-target-w299}
export KAYFABE_TAG=${KAYFABE_TAG:-w299$MODE}
export POST_CAPTURE_HOOK="$REPO/scripts/bench/cup3x2_hook.sh"
export KAYFABE_CUP3X2_MODE="$MODE"
# Two processes, each bounded at 300 s, plus the beacon analysis and two ladders.
export GQ_TIMEOUT=${GQ_TIMEOUT:-900}

rm -f /workspace/bench/qemu-build/qemu-system-x86_64

"$REPO/scripts/bench/w290p_run.sh" drain
BRC=$?

OUT=/workspace/${KAYFABE_TAG}.log
Q=/workspace/bench/run_${KAYFABE_TAG}_qemu.log
P=/workspace/bench/run_${KAYFABE_TAG}_probe.log
D=/workspace/bench/run_${KAYFABE_TAG}_hostdmesg.log

# ⊘⊘ GRADING LIVES IN `w299_grade.sh`, NOT HERE — and that split was PAID FOR.
#
# The first concurrent run graded inline and **printed `CUP3A_VAL=43` and no `CUP3B_VAL` line
# at all**, while the probe log held `CUP3B_VAL=43` and the verdict logic read it correctly.
# Cause: an apostrophe inside a `${VAR:-default}` opens a single-quoted region in bash which
# ran to the apostrophe in the NEXT line's default, swallowing that whole `echo`. `bash -n`
# accepts it; `dash` prints both lines. ⇒ the defect was in the text of the
# "THE MEASUREMENT DID NOT HAPPEN" fallbacks, so a genuinely MISSING value would have printed
# NOTHING — not even the warning written to catch it. See `w299_grade.sh` for the full note.
#
# ⇒ A standalone grader also means a boot can be RE-GRADED from its logs without re-booting,
#   which is what let the defective run be recovered instead of repeated.
"$REPO/scripts/bench/w299_grade.sh" "$KAYFABE_TAG" >>"$OUT" 2>&1

exit "$BRC"
