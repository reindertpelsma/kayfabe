#!/bin/bash
# chainv.sh TAG : G2 native VFIO reference (the 4070 is detached from nvidia by launch_v.sh and ALWAYS restored by its trap). Caller holds the fastguest lock.
cd /data/claude-gap; P=/data/claude-gap/perf; TAG=${1:-V1}
export OVL=/data/claude-gap/disk/K.qcow2 VARS=/data/claude-gap/disk/K.vars.fd
mkdir -p vfio ev/heaven
echo "VCHAIN_START $TAG $(date -Is)"
bash $P/memrec.sh pre_boot_$TAG > ev/arm/$TAG.mem_pre.txt 2>&1
(CHAIN_LOCKED=1 setsid nohup bash $P/launch_v.sh > ev/launchv-$TAG.out 2>&1 < /dev/null &)
for i in $(seq 60); do grep -q "VFIO_SESSION_READY\|VFIO_SESSION_EXIT" ev/launchv-$TAG.out && break; sleep 2; done
tail -3 ev/launchv-$TAG.out
if grep -q VFIO_SESSION_READY ev/launchv-$TAG.out; then
  bash $P/waitguest.sh
  sleep 40
  bash $P/hv2v.sh ${TAG,,}_d3d11 direct3d11 > ev/heaven/${TAG,,}_d3d11.sum 2>&1
  bash $P/hv2v.sh ${TAG,,}_gl opengl > ev/heaven/${TAG,,}_gl.sum 2>&1
fi
touch /tmp/kf-stop-perf
for i in $(seq 120); do grep -q VFIO_SESSION_EXIT ev/launchv-$TAG.out && break; sleep 3; done
tail -4 ev/launchv-$TAG.out
bash $P/memrec.sh post_$TAG > ev/arm/$TAG.mem_post.txt 2>&1
echo "gpu driver after: $(basename $(readlink -f /sys/bus/pci/devices/0000:01:00.0/driver)) smi=$(nvidia-smi --query-gpu=name,driver_version,pstate --format=csv,noheader)"
echo "VCHAIN_END $(date -Is)"
