#!/usr/bin/env bash
# setup_bm32.sh — RUN ON THE WORKSPACE (reaches both vast `vh` and bare-metal `.32`). Idempotent:
# stands up the Mode-2 stack on bare-metal box .32 (RTX 3050 = GA106) for the NON-NESTED baseline.
# Each stage skips if already done. Full plan/rationale: memory mode2_baremetal_32.md.
# Paths on .32 mirror vast exactly, so run_mode2_vm.sh works as-is with MEM=4G SMP=2 (4c/7.7Gi box).
set -u
B32=root@172.18.30.32
S32() { ssh -o ConnectTimeout=12 -o StrictHostKeyChecking=accept-new "$B32" "$@"; }
pipe_dir() { # $1=vast-parent $2=dir  $3=.32-parent  — stream a dir vast->.32 via workspace, skip if present
  S32 "test -e $3/$2" && { echo "  [skip] $3/$2 exists"; return; }
  echo "  [xfer] $1/$2 -> $B32:$3/$2"
  ssh vh "cd $1 && tar cf - $2" | S32 "mkdir -p $3 && cd $3 && tar xf -"
}

echo "===== S1: apt deps on .32 (idempotent) ====="
S32 'command -v qemu-system-x86_64 >/dev/null && command -v ninja >/dev/null && echo "  [ok] qemu+ninja present"' \
  || S32 'DEBIAN_FRONTEND=noninteractive apt-get install -y qemu-system-x86 qemu-utils ninja-build 2>&1 | tail -2'

echo "===== S2: (compression ABANDONED — image is ~28.8G incompressible CUDA libs + GGUF) ====="
# qemu-img -c only got 29G->18G and would fill vast disk. We stream the RAW qcow2 directly instead.

echo "===== S3: transfer to .32 (skip-if-present) ====="
pipe_dir /opt        qemu-nvkvm           /opt          # QEMU runtime (458M, portable)
pipe_dir /usr/lib/firmware/nvidia 580.159.04 /usr/lib/firmware/nvidia   # GSP firmware
pipe_dir /usr/src    nvidia-580.159.04    /usr/src      # open-driver source (9p-shared)
# guest base image (the big one): stream the RAW qcow2 vast->workspace->.32. NOT resumable; if it
# breaks, delete the partial on .32 and re-run. (Resumable alt: put vast key on .32 + rsync-pull,
# but that leaves a credential on the kiosk box — avoided per cleanup constraint.)
S32 'test -f /opt/nvkvm-guest/ubuntu-24.04.qcow2 && echo "  [skip] guest image present"' || {
  echo "  [xfer] guest image (RAW ~28.8G) vast -> .32 ..."
  SRC_SZ=$(ssh vh 'stat -c%s /opt/nvkvm-guest/ubuntu-24.04.qcow2')
  S32 'mkdir -p /opt/nvkvm-guest'
  ssh vh 'cat /opt/nvkvm-guest/ubuntu-24.04.qcow2' | S32 'cat > /opt/nvkvm-guest/ubuntu-24.04.qcow2'
  DST_SZ=$(S32 'stat -c%s /opt/nvkvm-guest/ubuntu-24.04.qcow2')
  [ "$SRC_SZ" = "$DST_SZ" ] || { echo "  [FAIL] size mismatch src=$SRC_SZ dst=$DST_SZ — partial; rm + re-run"; exit 1; }
  echo "  [ok] image transferred, size=$DST_SZ"
}
# small files
for f in seed.iso ga106_vbios.rom user-data meta-data; do
  S32 "test -f /opt/nvkvm-guest/$f" || ssh vh "cat /opt/nvkvm-guest/$f" | S32 "cat > /opt/nvkvm-guest/$f"
done
echo "  [xfer] repo scripts -> .32:/workspace/nvkvm"
rsync -az --delete /workspace/nvidia-gpu-passthrough/scripts/ "$B32:/workspace/nvkvm/scripts/" 2>/dev/null

echo "===== S3 verify ====="
S32 'echo "qemu: $(test -x /opt/qemu-nvkvm/bin/qemu-system-x86_64 && echo ok)"; \
     echo "img:  $(ls -lah /opt/nvkvm-guest/ubuntu-24.04.qcow2 2>/dev/null | awk "{print \$5}")"; \
     /opt/qemu-nvkvm/bin/qemu-img check /opt/nvkvm-guest/ubuntu-24.04.qcow2 2>&1 | tail -2; \
     echo "fw:   $(ls /usr/lib/firmware/nvidia/580.159.04/gsp_ga10x.bin 2>/dev/null && echo ok)"'

cat <<'NEXT'
===== NEXT (manual, on .32) =====
PHASE 0 (gate: does forwarding work vs host OPEN 595?):
  ssh root@172.18.30.32
  pkill -9 -f qemu-system-x86_64; sleep 2
  MEM=4G SMP=2 NVKVM_M2CEFWD=1 nohup bash /workspace/nvkvm/scripts/run_mode2_vm.sh >/tmp/m0_launch.log 2>&1 &
  # wait for guest ssh (port 2223), then run cup8 byte-exact:
  scp -P2223 .../cup8.c .../cup8_run_guest.sh ubuntu@localhost:/tmp/
  ssh -p2223 ubuntu@localhost 'CUP8_N=1024 bash /tmp/cup8_run_guest.sh'
  # PASS = CUP8 RESULT bad=0 + Xid=0  -> 595 forwarding works. FAIL on cuInit/alloc -> 595 ABI gap
  #   -> either extend abi_profile to 595, or install 580 open driver on .32 (invasive, reboot).
PHASE 1 (payoff): MEM=4G SMP=2 NVKVM_M2CEFWD=1 [+NVKVM_M2OPAQUE=1] LLM (llm_run_guest.sh) -> non-nested
  t/s. Compare to vast-nested (~22-36) + host-native on .32 (llama-cli -ngl 99 on the 3050 directly).
NEXT
