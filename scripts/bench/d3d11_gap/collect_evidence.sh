#!/bin/bash
# collect_evidence.sh : run on the dev box. Pulls the TEXT evidence of the campaign from the host (/data/claude-gap/ev) into traces/d3d11_gap_20261016/.
# Raw qemu logs (gz, ~300 KB each) and perf.data stay on the host under /data/claude-gap/ev/qemulog and ev/heaven/*/prof/perf.data (named in the report).
H=root@172.22.1.20; D=${1:-/workspace/kf-gap-wt/traces/d3d11_gap_20261016}; mkdir -p $D
ssh $H 'cd /data/claude-gap/ev && tar czf - \
  --exclude="*.data" --exclude="thr0.json" --exclude="thr1.json" --exclude="job.ps1" --exclude="kfp.out" --exclude="offcpu.bt" --exclude="offcpu_folded.txt" --exclude="perf.data" \
  heaven arm chain_*.log launch-*.out start_*.out stop_*.out edge_*.out edge_*.ps1 2>/dev/null' | tar xzf - -C $D
ssh $H 'cd /data/claude-gap && tar czf - tools_out 2>/dev/null' | tar xzf - -C $D 2>/dev/null
echo collected into $D; du -sh $D | tail -1
