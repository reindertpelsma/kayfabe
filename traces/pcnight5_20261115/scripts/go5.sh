#!/bin/bash
# usage: [EXTRA="--fb-mb 10240"] go5.sh <name> <kf3|vfio> <app> <runs>   (pcnight5; run as root on the PC)
N=$1; MODE=$2; APP=$3; R=$4
E=/workspace/bench/pcnight5_20261115; S=/workspace/bench/pcnight2_20261115/src
W=/workspace/bench/propimage_20261112/work_$N
Q=${QEMU:-/workspace/bench/kf3-bins/b20fd84b/qemu-system-x86_64}
cd $E
exec 8>/root/prov/pc_gpu.lock; flock -n 8 || { echo LOCK BUSY; exit 3; }
echo "$(date -u +%FT%TZ) pcnight5 START $MODE $APP x$R ($N): offline overlay restrict=on, flock$([ $MODE = vfio ] && echo '; stop gdm, GPU -> vfio-pci, restore after')" >> /root/prov/PC_ACTIVE.log
fin() {
  [ $MODE = vfio ] && { bash /workspace/bench/pcnight_20261114/vfio_host.sh restore > $E/${N}_restore.txt 2>&1; cat $E/${N}_restore.txt; }
  rm -rf $W
  echo "$(date -u +%FT%TZ) pcnight5 END $MODE $APP ($N); $W deleted; $(nvidia-smi --query-gpu=name --format=csv,noheader 2>&1|head -1)" >> /root/prov/PC_ACTIVE.log
}
trap fin EXIT
dmesg > $E/${N}_dmesg_before.txt
rm -f $E/release_$N $E/proceed_$N
export KF_WORK=$N
if [ $MODE = vfio ]; then
  bash /workspace/bench/pcnight_20261114/vfio_host.sh bind | tee $E/${N}_bind.txt
  grep -q 'vfio-pci' $E/${N}_bind.txt || { echo "bind failed"; exit 4; }
  timeout 5400 python3 -B -I /workspace/bench/pcnight_20261114/vfio_wrap.py $S/scripts/bench/windows/propimage_check.py --dir /workspace/bench/propimage_20261112/unp --work $W --out $E/ev/$N --qemu $Q --flags $S/scripts/bench/windows/kf3_flags.sample.env --display off --apps edge --hold-until-file $E/release_$N > $E/$N.out 2>&1 &
else
  timeout 5400 python3 -B -I $S/scripts/bench/windows/propimage_check.py --dir /workspace/bench/propimage_20261112/unp --work $W --out $E/ev/$N --qemu $Q --flags $S/scripts/bench/windows/kf3_flags.sample.env --broker $S/broker/nvkvm-display-broker --gop-efi /workspace/bench/propimage_20261112/firmware/kf-gop.signed.efi --apps edge --hold-until-file $E/release_$N $EXTRA > $E/$N.out 2>&1 &
fi
HP=$!
for i in $(seq 1 150); do grep -q '^verify' $E/$N.out && break; kill -0 $HP 2>/dev/null || break; sleep 5; done
if grep -q '^verify' $E/$N.out; then
  python3 -B -I gx3.py int setres setres.ps1; sleep 15
  python3 -B -I gx3.py ps 'Get-Content C:\kfhw\setres.txt' 60 > $E/${N}_setres.txt
  python3 -B -I gx3.py int gdmon guest_dmon.ps1; sleep 3
  if [ $MODE = kf3 ]; then bash dmon.sh $E/${N}_host_dmon.txt & DP=$!; fi
  if [ -n "$HOLD" ]; then for i in $(seq 1 1800); do [ -e $E/proceed_$N ] && break; sleep 2; done; fi
  python3 -B -I multi_run.py $APP $R $E/ev/${N}_runs > $E/${N}_runs.out 2>&1
  [ -n "$DP" ] && kill $DP 2>/dev/null
  python3 -B -I gx3.py ps 'Get-Process nvidia-smi -ErrorAction SilentlyContinue | Stop-Process -Force' 30 >/dev/null 2>&1
  python3 -B -I gx3.py pull C:/kfhw/guest_dmon.txt $E/${N}_guest_dmon.txt
else echo "no verify"; fi
touch $E/release_$N
wait $HP
dmesg > $E/${N}_dmesg_after.txt
tar czf $E/${N}ev.tgz -C $E ev/$N ev/${N}_runs $N.out ${N}_runs.out ${N}_*.txt $(cd $E; ls ${N}_*_dmon.txt 2>/dev/null) 2>/dev/null
echo TARRED
