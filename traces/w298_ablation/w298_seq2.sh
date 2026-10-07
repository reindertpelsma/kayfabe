#!/usr/bin/env bash
# w298 seq2 — the three arms the STALENESS GUARD ate, plus the decisive combined arm.
#
# ⊘ WHY THE FIRST THREE ARE RE-RUNS AND NOT RESULTS: `build_qom_shim.sh` refuses to install an
#   archive older than 30 minutes ("cargo did not rebuild it and this script will not install
#   an archive it did not just produce"). No Rust source changes between arms, so cargo did
#   nothing (`Finished in 0.08s`) and after ~33 min of boots the artefact aged past the guard.
#   ⇒ arms 10-12 exited rc=92 at the BUILD, never booted, and are UNMEASURED — not failures.
#   ★ The guard is right; the harness was wrong to assume one build could serve twelve boots.
#   Fix: touch the crate root before each arm so cargo genuinely reproduces the archive.
set -uo pipefail
cd /workspace/kayfabe_w298 || exit 90
S=/workspace/w298_summary.txt
run() {
  local label=$1; shift
  echo "############ w298 ARM $label [$*] $(date -Is) ############" | tee -a "$S"
  pkill -x qemu-system-x86 2>/dev/null && sleep 5
  # ★ mtime only — the tree stays git-clean, so the stamp gate is unaffected.
  touch crates/kayfabe-qemu-raw/src/lib.rs crates/kayfabe-qemu-raw/src/shim.rs
  bash scripts/bench/w298_ablate.sh "$label" "$@" >"/workspace/w298_$label.out" 2>&1
  echo "############ w298 ARM $label DONE rc=$? $(date -Is) ############" | tee -a "$S"
}
run isolates  KAYFABE_ISOLATES=stillborn
run ceexec    KAYFABE_CE_EXECUTOR=local
run ptwitness KAYFABE_PT_WITNESS_EXEC=off
# ★★★★★ THE DECISIVE COMBINED ARM. `ptsweep=off` PASSED at 43 while publishing nothing, and
#   `vaspub=assert` FAILED with the sweep ON. Those two facts together say the sweep may be
#   CREATING the population that the publication then has to carry. Two variables, deliberately,
#   because the question is exactly about their INTERACTION and neither single arm can answer it.
W298_MULTI=1 run bothoff KAYFABE_PT_SWEEP=off KAYFABE_VAS_PUBLISH=assert
echo "############ w298 SEQ2 COMPLETE $(date -Is) ############" | tee -a "$S"
