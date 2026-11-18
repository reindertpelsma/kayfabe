#!/bin/bash
# usage: restart_drv.sh <work> <outdir> <attempts>
cd /workspace/bench/pcnight3_20261115
export KF_WORK=$1
for p in $(pgrep -f 'python3 -B -I wukong_drive3.py'); do kill $p; done
sleep 1
python3 -B -I gx2.py ps 'New-Item -ItemType File -Force C:\kfhw\stop | Out-Null' 30
sleep 5
python3 -B -I gx2.py int wk_srv2 wk_srv2.ps1
sleep 6
rm -rf $2
python3 -B -I wukong_drive3.py $2 $3 /workspace/bench/propimage_20261112/work_$1/run/qemu.log > $2.out 2>&1
echo driver-finished
