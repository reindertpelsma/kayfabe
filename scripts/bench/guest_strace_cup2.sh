#!/usr/bin/env bash
# POST_CAPTURE_HOOK: strace cup2 (cuInit -> cuCtxCreate -> CE round-trip) inside the guest.
#
# ★ Why strace and not the LD_PRELOAD interposer: the interposer gates on
#   `_IOC_TYPE == 'F'` (NV_IOCTL_MAGIC), so it is STRUCTURALLY BLIND to /dev/nvidia-uvm
#   (whose magic is not 'F') and to every non-ioctl syscall. Boot gt1439 showed cuInit's
#   87-row RM plane matching a real GA106 record-for-record and status-for-status while
#   cuInit still returns 3 ⇒ the decision is not visible on the plane we can see.
#
# ⊘ Every phase asserts. The FIRST version of this hook found no probe source in the guest,
#   printed four "No such file" lines and still exited 0 — a harness that writes an empty
#   artifact and reports success is worse than none.
set -uo pipefail
G=/workspace/bench/kayfabe/scripts/bench/gssh_nv
die() { echo "★ strace_hook FAILED: $*"; exit 2; }

echo "=== push the probe source (it does NOT persist across guest boots) ==="
$G 'cat > /tmp/cup2.c' < /workspace/bench/cup2.c || die "could not push cup2.c"
$G 'gcc -O0 -o /tmp/cup2 /tmp/cup2.c -lcuda 2>&1; echo GCC_RC=$?'
$G 'test -x /tmp/cup2' || die "cup2 did not build in the guest"

echo "=== which strace ==="
$G 'command -v strace || sudo apt-get install -y strace >/dev/null 2>&1; command -v strace' \
  || die "no strace in the guest and it could not be installed"

echo "=== run cup2 under strace ==="
$G 'cd /tmp && rm -f /tmp/st.txt && timeout 180 strace -f -o /tmp/st.txt /tmp/cup2 2>&1; echo CUP2_RC=$?'
$G 'test -s /tmp/st.txt' || die "/tmp/st.txt is empty — the observation was not made"
echo "=== total lines ==="
$G 'wc -l /tmp/st.txt'
echo "=== every nvidia/dev open ==="
$G 'grep -n "openat.*/dev/" /tmp/st.txt'
echo "=== ioctl census by fd ==="
$G 'grep -o "ioctl([0-9]*" /tmp/st.txt | sort | uniq -c | sort -rn | head -20'
echo "=== every FAILING syscall ==="
$G 'grep -n "= -1" /tmp/st.txt | tail -80'
echo "=== LAST 120 lines ==="
$G 'tail -120 /tmp/st.txt'
