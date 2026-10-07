#!/usr/bin/env bash
# m579_opaque_validate_host.sh — ON HOST. Validate the M5.62 OPAQUE fast-path (skip the GPFIFO walk
# once a userspace channel is fully resident; re-arm on any dirty-sweep). CORRECTNESS IS THE GATE,
# not speed: this touches the submit path. Two fresh boots, both NVKVM_M2CEFWD=1 NVKVM_M2OPAQUE=1:
#   BOOT 1 = cup8 grid matmul -> must be BYTE-EXACT (CUP8 RESULT bad=0 -> PASS). The un-forgeable check.
#   BOOT 2 = LLM -> must be rc=0 + COHERENT text (garbled = broken submit) + report gen t/s & db[cpu].
# Also report: OPAQUE promote count, skip-walk count, any Xid (MUST be none new), gpga FAILED (0).
set -u
PORT=2223
SSHG="ssh -p $PORT -o StrictHostKeyChecking=no -o ConnectTimeout=6 -o UserKnownHostsFile=/dev/null ubuntu@localhost"
SRC=/workspace/nvkvm/src/qemu/nvkvm_gpu_emul.c

cp "$SRC" /opt/qemu-src/hw/misc/nvkvm_gpu_emul.c
( cd /opt/qemu-src/build && ninja install ) 2>&1 | tail -2 || { echo BUILD_FAIL; exit 1; }

boot() {  # fresh boot with opaque on
  pkill -9 -f qemu-system-x86_64 2>/dev/null; sleep 3
  NVKVM_M2CEFWD=1 NVKVM_M2OPAQUE=1 nohup bash /workspace/nvkvm/scripts/run_mode2_vm.sh >/tmp/m0_launch.log 2>&1 &
  sleep 3; pgrep -f qemu-system-x86_64 >/dev/null || { echo QEMU_DIED; tail -4 /tmp/m0_launch.log; return 1; }
  local up=0; for i in $(seq 1 40); do $SSHG echo OK 2>/dev/null | grep -q OK && { up=1; break; }; sleep 5; done
  [ "$up" = 1 ] || { echo NOBOOT; return 1; }
}

echo "################ BOOT 1: cup8 byte-exact under OPAQUE ################"
if boot; then
  scp -q -P $PORT -o StrictHostKeyChecking=no -o UserKnownHostsFile=/dev/null \
    /workspace/nvkvm/tests/mode2/cup8.c ubuntu@localhost:/tmp/cup8.c 2>/dev/null
  scp -q -P $PORT -o StrictHostKeyChecking=no -o UserKnownHostsFile=/dev/null \
    /workspace/nvkvm/scripts/mode2_diag/cup8_run_guest.sh ubuntu@localhost:/tmp/ 2>/dev/null
  $SSHG "CUP8_N=2048 bash /tmp/cup8_run_guest.sh" > /tmp/m579_cup8.log 2>&1
  echo "cup8: $(grep -aE 'CUP8 RESULT|cup8 exit rc=' /tmp/m579_cup8.log | tail -2 | tr '\n' ' ')"
  echo "  OPAQUE promotes: $(grep -ac 'M5.62 OPAQUE promote' /tmp/m0_qemu.log)  skip-walk doorbells: $(grep -ac 'OPAQUE skip-walk' /tmp/m0_qemu.log)"
  echo "  Xid: $(grep -ac 'Xid' /tmp/m0_qemu.log)  gpga FAILED: $(grep -ac 'gpga.*FAILED' /tmp/m0_qemu.log)"
fi

echo "################ BOOT 2: LLM coherence + perf under OPAQUE ################"
if boot; then
  scp -q -P $PORT -o StrictHostKeyChecking=no -o UserKnownHostsFile=/dev/null \
    /workspace/nvkvm/scripts/mode2_diag/llm_run_guest.sh ubuntu@localhost:/tmp/ 2>/dev/null
  $SSHG "LLM_NGEN=96 LLM_TIMEOUT=240 bash /tmp/llm_run_guest.sh" > /tmp/m579_llm.log 2>&1
  echo "LLM: $(grep -aiE 'Generation:|exit rc=' /tmp/m579_llm.log | tail -2 | tr '\n' ' ')"
  echo "  --- coherence (generated text snippet) ---"
  grep -aiE 'once upon|story|the |a ' /tmp/m579_llm.log | grep -avE 'Generation|rc=|===|t/s' | tail -4
  echo "  OPAQUE promotes: $(grep -ac 'M5.62 OPAQUE promote' /tmp/m0_qemu.log)  skip-walk doorbells: $(grep -ac 'OPAQUE skip-walk' /tmp/m0_qemu.log)"
  echo "  Xid: $(grep -ac 'Xid' /tmp/m0_qemu.log)"
  echo "  --- TWIN db[cpu] last 4 (skip-walk should drop db_cpu) ---"
  grep -a NVKVM-TWIN /tmp/m0_launch.log | tail -4 | grep -oaE "db=[0-9]+ms\[cpu=[0-9]+ms\]/[0-9]+us_per|INSIDE_qemu=[0-9.]+%" | paste - -
fi
echo "--- health ---"; uptime | grep -oE "load average.*"; ps -eo stat | grep -c '^D' | sed 's/^/D-state: /'
echo "################ END m579 ################"
