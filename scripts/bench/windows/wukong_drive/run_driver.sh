#!/bin/bash
# usage: restart_drv.sh <work> <outdir> <attempts>
cd /workspace/bench/pcnight_20261114
export KF_WORK=$1
for p in $(pgrep -f 'python3 -B -I wukong_drive.py'); do kill $p; done
sleep 1
python3 -B -I guest_gx.py ps 'New-Item -ItemType File -Force C:\kfhw\stop | Out-Null' 30
sleep 5
python3 -B -I guest_gx.py int wk_srv2 wukong_srv.ps1
sleep 6
rm -rf $2
python3 -B -I wukong_drive.py $2 $3 /workspace/bench/propimage_20261112/work_$1/run/qemu.log > $2.out 2>&1
echo driver-finished
