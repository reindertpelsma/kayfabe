#!/bin/bash
# chainh.sh TAG REV FLAGS FS NPASS : one fresh boot of run 3081 (overlay H), NPASS consecutive Heaven D3D11 Ultra passes
# (separate Heaven processes, same boot), full evidence. Caller holds nothing; startk takes the fastguest lock via the launcher.
G=/data/claude-hitch; cd $G; P=$G/perf; W=/var/lib/kf-windows-20261005; RUNLOG=$W/boundary-kayfabe-3081/qemu.log
tag=$1; rev=$2; fl=$3; fs=${4:-1}; npass=${5:-2}
mkdir -p ev/arm ev/heaven
for i in $(seq 90); do
  f=$(awk '/^full/{split($2,a,"=");print a[2]}' /proc/pressure/memory)
  m=$(awk '/MemAvailable/{print int($2/1048576)}' /proc/meminfo)
  [ "${f%.*}" -lt 8 ] && [ "$m" -ge 14 ] && break; sleep 10
done
echo "BOOT_START $tag rev=$rev flags=[$fl] fs=$fs npass=$npass mem_avail_gb=$m $(date -Is)"
JS=$(date '+%Y-%m-%d %H:%M:%S')
bash $P/memrec.sh pre_boot_$tag > ev/arm/$tag.mem_pre.txt 2>&1
REV=$rev bash $P/startk.sh $tag "$fl" > ev/start_$tag.out 2>&1
QP=$(sed -n "s/^QPID=//p" $W/windows-broker-run3081.state)
echo "QPID=$QP exe=$(readlink /proc/$QP/exe) sha256=$(sha256sum /proc/$QP/exe | cut -c1-64)" > ev/arm/$tag.environ.txt
tr '\0' '\n' < /proc/$QP/environ | grep -E '^(KF3_|WIN_RAM|WIN_FB|WIN_BAR1)' | sort >> ev/arm/$tag.environ.txt
( : > ev/arm/$tag.dispts.txt; tail -n0 -F $RUNLOG 2>/dev/null | grep -a --line-buffered -E 'display fps|kf3: family=|USER_RING_LIFECYCLE' | while IFS= read -r l; do printf '%s %s\n' "$(date +%s.%N)" "$l"; done >> ev/arm/$tag.dispts.txt ) &
TSP=$!
sleep 30
for p in $(seq $npass); do
  echo "PASS_START $tag p$p $(date +%s.%N) $(date -Is)"
  export HOOK=true HDELAY=1
  FS=$fs JOBT=${JOBT:-heaven_etw} bash $P/hv2.sh ${tag,,}_p$p direct3d11 > ev/heaven/${tag,,}_p$p.sum 2>&1
  echo "PASS_END $tag p$p $(date +%s.%N)"
  cp ev/heaven/${tag,,}_p$p/f9_epoch ev/arm/$tag.p$p.f9_epoch 2>/dev/null
  sleep 15
done
bash $P/memrec.sh post_run_$tag > ev/arm/$tag.mem_postrun.txt 2>&1
bash $P/stopk.sh $tag > ev/stop_$tag.out 2>&1
pkill -P $TSP 2>/dev/null; kill $TSP 2>/dev/null; pkill -f "tail -n0 -F $RUNLOG" 2>/dev/null
journalctl -k -o short-iso --since "$JS" 2>/dev/null | grep -a 'NVRM: Xid' > ev/arm/$tag.xid_journal.txt
echo "XID_JOURNAL tag=$tag qpid=$QP lines=$(wc -l < ev/arm/$tag.xid_journal.txt) for_this_qemu=$(grep -c "pid=$QP[ ,)]" ev/arm/$tag.xid_journal.txt) $(date -Is)"
echo "BOOT_END $tag $(date -Is)"
