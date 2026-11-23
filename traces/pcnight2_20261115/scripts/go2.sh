#!/bin/bash
# usage: EXTRA="--fb-mb N" go2.sh <name> <attempts>   (hold boot, Wukong driver, ini dump, release)
N=$1; A=$2
cd /workspace/bench/pcnight2_20261115
rm -f release_$N
bash j3run.sh $N edge --hold-until-file /workspace/bench/pcnight2_20261115/release_$N $EXTRA > $N.out 2>&1 &
for i in $(seq 1 120); do grep -q '^verify' $N.out && break; grep -q '^EXIT' $N.out && exit 1; sleep 5; done
export KF_WORK=$N
python3 -B -I gx2.py ps '$d="C:\Program Files (x86)\Steam\steamapps\common\Black Myth Wukong Benchmark Tool\b1\Saved\Config\Windows"; foreach($f in "GameUserSettings.ini","Engine.ini","Scalability.ini"){ "=== $f"; Get-Content "$d\$f" }' 60 > ini_before_$N.txt 2>&1
bash restart_drv.sh $N wuk_$N $A
python3 -B -I gx2.py ps '$d="C:\Program Files (x86)\Steam\steamapps\common\Black Myth Wukong Benchmark Tool\b1\Saved\Config\Windows"; foreach($f in "GameUserSettings.ini"){ "=== $f"; Get-Content "$d\$f" }' 60 > ini_after_$N.txt 2>&1
touch release_$N
wait
