#!/usr/bin/env bash
# bm32_phase0_gate.sh — RUN ON .32 (bare-metal GA106). Phase 0 of the non-nested baseline:
# does Mode-2 forwarding work against the host's OPEN 595 driver? Boot a fresh Mode-2 guest,
# build+run cup8 (grid matmul) IN the guest, gate on byte-exact (bad=0) + Xid=0.
# PASS  => 595 forwarding works -> proceed to Phase 1 (LLM t/s, the payoff).
# FAIL on cuInit/alloc => 595 ABI gap -> extend abi_profile to 595, or drop in 580 (invasive).
set -u
PORT=2223
# .32's root pubkey was injected into the guest image (inject_key.sh), so key auth works.
SSHO="-o StrictHostKeyChecking=no -o ConnectTimeout=6 -o UserKnownHostsFile=/dev/null"
SSHG="ssh -p $PORT $SSHO ubuntu@localhost"
SCPG() { scp -P $PORT $SSHO "$@"; }
N=${CUP8_N:-1024}
CUP8_SRC=/workspace/nvkvm/tests/mode2/cup8.c
CUP8_RUN=/workspace/nvkvm/scripts/mode2_diag/cup8_run_guest.sh

echo "=== host GPU + driver (the 595 risk) ==="
nvidia-smi --query-gpu=name,driver_version,memory.total --format=csv,noheader 2>&1 | head -1

echo "=== fresh-boot Mode-2 guest (MEM=4G SMP=2 NVKVM_M2CEFWD=1) ==="
pkill -9 -f qemu-system-x86_64 2>/dev/null; sleep 3
MEM=${MEM:-4G} SMP=${SMP:-2} NVKVM_M2CEFWD=1 ${NVKVM_M2OPAQUE:+NVKVM_M2OPAQUE=1} \
  nohup bash /workspace/nvkvm/scripts/run_mode2_vm.sh >/tmp/bm32_launch.log 2>&1 &
echo "  waiting for guest ssh on :$PORT (up to ~5min) ..."
up=0; for i in $(seq 1 60); do $SSHG echo OK 2>/dev/null | grep -q OK && { up=1; break; }; sleep 5; done
[ "$up" = 1 ] || { echo "NOBOOT — tail launch log:"; tail -30 /tmp/bm32_launch.log; exit 1; }
echo "  guest up."

echo "=== stage cup8.c into guest /tmp ==="
SCPG "$CUP8_SRC" ubuntu@localhost:/tmp/cup8.c
SCPG "$CUP8_RUN" ubuntu@localhost:/tmp/cup8_run_guest.sh

echo "=== run cup8 (CUP8_N=$N) in guest ==="
$SSHG "CUP8_N=$N bash /tmp/cup8_run_guest.sh" 2>&1 | tee /tmp/bm32_cup8.log

echo "=== host-side Xid check ==="
sudo dmesg 2>/dev/null | grep -iE "xid|nvrm" | tail -5 || echo "  (no dmesg access / none)"

echo "=== VERDICT ==="
if grep -qE "bad=0|RESULT.*PASS|byte-exact" /tmp/bm32_cup8.log; then
  echo "  PHASE0 PASS — 595 forwarding works. -> Phase 1 (LLM t/s)."
else
  echo "  PHASE0 FAIL/UNCLEAR — inspect /tmp/bm32_cup8.log (cuInit/alloc class = 595 ABI gap)."
fi
