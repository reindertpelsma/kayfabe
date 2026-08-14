#!/usr/bin/env bash
# ★★★★★ w317 DIAGNOSTIC — **IS THE 92 ms CHUNK 64 UNIFORMLY-SLOW DISPOSALS, OR ONE SLOW ONE?**
#
# The two have OPPOSITE fixes and the shipped instrument cannot tell them apart:
#   `DRAIN-TIMING disposed=64 turns=1 max_drain_us=91833` fits BOTH
#     (M1) 64 × ~1.4 ms          ⇒ a smaller chunk cures it, proportionally
#     (M2) 1 × ~80 ms + 63 × fast ⇒ NO chunk cures it; one disposal is indivisible
#
# ⊘ Retuning `RETIRED_DRAIN_CHUNK` on data that fits both models would be FITTING, not measuring.
#
# ★ THE DISCRIMINATOR, and it needs no new instrument: build with `RETIRED_DRAIN_CHUNK = 1`, so
#   one turn == one disposal and the deadline is re-read after EVERY disposal.
#     M1 ⇒ max_drain_us ≈ 41 000 (the 40 ms budget + one ~1.4 ms disposal), turns ≈ 29
#     M2 ⇒ max_drain_us ≈ the single slow disposal (tens of ms), turns small
#   ⚠ PRE-REGISTERED above, before the boot, so neither reads as the favourable one.
#
# ⊘ THROWAWAY. It commits on a scratch branch so the stamp gate sees a clean tree, and resets
#   back afterwards. Nothing here is shipped.
set -uo pipefail
TREE=${1:-/workspace/kf-w317}
TAG=${2:-w317c1diag}
LOG=/workspace/w317_chunk1_diag.log
export PATH=/root/.cargo/bin:$PATH
{
echo "=== W317 CHUNK1 DIAG START $(date -Is) tree=$TREE tag=$TAG"
cd "$TREE" || exit 90
BASE=$(git rev-parse HEAD); echo "=== base HEAD=$BASE"
git checkout -q -B w317-chunk1-diag "$BASE" || exit 91
sed -i 's/^pub const RETIRED_DRAIN_CHUNK: usize = 64;/pub const RETIRED_DRAIN_CHUNK: usize = 1;/' crates/kayfabe-qemu-raw/src/shim.rs
grep -n 'RETIRED_DRAIN_CHUNK: usize =' crates/kayfabe-qemu-raw/src/shim.rs
git -c user.email=bench@vh -c user.name=bench commit -qam 'DIAG ONLY: chunk=1 to separate M1 from M2' || exit 92
echo "=== diag HEAD=$(git rev-parse HEAD)"
export KAYFABE_REPO=$TREE
export CARGO_TARGET_DIR=/workspace/bench/cargo-target-$(basename "$TREE")
export KAYFABE_TAG=$TAG
pkill -x qemu-system-x86 2>/dev/null; sleep 4
bash "$TREE/scripts/bench/w297_cup3.sh" >/dev/null 2>&1
Q=/workspace/bench/run_${TAG}_qemu.log
P=/workspace/bench/run_${TAG}_probe.log
echo "=== ★ THE ANSWER ==="
echo "    CUP3    = [$(grep -aoE '^CUP3_VAL=[A-Za-z0-9_]+' "$P" 2>/dev/null | tail -1)] [$(grep -aoE '^CUP3_RC=[0-9]+' "$P" 2>/dev/null|tail -1)]"
echo "    EVERY DRAIN-TIMING line (each a new max):"
grep -ao 'DRAIN-TIMING max_drain_us=[0-9]* disposed=[0-9]* residue=[0-9]* turns=[0-9]* budget_hit=[a-z]*' "$Q" 2>/dev/null | sed 's/^/      /' || echo "      ⊘ NONE — UNMEASURED"
echo "    max_reap_us = [$(grep -ao 'max_reap_us=[0-9]*' "$Q" 2>/dev/null | grep -oE '[0-9]+$' | sort -n | tail -1)]"
echo "    DRAIN-DEFER: $(grep -ao 'DRAIN-DEFER deferred_for_drain=[0-9]* still_retired=[0-9]*' "$Q" 2>/dev/null | tr '\n' ' | ')"
cd "$TREE" && git checkout -q w317-budgeted-drain && git branch -qD w317-chunk1-diag
echo "=== restored HEAD=$(git rev-parse HEAD)  dirt=[$(git status --porcelain --untracked-files=no|head -2)]"
echo "=== W317 CHUNK1 DIAG EXIT status=0 $(date -Is)"
} >"$LOG" 2>&1
echo "=== W317 CHUNK1 DIAG TERMINATOR rc=0 $(date -Is)" >>"$LOG"
