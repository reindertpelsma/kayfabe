#!/usr/bin/env bash
# planes_oracle.sh <kf3-rev> <tag> — the display-plane oracle on the trusted host
# (docs/design/V3_DISPLAY_PLANES_ORACLE.md). One Linux guest (the provisioned desktop image, a
# throw-away qcow2 child), kf3 with display=on and NO broker, small RAM, its OWN lock, its own QEMU
# name (kf-planes-oracle) and pid file: it never touches another VM, never takes the shared
# fast-guest lock, and tears down only its own pid.
#   1. boots, stops the guest's display manager (kfplanes must be the DRM master)
#   2. builds kfplanes in the guest (falls back to a host build copied in)
#   3. kfplanes list / all <n>; then `show` rgb|nv12|nv12-scaled|p010|yuyv with a host screendump of
#      the kf3 console each, compared with the reference PPM (kfplanes ppm, built on the host)
#   4. collects the guest dmesg slice, kf3's display lines, the host Xid delta; ACPI powerdown
# env: KF_RAM_MB (4096), KF_FB_MB (2048), PLANES_N (120), PLANES_EXTRA_KF3 (more kf3 properties),
#      KF3_BINS (/var/lib/kf-windows-20261005/kf3-bins), KF_BASE_IMG (desktop.qcow2), KF_ENV (env for QEMU)
# output: $OUT (default /var/lib/kf-windows-20261005/claude-planes-<tag>/), PLANES_* lines on stdout,
#         a start marker and a PLANES_EXIT line.
set -uo pipefail
REV=${1:?usage: planes_oracle.sh <kf3-rev> <tag>}; TAG=${2:?tag}
HERE="$(cd "$(dirname "$0")" && pwd)"
W=/var/lib/kf-windows-20261005
BINS=${KF3_BINS:-$W/kf3-bins}
Q=$BINS/$REV/qemu-system-x86_64
BASE=${KF_BASE_IMG:-$W/broker-interactive/desktop.qcow2}
OUT=${PLANES_OUT:-$W/claude-planes-$TAG}
KEY=/workspace/bench/guest_key
N=${PLANES_N:-120}
G=(ssh -i "$KEY" -o StrictHostKeyChecking=no -o UserKnownHostsFile=/dev/null -o LogLevel=ERROR
   -o ConnectTimeout=5 -o ServerAliveInterval=10 -o ServerAliveCountMax=6 ubuntu@192.168.77.2)
say(){ echo "PLANES_$*"; }
[ -x "$Q" ] || { say "FAIL no kf3 binary at $Q"; exit 2; }
[ -e "$OUT" ] && { say "FAIL $OUT exists (one directory per tag)"; exit 2; }
mkdir -p "$OUT"
exec 9>"$OUT/private.lock"; flock -n 9 || { say "FAIL lock"; exit 2; }
# the tap is shared by every fat-guest launcher on this host: refuse when any QEMU uses it
if ps -eo args | grep -v grep | grep -q 'ifname=nvktap0'; then say "FAIL nvktap0 is in use by another QEMU"; exit 2; fi
ip link set nvktap0 up 2>/dev/null
START=$(date -Is); echo "PLANES_START tag=$TAG rev=$REV $START" | tee "$OUT/START"
xid0=$(journalctl -k --no-pager 2>/dev/null | grep -c 'NVRM: Xid')
owner=$(pgrep -f 'name boundary-kayfabe' | head -1)
say "OWNER_QEMU pid=${owner:-none} (never touched)"
free -m | sed -n 2p | sed 's/^/PLANES_HOST_MEM_BEFORE /'

