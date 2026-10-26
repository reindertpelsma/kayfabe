#!/bin/bash
# kvm_tests.sh <prov-dir> <tag> — the OPT-IN with-KVM unit tests of the merge bar. Run from the checkout.
#   KF_MERGE_KVM_TESTS=1 bash scripts/bench/box/merge_check.sh <branch> <tag>
# [2026-10-26] The merge bar no longer repeats the GPU-free `cargo test` of every kf-* crate: CI's
# `stable` job runs it on every commit, and no kf-* test opens /dev/nvidia*, a kf3 binary or QEMU
# (live gate run 40263112941: that repeat cost 31 minutes and ended rc=101 on a box). What CI cannot
# run is the /dev/kvm-gated tests (KAYFABE_NO_KVM=1 there), which live in kf-linux-raw and kf-chan.
# This stage runs exactly those two crates and proves the gate was REACHED, not skipped:
#   KVM_TESTS_REQUESTED=1                                          always, first
#   KVM_TESTS_RC=<rc>                                              only when cargo exited non-zero
#   KVM_TESTS passed <n> failed <n> kvm_ran=<n> kvm_skipped=<n>    totals; exit 1 unless failed=0,
#                                                                  passed>0, kvm_ran>0, kvm_skipped=0
# The cargo log is kept as <prov-dir>/<tag>_tests.log; the payload reads failed test names from it.
set -uo pipefail
PROV=${1:?prov dir}; T=${2:?tag}
log="$PROV/${T}_tests.log"
echo "KVM_TESTS_REQUESTED=1"
cargo test -q --no-fail-fast -p kf-linux-raw -p kf-chan >"$log" 2>&1
rc=$?
[ "$rc" -eq 0 ] || echo "KVM_TESTS_RC=$rc"
ran=$(grep -c 'KVM-GATE: RAN' "$log" || true)
skipped=$(grep -c 'KVM-GATE: SKIPPED' "$log" || true)
summary=$(awk -v ran="$ran" -v skipped="$skipped" \
    '/^test result:/{s+=$4; f+=$6} END{print "KVM_TESTS passed", s+0, "failed", f+0, "kvm_ran=" ran, "kvm_skipped=" skipped}' "$log")
echo "$summary"
[ "$rc" -eq 0 ] && [[ "$summary" =~ ^KVM_TESTS\ passed\ [1-9][0-9]*\ failed\ 0\ kvm_ran=[1-9][0-9]*\ kvm_skipped=0$ ]]
