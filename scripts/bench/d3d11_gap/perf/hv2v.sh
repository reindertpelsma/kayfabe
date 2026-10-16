#!/bin/bash
# hv2v.sh LABEL API : Heaven on the VFIO native guest (G2). Guest dmon (SMI=2) instead of the host's; asserts guest driver and the QEMU command line.
LABEL=$1; API=$2; P=/data/claude-gap/perf; EV=/data/claude-gap/ev/heaven/$LABEL; mkdir -p $EV
{ echo "ASSERT time=$(date -Is)"; echo "ASSERT guest_driver=$(timeout 60 python3 /data/kfagent_server.py run '(& nvidia-smi --query-gpu=driver_version --format=csv,noheader) -join ""' 40 | tr -d '\r\n')"
  echo "ASSERT vfio_qemu=$(ps -eo args | grep '[v]fio-perf' | cut -c1-200)"; echo "ASSERT host_gpu_driver=$(basename $(readlink -f /sys/bus/pci/devices/0000:01:00.0/driver))"; } | tee $EV/assert.txt
bash $P/memrec.sh pre > $EV/env_pre.txt 2>&1
timeout 420 vmstat 1 > $EV/vmstat.txt 2>&1 & VMP=$!
echo "HV_START $LABEL $API $(date -Is)" > $EV/startmarker
HOOK="bash $P/kfp.sh pi_$LABEL interactive $P/jobs/presentinfo.ps1 120" HDELAY=100 bash $P/heaven_run.sh $LABEL $API V 2 > /data/claude-gap/ev/heaven/$LABEL.out 2>&1
kill $VMP 2>/dev/null; bash $P/memrec.sh post > $EV/env_post.txt 2>&1
echo "HV_EXIT $LABEL $(date -Is)" >> $EV/startmarker
grep -a -E "FPS:" /data/claude-gap/ev/heaven/$LABEL.out | sed 's/&nbsp;/ /g' | head -2
