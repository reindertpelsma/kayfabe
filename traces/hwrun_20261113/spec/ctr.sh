#!/bin/bash
# ctr.sh LOG : the six lifecycle counters + refusal/Xid lines from a kf3 log (counts only)
L=${1:-$(ls -t /workspace/winvm/kfwin/logs/spec_hw1_run*_b*_qemu.log | head -1)}
echo "log=$L size=$(stat -c %s $L)"
echo "LIFECYCLE_SUMMARY lines: $(grep -ac 'USER_RING_LIFECYCLE SUMMARY' $L)"
grep -a 'USER_RING_LIFECYCLE SUMMARY' $L | tail -1 | tr ' ' '\n' | grep -aE '^(segments_streamed|stream_parts_max|stream_restarts|stream_restarts_max|stream_publish_retries|window_entry_max_bytes|refused_scan_budget|refused_staged_span_budget)=' | tr '\n' ' '; echo
echo "USER_RING_REFUSED: $(grep -ac 'USER_RING_REFUSED' $L)  SCAN_BUDGET: $(grep -ac 'USER_RING_SCAN_BUDGET' $L)  REFUSED(any kf3): $(grep -ac 'REFUSED' $L)"
echo "host Xid: $(dmesg | grep -ac 'NVRM: Xid')"
