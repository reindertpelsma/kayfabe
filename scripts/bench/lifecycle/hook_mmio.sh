#!/bin/bash
# hook: MMIO probe with the GPU under a graphics load
P=/data/claude-perf2/target/release/kf-mmio-probe; E=/data/claude-perf2/ev/s2
nvidia-smi --query-gpu=pstate,clocks.sm,clocks.mem,pcie.link.gen.current,pcie.link.width.current,utilization.gpu --format=csv,noheader > $E/smi_before_mmio_loaded.txt
taskset -c 2 timeout 100 $P --mib 64 --threads 1 --secs 3 --mode rand --sizes 8,64,256,1024,4096 > $E/mmio_rand_t1_loaded.txt 2>&1
taskset -c 2,3 timeout 100 $P --mib 64 --threads 2 --secs 3 --mode rand --sizes 8,64,256,1024,4096 > $E/mmio_rand_t2_loaded.txt 2>&1
taskset -c 2 timeout 100 $P --mib 64 --threads 1 --secs 3 --mode seq --sizes 64,256,1024,4096,65536 > $E/mmio_seq_t1_loaded.txt 2>&1
nvidia-smi --query-gpu=pstate,clocks.sm,clocks.mem,pcie.link.gen.current,pcie.link.width.current,utilization.gpu --format=csv,noheader > $E/smi_after_mmio_loaded.txt
