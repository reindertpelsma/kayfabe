#!/bin/bash
# l2_assert.sh : for every QEMU the sampler saw, assert binary path/revision/sha256, host and guest driver, KF3_* environment.
D=/data/claude-life; EV=$D/ev; B=$D/bench
hostdrv=$(cat /sys/module/nvidia/version)
declare -A WANT_SHA=( [e1e9947c]=eab8c9c778fa0ccbd6d278636fd6f7e9a3ba815f332677466b11b31f48a96092 [0a746117]=0445f3d69d3a5d436ed89600b3dc095c681062ea8630ab5b6b5efe52f128c5a8 )
fail=0
echo "host driver (live): $hostdrv   gpu: $(nvidia-smi --query-gpu=name,driver_version --format=csv,noheader)"
for spec in "l2 lc1b e1e9947c" "l2 lc1b2 e1e9947c" "l2 lc1b3 e1e9947c" "l2 base1 0a746117" "l2 base2 0a746117"; do
  set -- $spec; f=$EV/$1/qemus_$2.txt; rev=$3
  n=$(grep -c '^QEMU' $f)
  paths=$(grep '^QEMU' $f | sed 's/.*exe=\([^ ]*\).*/\1/' | sort -u | tr '\n' ' ')
  shas=$(grep '^QEMU' $f | sed 's/.*sha256=\([0-9a-f]*\).*/\1/' | sort -u | tr '\n' ' ')
  uids=$(grep '^QEMU' $f | sed 's/.*uid=\([0-9]*\).*/\1/' | sort -u | tr '\n' ' ')
  kf3=$(grep '  env:' $f | grep -o 'KF3_[A-Z0-9_]*=[^ ]*' | sort -u | tr '\n' ' ')
  envset=$(grep '  env:' $f | sed 's/KF_ARMS=[^ ]*//' | sort -u | tr '\n' '|')
  guest=$(for s in $B/fast_${2}_*_serial.log; do grep -ao 'NVRM: loading NVIDIA UNIX Open Kernel Module for x86_64  [0-9.]*' $s | head -1 | grep -o '[0-9.]*$'; done | sort | uniq -c | tr '\n' ' ')
  ok=1; [ "$n" = 30 ] && [ "$paths" = "/var/lib/kf-windows-20261005/kf3-bins/$rev/qemu-system-x86_64 " ] && [ "$shas" = "${WANT_SHA[$rev]} " ] && [ -z "$(echo $kf3 | tr " " "\n" | grep -v "^KF3_REV=")" ] || ok=0
  echo "tag=$2 rev=$rev QEMUs=$n exe=[$paths] sha256=[${shas:0:16}..] uid=[$uids] KF3_vars=[${kf3:-none}] env=[$envset] guest_driver(arms)=[$guest] ASSERT=$([ $ok = 1 ] && echo OK || echo FAIL)"
  [ $ok = 1 ] || fail=1
done
for t in lcb0 lcb1 lcb2 lcb3 lcb4 lcb5 lcb6 lcb7 lcb8 lcb9; do
  f=$EV/l3/qemus_$t.txt; rev=$(grep -o 'kf3=[0-9a-f]*' $EV/l3/l3_$t.txt | head -1 | cut -d= -f2)
  paths=$(grep '^QEMU' $f | sed 's/.*exe=\([^ ]*\).*/\1/' | sort -u | tr '\n' ' '); shas=$(grep '^QEMU' $f | sed 's/.*sha256=\([0-9a-f]*\).*/\1/' | sort -u | tr '\n' ' ')
  kf3=$(grep '  env:' $f | grep -o 'KF3_[A-Z0-9_]*=[^ ]*' | sort -u | tr '\n' ' '); dev=$(grep '  device=' $f | sed 's/.*device=//' | cut -c1-200)
  gd=$(grep -h 'NVRM version' $EV/l3/guest_checks_$t.txt | head -1 | grep -o '[0-9][0-9][0-9]\.[0-9]*\.[0-9]*' | head -1)
  ok=1; [ "$(grep -c '^QEMU' $f)" = 1 ] && [ "$paths" = "/data/claude-life/bench/kf3-bins/$rev/qemu-system-x86_64 " -o "$paths" = "$(readlink -f /data/claude-life/bench/kf3-bins)/$rev/qemu-system-x86_64 " ] && [ "$shas" = "${WANT_SHA[$rev]} " ] && [ -z "$(echo $kf3 | tr " " "\n" | grep -v "^KF3_REV=")" ] || ok=0
  echo "tag=$t rev=$rev QEMUs=$(grep -c '^QEMU' $f) exe=[$paths] KF3_vars=[${kf3:-none}] guest_driver=$gd device=[${dev:0:160}] ASSERT=$([ $ok = 1 ] && echo OK || echo FAIL)"
  [ $ok = 1 ] || fail=1
done
echo "ASSERT_ALL=$([ $fail = 0 ] && echo OK || echo FAIL)"
