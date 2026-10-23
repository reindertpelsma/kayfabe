#!/usr/bin/env bash
# Build an explicitly stamped, optional bench binary; the ordinary builder stays unchanged.
set -euo pipefail
repo=$(cd "$(dirname "$0")/../.." && pwd)
qemu=${1:?private QEMU source}
build=${2:?new private QEMU build directory}
export CARGO_TARGET_DIR=${3:?own cargo target directory}
[ ! -e "$build" ] || { echo 'Use a new private build directory' >&2; exit 2; }
[ -z "$(git -C "$repo" status --porcelain)" ] || { echo 'Commit the source first' >&2; exit 2; }
export KF_RC_SOURCE_REV=$(git -C "$repo" rev-parse HEAD)
export KF_RC_REAL_CARGO=$(command -v cargo)
shim=$(mktemp -d)
trap 'find "$shim" -mindepth 1 -delete; rmdir "$shim"' EXIT
cat >"$shim/cargo" <<'SH'
#!/bin/sh
exec flock -w 600 /tmp/claude-0/kf-local-cargo.lock env CARGO_TARGET_DIR="$CARGO_TARGET_DIR" nice -n 19 "$KF_RC_REAL_CARGO" "$@" --features kf-qemu/rc-negative-control -j2
SH
chmod +x "$shim/cargo"
PATH="$shim:$PATH" bash "$repo/scripts/bench/build_kf3.sh" "$qemu" "$build"
