#!/bin/bash
# s1_bm.sh — BARE METAL controls on this box: the app matrix (host side) and the graphics set twice
# (hostroot: the guest image's own userspace, suite.sh's phase 1 replicated so the guest runs can
# reuse it with GSET_HOST_FROM=r4bm), then 12 extra bare-metal Cycles renders (the measured spread).
. /root/r4/common.sh
echo "S1_START $(date -Is) rev=$REV"
alive_qemu && { echo "A QEMU is up before s1"; exit 6; }
nvidia-smi --query-gpu=name,driver_version,memory.total,persistence_mode --format=csv,noheader
step apps_host bash scripts/apps/apps_matrix.sh host r4bm all
tail -2 $L/apps_host.log
R=$GR/r4bm; mkdir -p "$R/h2" "$GR/nd"
exec 9>/tmp/kayfabe-fastguest.lock; flock 9
step gset_host1 bash scripts/bench/gfxset/host.sh "$R"
I2=$(sed -n 's/^GSET_RES side=host item=\([^ ]*\) .*/\1/p' "$R/host.res" | grep -vxE 'vkpeak|geekbench_vulkan|blender_opendata' | tr '\n' ' ')
echo "GSET_HOST2_ITEMS=$I2"
step gset_host2 bash scripts/bench/gfxset/host.sh "$R/h2" $I2
cp -f "$R/h2/host.res" "$R/host2.res"; cp -f "$R/h2/host.dig" "$R/host2.dig"
echo "GSET_BM1 $(grep -c 'verdict=PASS' $R/host.res)/$(grep -c '^GSET_RES' $R/host.res) GSET_BM2 $(grep -c 'verdict=PASS' $R/host2.res)/$(grep -c '^GSET_RES' $R/host2.res) host_xid1=$(grep -o 'host_xid=[1-9][0-9]*' $R/host.res | wc -l) host_xid2=$(grep -o 'host_xid=[1-9][0-9]*' $R/host2.res | wc -l)"
for k in $(seq 1 12); do
  step nd_h$k bash scripts/bench/gfxset/host.sh "$GR/nd/h$k" blender_cycles_cuda blender_cycles_optix
done
flock -u 9; exec 9>&-
echo "ND $(cat $GR/nd/h*/host.res | grep -c verdict=PASS)/$(cat $GR/nd/h*/host.res | grep -c '^GSET_RES')"
echo "S1_EXIT rc=0 $(date -Is)"
