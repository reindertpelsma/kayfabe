#!/usr/bin/env bash
# GPU-free: two QEMU-named dummy processes, exact identity, stale/missing/dead targets.
set -euo pipefail
HERE=$(cd "$(dirname "$0")" && pwd)
source "$HERE/qemu_identity.sh"
first="" second=""
cleanup() {
    for pid in "$first" "$second"; do
        if [[ -n "$pid" ]]; then kill "$pid" 2>/dev/null || true; wait "$pid" 2>/dev/null || true; fi
    done
}
trap cleanup EXIT
# Set only these dummy processes' comm; no real QEMU or GPU is involved.
stub='import ctypes,time; assert ctypes.CDLL(None).prctl(15,b"qemu-system-x86",0,0,0)==0; time.sleep(30)'
python3 -c "$stub" & first=$!
python3 -c "$stub" & second=$!
# Wait only for exec of the dummy processes, not for a fixed scheduling delay.
for _ in $(seq 1 100); do
    if a=$(kf_qemu_starttime "$first") && b=$(kf_qemu_starttime "$second"); then break; fi
    sleep 0.01
done
kf_qemu_identity_matches "$first" "$a"
kf_qemu_identity_matches "$second" "$b"
reject() {
    if kf_qemu_identity_matches "$@"; then
        echo "unexpectedly accepted identity: $*" >&2
        return 1
    fi
}
reject "$second" "$(( b + 1 ))"
reject "" "$a"
reject "$first" ""
reject ../self "$a"
reject "$$" "$a" # this shell is not QEMU
kill "$second"; wait "$second" 2>/dev/null || true
reject "$second" "$b"
second=""
kf_qemu_identity_matches "$first" "$a" # do not select or kill the other VM
echo 'QEMU_IDENTITY_SELFTEST PASS'
