#!/bin/bash
# usage: [FLAGS=file] [QEMU=path] j3run.sh <name> <apps> [extra args]   (pcnight4)
N=$1; APPS=$2; shift 2
E=/workspace/bench/pcnight4_20261115; S=/workspace/bench/pcnight2_20261115/src
W=/workspace/bench/propimage_20261112/work_$N
Q=${QEMU:-/workspace/bench/kf3-bins/b20fd84b/qemu-system-x86_64}
F=${FLAGS:-$S/scripts/bench/windows/kf3_flags.sample.env}
exec 8>/root/prov/pc_gpu.lock; flock -n 8 || { echo LOCK BUSY; exit 3; }
echo "$(date -u +%FT%TZ) pcnight4 START J3 Windows $APPS ($N) offline overlay restrict=on kf3 b20fd84b, flock" >> /root/prov/PC_ACTIVE.log
dmesg > $E/${N}_dmesg_before.txt
timeout 5400 python3 -B -I $S/scripts/bench/windows/propimage_check.py --dir /workspace/bench/propimage_20261112/unp --work $W --out $E/ev/$N --qemu $Q --flags $F --broker $S/broker/nvkvm-display-broker --gop-efi /workspace/bench/propimage_20261112/firmware/kf-gop.signed.efi --apps $APPS "$@"
echo "EXIT rc=$? $(date -u -Is)"
dmesg > $E/${N}_dmesg_after.txt
rm -rf $W
echo "$(date -u +%FT%TZ) pcnight4 END J3 Windows ($N); $W deleted" >> /root/prov/PC_ACTIVE.log
