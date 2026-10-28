#!/usr/bin/env bash
# Repeatable Linux two-process sharing experiment. Requires staged, pinned assets.
# Usage: two_vm_share.sh <manifest.json> <base-fastguest> <cuda-fastguest> <output-dir>
# Run the whole campaign under flock -o /tmp/kayfabe-fastguest.lock.
set -euo pipefail
REPO=$(cd "$(dirname "$0")/../.." && pwd)
exec python3 "$REPO/scripts/bench/z1/two_vm_share.py" "$@"
