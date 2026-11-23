#!/bin/bash
# usage: vfio_go.sh <name> <attempts>  -- ONE VM session on the real GPU via VFIO; GPU restored at the end whatever happens
N=$1; A=$2
E=/workspace/bench/pcnight_20261114; S=$E/src
W=/workspace/bench/propimage_20261112/work_$N
cd $E
exec 8>/root/prov/pc_gpu.lock; flock -n 8 || { echo LOCK BUSY; exit 3; }
echo "$(date -u +%FT%TZ) pcnight START J4 VFIO reference Wukong ($N): stop gdm, GPU 01:00.0/.1 -> vfio-pci, same image, offline overlay restrict=on, flock; restore nvidia+gdm after" >> /root/prov/PC_ACTIVE.log
restore() { bash $E/vfio_host.sh restore > $E/${N}_restore.txt 2>&1; cat $E/${N}_restore.txt; rm -rf $W
  echo "$(date -u +%FT%TZ) pcnight END J4 VFIO ($N); nvidia+gdm restored ($(nvidia-smi --query-gpu=name --format=csv,noheader 2>&1 | head -1)); $W deleted" >> /root/prov/PC_ACTIVE.log; }
trap restore EXIT
dmesg > $E/${N}_dmesg_before.txt
bash $E/vfio_host.sh bind | tee $E/${N}_bind.txt
grep -q 'vfio-pci' $E/${N}_bind.txt || { echo "bind failed"; exit 4; }
rm -f $E/release_$N
python3 -B -I $E/vfio_wrap.py $S/scripts/bench/windows/propimage_check.py --dir /workspace/bench/propimage_20261112/unp --work $W --out $E/ev/$N --qemu /workspace/bench/kf3-bins/b20fd84b/qemu-system-x86_64 --flags $S/scripts/bench/windows/kf3_flags.sample.env --display off --apps edge --hold-until-file $E/release_$N > $E/$N.out 2>&1 &
HP=$!
for i in $(seq 1 120); do grep -q '^verify' $E/$N.out && break; kill -0 $HP 2>/dev/null || break; sleep 5; done
if grep -q '^verify' $E/$N.out; then (cd $E; KF_WORK=$N python3 -B -I guest_gx.py int setres setres.ps1; sleep 15; KF_WORK=$N python3 -B -I guest_gx.py ps 'Get-Content C:\kfhw\setres.txt' 60 > $E/${N}_setres.txt); bash $E/restart_drv.sh $N wuk_$N $A; fi
touch $E/release_$N
wait $HP
dmesg > $E/${N}_dmesg_after.txt
