#!/bin/bash
# A leg, done properly: move aside EVERY libcuda the loader can resolve, so the
# guest matches the true pre-fix state of a steamos-built guest.
cd /opt/nvkvm-steamos-latest
S=./steamos-ssh
{
echo "### before"
$S "ldconfig -p | grep libcuda; sudo -n find / -xdev -name 'libcuda.so*' 2>/dev/null"
echo
echo "### move the nvkvm-pv CUDADIR copies aside too (reversible)"
$S "sudo -n mkdir -p /root/libcuda-manual-backup/cudadir && sudo -n mv -v /usr/local/nvidia-guest/lib/libcuda.so* /root/libcuda-manual-backup/cudadir/ && sudo -n ldconfig" 2>&1 | tail -5
echo "libcuda resolvable now (expect none):"
$S "ldconfig -p | grep -c libcuda; sudo -n find / -xdev -name 'libcuda.so*' 2>/dev/null | head -3"
echo
echo "### repro with NO libcuda anywhere -- this is the true pre-fix state"
$S "/tmp/vkext" 2>&1 | grep "^RESULT:"
for e in VK_KHR_acceleration_structure VK_KHR_ray_tracing_pipeline VK_NV_cuda_kernel_launch; do
  printf "%-34s " "$e"; $S "/tmp/vkext $e" 2>&1 | grep "^RESULT:"
done
echo
echo "### reboot under OLD code -- converge must NOT repair"
$S "sudo -n systemctl reboot" >/dev/null 2>&1
sleep 45
for i in $(seq 1 60); do $S true >/dev/null 2>&1 && { echo "back after $((i*15))s"; break; }; sleep 15; done
$S "uptime -s; ldconfig -p | grep -c libcuda"
printf "%-34s " "after old-code converge"; $S "/tmp/vkext VK_KHR_ray_tracing_pipeline" 2>&1 | grep "^RESULT:"
echo "### AB2-DONE"
} > /root/pc-ab2.log 2>&1
