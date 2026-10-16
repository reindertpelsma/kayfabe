#!/bin/bash
cd /data/claude-gap; P=/data/claude-gap/perf
echo CHAIN_START $(date -Is)
REV=0a746117 bash $P/startk.sh B0 > ev/startB0.out 2>&1
sleep 40
bash $P/hv.sh b_d3d11_w direct3d11
bash $P/hv.sh b_d3d11_r1 direct3d11
bash $P/hv.sh b_gl_r1 opengl
echo CHAIN_END $(date -Is)
