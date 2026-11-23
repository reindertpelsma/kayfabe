#!/bin/bash
# A1: the DGC case alone on the FIXED kf3 (ad1a0c4c = tree2 with PR 480); B1: the sparse family in ONE boot, each case its own deqp process: the fully bound
# size_2_16 first, then the half-resident size_2_24, then size_2_16 again; A0: the DGC case alone on the CONTROL kf3 (ad1a0c4c-dirty: only apply.rs unfixed).
until [ -f /workspace/steps/apps_prov.exit ]; do sleep 3; done
FIX=/workspace/bench/kf3-bins/ad1a0c4c/qemu-system-x86_64; CTL=/workspace/bench/kf3-bins/ad1a0c4c-dirty/qemu-system-x86_64; L=/workspace/ogr
echo "CHAIN_OGR A1 $(date -u +%T)"; KF3_BIN=$FIX $L/ogr_boot.sh A1 "dgc=$L/list_dgc.tsv"
echo "CHAIN_OGR B1 $(date -u +%T)"; KF3_BIN=$FIX $L/ogr_boot.sh B1 "t16a=$L/list_t16.tsv resid=$L/list_resid.tsv t16b=$L/list_t16.tsv"
echo "CHAIN_OGR A0 $(date -u +%T)"; KF3_BIN=$CTL $L/ogr_boot.sh A0 "dgc=$L/list_dgc.tsv"
echo "CHAIN_OGR done $(date -u +%T)"
