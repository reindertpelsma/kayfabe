#!/bin/bash
# the control binary: this checkout (tree2, PR 480 merged) with ONLY crates/kf-mem/src/apply.rs taken from the tree before the merge (343a39061); incremental cargo build,
# installed by build_kf3.sh as kf3-bins/<rev>-dirty. The fixed apply.rs is put back afterwards (the working tree must be the tested revision again).
set -e
export PATH=$HOME/.cargo/bin:$PATH
cd /workspace/kf-ci-src
curl -fsSL https://raw.githubusercontent.com/reindertpelsma/kayfabe/343a3906175f95872f00d346d33df4782dddf3cc/crates/kf-mem/src/apply.rs -o /workspace/apply_unfixed.rs
grep -c "RAM holds no Z surface" /workspace/apply_unfixed.rs
cp crates/kf-mem/src/apply.rs /workspace/apply_fixed.rs
cp /workspace/apply_unfixed.rs crates/kf-mem/src/apply.rs
bash scripts/bench/build_kf3.sh /workspace/bench/qemu-10.2.4 /workspace/bench/qemu-build-kf3 || rc=$?
cp /workspace/apply_fixed.rs crates/kf-mem/src/apply.rs
git status --short | head -3
ls /workspace/bench/kf3-bins/
exit ${rc:-0}
