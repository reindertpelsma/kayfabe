#!/usr/bin/env bash
# m577_nolog_proof_host.sh — ON HOST. PROVE that per-doorbell qemu_log() spew (~94 lines/doorbell,
# 359k lines / 28MB in a 28s run) is the dominant exec_doorbell cost (m576: INSIDE_qemu ~55-60%,
# ~1ms/doorbell, event=0 so completion-latency H2 is REFUTED). Zero code change: launch QEMU with
# `-d unimp,guest_errors` REMOVED so qemu_log_enabled()==false and every device qemu_log() no-ops
# (qemu_logfile path -D alone, without -d, leaves qemu_loglevel==0). Same LLM, same fresh boot.
#   baseline (logging on,  m576) = 23.3 t/s
#   if logging off jumps to ~35+ -> log spew confirmed as THE lever -> do the mechanical DIAG gate.
#   if ~unchanged                -> cost is real exec_doorbell work (GMMU walks/maps), profile deeper.
set -u
PORT=2223
SSHG="ssh -p $PORT -o StrictHostKeyChecking=no -o ConnectTimeout=6 -o UserKnownHostsFile=/dev/null ubuntu@localhost"
NGEN=${LLM_NGEN:-96}; TIMEOUT=${LLM_TIMEOUT:-240}

# Build a logging-OFF variant of the run script: drop `-d unimp,guest_errors`, send -D to /dev/null.
sed 's#-D "$QLOG" -d unimp,guest_errors \\#-D /dev/null \\#' \
    /workspace/nvkvm/scripts/run_mode2_vm.sh > /tmp/run_nolog.sh
grep -qE '^[^#]*-d unimp' /tmp/run_nolog.sh && { echo "SED_FAILED (still has -d flag)"; exit 1; }
grep -qE '^[^#]*-D /dev/null' /tmp/run_nolog.sh || { echo "SED_FAILED (no -D /dev/null)"; exit 1; }

run_one() {  # $1=label  $2=script
  pkill -9 -f qemu-system-x86_64 2>/dev/null; sleep 3
  NVKVM_M2CEFWD=1 nohup bash "$2" >/tmp/m0_launch.log 2>&1 &
  sleep 3; pgrep -f qemu-system-x86_64 >/dev/null || { echo "$1 QEMU_DIED"; tail -5 /tmp/m0_launch.log; return; }
  local up=0; for i in $(seq 1 40); do $SSHG echo OK 2>/dev/null | grep -q OK && { up=1; break; }; sleep 5; done
  [ "$up" = 1 ] || { echo "$1 NOBOOT"; return; }
  scp -q -P $PORT -o StrictHostKeyChecking=no -o UserKnownHostsFile=/dev/null \
    /workspace/nvkvm/scripts/mode2_diag/llm_run_guest.sh ubuntu@localhost:/tmp/ 2>/dev/null
  $SSHG "LLM_NGEN=$NGEN LLM_TIMEOUT=$TIMEOUT bash /tmp/llm_run_guest.sh" > /tmp/m577_$1.log 2>&1
  echo "[$1] $(grep -iE 'Generation:|exit rc=' /tmp/m577_$1.log | tail -2 | tr '\n' ' ')"
}

( cd /opt/qemu-src/build && ninja install ) 2>&1 | tail -1
echo "############ M577 logging on/off A/B (fresh boot each) ############"
echo "RUN1 logOFF: $(run_one logoff /tmp/run_nolog.sh)"
echo "RUN2 logON : $(run_one logon  /workspace/nvkvm/scripts/run_mode2_vm.sh)"
echo "RUN3 logOFF: $(run_one logoff2 /tmp/run_nolog.sh)"
echo "RUN4 logON : $(run_one logon2  /workspace/nvkvm/scripts/run_mode2_vm.sh)"
echo "--- health ---"; uptime | grep -oE "load average.*"; ps -eo stat | grep -c '^D' | sed 's/^/D-state: /'
echo "############ END m577 ############"
