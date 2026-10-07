#!/usr/bin/env bash
# rtp_run_guest.sh — ON THE GUEST. cudart-vs-driver-API isolation on ONE pinned,
# single-init adapter (memory mode2_execfwd_layer2 CORRECTION 3). Multi-step so the
# host harness can snapshot the QEMU GSP-RPC (M4) log between the driver-API probe and
# the cudart probe and DIFF them — the cudart blocker is an RM reply whose in-payload
# status libcudart rejects, invisible to strace/dmesg but visible in the M4 RPC log.
#
# Subcommands (each its own ssh from the host; kernel + the held-fd pin persist between):
#   setup : isolate->rmmod->insmod nvidia+uvm, mknods, PIN the adapter (held fd =>
#           ONE GSP init for the whole session, no WPR2 reload churn), build dvp + rtp.
#   dvp   : run the driver-API probe (cuInit/ctx/enumerate) — the WORKING baseline.
#   rtp   : run the cudart probe (cudaGetDeviceCount) — the FAILING path.
#   dmesg : dump guest RM/UVM/assert lines (want: clean for both, per CORRECTION 3).
set -u
NVMODS=/home/ubuntu/nvmods
GUESTLIB=/usr/local/nvidia-guest/lib
LLMLIB=$HOME/llm/lib            # CUDA-12 cudart stack (libcudart.so.12) used by llama
PINFD=/tmp/rtp_pin.pid

