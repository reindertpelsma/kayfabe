#!/usr/bin/env bash
# ★★★★★ w322 — THE SERIAL BATCH. ONE launcher, because the GPU is one GPU.
#
# ⊘ w319 recorded two detached batches `pkill`ing each other's QEMU and emitting a full ladder
#   of `UNMEASURED` in 50 s. Everything that needs the GPU goes in HERE, in order, and the
#   order is MOST-DECISIVE-FIRST so a deadline costs the least important arm:
#
#   1. bwneg   — the KNOWN-POSITIVE. Without it every `bad=0` above is vacuous, so it is
#                first: an arm whose absence invalidates the others cannot be the one that
#                gets cut.
#   2. bw      — a SECOND boot of the headline measurement. n=1 was all the first pass had.
#   3. bwhost  — the guest's own `cuMemHostAlloc`. Discriminates whether the guest's two
#                allocation paths reach the same host backing, using no source knowledge.
#   4. sizes   — the matmul curve: the same-hour correctness control (`bad=0 maxerr=0`) and a
#                re-measurement of the numbers w320 is quoted for.
set -uo pipefail
cd "$(dirname "$0")/../.."   # scripts/bench -> repo root
export CARGO_TARGET_DIR=${CARGO_TARGET_DIR:-/workspace/bench/cargo-target-w318}
for A in bwneg bw2 bwhost sizes; do
  case "$A" in
    bw2) ARM=bw; export KAYFABE_TAG=w322bw2 ;;
    *)   ARM=$A; unset KAYFABE_TAG ;;
  esac
  echo "=== ★ BATCH arm=$A ($(date -Is)) ==="
  bash scripts/bench/w322_operands.sh "$ARM"
  echo "=== ★ BATCH arm=$A rc=$? ($(date -Is)) ==="
done
echo "W322_BATCH_TERMINATOR rc=0 $(date -Is)"
