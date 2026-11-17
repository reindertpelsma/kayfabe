#!/usr/bin/env bash
# run_standalone.sh [OUT] [ROOT]: the open FFmpeg/DCGM rows on THIS machine, from a plain git clone (needs root and the network once).
# Usage: sudo bash scripts/apps/open_rows/run_standalone.sh /tmp/open_rows_out
# Both rows are discovery rows: the exit status says whether the harness ran, the SUMMARY lines and the TSV files carry the results.
set -uo pipefail
HERE="$(cd "$(dirname "$0")" && pwd)"
OUT=${1:-./open_rows_out}; ROOT=${2:-/opt/open_rows}
mkdir -p "$OUT"
bash "$HERE/provision.sh" "$ROOT" | tee "$OUT/provision.txt"
python3 "$HERE/nvcodec_matrix.py" run --out "$OUT/nvcodec_matrix" --bundle "$ROOT" | tail -n 1 | tee "$OUT/nvcodec_matrix.summary"
python3 "$HERE/dcgm_nvml.py" run --out "$OUT/nvml_dcgm" --bundle "$ROOT" | tail -n 1 | tee "$OUT/nvml_dcgm.summary"
