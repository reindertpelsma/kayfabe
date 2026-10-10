#!/usr/bin/env bash
# A/B: which kf3 display config stalls the scanout copy on this host (595.91.07)
set -u
W=/var/lib/kf-windows-20261005/broker-interactive/ab-$(date +%H%M%S); mkdir -p $W
Q=${QBIN:-/workspace/bench/kf3-bins/0e64a960/qemu-system-x86_64}
exec 9>/tmp/kayfabe-fastguest.lock; flock -w 60 9 || { echo lock; exit 1; }
echo "AB_START $(date -Is) qemu=$Q"
for cfg in "${@}"; do
  name=${cfg%%:*}; props=${cfg#*:}
  d=$W/$name; mkdir -p $d; cp /usr/share/OVMF/OVMF_VARS_4M.fd $d/vars.fd
  bsz=${name#*@}; [ "$bsz" = "$name" ] && bsz=""
  delay=0; case "$bsz" in *~*) delay=${bsz#*~}; bsz=${bsz%~*};; esac
  start_broker(){
    chmod 0711 $W $d; mkfifo -m 0600 $d/in; chown ubuntu $d/in; exec 7<>$d/in
    install -d -o ubuntu -m 0700 /run/user/1000/nvkvm; rm -f /run/user/1000/nvkvm/display.sock
    runuser -u ubuntu -- /opt/nvkvm-broker/nvkvm-display-broker --socket /run/user/1000/nvkvm/display.sock --backend test --persist --verbose --size $bsz < $d/in > $d/broker.log 2>&1 &
    sleep 1; printf 'f 1\np 1\n' >&7
  }
  [ -n "$bsz" ] && [ "$delay" = 0 ] && start_broker
  $Q -name kayfabe-ab -object memory-backend-memfd,id=ram0,size=8192M,share=on -machine q35,accel=kvm,memory-backend=ram0 -m 8192 -cpu host -smp 6 \
    -drive if=pflash,format=raw,unit=0,readonly=on,file=/usr/share/OVMF/OVMF_CODE_4M.fd -drive if=pflash,format=raw,unit=1,file=$d/vars.fd \
    -drive if=virtio,file=/var/lib/kf-windows-20261005/broker-interactive/desktop.qcow2,format=qcow2 -snapshot \
    -netdev tap,id=n0,ifname=nvktap0,script=no,downscript=no -device virtio-net-pci,netdev=n0,mac=52:54:00:12:34:56,romfile= \
    -vga none -device "kf3-gpu,id=kf0,guest-driver=580.159.04,fb-mb=8192,bar1-size=134217728,bar2-size=33554432,x11-dispsw=on,$props" \
    -device virtio-keyboard-pci -device virtio-tablet-pci,display=kf0,head=0 -device virtio-mouse-pci \
    -display none -msg timestamp=on -serial file:$d/serial.log -monitor unix:$d/mon,server,nowait > $d/qemu.log 2>&1 &
  q=$!
  r=timeout
  [ -n "$bsz" ] && [ "$delay" != 0 ] && { sleep "$delay"; start_broker; echo "AB_LATE broker started after ${delay}s; copies so far: $(grep -ao 'copies=[0-9.]*' $d/qemu.log | tail -1)"; }
  for i in $(seq 120); do
    if grep -aq 'did not complete' $d/qemu.log; then r=STALL; break; fi
    if grep -ao 'copies=[0-9.]*' $d/qemu.log | grep -vq 'copies=0.0$'; then r=COPIES; break; fi
    kill -0 $q 2>/dev/null || { r=died; break; }
    sleep 1
  done
  sleep 3
  printf 'screendump %s/shot.ppm kf0\n' $d | timeout 5 socat - UNIX-CONNECT:$d/mon >/dev/null 2>&1; sleep 2
  echo "AB $name props=[$props] result=$r after=${i}s $(grep -ao 'display fps\[[^]]*\]' $d/qemu.log | tail -1) $(grep -a -m1 'did not complete' $d/qemu.log | sed 's/^[^ ]* //') shot=$(stat -c %s $d/shot.ppm 2>/dev/null) progress_err=$(grep -ac 'waiting for GPU progress' $d/serial.log)"
  kill $q; sleep 3; kill -9 $q 2>/dev/null; wait $q 2>/dev/null
  if [ -n "$bsz" ]; then pkill -f "^/opt/nvkvm-broker/nvkvm-display-broker --socket /run/user/1000/nvkvm/display.sock"; exec 7>&-; echo "AB_BROKER $name $(grep -a -m1 'client attached' $d/broker.log | cut -c1-120)"; fi
done
echo "AB_EXIT $(date -Is)"
