#!/bin/bash
cd /data/claude-gap; P=/data/claude-gap/perf
echo CHAIN_START $(date -Is)
REV=8c6ab6d1 bash $P/startk.sh S1 "KF3_LIFECYCLE_COUNT=1 KF3_LIFECYCLE_PERIOD_S=5" > ev/startS1.out 2>&1
sleep 40
bash $P/hv.sh s1_d3d11 direct3d11
bash $P/hv.sh s1_gl opengl
echo CHAIN_END $(date -Is)
