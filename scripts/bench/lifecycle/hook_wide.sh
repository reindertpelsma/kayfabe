#!/bin/bash
P=/data/claude-perf2/target/release/kf-mmio-probe; E=/data/claude-perf2/ev/s2
nvidia-smi --query-gpu=pstate,clocks.sm,pcie.link.gen.current,utilization.gpu --format=csv,noheader > $E/smi_before_wide.txt
for k in 0 1 2; do
  taskset -c 2 timeout 100 $P --mib 64 --threads 1 --secs 3 --mode rand --read $k --sizes 64,256,416,1024,4096 > $E/mmio_wide_k${k}_t1_loaded.txt 2>&1
done
for k in 0 1 2; do
  taskset -c 2,3 timeout 100 $P --mib 64 --threads 2 --secs 3 --mode rand --read $k --sizes 256,4096 > $E/mmio_wide_k${k}_t2_loaded.txt 2>&1
done
nvidia-smi --query-gpu=pstate,clocks.sm,pcie.link.gen.current,utilization.gpu --format=csv,noheader > $E/smi_after_wide.txt