qemu-img create -q -f qcow2 -F qcow2 -b "$BASE" "$OUT/disk.qcow2" || { say "FAIL overlay"; exit 2; }
cp /usr/share/OVMF/OVMF_VARS_4M.fd "$OUT/ovmf_vars.fd"
RAM=${KF_RAM_MB:-4096}
# shellcheck disable=SC2054
QARGS=(-name kf-planes-oracle
  -object "memory-backend-memfd,id=ram0,size=${RAM}M,share=on"
  -machine "q35,accel=kvm,memory-backend=ram0" -m "$RAM" -cpu host -smp 4
  -drive "if=pflash,format=raw,unit=0,readonly=on,file=/usr/share/OVMF/OVMF_CODE_4M.fd"
  -drive "if=pflash,format=raw,unit=1,file=$OUT/ovmf_vars.fd"
  -drive "if=virtio,file=$OUT/disk.qcow2,format=qcow2"
  -netdev tap,id=n0,ifname=nvktap0,script=no,downscript=no
  -device virtio-net-pci,netdev=n0,mac=52:54:00:12:34:57,romfile=
  -vga none
  -device "kf3-gpu,id=kf0,guest-driver=580.159.04,fb-mb=${KF_FB_MB:-2048},bar1-size=134217728,bar2-size=33554432,display=on,gop=on${PLANES_EXTRA_KF3:+,$PLANES_EXTRA_KF3}"
  -display none -msg timestamp=on
  -serial "file:$OUT/serial.log" -monitor "unix:$OUT/qemu.mon,server,nowait")
# shellcheck disable=SC2086
env ${KF_ENV:-} "$Q" "${QARGS[@]}" > "$OUT/qemu.log" 2>&1 &
QPID=$!
echo "$QPID" > "$OUT/qemu.pid"
say "QEMU pid=$QPID"
finish(){
    rc=$?
    if kill -0 "$QPID" 2>/dev/null; then
        printf 'system_powerdown\n' | timeout 5 socat - "UNIX-CONNECT:$OUT/qemu.mon" >/dev/null 2>&1
        for _ in $(seq 60); do kill -0 "$QPID" 2>/dev/null || break; sleep 1; done
        kill -0 "$QPID" 2>/dev/null && { kill -9 "$QPID"; say "QEMU $QPID killed after 60 s"; }
    fi
    xid1=$(journalctl -k --no-pager 2>/dev/null | grep -c 'NVRM: Xid')
    journalctl -k --since "$START" --no-pager 2>/dev/null | grep 'NVRM: Xid' > "$OUT/host-xid.log"
    say "HOST_XID delta=$((xid1 - xid0)) (journal since start: $(wc -l < "$OUT/host-xid.log"))"
    say "OWNER_ALIVE $( [ -n "$owner" ] && kill -0 "$owner" 2>/dev/null && echo yes || echo n/a)"
    echo "PLANES_EXIT rc=$rc $(date -Is)" | tee "$OUT/EXIT"
}
trap finish EXIT

ready=0
for _ in $(seq 1 100); do
    kill -0 "$QPID" 2>/dev/null || { say "FAIL QEMU exited"; tail -20 "$OUT/qemu.log"; exit 3; }
    if timeout 8 "${G[@]}" true 2>/dev/null; then ready=1; break; fi
    sleep 3
done
[ "$ready" = 1 ] || { say "FAIL guest ssh never came up"; exit 3; }
say "GUEST_UP $(date -Is)"
gq(){ timeout "${2:-120}" "${G[@]}" "$1" 2>&1 | tr -d '\r'; }
gq 'uname -r; cat /proc/driver/nvidia/version | head -1; cat /sys/module/nvidia_drm/parameters/modeset 2>&1' > "$OUT/guest-id.log"
sed 's/^/PLANES_GUEST_ID /' "$OUT/guest-id.log"
# stop the display manager (and anything holding DRM master), keep nvidia-drm loaded
gq 'for s in display-manager lightdm gdm gdm3 sddm; do sudo systemctl stop $s 2>/dev/null; done; sleep 3; sudo pkill -x Xorg; sleep 2; pgrep -a -x Xorg || echo no-xorg' > "$OUT/stop-dm.log"
say "STOP_DM $(tr '\n' ' ' < "$OUT/stop-dm.log")"
d0=$(gq 'sudo dmesg | wc -l'); d0=${d0:-0}
# build kfplanes in the guest; else on the host and copy the binary in
tar -C "$HERE" -cf - kfplanes.c | "${G[@]}" 'mkdir -p ~/planes && tar -xf - -C ~/planes'
gq 'cd ~/planes && gcc -O2 -Wall -o kfplanes kfplanes.c $(pkg-config --cflags --libs libdrm) -lm && echo BUILD_OK' > "$OUT/build.log"
if ! grep -q BUILD_OK "$OUT/build.log"; then
    gcc -O2 -Wall -I/usr/include/libdrm -o "$OUT/kfplanes" "$HERE/kfplanes.c" -ldrm -lm \
      && "${G[@]}" 'cat > ~/planes/kfplanes && chmod +x ~/planes/kfplanes' < "$OUT/kfplanes" && echo HOST_BUILD_COPIED >> "$OUT/build.log"
