#!/bin/bash
# usage: [EXTRA="--fb-mb N"] [PATCH=patch_x.ps1] [FLAGS=..] [QEMU=..] [RELAUNCH_WAIT=120] go.sh <name> <attempts>
N=$1; A=$2
cd /workspace/bench/pcnight4_20261115
rm -f release_$N ready_$N proceed_$N
L=/workspace/bench/propimage_20261112/work_$N/run/qemu.log
( for i in $(seq 1 900); do [ -e $L ] && break; sleep 2; done; bash sampler.sh $L samp_$N.txt ) &
( for i in $(seq 1 900); do [ -e $L ] && break; sleep 2; done; while [ -e $L ]; do cp $L qemu_${N}_copy.log.tmp && mv qemu_${N}_copy.log.tmp qemu_${N}_copy.log; sleep 15; done ) &
bash j3run.sh $N edge --hold-until-file /workspace/bench/pcnight4_20261115/release_$N $EXTRA > $N.out 2>&1 &
for i in $(seq 1 120); do grep -q '^verify' $N.out && break; grep -q '^EXIT' $N.out && exit 1; sleep 5; done
export KF_WORK=$N
python3 -B -I gx2.py ps ${PATCH:-patch_vhnofg.ps1} 60 > ini_patch_$N.txt 2>&1
python3 -B -I gx2.py ps '$d="C:\Program Files (x86)\Steam\steamapps\common\Black Myth Wukong Benchmark Tool\b1\Saved\Config\Windows"; foreach($f in "GameUserSettings.ini"){ "=== $f"; Get-Content "$d\$f" }' 60 > ini_after_$N.txt 2>&1
if [ -n "$TDRREG" ]; then python3 -B -I gx2.py ps tdr_reg.ps1 60 > tdrreg_$N.txt 2>&1; fi
if [ -n "$REBOOT" ]; then bash reboot_guest.sh $N > reboot_$N.txt 2>&1; python3 -B -I gx2.py ps tdr_reg_read.ps1 30 >> reboot_$N.txt 2>&1; fi
touch ready_$N
if [ -n "$GATE" ]; then for i in $(seq 1 1800); do [ -e proceed_$N ] && break; sleep 2; done; fi
bash restart_drv.sh $N wuk_$N $A
touch release_$N
wait
sleep 3
cp $L qemu_${N}_copy.log 2>/dev/null
tar czf ${N}ev.tgz wuk_$N $N.out samp_$N.txt qemu_${N}_copy.log ini_after_$N.txt ini_patch_$N.txt ev/$N ${N}_dmesg_before.txt ${N}_dmesg_after.txt 2>/dev/null
echo TARRED
