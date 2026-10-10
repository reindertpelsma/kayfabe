#!/usr/bin/env bash
# e1.sh <name> <qemu-bin> <kf3-props> <broker:none|pre> <shot-at-s|-> <paused-s|0> [extra env...]
# One boot of the early-frame experiment (V3_DISPLAY.md §8.18); prints one E1 line.
set -u
name=$1 Q=$2 props=$3 brk=$4 shot=$5 paused=$6
W=/var/lib/kf-windows-20261005/broker-interactive/e1-$(date +%H%M%S)-$name; mkdir -p "$W"; chmod 0711 "$W"
cp /usr/share/OVMF/OVMF_VARS_4M.fd "$W/vars.fd"
SOCK=/run/user/1000/nvkvm/display.sock
if [ "$brk" = pre ]; then
  mkfifo -m 0600 "$W/in"; chown ubuntu "$W/in"; exec 7<>"$W/in"
  install -d -o ubuntu -m 0700 /run/user/1000/nvkvm; rm -f "$SOCK"
  runuser -u ubuntu -- /opt/nvkvm-broker/nvkvm-display-broker --socket "$SOCK" --backend test --persist --verbose < "$W/in" > "$W/broker.log" 2>&1 &
  sleep 1; printf 'f 1\np 1\n' >&7
fi
S=(); [ "$paused" != 0 ] && S=(-S)
"$Q" -name kayfabe-e1 -object memory-backend-memfd,id=ram0,size=${RAM:-8192}M,share=on -machine q35,accel=kvm,memory-backend=ram0 -m ${RAM:-8192} -cpu host -smp 6 \
  -drive if=pflash,format=raw,unit=0,readonly=on,file=/usr/share/OVMF/OVMF_CODE_4M.fd -drive if=pflash,format=raw,unit=1,file="$W/vars.fd" \
  -drive if=virtio,file=/var/lib/kf-windows-20261005/broker-interactive/desktop.qcow2,format=qcow2 -snapshot \
  -netdev tap,id=n0,ifname=nvktap0,script=no,downscript=no -device virtio-net-pci,netdev=n0,mac=52:54:00:12:34:56,romfile= \
  -vga none -device "kf3-gpu,id=kf0,guest-driver=580.159.04,fb-mb=8192,bar1-size=134217728,bar2-size=33554432,x11-dispsw=on,$props" \
  -device virtio-keyboard-pci -device virtio-tablet-pci,display=kf0,head=0 -device virtio-mouse-pci \
  -display none -msg timestamp=on -serial file:"$W/serial.log" -monitor unix:"$W/mon",server,nowait "${S[@]}" > "$W/qemu.log" 2>&1 &
q=$!
t0=$(date +%s.%N)
mon(){ printf '%s\n' "$1" | timeout 5 socat - UNIX-CONNECT:"$W/mon" >/dev/null 2>&1; }
if [ "$shot" != - ]; then sleep "$shot"; mon "screendump $W/early.ppm kf0"; fi
if [ "$paused" != 0 ]; then sleep "$paused"; mon cont; fi
r=timeout
for i in $(seq 40); do
  grep -aq 'did not complete' "$W/qemu.log" && { r=STALL; break; }
  grep -aq 'TRACE scanout copy 1 done\|copies=[1-9]' "$W/qemu.log" && { r=LIVE; break; }
  grep -ao 'copies=[0-9.]*' "$W/qemu.log" | grep -vq 'copies=0.0$' && { r=LIVE; break; }
  kill -0 $q 2>/dev/null || { r=died; break; }
  sleep 1
done
sleep 2
mon "screendump $W/late.ppm kf0"; sleep 2
echo "E1 $name result=$r after=${i}s broker=$brk shot=$shot paused=$paused late_shot_nonblack=$(python3 -c "
import sys
d=open('$W/late.ppm','rb').read()
print(sum(1 for b in d[-3*1920*1080:] if b>16) if len(d)>100 else 'none')" 2>/dev/null) $(grep -a 'did not complete\|TRACE scanout copy [12] ' "$W/qemu.log" | sed 's/^[^ ]* //' | head -3 | tr '\n' '|' | cut -c1-300) dir=$W"
sleep ${HOLD:-0}; grep -ao "broker\[[^]]*\]" $W/qemu.log | tail -1 | cut -c1-220; grep -a "frames go as\|the display C" $W/qemu.log | sed "s/^[^ ]* //" | cut -c1-150; kill $q; sleep 3; kill -9 $q 2>/dev/null; wait $q 2>/dev/null
[ "$brk" = pre ] && { pkill -f "^/opt/nvkvm-broker/nvkvm-display-broker --socket $SOCK"; exec 7>&-; }
exit 0