stage_fw(){
  if [ ! -f /lib/firmware/nvidia/580.159.04/gsp_ga10x.bin ]; then
    sudo mkdir -p /lib/firmware/nvidia/580.159.04 /mnt/nvfw
    sudo mount -t 9p -o trans=virtio,version=9p2000.L,ro nvfw /mnt/nvfw 2>/dev/null || true
    sudo cp -n /mnt/nvfw/*.bin /lib/firmware/nvidia/580.159.04/ 2>/dev/null || true
    sudo umount /mnt/nvfw 2>/dev/null || true
  fi
}

case "${1:-setup}" in
  setup)
    sudo systemctl isolate multi-user.target 2>/dev/null || true
    sleep 2
    sudo rmmod nvidia_uvm nvidia nvkvm_guest 2>/dev/null || true
    sudo modprobe ecdh_generic ecc 2>/dev/null || true
    sudo dmesg -C || true
    stage_fw
    sudo insmod "$NVMODS/nvidia.ko" NVreg_EnableGpuFirmware=1 NVreg_RegistryDwords="RmGspBootRetryAttempts=1" 2>&1 | tail -1 || true
    sudo insmod "$NVMODS/nvidia-uvm.ko" 2>&1 | tail -1 || true
    UVM_MAJ=$(awk '$2=="nvidia-uvm"{print $1}' /proc/devices)
    sudo mknod /dev/nvidia0 c 195 0 2>/dev/null || true
    sudo mknod /dev/nvidiactl c 195 255 2>/dev/null || true
    if [ -n "$UVM_MAJ" ]; then
      sudo rm -f /dev/nvidia-uvm /dev/nvidia-uvm-tools
      sudo mknod /dev/nvidia-uvm c "$UVM_MAJ" 0 2>/dev/null || true
      sudo mknod /dev/nvidia-uvm-tools c "$UVM_MAJ" 1 2>/dev/null || true
    fi
    sudo chmod 666 /dev/nvidia* /dev/nvidiactl 2>/dev/null || true
    sudo ln -sf "$GUESTLIB/libcuda.so.580.159.04" /lib/x86_64-linux-gnu/libcuda.so.1
sudo ln -sf "$GUESTLIB/libcuda.so.580.159.04" /usr/lib/x86_64-linux-gnu/libcuda.so 2>/dev/null; sudo ldconfig 2>/dev/null
    # PIN: hold /dev/nvidia0 open in a detached process => first+only RmInitAdapter,
    # held across the dvp and rtp ssh sessions, no last-client-close de-init/WPR2 churn.
    setsid bash -c 'exec 9<>/dev/nvidia0; echo $$ > '"$PINFD"'; sleep 900' >/dev/null 2>&1 &
    sleep 2
    echo "pin pid=$(cat $PINFD 2>/dev/null) fd-held=$(sudo ls -l /proc/$(cat $PINFD 2>/dev/null)/fd 2>/dev/null | grep -c nvidia0)"
    gcc -O0 -g -o /tmp/dvp /tmp/reload_probe.c -lcuda -L"$GUESTLIB" 2>&1 | tail -2
    gcc -O0 -g -o /tmp/rtp /tmp/rtp.c -lcudart -L"$LLMLIB" 2>&1 | tail -2
    [ -x /tmp/dvp ] && echo "dvp built" || echo "DVP BUILD FAILED"
    [ -x /tmp/rtp ] && echo "rtp built" || echo "RTP BUILD FAILED"
    echo "lsmod nvidia: $(lsmod | grep -E '^nvidia ' | awk '{print $1,$3}')"
    echo SETUP_DONE
    ;;
  dvp)
    sudo dmesg -C || true
    echo "=== DVP (driver-API) ==="
    LD_LIBRARY_PATH="$GUESTLIB" timeout 60 stdbuf -oL -eL /tmp/dvp
    echo "=== DVP rc=$? ==="
    echo "--- DVP guest dmesg ---"; sudo dmesg | grep -iE "NVRM|nvidia|uvm|WPR2|Booter|assert|Xid|POST_EVENT|CliGetEvent" | tail -25
    ;;
  rtp)
    sudo dmesg -C || true
    echo "=== RTP (cudart) ==="
    LD_LIBRARY_PATH="$LLMLIB" timeout 60 stdbuf -oL -eL /tmp/rtp
    echo "=== RTP rc=$? ==="
    echo "--- RTP guest dmesg (CORRECTION 3 says this is EMPTY) ---"; sudo dmesg | grep -iE "NVRM|nvidia|uvm|WPR2|Booter|assert|Xid|POST_EVENT|CliGetEvent" | tail -25
    ;;
  llm)
    # Run llama.cpp on the ALREADY-pinned, already-loaded adapter (NO rmmod/reinsmod,
    # so no 2nd-init WPR2 churn — the documented reload bug is sidestepped by the pin).
    LLM=$HOME/llm
    MODEL=${LLM_MODEL:-$LLM/qwen.gguf}
    NGEN=${LLM_NGEN:-32}
    TIMEOUT=${LLM_TIMEOUT:-180}
    PROMPT=${LLM_PROMPT:-Explain in two sentences why GPU virtualization is useful for cloud computing.}
    sudo dmesg -C || true
    echo "=== llama-cli (ngl 99, n=$NGEN, timeout ${TIMEOUT}s) on PINNED adapter ==="
    LD_LIBRARY_PATH="$LLM/lib" timeout --signal=INT "$TIMEOUT" stdbuf -oL -eL \
      "$LLM/llama-cli" -m "$MODEL" -ngl 99 -c 2048 -n "$NGEN" -st -p "$PROMPT" 2>&1
    echo "=== llm exit rc=$? (124=timeout/hang) ==="
    echo "--- llm guest dmesg ---"; sudo dmesg | grep -iE "NVRM|nvidia|uvm|WPR2|Booter|assert|Xid" | tail -15
    ;;
  cup5)
    # Bulk data-plane forcing function on the ALREADY-pinned adapter (driver API).
    sudo dmesg -C || true
    gcc -O2 -o /tmp/cup5 /tmp/cup5.c -lcuda -L"$GUESTLIB" 2>&1 | tail -3
    [ -x /tmp/cup5 ] || { echo "CUP5 BUILD FAILED"; exit 0; }
    echo "=== cup5 CUP5_MB=${CUP5_MB:-64} on PINNED adapter ==="
    LD_LIBRARY_PATH="$GUESTLIB" CUP5_MB="${CUP5_MB:-64}" timeout "${CUP5_TIMEOUT:-120}" stdbuf -oL -eL /tmp/cup5
    echo "=== cup5 rc=$? (124=timeout/hang) ==="
    echo "--- cup5 guest dmesg ---"; sudo dmesg | grep -iE "NVRM|nvidia|uvm|WPR2|Booter|assert|Xid" | tail -10
    ;;
  cup6)
    # DISCRIMINATOR: 3 timed HtoD to the SAME buffer + 25s hold (read pat memtype
    # live). UC-persistent => all 3 slow; first-touch fault/residency => #2/#3 fast.
    sudo dmesg -C || true
    gcc -O2 -o /tmp/cup6 /tmp/cup6.c -lcuda -L"$GUESTLIB" 2>&1 | tail -3
    [ -x /tmp/cup6 ] || { echo "CUP6 BUILD FAILED"; exit 0; }
    echo "=== cup6 CUP6_MB=${CUP6_MB:-64} on PINNED adapter (root: pagemap PFNs) ==="
    # Run as root via `sudo env` so cup6's self-introspection resolves real PFNs from
    # /proc/self/pagemap (unprivileged reads zero the PFN). The held-fd pin keeps the
    # single adapter init; root opening another /dev/nvidia0 fd joins the same init.
    sudo env LD_LIBRARY_PATH="$GUESTLIB" CUP6_MB="${CUP6_MB:-64}" \
      timeout "${CUP6_TIMEOUT:-120}" stdbuf -oL -eL /tmp/cup6 &
    CP6=$!
    sleep 10
    echo "--- pat_memtype_list: UC-/WC/WB summary while buffer held ---"
    sudo cat /sys/kernel/debug/x86/pat_memtype_list 2>/dev/null | grep -oE "uncached-minus|write-combining|write-back" | sort | uniq -c
    echo "--- pat_memtype_list: FULL (match dp phys0/physmid to a memtype line) ---"
    sudo cat /sys/kernel/debug/x86/pat_memtype_list 2>/dev/null
    wait $CP6
    echo "=== cup6 rc=$? (124=timeout/hang) ==="
    echo "--- cup6 guest dmesg ---"; sudo dmesg | grep -iE "NVRM|nvidia|uvm|WPR2|Booter|assert|Xid" | tail -10
    ;;
  dmesg)
    sudo dmesg | grep -iE "NVRM|nvidia|uvm|WPR2|Booter|assert|Xid|POST_EVENT|CliGetEvent" | tail -40
    ;;
esac
