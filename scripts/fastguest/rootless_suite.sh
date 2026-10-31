#!/usr/bin/env bash
# Runtime only: use readable, hash-pinned assets staged by an administrator.
# See --help for the environment contract. Holds the shared lock itself unless
# the parent campaign already holds it with flock -o and sets KF_Z1_LOCK_HELD=1.
set -euo pipefail
REPO=$(cd "$(dirname "$0")/../.." && pwd)
if [ "${1:-}" = --help ]; then
    echo 'rootless_suite.sh <tag> [budget=180] [arm ...]'
    echo 'Required: BENCH_DIR, KF_FASTGUEST_DIR, KF_REAL_QEMU, KF_QEMU_SHA256'
    echo 'Runtime: non-root, supplementary group kvm only, CapEff=CapPrm=0.'
    echo 'Assets and the bench directory must be readable/writable by this user.'
    exit 0
fi
: "${BENCH_DIR:?}" "${KF_REAL_QEMU:?}" "${KF_QEMU_SHA256:?}"
: "${KF_FASTGUEST_DIR:?}"
if [ "${KF_Z1_LOCK_HELD:-0}" != 1 ]; then
    exec flock -o /tmp/kayfabe-fastguest.lock env KF_Z1_LOCK_HELD=1 bash "$0" "$@"
fi
export KF_LOCK="$BENCH_DIR/private.lock"
export QEMU_BIN="$REPO/scripts/fastguest/rootless_qemu.sh"
export KF_VM_NAME="kf-z1-rootless"
export KF_Z1_EVIDENCE="$BENCH_DIR/processes"
mkdir -p "$KF_Z1_EVIDENCE"
python3 "$REPO/scripts/bench/z1/rootless_launch.py" --preflight
exec bash "$REPO/scripts/fastguest/fast_suite.sh" "$@"
