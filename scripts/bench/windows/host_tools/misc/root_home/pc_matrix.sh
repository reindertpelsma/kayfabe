#!/bin/bash
# Two arms, both as a SECOND vm (desktop VM on 2222 untouched).
G(){ timeout 60 sshpass -p "${KF_GUEST_SSH_PW:?set KF_GUEST_SSH_PW}" ssh -o ConnectTimeout=8 -o StrictHostKeyChecking=no -o UserKnownHostsFile=/dev/null -p 2322 ubuntu@localhost "$@" 2>&1; }

run_arm() {
  local name="$1"; local script="$2"; local extra_env="$3"; local log="$4"
  for p in $(ps -eo pid,args | grep "[q]emu-system-x86_64" | grep 2322 | awk '{print $1}'); do kill -9 $p 2>/dev/null; done
  sleep 3; rm -f "$log"
  cd /srv/nvkvm-pv
  env XDG_RUNTIME_DIR=/run/user/1000 WAYLAND_DISPLAY=wayland-0 \
    VM_IMG=/opt/nvkvm-guest/ubuntu-24.04.qcow2 VM_SSH_PORT=2322 \
    VM_DISPLAY="sdl,gl=on" VM_RELATIVE_MOUSE=1 VM_SERIAL=none VM_MEM=8G VM_SMP=4 \
    NVKVM_PRESENT_TIMING=1 $extra_env \
    setsid bash "$script" > "$log" 2>&1 < /dev/null &
  sleep 6
  echo "### ARM: $name"
  grep -oE "using [0-9]+ bits \(limit 0x[0-9a-f]+[^)]*\)" "$log" | head -1
  grep -oE "SPARSE WINDOW SHRUNK" "$log" | head -1
  for i in $(seq 1 40); do G "true" >/dev/null 2>&1 && break; sleep 6; done
  G "sudo cloud-init status --wait >/dev/null 2>&1; sudo mkdir -p /mnt/nvkvm; sudo mount -t 9p -o trans=virtio,version=9p2000.L,msize=512000 nvkvm_src /mnt/nvkvm >/dev/null 2>&1; sudo bash /mnt/nvkvm/scripts/stage_guest_libs.sh >/dev/null 2>&1; lsmod|grep -c nvkvm" >/dev/null
  G "sudo mkdir -p /run/user/0; sudo chmod 700 /run/user/0; sudo setsid seatd >/tmp/seatd.log 2>&1 </dev/null & sleep 2; sudo setsid env XDG_RUNTIME_DIR=/run/user/0 weston --backend=drm-backend.so >/tmp/weston.log 2>&1 </dev/null & sleep 14; grep -c 'GL renderer' /tmp/weston.log" >/dev/null
  G "sudo setsid env XDG_RUNTIME_DIR=/run/user/0 WAYLAND_DISPLAY=wayland-1 weston-simple-egl >/tmp/se.log 2>&1 </dev/null & sleep 4; true" >/dev/null
  sleep 70
  echo "  mode:   $(grep -oE 'display mode = [^;]*' "$log" | head -1)"
  echo "  stats:  $(grep 'disp stats' "$log" | tail -1)"
  echo "  KVM_RUN_FAILED = $(grep -c 'kvm run failed' "$log")"
  echo "  guest:  $(G 'uptime' | tail -1)"
  echo
}

run_arm "A: 39-bit phys + native GL"   /srv/nvkvm-pv/scripts/run_39bit.sh              ""                              /tmp/armA.log
run_arm "B: 48-bit phys + READBACK"    /srv/nvkvm-pv/scripts/run_test_vm.sh "NVKVM_PRESENT_MODE=readback" /tmp/armB.log
echo "desktop VM untouched: $(ps -eo args | grep '[q]emu-system-x86_64' | grep -c 2222)"
