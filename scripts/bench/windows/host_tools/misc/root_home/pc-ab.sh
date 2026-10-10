#!/bin/bash
# A/B on the reference machine: a guest in the TRUE pre-fix state (manual
# libcuda moved aside), first under the old code, then under the fix.
cd /opt/nvkvm-steamos-latest
S=./steamos-ssh
{
echo "### move the hand-staged libcuda aside (reversible)"
$S "sudo -n mkdir -p /root/libcuda-manual-backup && sudo -n mv -v /usr/lib/libcuda.so.595.84 /root/libcuda-manual-backup/ && sudo -n rm -f /usr/lib/libcuda.so /usr/lib/libcuda.so.1 && sudo -n ldconfig; echo 'libcuda now:'; ldconfig -p | grep -c libcuda" 2>&1 | tail -4
echo
echo "### A: reboot under the OLD code -- converge must NOT repair it"
$S "sudo -n systemctl reboot" >/dev/null 2>&1
sleep 40
for i in $(seq 1 60); do $S true >/dev/null 2>&1 && { echo "guest back after $((i*15))s"; break; }; sleep 15; done
echo "libcuda after old-code converge (expect 0):"
$S "ldconfig -p | grep -c libcuda; ls /usr/lib/libcuda.so* 2>&1 | head -2" 2>&1 | tail -3
echo "repro under old code:"
base64 /root/vkext 2>/dev/null | $S "base64 -d > /tmp/vkext && chmod +x /tmp/vkext" 2>/dev/null
$S "/tmp/vkext VK_KHR_ray_tracing_pipeline" 2>&1 | grep "^RESULT:"
$S "/tmp/vkext" 2>&1 | grep "^RESULT:"
echo "### AB-A-DONE"
} > /root/pc-ab.log 2>&1
