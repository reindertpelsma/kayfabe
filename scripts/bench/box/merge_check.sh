#!/bin/bash
# merge_check.sh — the merge bar for promoting a revision to master/v3, run ON a provisioned box.
#   bash merge_check.sh <branch> <tag>        (log: /root/prov/<tag>.log, ends with an EXIT line)
# Bar: every kf-* crate test (--no-fail-fast: a red crate must not hide later crates — it once
# reported "483 tests" instead of ~1500), v3 gates 9/9, a kf3 build of THIS revision, the raw client
# + fast guest rebuilt, and the 30-arm thin-guest suite at budget 180 → 30/30.
# ⊘ Never hold /tmp/kayfabe-fastguest.lock across fast_suite: run_fast_guest takes it per arm.
# ⊘ FG_RC=1 with "nbd0p1 never appeared" is a host nbd flake; the suite then runs the previous
#   initrd — say so in the promotion message (acceptable only if the raw client crates did not change).
B=${1:?branch}; T=${2:?tag}
exec > /root/prov/$T.log 2>&1
echo "START $(date -Is)"
export PATH=$HOME/.cargo/bin:$PATH BENCH=/workspace/bench BENCH_DIR=/workspace/bench KF_DEVICE=kf3
rm -rf /root/kf-mc; git -C /root/kayfabe worktree prune
git -C /root/kayfabe fetch -q origin "$B" && git -C /root/kayfabe worktree add -f /root/kf-mc "origin/$B" -q
cd /root/kf-mc && git log --oneline -1
pk=$(ls crates | grep '^kf-' | sed 's/^/-p /' | tr '\n' ' ')
cargo test -q --no-fail-fast $pk 2>&1 | grep -E "^test result|FAILED|panicked|^error" | awk '/test result/{s+=$4; f+=$6} /FAILED|panicked|^error/{print} END{print "TESTS passed",s,"failed",f}'
bash scripts/bench/v3_gates.sh > /root/prov/${T}_gates.log 2>&1; echo "GATES_RC=$?"; grep -a "SUMMARY\|_VERDICT=FAIL" /root/prov/${T}_gates.log
bash scripts/bench/build_kf3.sh /workspace/bench/qemu-10.2.4 /workspace/bench/qemu-build-kf3 > /root/prov/${T}_kf3.log 2>&1; echo "KF3_RC=$?"; tail -1 /root/prov/${T}_kf3.log
cargo build -q --release -p kayfabe-rm-ladder --bin kayfabe-rm-ladder 2>/dev/null
bash scripts/fastguest/build_fast_guest.sh /workspace/bench/guest.qcow2 /workspace/bench/fastguest > /root/prov/${T}_fg.log 2>&1; echo "FG_RC=$?"
bash scripts/fastguest/fast_suite.sh "$T" 180 > /root/prov/${T}_suite.run 2>&1; echo "SUITE_RC=$?"
grep -E "FAST_SUITE_PASS|FAIL |TIMEOUT|VMM_DIED" /workspace/bench/${T}_suite.out
echo "EXIT $(date -Is)"
