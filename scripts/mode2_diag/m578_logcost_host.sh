#!/usr/bin/env bash
# m578_logcost_host.sh — ON HOST. Is the per-doorbell qemu_log spew (~94 lines/doorbell) the
# dominant exec_doorbell cost? The binary-search index (m576) cut the overlay scan but db only
# fell ~9%, so ~833us/doorbell is elsewhere — qemu_log is the prime suspect. m577's end-to-end
# t/s A/B was swamped by ±40% variance. Here we use the VARIANCE-ROBUST db-us/doorbell ratio
# from NVKVM-TWIN (now emitted to STDERR so it survives `-d` removal):
#   logON  = stock run (run_mode2_vm.sh, -d unimp,guest_errors -> all M5.x qemu_log spew).
#   logOFF = /tmp/run_nolog.sh (-d dropped, -D /dev/null -> qemu_log_enabled()==false, all spew off).
# Both emit TWIN to stderr -> /tmp/m0_launch.log. If db-us/doorbell COLLAPSES logOFF -> logging IS
# the lever (gate the per-doorbell DIAG behind a trace flag). If ~unchanged -> it's real walk work.
set -u
PORT=2223
SSHG="ssh -p $PORT -o StrictHostKeyChecking=no -o ConnectTimeout=6 -o UserKnownHostsFile=/dev/null ubuntu@localhost"
NGEN=${LLM_NGEN:-96}; TIMEOUT=${LLM_TIMEOUT:-240}
SRC=/workspace/nvkvm/src/qemu/nvkvm_gpu_emul.c

cp "$SRC" /opt/qemu-src/hw/misc/nvkvm_gpu_emul.c
( cd /opt/qemu-src/build && ninja install ) 2>&1 | tail -2 || { echo BUILD_FAIL; exit 1; }
sed 's#-D "$QLOG" -d unimp,guest_errors \\#-D /dev/null \\#' \
    /workspace/nvkvm/scripts/run_mode2_vm.sh > /tmp/run_nolog.sh
grep -qE '^[^#]*-d unimp' /tmp/run_nolog.sh && { echo "SED_FAIL"; exit 1; }

mean_db() {  # stdin = TWIN lines; mean db us/doorbell over steady (audit_left=0) windows
  grep -a 'audit_left=0' | grep -oE 'db=[0-9]+ms/[0-9]+us_per' | \
    sed -E 's#db=[0-9]+ms/([0-9]+)us_per#\1#' | \
    awk '{s+=$1;c++} END{if(c)printf "mean=%d us/doorbell (n=%d steady windows)\n",s/c,c; else print "no-steady-data"}'
}

run_one() {  # $1=label $2=script
  pkill -9 -f qemu-system-x86_64 2>/dev/null; sleep 3
  NVKVM_M2CEFWD=1 nohup bash "$2" >/tmp/m0_launch.log 2>&1 &
  sleep 3; pgrep -f qemu-system-x86_64 >/dev/null || { echo "$1 QEMU_DIED"; return; }
  local up=0; for i in $(seq 1 40); do $SSHG echo OK 2>/dev/null | grep -q OK && { up=1; break; }; sleep 5; done
  [ "$up" = 1 ] || { echo "$1 NOBOOT"; return; }
  scp -q -P $PORT -o StrictHostKeyChecking=no -o UserKnownHostsFile=/dev/null \
    /workspace/nvkvm/scripts/mode2_diag/llm_run_guest.sh ubuntu@localhost:/tmp/ 2>/dev/null
  $SSHG "LLM_NGEN=$NGEN LLM_TIMEOUT=$TIMEOUT bash /tmp/llm_run_guest.sh" > /tmp/m578_$1.log 2>&1
  cp /tmp/m0_launch.log /tmp/m578_${1}_qemu.log
  echo "[$1] $(grep -iE 'Generation:' /tmp/m578_$1.log | tail -1)"
  echo "  db-cost: $(grep -a NVKVM-TWIN /tmp/m578_${1}_qemu.log | tail -n +2 | mean_db)"
  echo "  steady TWIN (last 3):"; grep -a NVKVM-TWIN /tmp/m578_${1}_qemu.log | grep -a 'audit_left=0' | tail -3 | \
    grep -oE 'wall=[0-9]+ms|db=[0-9]+ms/[0-9]+us_per|INSIDE_qemu=[0-9.]+%|idx_mismatch=[0-9]+' | paste -sd' '
}

echo "############ M578 log-cost A/B (db-us/doorbell, fresh boot each) ############"
echo "RUN1 logON : "; run_one logon  /workspace/nvkvm/scripts/run_mode2_vm.sh
echo "RUN2 logOFF: "; run_one logoff /tmp/run_nolog.sh
echo "RUN3 logON : "; run_one logon2 /workspace/nvkvm/scripts/run_mode2_vm.sh
echo "RUN4 logOFF: "; run_one logoff2 /tmp/run_nolog.sh
echo "--- health ---"; uptime | grep -oE "load average.*"; ps -eo stat | grep -c '^D' | sed 's/^/D-state: /'
echo "############ END m578 ############"
