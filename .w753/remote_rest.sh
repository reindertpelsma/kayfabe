#!/usr/bin/env bash
# w753 — everything after the driver swap, in one detached run.
# ⚠ Traps encoded inline: start marker + TERMINATOR (an empty file is a STATE, not "not yet").
set -uo pipefail
export PATH="$HOME/.cargo/bin:$PATH"
REPO=/root/kayfabe
BR=w753-route-k-phase2

say(){ echo "=== $* $(date -Is)"; }

say "STEP 0 — put the tree on the branch under test"
cd "$REPO" || exit 1
git fetch -q origin "$BR" || { echo "FETCH_FAILED"; exit 1; }
git checkout -q -B "$BR" "origin/$BR" || { echo "CHECKOUT_FAILED"; exit 1; }
echo "TREE_REV=$(git rev-parse HEAD)"
echo "TREE_BRANCH=$(git rev-parse --abbrev-ref HEAD)"
# ★ NON-VACUITY: the branch must actually contain route K, or the boot grades master.
grep -rq "mod birth_conn" crates/kayfabe-isolate-host/src/rm.rs \
  || { echo "BRANCH_HAS_NO_ROUTE_K — the checkout did not land"; exit 1; }
grep -q 'Some("k") => Ok(VasOwner::BirthClient)' crates/kayfabe-qemu-raw/src/scratchpad.rs \
  || { echo "BRANCH_HAS_NO_K_ARM"; exit 1; }
echo "BRANCH_CONTENT_GATE=OK"

say "STEP 1 — bench tree (QEMU src+build, guest image, tap, guest client)"
bash scripts/bench/provision_bench_tree.sh 2>&1 | tail -40
echo "BENCH_TREE_RC=$?"

say "STEP 2 — the three arms, arm 3 = route K"
ARM3_VAS_OWNER=k bash scripts/bench/w736_fbstore_run.sh w753 2>&1
echo "RUN_RC=$?"
