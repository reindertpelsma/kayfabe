#!/usr/bin/env bash
# m554_reload_host.sh — ON HOST. Q2 DRIVER-RELOAD reproducer. Fresh boot, then exercise
# THREE adapter-init cycles in one QEMU lifetime with the pure driver-API reload_probe:
#   A1 first cuInit (expect 0) -> A2 close+reopen, no reload (expect 0) -> B rmmod+insmod
#   then cuInit (expect 0). BUG (2026-06-13): A1=0 but A2/B=999 "WPR2 still up" — the
#   emulated GSP supports only ONE init per lifetime (STARTCPU conflation: teardown's
#   Booter-Unload STARTCPU re-raises WPR2). PASS = all three cuInit=0. Driver-API only =
#   no cudart/UVM/event confounds. Pairs with tests/mode2/reload_probe.c + reload_run_guest.sh.
set -u
PORT=2223; QLOG=/tmp/m0_qemu.log
G="ssh -p $PORT -o StrictHostKeyChecking=no -o ConnectTimeout=6 -o UserKnownHostsFile=/dev/null ubuntu@localhost"
PUSH="scp -q -P $PORT -o StrictHostKeyChecking=no -o UserKnownHostsFile=/dev/null"
pkill -9 -f qemu-system-x86_64 2>/dev/null; sleep 3
nohup bash /workspace/nvkvm/scripts/run_mode2_vm.sh >/tmp/m0_launch.log 2>&1 &
sleep 3; pgrep -f qemu-system-x86_64 >/dev/null || { echo QEMU_DIED; exit 1; }
up=0; for i in $(seq 1 50); do $G echo OK 2>/dev/null | grep -q OK && { up=1; break; }; sleep 5; done
[ "$up" = 1 ] || { echo NOBOOT; exit 2; }
$PUSH /workspace/nvkvm/tests/mode2/reload_probe.c /workspace/nvkvm/scripts/mode2_diag/reload_run_guest.sh ubuntu@localhost:/tmp/ 2>/dev/null
gsp_delta(){ local QB=$1; local QE=$(wc -l < "$QLOG"); sed -n "$((QB+1)),${QE}p" "$QLOG" | grep -E "STARTCPU|WPR2 up|WPR2 down|UNLOADING|GSP_INIT_DONE"; }
echo "==> setup + first init"; $G 'bash /tmp/reload_run_guest.sh setup' 2>&1
Q=$(wc -l < "$QLOG"); echo "===== A1: dvp (first init) ====="; $G 'bash /tmp/reload_run_guest.sh dvp' 2>&1; echo "  -- GSP transitions --"; gsp_delta $Q
Q=$(wc -l < "$QLOG"); echo "===== A2: dvp again (close-then-reopen, NO reload) ====="; $G 'bash /tmp/reload_run_guest.sh dvp' 2>&1; echo "  -- GSP transitions --"; gsp_delta $Q
Q=$(wc -l < "$QLOG"); echo "===== B: rmmod + insmod (driver RELOAD) ====="; $G 'bash /tmp/reload_run_guest.sh reload' 2>&1; echo "  -- GSP transitions --"; gsp_delta $Q
Q=$(wc -l < "$QLOG"); echo "===== B-dvp: dvp after reload (THE TEST) ====="; $G 'bash /tmp/reload_run_guest.sh dvp' 2>&1; echo "  -- GSP transitions --"; gsp_delta $Q
echo "==> guest dmesg"; $G 'bash /tmp/reload_run_guest.sh dmesg' 2>&1
