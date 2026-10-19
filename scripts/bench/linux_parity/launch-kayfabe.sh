#!/usr/bin/env bash
# Called only inside LX1's outer global lease; never acquires a private GPU lease.
set -euo pipefail
here=$(cd "$(dirname "$0")" && pwd)
arm=${1:?A, B or correctness-only D}; config=${2:?configuration file}
case "$arm" in A|B|D) ;; *) echo 'LX1_REFUSED: kayfabe arm must be A/B/D' >&2; exit 2;; esac
exec python3 "$here/host.py" run --arm "$arm" --config "$config"
