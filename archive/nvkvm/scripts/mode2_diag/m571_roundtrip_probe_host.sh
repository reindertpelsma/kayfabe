#!/usr/bin/env bash
# m571_roundtrip_probe_host.sh — ON HOST. Perf step 3: is generation GUEST-CPU-bound or
# ROUND-TRIP-WAIT-bound? (NOT cacheability / window traps — those measured ~0.) During the
# LLM generation phase the host GPU is idle (util~0) and every QEMU bucket is small, so the
# ~45ms/token is either the guest vCPU spinning (llama.cpp + the emulated driver's completion
# wait) or the guest BLOCKED waiting on a submit->host->sema/IRQ->wake round-trip. The clean
# discriminator: guest vCPU busy% vs idle% during generation. Longer gen (NGEN=200) so the
# 1-Hz sampling is meaningful. Pure observation — no device change.
#   guest CPU ~100% busy + host GPU idle  => GUEST-CPU/spin bound (optimize guest-side path)
#   guest CPU mostly idle (in iowait/idle) => ROUND-TRIP-WAIT bound (optimize IRQ/completion)
set -u
PORT=2223
SSHG="ssh -p $PORT -o StrictHostKeyChecking=no -o ConnectTimeout=5 -o UserKnownHostsFile=/dev/null ubuntu@localhost"
NGEN=${LLM_NGEN:-200}
TIMEOUT=${LLM_TIMEOUT:-300}

echo "==> install QEMU (current build)"; ( cd /opt/qemu-src/build && ninja install ) 2>&1 | tail -1

echo "==> fresh restart VM (NVKVM_M2CEFWD=1, page-batch build)"
pkill -9 -f qemu-system-x86_64 2>/dev/null; sleep 3
NVKVM_M2CEFWD=1 nohup bash /workspace/nvkvm/scripts/run_mode2_vm.sh >/tmp/m0_launch.log 2>&1 &
sleep 3; pgrep -f qemu-system-x86_64 >/dev/null || { echo QEMU_DIED; tail -3 /tmp/m0_launch.log; exit 1; }
echo "==> wait guest"
up=0; for i in $(seq 1 40); do $SSHG echo OK 2>/dev/null | grep -q OK && { up=1; echo "  up ${i}x5s"; break; }; sleep 5; done
[ "$up" = 1 ] || { echo NOBOOT; exit 1; }
QPID=$(pgrep -f qemu-system-x86_64 | head -1); echo "  qemu pid=$QPID, nproc(guest)=$($SSHG nproc 2>/dev/null)"

echo "==> stage runner"
scp -q -P $PORT -o StrictHostKeyChecking=no -o UserKnownHostsFile=/dev/null \
  /workspace/nvkvm/scripts/mode2_diag/llm_run_guest.sh ubuntu@localhost:/tmp/ 2>/dev/null

# Host-side sampler: QEMU process %CPU (sum of all threads) + host GPU util, ~1 Hz.
( for t in $(seq 1 $((TIMEOUT))); do
    qcpu=$(ps -p "$QPID" -o %cpu= 2>/dev/null | tr -d ' ')
    gutil=$(nvidia-smi --query-gpu=utilization.gpu --format=csv,noheader,nounits 2>/dev/null)
    echo "t${t}s host_qemu_cpu=${qcpu}% gpu_util=${gutil}%"; sleep 1
  done ) > /tmp/m571_host.log 2>&1 &
# Guest-side sampler: CPU idle% / iowait% via mpstat-free /proc/stat delta, ~1 Hz.
$SSHG 'nohup bash -c "
  read a b c d idle1 iow1 r < /proc/stat
  for t in \$(seq 1 '"$TIMEOUT"'); do
    sleep 1
    read a b2 c2 d2 idle2 iow2 r < /proc/stat
    # busy = nonidle delta vs total delta (rough, 4 vCPUs aggregated)
    echo \"gt\${t}s idle_jiffies_delta=\$((idle2-idle1)) iow_delta=\$((iow2-iow1))\"
    idle1=\$idle2; iow1=\$iow2
  done
" >/tmp/m571_guest_cpu.log 2>&1 &' 2>/dev/null

echo "==> run llm (NGEN=$NGEN, longer gen window for sampling)"
$SSHG "LLM_NGEN=$NGEN LLM_TIMEOUT=$TIMEOUT bash /tmp/llm_run_guest.sh" > /tmp/m571_guest.log 2>&1
echo "--- rc + rate ---"; grep -iE "exit rc=|Generation:|Prompt:" /tmp/m571_guest.log | tail -3
$SSHG 'cat /tmp/m571_guest_cpu.log 2>/dev/null' > /tmp/m571_guest_cpu_pulled.log 2>/dev/null

echo ""; echo "============ M571 ROUND-TRIP PROBE EVIDENCE ============"
echo "--- host QEMU %CPU during run (>~100% per vCPU busy = guest spinning) ---"
grep -oE "host_qemu_cpu=[0-9.]+" /tmp/m571_host.log | cut -d= -f2 | sort -rn | head -5 | sed 's/^/  peak qemu %cpu: /'
echo "  (4 vCPUs => 400% = all busy; ~100% = ~1 core busy; low = blocked/idle)"
tail -8 /tmp/m571_host.log
echo "--- guest CPU idle jiffies/sec during gen (HIGH idle = WAIT-bound; LOW idle = CPU-bound) ---"
echo "  (4 vCPUs @ 100Hz => ~400 idle jiffies/s = fully idle; ~0 = fully busy)"
tail -12 /tmp/m571_guest_cpu_pulled.log
echo "--- TIMESHARE (whole run) ---"; grep "NVKVM-TIMESHARE" /tmp/m0_qemu.log | tail -1
echo "--- GPU health ---"; uptime | grep -oE "load average.*"; ps -eo stat | grep -c '^D' | sed 's/^/  D-state: /'
echo "============ END ============"
