#!/usr/bin/env bash
# MEASUREMENT ONLY. One fast-guest boot that runs /bin/kftool (trapprobe) and prints its TP lines. Caller holds the GPU lock.
#   usage: fg_probe.sh <tag> <kf3-bins dir name> [ENV=VAL ...]   (env pairs reach QEMU: KF3_DIAG_INTR_SPIN=1 ...)
set -uo pipefail
ROOT=/data/claude-trapexp-20261019; TAG=$1; REV=$2; shift 2
Q=$ROOT/kf3-bins/$REV/qemu-system-x86_64; FG=$ROOT/fgout; OUT=$ROOT/fgprobe; mkdir -p $OUT
SER=$OUT/${TAG}_serial.log; rm -f $SER
pkill -9 -f '[k]f-fastguest' 2>/dev/null; sleep 1
timeout --kill-after=3 90 env "$@" "$Q" -name kf-fastguest \
  -object memory-backend-memfd,id=ram0,size=2048M,share=on -machine q35,accel=kvm,memory-backend=ram0 -m 2048 -cpu host -smp 3 \
  -kernel $FG/vmlinuz -initrd $FG/initrd.cpio.gz \
  -append "console=ttyS0 console=hvc0 panic=1 loglevel=6 KF_TOOL=1 KF_TOOL_ARGS=x KF_BUDGET_S=80 KF_IOCTL_TRACE=ring" \
  -device kf3-gpu,fb-mb=8192,bar1-size=134217728,bar2-size=33554432,id=kf0 -msg timestamp=on \
  -serial file:$OUT/${TAG}_ttyS0.log -device virtio-serial-pci,id=kfvs0 -chardev file,id=kfcon0,path=$SER -device virtconsole,chardev=kfcon0,bus=kfvs0.0 \
  -display none > $OUT/${TAG}_qemu.log 2>&1
echo "rc=$?"; grep -a "TP \|FASTGUEST: tool\|FASTGUEST: client\|Kernel panic" $SER | tr -d '\r'
grep -a "trapexp" $OUT/${TAG}_qemu.log | tail -3
