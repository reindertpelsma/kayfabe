#!/usr/bin/env bash
set -euo pipefail
REPO=$(cd "$(dirname "$0")/../.." && pwd)
exec python3 "$REPO/scripts/bench/z1/rootless_launch.py" "$@"
