#!/usr/bin/env bash
# Optional bench supervisor; production QEMU and guest boot need no new flag.
set -euo pipefail
root=$(cd "$(dirname "$0")/../.." && pwd)
exec python3 -B "$root/scripts/bench/native_xid_guard.py" guest-exec \
  "${KF_GUARD_RECEIPT:?}" "${KF_GUARD_REVISION:?}" "${KF_GUARD_QEMU:?}" -- "$@"
