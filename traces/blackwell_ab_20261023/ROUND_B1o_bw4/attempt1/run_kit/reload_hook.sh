#!/usr/bin/env bash
# B1n DIAGNOSTIC arm lx_reload, POST_CAPTURE_HOOK of boot_capture.sh (owner 2026-10-24):
# the guest has completed one RM life (modprobe + nvidia-smi); unload every NVIDIA module,
# reload, run nvidia-smi again (second guest RM life). Every step is bounded.
set -u
tag=${1:-lx}
G=/root/kayfabe/scripts/bench/gssh_nv
[ -x "$G" ] || G=/workspace/bench/gssh_nv
out=/workspace/bench/run_${tag}_reload.log
echo "RELOAD_HOOK_START $(date -u -Is) qemu_pid=${KF_QEMU_PID:-} start=${KF_QEMU_STARTTIME:-}"
{
 timeout 90 "$G" 'sudo dmesg -C; for m in nvidia_drm nvidia_modeset nvidia_uvm nvidia; do
   if lsmod | grep -q "^$m "; then sudo rmmod "$m"; echo "RMMOD_$m=$?"; fi; done;
   echo "NVIDIA_LOADED_AFTER_RMMOD=$(lsmod | grep -c "^nvidia")"'
 echo "RMMOD_STEP_RC=$? $(date -u -Is)"
 timeout 90 "$G" 'sudo modprobe nvidia; echo "MODPROBE2_RC=$?"'
 echo "MODPROBE_STEP_RC=$? $(date -u -Is)"
 timeout 120 "$G" 'timeout 90 nvidia-smi; echo "SMI2_RC=$?"'
 echo "SMI_STEP_RC=$? $(date -u -Is)"
 timeout 60 "$G" 'sudo dmesg'
} >"$out" 2>&1
cat "$out"
fails=$(grep -c 'RmInitAdapter failed' "$out" || true)
if grep -q '^SMI2_RC=0$' "$out" && grep -q '^MODPROBE2_RC=0$' "$out" &&
 grep -q '^NVIDIA_LOADED_AFTER_RMMOD=0$' "$out" && [ "$fails" = 0 ]; then
 echo "RELOAD_VERDICT PASS second_life=nvidia-smi rc0 rminit_failed=0 $(date -u -Is)"
else
 echo "RELOAD_VERDICT FAIL rminit_failed=$fails $(date -u -Is)"
fi
