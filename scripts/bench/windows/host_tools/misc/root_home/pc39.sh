#!/bin/bash
G(){ timeout 60 sshpass -p "${KF_GUEST_SSH_PW:?set KF_GUEST_SSH_PW}" ssh -o ConnectTimeout=8 -o StrictHostKeyChecking=no -o UserKnownHostsFile=/dev/null -p 2322 ubuntu@localhost "$@" 2>&1; }
rm -f /tmp/pc39.log
cd /srv/nvkvm-pv
env XDG_RUNTIME_DIR=/run/user/1000 WAYLAND_DISPLAY=wayland-0 \
  VM_IMG=/opt/nvkvm-guest/ubuntu-24.04.qcow2 VM_SSH_PORT=2322 \
  VM_DISPLAY="sdl,gl=on" VM_RELATIVE_MOUSE=1 VM_SERIAL=none VM_MEM=8G VM_SMP=4 \
  NVKVM_PRESENT_TIMING=1 \
  setsid bash /root/run_39bit.sh > /tmp/pc39.log 2>&1 < /dev/null &
sleep 5
echo "=== GPA config (want: 39 bits + SHRUNK) ==="
grep -E "GPA width|GPA windows" /tmp/pc39.log | head -3
for i in $(seq 1 40); do G "true" >/dev/null 2>&1 && break; sleep 6; done
echo "guest up (~$((i*6))s)"
G "sudo cloud-init status --wait >/dev/null 2>&1; sudo mkdir -p /mnt/nvkvm; sudo mount -t 9p -o trans=virtio,version=9p2000.L,msize=512000 nvkvm_src /mnt/nvkvm 2>&1|tail -1; sudo bash /mnt/nvkvm/scripts/stage_guest_libs.sh 2>&1|tail -1"
G "lsmod | grep -c nvkvm; ls /dev/dri 2>&1 | tr '\n' ' '"
G "sudo mkdir -p /run/user/0; sudo chmod 700 /run/user/0; sudo setsid seatd >/tmp/seatd.log 2>&1 </dev/null & sleep 2; sudo setsid env XDG_RUNTIME_DIR=/run/user/0 weston --backend=drm-backend.so >/tmp/weston.log 2>&1 </dev/null & sleep 14; grep -E 'GL renderer|using /dev/dri' /tmp/weston.log|head -2"
G "sudo setsid env XDG_RUNTIME_DIR=/run/user/0 WAYLAND_DISPLAY=wayland-1 weston-simple-egl >/tmp/se.log 2>&1 </dev/null & sleep 4; ps -eo comm|grep -c weston-simple"
echo "--- soaking 70s ---"; sleep 70
echo "=== RESULT (39-bit, native display, RTX 4070) ==="
echo "kvm_run_failed=$(grep -c 'kvm run failed' /tmp/pc39.log)"
grep -E "display mode" /tmp/pc39.log | head -1
grep "disp stats" /tmp/pc39.log | tail -2
echo "guest: $(G 'uptime' | tail -1)"
