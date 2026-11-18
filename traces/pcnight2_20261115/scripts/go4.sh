#!/bin/bash
# usage: EXTRA="--fb-mb N" go4.sh <name> <attempts> : go2 + qemu.log sampler/copier; then copy-friendly evidence tarball
N=$1; A=$2
cd /workspace/bench/pcnight2_20261115
L=/workspace/bench/propimage_20261112/work_$N/run/qemu.log
( for i in $(seq 1 900); do [ -e $L ] && break; sleep 2; done; bash sampler.sh $L samp_$N.txt ) &
( for i in $(seq 1 900); do [ -e $L ] && break; sleep 2; done; while [ -e $L ]; do cp $L qemu_${N}_copy.log.tmp && mv qemu_${N}_copy.log.tmp qemu_${N}_copy.log; sleep 15; done ) &
bash go2.sh $N $A
sleep 3
tar czf ${N}ev.tgz wuk_$N $N.out samp_$N.txt qemu_${N}_copy.log ini_before_$N.txt ini_after_$N.txt ev/$N ${N}_dmesg_before.txt ${N}_dmesg_after.txt 2>/dev/null
echo TARRED
