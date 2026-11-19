#!/usr/bin/env bash
# run_standalone.sh [OUT]: the three open ML rows (jax, tensorrt, vllm) on THIS machine from a plain checkout of the repository: packages, setup (hashed locks), run, summaries.
# Root is needed for the apt line only; the network is needed once (PyPI, NVIDIA's own index for TensorRT, the model host). About 20 GB of downloads, a few minutes each.
# Everything lands under OUT (default /tmp/mlrows_out); `rm -rf OUT` is the whole cleanup. A row the machine cannot take says NOT_RUN with the reason (APPNOTRUN).
set -euo pipefail
HERE=$(cd "$(dirname "$0")" && pwd); OUT=${1:-/tmp/mlrows_out}; ROOT=${MLROWS_ROOT:-$OUT/root}
mkdir -p "$OUT"
if [ "$(id -u)" = 0 ] && command -v apt-get >/dev/null; then
  DEBIAN_FRONTEND=noninteractive apt-get install -y -qq python3 python3-venv python3-dev gcc ca-certificates >"$OUT/apt.log" 2>&1
fi
python3 "$HERE/mlrows.py" check --root "$ROOT"
python3 "$HERE/mlrows.py" setup --root "$ROOT"
for row in jax tensorrt vllm; do
  python3 "$HERE/mlrows.py" run "$row" --root "$ROOT" --out "$OUT/$row" >"$OUT/$row.log" 2>&1 || true
  grep -E '^(VLLM|TENSORRT|JAX)_SUMMARY|^APPNOTRUN|^GPU_USE_PROOF' "$OUT/$row.log" || echo "$row: no summary line, see $OUT/$row.log"
done
