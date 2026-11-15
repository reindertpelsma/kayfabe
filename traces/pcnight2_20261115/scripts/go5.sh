#!/bin/bash
# usage: EXTRA="--fb-mb N" go5.sh <name> <attempts> : like go4 but patches GameUserSettings.ini to LOW + DLSS/FG off before the driver
N=$1; A=$2
cd /workspace/bench/pcnight2_20261115
sed 's#^bash restart_drv.sh#python3 -B -I gx2.py ps ${PATCH:-patch_low.ps1} 60 > ini_patch_$N.txt 2>\&1; bash restart_drv.sh#' go2.sh > go2_low.sh
sed 's#go2.sh#go2_low.sh#; s#go4.sh#go5.sh#' go4.sh > /dev/null
L=/workspace/bench/propimage_20261112/work_$N/run/qemu.log
( for i in $(seq 1 900); do [ -e $L ] && break; sleep 2; done; bash sampler.sh $L samp_$N.txt ) &
( for i in $(seq 1 900); do [ -e $L ] && break; sleep 2; done; while [ -e $L ]; do cp $L qemu_${N}_copy.log.tmp && mv qemu_${N}_copy.log.tmp qemu_${N}_copy.log; sleep 15; done ) &
bash go2_low.sh $N $A
sleep 3
tar czf ${N}ev.tgz wuk_$N $N.out samp_$N.txt qemu_${N}_copy.log ini_before_$N.txt ini_after_$N.txt ini_patch_$N.txt ev/$N ${N}_dmesg_before.txt ${N}_dmesg_after.txt 2>/dev/null
echo TARRED
