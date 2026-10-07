#!/usr/bin/env bash
# POST_CAPTURE_HOOK: diagnostics for the no-Xid guest failures of the V3 app matrix
# (EGL / Vulkan / gpu_burn / CuPy). Output goes to the boot's probe log.
set -uo pipefail
HERE="$(cd "$(dirname "$0")" && pwd)"; G="$HERE/../bench/gssh_nv"
$G 'sudo modprobe nvidia_drm; lsmod | grep ^nvidia; ls -la /dev/dri /dev/nvidia* 2>&1'
echo "=== EGL/Vulkan vendor files"
$G 'ls /usr/share/glvnd/egl_vendor.d/ /usr/share/egl/egl_external_platform.d/ /etc/vulkan/icd.d/ /usr/share/vulkan/icd.d/ 2>&1; ls /usr/lib/x86_64-linux-gnu/ | grep -E "EGL_nvidia|GLX_nvidia|nvidia-eglcore|nvidia-glcore|libEGL.so|nvidia-encode|nvcuvid" 2>&1'
echo "=== egl_offscreen under strace (failing opens/ioctls)"
$G 'cd /tmp && sudo strace -f -e trace=openat,ioctl -o /tmp/egl.st /opt/apps/bundle/bin/egl_offscreen 2>&1 | tail -3; grep -E "ENOENT|EINVAL|EPERM|EFAULT|= -1" /tmp/egl.st | grep -v "\.so\|/etc/ld\|locale\|gconv" | tail -25'
echo "=== vulkaninfo (timeout 60) under strace, last 30 syscalls"
$G 'sudo timeout -k 5 60 strace -f -tt -e trace=openat,ioctl,poll,futex,nanosleep,clock_nanosleep -o /tmp/vk.st vulkaninfo --summary > /tmp/vk.out 2>&1; echo rc=$?; tail -5 /tmp/vk.out; tail -30 /tmp/vk.st | cut -c1-200'
echo "=== guest dmesg tail after vulkan"
$G 'sudo dmesg | tail -15'
echo "=== gpu_burn under strace, last 25 syscalls"
$G 'cd /opt/apps/bundle/gpu-burn && sudo timeout -k 5 120 strace -f -o /tmp/gb.st ./gpu_burn 5 2>&1 | tail -3; echo rc=$?; tail -25 /tmp/gb.st | cut -c1-200'
echo "DIAG_DONE"
