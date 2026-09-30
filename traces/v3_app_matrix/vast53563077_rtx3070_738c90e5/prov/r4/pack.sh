#!/bin/bash
# pack.sh (ON THE BOX) <kind> <run> — bundle one run's TEXT evidence into /root/r4/out/<run>.tgz plus
# a per-boot digest of the device logs (mode, fast-path counters, refusal ledger, startup-race signature).
set -u
K=${1:?apps|gfx|prov|k49}; RUN=${2:-x}; O=/root/r4/out; mkdir -p $O
digest_qemu(){  # $1 = qemu log (plain or .zst)
  local f=$1 cat=cat; case $f in *.zst) cat="zstd -dc";; esac
  $cat "$f" 2>/dev/null | awk -v F="$(basename $f)" '
    /doorbell fast path/ && !fp {fp=$0}
    /dbfast\[/ {match($0,/dbfast\[[^]]*\]/); db=substr($0,RSTART,RLENGTH)}
    /gsp_refusals\[/ {match($0,/gsp_refusals\[.*\]/); gr=substr($0,RSTART,RLENGTH)}
    /no fd-backed guest RAM block/ {nofd++}
    /no RAM object/ {noram++}
    /kf3: device ABI|kf3-bin-rev|KF3_ABI/ && !abi {abi=$0}
    / REFUSED/ {ref++}
    /RC host twin/ {rc++}
    END {printf "QDIG %s nofd=%d noram_obj=%d refused_lines=%d rc_twin=%d\n  %s\n  %s\n  %s\n", F, nofd, noram, ref, rc, substr(fp,1,200), substr(db,1,400), substr(gr,1,600)}'
}
case $K in
apps)
  R=/workspace/apps/results/$RUN; cd $R || exit 2
  { for q in $(ls boot_*.qemu.log.zst iso/boot_*.qemu.log.zst 2>/dev/null); do digest_qemu $q; done; } > $R/qemu_digest.txt
  python3 /root/kayfabe/scripts/apps/summarize.py $R > $R/summary.md 2>&1
  python3 /root/kayfabe/scripts/apps/triage.py $R > $R/triage.txt 2>&1
  [ -f iso/guest.res ] && python3 /root/kayfabe/scripts/apps/triage.py $R/iso > $R/triage_iso.txt 2>&1
  tar czf $O/apps_$RUN.tgz --exclude='*.qemu.log.zst' . ;;
gfx)
  R=/workspace/gfxset/results/$RUN; cd $R || exit 2
  { for q in $(ls boot_*.qemu.log.zst iso/boot_*.qemu.log.zst 2>/dev/null); do digest_qemu $q; done; } > $R/qemu_digest.txt
  python3 /root/kayfabe/scripts/bench/gfxset/triage.py $R > $R/triage.txt 2>&1
  python3 /root/kayfabe/scripts/bench/gfxset/perf.py $R > $R/perf.md 2>&1
  tar czhf $O/gfx_$RUN.tgz --exclude='*.qemu.log.zst' --exclude='*_art.tar' --exclude='*.jsonl' . ;;
prov)
  cd /root; tar czf $O/prov.tgz prov/*.log r4/logs/*.log r4/*.log /workspace/bench/gfxset.receipt /workspace/bench/gfx_lane.receipt 2>/dev/null ;;
k49)
  cd /workspace/bench; { for q in fast_r4k*_qemu.log; do digest_qemu $q; done; } > /root/r4/k49_qemu_digest.txt
  tar czf $O/k49.tgz fast_r4k*_ttyS0.log fast_r4k*_serial.log r4k*_suite.out /root/r4/k49_qemu_digest.txt /root/r4/logs/k*.run 2>/dev/null
  for q in fast_r4k*_qemu.log; do zstd -q -f $q -o $O/$q.zst; done ;;
esac
ls -la $O | tail -5
