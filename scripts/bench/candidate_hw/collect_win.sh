#!/bin/bash
# collect_win.sh <outdir> TAG... : pull the screened text evidence of the Windows boots from the host (ssh root@172.22.1.20, /data/claude-candhw/gap/ev) into <outdir>/<tag>/.
# Kept: the QEMU environment record, Heaven result lines, events, assert (gpu-uuid redacted), memory records, journal Xid lines, the lazy / lifecycle lines (only), shot statistics, dmon summary inputs.
# Not kept (stay on the host): qemu logs, vmstat, dmon rows, thread CPU json, display-line logs, broker logs.
O=$1; shift; H=root@172.22.1.20; E=/data/claude-candhw/gap/ev
for t in "$@"; do
  l=$(echo "$t" | tr 'A-Z' 'a-z'); D=$O/$t; mkdir -p $D
  for f in arm/$t.environ.txt arm/$t.xid_summary.txt arm/$t.xid_journal.txt arm/$t.mem_pre.txt arm/$t.mem_prerun.txt arm/$t.mem_postrun.txt arm/$t.broker.txt; do
    ssh $H "cat $E/$f 2>/dev/null" | sed -E 's/gpu-uuid=[0-9a-fA-F-]+/gpu-uuid=REDACTED/; s/(WIN_MAC|WIN_GPU_UUID)=\S+/\1=REDACTED/' > $D/$(basename $f)
  done
  for api in d3d11 gl; do
    ssh $H "cat $E/heaven/${l}_$api.out $E/heaven/${l}_$api.sum 2>/dev/null" | sed 's/&nbsp;/ /g' > $D/heaven_$api.txt
    ssh $H "cat $E/heaven/${l}_$api/guest_events.txt $E/heaven/${l}_$api/startmarker 2>/dev/null" > $D/events_$api.txt
    ssh $H "cat $E/heaven/${l}_$api/assert.txt 2>/dev/null" | sed -E 's/gpu-uuid=[0-9a-fA-F-]+/gpu-uuid=REDACTED/' > $D/assert_$api.txt
  done
  ssh $H "cat $E/heaven/shot_${t}_d3d11.out 2>/dev/null" | grep -a 'SHOT' > $D/shot_d3d11.txt
  ssh $H "grep -a -E 'USER_RING_LAZY_OBSERVE|USER_RING_LIFECYCLE|USER_RING_REFUSED|USER_RING_POINTER|USER_RING_PUT_BOUNDARY|USER_RING_WINDOW_FRESH|USER_RING_MIGRATED|USER_RING_SOFTWARE_OBJECT_GONE' $E/arm/$t.lines.txt 2>/dev/null" | cut -c1-3000 > $D/user_ring_lines.txt
  # power / SM from the dmon rows (the summary only)
  for api in d3d11 gl; do ssh $H "awk '\$1 ~ /^[0-9][0-9]:[0-9][0-9]:/ && \$6+0 > 50 {p+=\$3; s+=\$6; n++} END {if (n) printf \"dmon_%s rows=%d avg_power_w=%.1f avg_sm_pct=%.1f\\n\", \"$api\", n, p/n, s/n}' $E/heaven/${l}_$api/host_dmon.txt 2>/dev/null"; done > $D/dmon_summary.txt
done