fi
say "BUILD $(tail -1 "$OUT/build.log")"
gcc -O2 -Wall -DKFPLANES_NO_DRM -o "$OUT/kfplanes_ppm" "$HERE/kfplanes.c" -lm
card=$(gq 'for c in /sys/class/drm/card[0-9]*; do [ -e "$c/device/driver" ] && basename "$(readlink -f $c/device/driver)" | grep -qx nvidia && { echo /dev/dri/$(basename $c); break; }; done')
card=${card:-/dev/dri/card0}
gq "sudo ~/planes/kfplanes list $card" 60 > "$OUT/list.log"
grep '^KFPLANES_' "$OUT/list.log" | sed 's/^/PLANES_/'
gq "sudo ~/planes/kfplanes all $card $N" 900 > "$OUT/all.log"
grep '^KFPLANES_RESULT\|^KFPLANES_ENABLE\|^KFPLANES_DISABLE\|^KFPLANES_TIMEOUT\|^KFPLANES_DONE\|^KFPLANES_FAIL\|^KFPLANES_MODESET' "$OUT/all.log" | sed 's/^/PLANES_/'

# show + screendump + compare (the console composition of each overlay format)
mode=$(grep -o 'mode=[0-9]*x[0-9]*' "$OUT/list.log" | head -1 | cut -d= -f2)
MW=${mode%x*}; MH=${mode#*x}
for what in rgb nv12 nv12-scaled p010 yuyv; do
    ( gq "sudo ~/planes/kfplanes show $card $what 8" 40 > "$OUT/show-$what.log" ) &
    sp=$!
    for _ in $(seq 40); do grep -q 'KFPLANES_SHOWING\|KFPLANES_SHOW_FAIL' "$OUT/show-$what.log" 2>/dev/null && break; sleep 0.5; done
    if grep -q 'KFPLANES_SHOWING' "$OUT/show-$what.log"; then
        sleep 2
        printf 'screendump %s kf0\n' "$OUT/shot-$what.ppm" | timeout 10 socat - "UNIX-CONNECT:$OUT/qemu.mon" >/dev/null 2>&1
        sleep 2
        "$OUT/kfplanes_ppm" ppm "$MW" "$MH" "$what" > "$OUT/ref-$what.ppm"
        python3 -I "$HERE/planes_compare.py" "$OUT/shot-$what.ppm" "$OUT/ref-$what.ppm" "$what" > "$OUT/cmp-$what.log" 2>&1
        sed 's/^/PLANES_/' "$OUT/cmp-$what.log"
    else
        say "SHOW what=$what $(tr '\n' ' ' < "$OUT/show-$what.log" | cut -c1-200)"
    fi
    wait "$sp"
done

gq "sudo dmesg | tail -n +$((d0 + 1))" > "$OUT/guest-dmesg.log"
say "GUEST_FLIP_EVENT_TIMEOUTS=$(grep -c 'Flip event timeout' "$OUT/guest-dmesg.log") LUT_TIMEOUTS=$(grep -c 'LUT notifier timeout' "$OUT/guest-dmesg.log") NVASSERT=$(grep -c -i 'nvAssert\|assert' "$OUT/guest-dmesg.log") GPU_PROGRESS=$(grep -c 'waiting for GPU progress' "$OUT/guest-dmesg.log") XID=$(grep -c 'Xid' "$OUT/guest-dmesg.log")"
grep -a 'display' "$OUT/qemu.log" | grep -a -E 'STALL|Exception|REFUSED|refused|left out|halt|failed' | head -40 > "$OUT/kf3-display-flags.log"
say "KF3_DISPLAY_FLAGS lines=$(wc -l < "$OUT/kf3-display-flags.log") stall=$(grep -c STALL "$OUT/kf3-display-flags.log") exception=$(grep -c Exception "$OUT/kf3-display-flags.log") refused=$(grep -c -i refused "$OUT/kf3-display-flags.log")"
exit 0
