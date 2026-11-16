#!/bin/bash
set -u
export PATH=/usr/local/cuda-12.2/bin:$PATH
cd /workspace/kf-ci-src/cuda/walk
L=/workspace/kf_tests_out; mkdir -p $L
echo "START $(date -u +%FT%TZ) rev=$(git -C /workspace/kf-ci-src rev-parse HEAD) $(nvcc --version | tail -1) $(nvidia-smi --query-gpu=name,driver_version --format=csv,noheader) kernel_bytes=$(stat -c%s /workspace/kernel.log)" > $L/run.log
make check-invariants > $L/check_invariants.txt 2>&1; echo "check-invariants rc=$?" >> $L/run.log
make kf_tests > $L/build.txt 2>&1; echo "build rc=$?" >> $L/run.log
make check-ptx > $L/check_ptx.txt 2>&1; echo "check-ptx rc=$?" >> $L/run.log
timeout 120 ./kf_tests hostile/report_run_cap > $L/case_report_run_cap.txt 2>&1; echo "case hostile/report_run_cap rc=$?" >> $L/run.log
timeout 900 ./kf_tests > $L/kf_tests_full.txt 2>&1; echo "kf_tests full rc=$?" >> $L/run.log
echo "END $(date -u +%FT%TZ) kernel_bytes=$(stat -c%s /workspace/kernel.log)" >> $L/run.log
