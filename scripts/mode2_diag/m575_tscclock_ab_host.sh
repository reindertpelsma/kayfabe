#!/usr/bin/env bash
# m575_tscclock_ab_host.sh — ON HOST. FAIR clocksource A/B: a FRESH BOOT per clocksource (no
# stale-GSP confound that invalidated m574's 2nd run). Sets the guest clocksource right before the
# LLM, runs NGEN=64, reports generation t/s. Establishes whether tsc (vDSO clock_gettime) beats
# kvm-clock (every clock_gettime = syscall) cleanly. Expected: tsc modestly faster (removes the
# ~27% syscall overhead in libcuda's cuStreamSynchronize spin); the rest of the gap to 60 is the
# completion-sema latency (H2), a separate lever.
set -u
PORT=2223
SSHG="ssh -p $PORT -o StrictHostKeyChecking=no -o ConnectTimeout=6 -o UserKnownHostsFile=/dev/null ubuntu@localhost"
NGEN=${LLM_NGEN:-64}; TIMEOUT=${LLM_TIMEOUT:-200}
( cd /opt/qemu-src/build && ninja install ) 2>&1 | tail -1

run_one() {  # $1 = clocksource
  local CS="$1"
  pkill -9 -f qemu-system-x86_64 2>/dev/null; sleep 3
  NVKVM_M2CEFWD=1 nohup bash /workspace/nvkvm/scripts/run_mode2_vm.sh >/tmp/m0_launch.log 2>&1 &
  sleep 3; pgrep -f qemu-system-x86_64 >/dev/null || { echo "$CS QEMU_DIED"; return; }
  local up=0; for i in $(seq 1 40); do $SSHG echo OK 2>/dev/null | grep -q OK && { up=1; break; }; sleep 5; done
  [ "$up" = 1 ] || { echo "$CS NOBOOT"; return; }
  scp -q -P $PORT -o StrictHostKeyChecking=no -o UserKnownHostsFile=/dev/null \
    /workspace/nvkvm/scripts/mode2_diag/llm_run_guest.sh ubuntu@localhost:/tmp/ 2>/dev/null
  $SSHG "echo $CS | sudo tee /sys/devices/system/clocksource/clocksource0/current_clocksource >/dev/null" 2>/dev/null
  local got=$($SSHG "cat /sys/devices/system/clocksource/clocksource0/current_clocksource" 2>/dev/null)
  $SSHG "LLM_NGEN=$NGEN LLM_TIMEOUT=$TIMEOUT bash /tmp/llm_run_guest.sh" > /tmp/m575_${CS}.log 2>&1
  echo "[$CS got=$got] $(grep -iE 'Generation:|exit rc=' /tmp/m575_${CS}.log | tail -2 | tr '\n' ' ')"
}

echo "############ M575 FAIR A/B (fresh boot each) ############"
echo "RUN1: $(run_one tsc)"
echo "RUN2: $(run_one kvm-clock)"
echo "RUN3: $(run_one tsc)"
echo "RUN4: $(run_one kvm-clock)"
echo "--- health ---"; uptime | grep -oE "load average.*"; ps -eo stat | grep -c '^D' | sed 's/^/D-state: /'
echo "############ END ############"
