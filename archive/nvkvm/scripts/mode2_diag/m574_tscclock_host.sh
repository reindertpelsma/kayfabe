#!/usr/bin/env bash
# m574_tscclock_host.sh — ON HOST. Test the clocksource fix: the guest's kvm-clock is NOT
# vDSO-capable (500k clock_gettime = 500k SYSCALLS), so libcuda's cuStreamSynchronize spin traps
# to the kernel every iteration (~27% of gen cycles in syscall entry/exit per m573). Switching to
# `tsc` makes clock_gettime vDSO (0 syscalls). A/B: run the LLM under tsc, compare to ~22 tok/s
# (kvm-clock baseline). If t/s jumps -> the clocksource was a real (partial) throughput tax. If
# unchanged -> the spin is bound by the completion sema NOT updating (H2 = coherency/mapping), and
# the clock only wasted CPU. Either result is decisive for the next lever.
set -u
PORT=2223
SSHG="ssh -p $PORT -o StrictHostKeyChecking=no -o ConnectTimeout=6 -o UserKnownHostsFile=/dev/null ubuntu@localhost"
NGEN=${LLM_NGEN:-64}
TIMEOUT=${LLM_TIMEOUT:-200}

echo "==> install + fresh boot (NVKVM_M2CEFWD=1)"
( cd /opt/qemu-src/build && ninja install ) 2>&1 | tail -1
pkill -9 -f qemu-system-x86_64 2>/dev/null; sleep 3
NVKVM_M2CEFWD=1 nohup bash /workspace/nvkvm/scripts/run_mode2_vm.sh >/tmp/m0_launch.log 2>&1 &
sleep 3; pgrep -f qemu-system-x86_64 >/dev/null || { echo QEMU_DIED; exit 1; }
up=0; for i in $(seq 1 40); do $SSHG echo OK 2>/dev/null | grep -q OK && { up=1; break; }; sleep 5; done
[ "$up" = 1 ] || { echo NOBOOT; exit 1; }
scp -q -P $PORT -o StrictHostKeyChecking=no -o UserKnownHostsFile=/dev/null \
  /workspace/nvkvm/scripts/mode2_diag/llm_run_guest.sh ubuntu@localhost:/tmp/ 2>/dev/null

for CS in tsc kvm-clock; do
  echo ""; echo "########## clocksource = $CS ##########"
  $SSHG "echo $CS | sudo tee /sys/devices/system/clocksource/clocksource0/current_clocksource >/dev/null; \
         echo got=\$(cat /sys/devices/system/clocksource/clocksource0/current_clocksource)" 2>/dev/null
  QB=$(wc -l < /tmp/m0_qemu.log 2>/dev/null || echo 0)
  $SSHG "LLM_NGEN=$NGEN LLM_TIMEOUT=$TIMEOUT bash /tmp/llm_run_guest.sh" > /tmp/m574_${CS}.log 2>&1
  echo "--- $CS: rc + token rate ---"; grep -iE "exit rc=|Generation:|Prompt:" /tmp/m574_${CS}.log | tail -3
  QE=$(wc -l < /tmp/m0_qemu.log 2>/dev/null || echo 0)
  echo "--- $CS: qemu log lines emitted during run: $((QE-QB)) ---"
  # NOTE: 2nd run in same boot may hit stale-GSP; the tsc run (1st) is the clean signal.
done
echo "============ M574 SUMMARY (tsc 1st = clean; compare to kvm-clock ~22 t/s prior baseline) ============"
echo "--- tsc ---";       grep -iE "Generation:|exit rc=" /tmp/m574_tsc.log | tail -2
echo "--- kvm-clock ---"; grep -iE "Generation:|exit rc=" /tmp/m574_kvm-clock.log | tail -2
uptime | grep -oE "load average.*"; ps -eo stat | grep -c '^D' | sed 's/^/D-state: /'
echo "============ END ============"
