#!/bin/bash
cd /data/claude-perf2; C=/data/claude-perf2/cuda_copy_probe
echo CHAIN_START $(date -Is)
bash host_heaven.sh ld_ctrl1 "" > ev/s2/ld_ctrl1.out 2>&1
bash host_heaven.sh ld_mc1000 "taskset -c 4 timeout 250 $C rate memcpy 256 1 1000 200" > ev/s2/ld_mc1000.out 2>&1
bash host_heaven.sh ld_mc5000 "taskset -c 4 timeout 250 $C rate memcpy 256 1 5000 200" > ev/s2/ld_mc5000.out 2>&1
bash host_heaven.sh ld_mc20000 "taskset -c 4 timeout 250 $C rate memcpy 256 1 20000 200" > ev/s2/ld_mc20000.out 2>&1
bash host_heaven.sh ld_g8_5000 "taskset -c 4 timeout 250 $C rate gather 256 8 5000 200" > ev/s2/ld_g8_5000.out 2>&1
bash host_heaven.sh ld_ctrl2 "" > ev/s2/ld_ctrl2.out 2>&1
echo CHAIN_END $(date -Is)
