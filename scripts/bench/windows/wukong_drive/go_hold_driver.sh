#!/bin/bash
# usage: EXTRA="--fb-mb N" go3.sh <name> <attempts>  : hold boot + Wukong driver + host qemu.log sampler/copier
N=$1; A=$2
cd /workspace/bench/pcnight_20261114
L=/workspace/bench/propimage_20261112/work_$N/run/qemu.log
( for i in $(seq 1 900); do [ -e $L ] && break; sleep 2; done; bash sampler.sh $L samp_$N.txt ) &
( for i in $(seq 1 900); do [ -e $L ] && break; sleep 2; done; while [ -e $L ]; do cp $L qemu_${N}_copy.log.tmp && mv qemu_${N}_copy.log.tmp qemu_${N}_copy.log; sleep 15; done ) &
bash go2.sh $N $A
