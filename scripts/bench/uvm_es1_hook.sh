#!/usr/bin/env bash
# ★ POST_CAPTURE_HOOK — E-S1, the Bug 1624521 negative control (`docs/design/V3_UVM_GUEST_FAULT_PLANE.md`
# §5): while a guest CUDA process holds a managed-memory context (so kf3 holds an EFS twin space), a
# FOREIGN host process names the VMM's client + twin VA-space handles (read from the VMM's own log)
# in UVM_REGISTER_GPU_VASPACE (`tools/uvm_efs/tests/efs_foreign.cpp`), with a positive control on a
# space of its own. Builds both probes on the host. usage: as uvm_guest_hook.sh.
set -uo pipefail
HERE="$(cd "$(dirname "$0")" && pwd)"; REPO="$(cd "$HERE/../.." && pwd)"; G="$HERE/gssh_nv"
TAG=${1:?tag}; Q=/workspace/bench/run_${TAG}_qemu.log
echo "ES1_HOOK_START $(date -Is) tag=$TAG rev=$(git -C "$REPO" rev-parse --short=8 HEAD)"
OGKM=${OGKM:-/root/ogkm}; UVM_SRC=${UVM_SRC:-/root/efs/patched/nvidia-uvm}
INC="-I$OGKM/src/common/sdk/nvidia/inc -I$OGKM/src/common/sdk/nvidia/inc/class -I$UVM_SRC -I$OGKM/kernel-open/common/inc"
g++ -O2 -std=c++14 -o /workspace/bench/efs_foreign "$REPO/tools/uvm_efs/tests/efs_foreign.cpp" $INC || { echo "ES1_HOOK_NOTRUN efs_foreign build"; exit 2; }
/usr/local/cuda/bin/nvcc -O2 -arch=sm_86 -cudart static -o /workspace/bench/um_hold "$HERE/um_hold.cu" || { echo "ES1_HOOK_NOTRUN um_hold build"; exit 2; }
scp -i /workspace/bench/guest_key -o StrictHostKeyChecking=no -o UserKnownHostsFile=/dev/null -o LogLevel=ERROR \
    /workspace/bench/um_hold ubuntu@192.168.77.2:/tmp/um_hold || { echo "ES1_HOOK_NOTRUN scp"; exit 2; }
n0=$(grep -c 'EFS mirror space=' "$Q")
( $G "cd /tmp && ./um_hold 25" > /workspace/bench/es1_hold_${TAG}.log 2>&1 & )
for i in $(seq 1 60); do [ "$(grep -c 'EFS mirror space=' "$Q")" -gt "$n0" ] && grep -q 'HOLD READY' /workspace/bench/es1_hold_${TAG}.log 2>/dev/null && break; sleep 1; done
line=$(grep 'EFS mirror space=' "$Q" | tail -1)
echo "ES1 the VMM's newest EFS space: $line"
vas=$(echo "$line" | sed -n 's/.*EFS mirror space=\(0x[0-9a-f]*\).*/\1/p'); client=$(echo "$line" | sed -n 's/.* client=\(0x[0-9a-f]*\).*/\1/p')
[ -n "$vas" ] && [ -n "$client" ] || { echo "ES1_HOOK_NOTRUN no EFS space in the log"; exit 2; }
/workspace/bench/efs_foreign "$client" "$vas"; echo "ES1_FOREIGN_RC=$?"
for i in $(seq 1 40); do grep -q 'HOLD DONE\|HOLD FAIL' /workspace/bench/es1_hold_${TAG}.log 2>/dev/null && break; sleep 1; done
echo "--- guest hold process:"; cat /workspace/bench/es1_hold_${TAG}.log
echo "--- host dmesg tail:"; dmesg | tail -5
echo "ES1_HOOK_DONE $(date -Is)"
