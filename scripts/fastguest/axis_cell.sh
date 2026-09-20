#!/usr/bin/env bash
# ★★★★★ ONE CELL OF THE AXIS MATRIX — the raw client, bare metal, on whatever GPU this box has.
#
# > `[owner, 2026-09-21]` *"it's also useful if the raw client 30/30 can be run on other GPUs and
# > drivers later, as it's a test for when iterating kayfabe itself on the axis matrix."*
#
# usage (on a rented box, after the tree is cloned and rust is installed):
#     bash scripts/fastguest/axis_cell.sh            # build + run + emit the cell line
#
# ⊘ **NO KVM NEEDED.** This lane never boots a guest, so a plain CUDA container is enough — which
# is the whole economic point: `[owner]` *"a kvm box can be used for both kvm gpu, non kvm gpu,
# cpu. a container only for non kvm gpu and cpu."* Containers are cheap and plentiful across GPU
# types; KVM-capable boxes are neither. ⇒ Die coverage scales on THIS lane, not the guest lane.
#
# ⚠ It measures the `(die x host driver)` cell ONLY. It says nothing about kayfabe's guest path —
# that is deliberate, and is what makes a red arm here attributable to the die.
set -uo pipefail
REPO=${KAYFABE_REPO:-/root/kayfabe}
BRANCH=${KF_BRANCH:-w749-fable-legb}

# ⊘ PULL FIRST. `[measured w822]` a runner that built without fetching re-ran a stale harness and
# reproduced the exact output the pushed fix was meant to remove — which reads as "the fix failed"
# rather than "the fix was never here". The revision is part of the result.
cd "$REPO" || { echo "axis_cell: ⊘ no repo at $REPO"; exit 2; }
git fetch -q origin && git reset --hard -q "origin/$BRANCH" || { echo "axis_cell: ⊘ fetch failed"; exit 2; }
echo "axis_cell: rev=$(git rev-parse --short HEAD)"

. "$HOME/.cargo/env" 2>/dev/null || true
cargo build --release -p kayfabe-rm-ladder --bin kayfabe-rm-ladder 2>&1 | tail -2 \
  || { echo "axis_cell: ⊘ build failed — a BUILD fault, not a cell result"; exit 2; }

export BENCH_DIR=${BENCH_DIR:-/workspace/bench}
export KF_LADDER=${KF_LADDER:-$REPO/target/release/kayfabe-rm-ladder}
bash scripts/fastguest/bare_metal_suite.sh axis "${BUDGET:-120}"
