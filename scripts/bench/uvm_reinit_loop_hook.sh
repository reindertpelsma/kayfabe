#!/usr/bin/env bash
# POST_CAPTURE_HOOK: N CUDA processes in a row in the fat guest WITHOUT persistence mode — each
# process's exit tears the guest adapter down (a full emulated-GSP unload/reboot) and the next one
# re-initialises it, and nvidia-uvm unregisters and re-registers the GPU (a new internal VA space,
# at the same guest root) every time. The no-PM "5th CUDA process" UVM wall (V3_BUILD.md, w828b)
# in about a minute instead of an LLM boot.
#   KF_DEVICE=kf3 QEMU_BIN=... NVKVM_RAM_MB=8192 URL_N=12 \
#     POST_CAPTURE_HOOK=$PWD/scripts/bench/uvm_reinit_loop_hook.sh bash scripts/bench/boot_capture.sh <tag>
# Prints one `URL proc=<i> rc=<rc> ...` line per process and `URL_OUTCOME=` at the end. A process
# that does not exit inside URL_TMO seconds is a FAIL (a dead UVM channel hangs, it does not error).
set -uo pipefail
HERE="$(cd "$(dirname "$0")" && pwd)"
G="$HERE/gssh_nv"
N=${URL_N:-12}
TMO=${URL_TMO:-120}
PY=${URL_PY:-/home/ubuntu/llmvenv/bin/python}
# The workload: CUDA init, one allocation, one kernel, one copy back — enough to register the GPU
# with UVM and drive its copy channel.
PROG=${URL_PROG:-"import torch; x=torch.ones(1<<20, device='cuda'); print('URL_SUM', int((x*2).sum().item()))"}
QLOG=${BENCH_DIR:-/workspace/bench}/run_${1:-none}_qemu.log
$G true >/dev/null 2>&1 || { echo "URL_OUTCOME=UNMEASURED_GUEST_UNREACHABLE"; exit 0; }
# URL_GUEST_PM=1: persistence mode ON first (one adapter init for the whole loop) — the control arm.
[ "${URL_GUEST_PM:-0}" = 1 ] && $G 'sudo nvidia-smi -pm 1' >/dev/null 2>&1
echo "URL_PM=$($G 'nvidia-smi --query-gpu=persistence_mode --format=csv,noheader' 2>&1 | tr -d '\r' | head -1)"
pass=0; fail=0
for i in $(seq 1 "$N"); do
    t0=$(date +%s%N)
    # ⊘ The guest-side `timeout` cannot kill a process stuck in the driver (D state), so the ssh
    # itself is bounded too [measured uw1: proc 6 hung the hook for 20 min].
    out=$(timeout $((TMO + 30)) $G "timeout $TMO $PY -c \"$PROG\" 2>&1 | tail -3" 2>&1 | tr -d '\r')
    rc=$?
    ms=$(( ($(date +%s%N) - t0) / 1000000 ))
    sum=$(echo "$out" | sed -n 's/.*URL_SUM \([0-9]*\).*/\1/p' | tail -1)
    dead=$(grep -ac ' DEAD: ' "$QLOG" 2>/dev/null || echo 0)
    if [ "$sum" = "$((2 << 20))" ]; then pass=$((pass + 1)); v=PASS; else fail=$((fail + 1)); v=FAIL; fi
    echo "URL proc=$i verdict=$v rc=$rc ms=$ms sum=${sum:-none} dead_lines=$dead"
    [ "$v" = FAIL ] && echo "URL proc=$i output: $(echo "$out" | tail -3 | tr '\n' '|')"
    # ⊘ Stop at the first failure: after a dead channel every later process measures the wreck.
    [ "$v" = FAIL ] && break
done
echo "URL_OUTCOME=pass=$pass fail=$fail of $N"
