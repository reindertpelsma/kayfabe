#!/bin/bash
# L1: build e1e9947cc's QEMU (build_kf3.sh -> kf3-bins/<rev>) and run the GPU-free tests on the host.
# usage: l1_build.sh <worktree> <outdir>      (start marker + exit line; CPU only, no GPU job)
set -uo pipefail
W=${1:?worktree}; O=${2:?outdir}; mkdir -p "$O"
export PATH=$HOME/.cargo/bin:$PATH CARGO_TARGET_DIR=/data/claude-life/target CARGO_BUILD_JOBS=${JOBS:-12}
cd "$W"
echo "L1_START rev=$(git rev-parse HEAD) dirty=$(git status --porcelain --untracked-files=no | wc -l) $(date -Is)" | tee "$O/l1_summary.txt"
nice -n 10 bash scripts/bench/build_kf3.sh /var/lib/kf-windows-20261005/qemu-10.2.4 /var/lib/kf-windows-20261005/qemu-build-kf3 > "$O/build_kf3.log" 2>&1
echo "BUILD_KF3 rc=$? $(grep -a '^KF3_BUILT' "$O/build_kf3.log")" | tee -a "$O/l1_summary.txt"
nice -n 10 cargo test --release --no-fail-fast -p kf-chan -p kf-rm -p kf-trap -p kf-qemu > "$O/cargo_test.log" 2>&1
rc=$?
passed=$(grep -a '^test result:' "$O/cargo_test.log" | sed 's/.* \([0-9]*\) passed.*/\1/' | awk '{s+=$1} END{print s+0}')
failed=$(grep -a '^test result:' "$O/cargo_test.log" | sed 's/.* \([0-9]*\) failed.*/\1/' | awk '{s+=$1} END{print s+0}')
ign=$(grep -a '^test result:' "$O/cargo_test.log" | sed 's/.* \([0-9]*\) ignored.*/\1/' | awk '{s+=$1} END{print s+0}')
echo "CARGO_TEST rc=$rc passed=$passed failed=$failed ignored=$ign result_lines=$(grep -ac '^test result:' "$O/cargo_test.log")" | tee -a "$O/l1_summary.txt"
grep -a '^test .* FAILED\|panicked at' "$O/cargo_test.log" | head -20 | tee -a "$O/l1_summary.txt"
echo "L1_EXIT dirty_after=$(git status --porcelain --untracked-files=no | wc -l) $(date -Is)" | tee -a "$O/l1_summary.txt"
