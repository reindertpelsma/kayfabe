#!/usr/bin/env bash
# planes_hw_reference.sh — the BARE-METAL reference for the plane oracle (V3_DISPLAY_PLANES_ORACLE.md),
# run INSIDE a rented vast KVM VM that owns a real NVIDIA GPU by VFIO (no monitor attached).
#   1. installs the same open driver the kayfabe Linux guest runs (580.159.04, kernel-open)
#   2. loads nvidia-drm modeset=1, FORCES one HDMI/DVI connector on with an EDID override (the kayfabe
#      guest's own EDID, kayfabe-authored, so both sides validate the same 1080p60 mode): drm's
#      runtime edid_firmware parameter + the connector's sysfs `status` = on
#   3. kfplanes list / all — the same binary source and arguments as the kayfabe run
# Untrusted box: no secrets, text results only. Run as root. Evidence: /root/planes-hw/.
# env: NV_VER (580.159.04), PLANES_N (120)
set -uo pipefail
NV_VER=${NV_VER:-580.159.04}
OUT=/root/planes-hw; mkdir -p "$OUT"
HERE="$(cd "$(dirname "$0")" && pwd)"
say(){ echo "HW_$*" | tee -a "$OUT/summary.log"; }
say "START $(date -Is) kernel=$(uname -r) gpu=$(lspci -nn | grep -i 'vga\|3d' | grep -i nvidia | head -1 | cut -c1-120)"
if ! lsmod | grep -q '^nvidia '; then
    export DEBIAN_FRONTEND=noninteractive
    apt-get update -qq >/dev/null 2>&1
    apt-get install -y -qq build-essential "linux-headers-$(uname -r)" pkg-config libdrm-dev wget kmod >/dev/null 2>&1
    f=NVIDIA-Linux-x86_64-$NV_VER.run
    [ -f "/root/$f" ] || wget -q -O "/root/$f" "https://download.nvidia.com/XFree86/Linux-x86_64/$NV_VER/$f"
    modprobe -r nouveau 2>/dev/null
    sh "/root/$f" --silent --kernel-module-type=open --no-questions --ui=none --no-x-check --no-nouveau-check > "$OUT/install.log" 2>&1
    say "INSTALL rc=$? $(tail -2 "$OUT/install.log" | tr '\n' ' ' | cut -c1-200)"
fi
modprobe -r nvidia_drm 2>/dev/null
modprobe nvidia-drm modeset=1 fbdev=1
sleep 2
say "DRIVER $(head -1 /proc/driver/nvidia/version | cut -c1-120) modeset=$(cat /sys/module/nvidia_drm/parameters/modeset)"
card=""
for c in /sys/class/drm/card[0-9]*; do
    [ -e "$c/device/driver" ] && basename "$(readlink -f "$c/device/driver")" | grep -qx nvidia && { card=$(basename "$c"); break; }
done
say "CARD $card connectors: $(ls -d /sys/class/drm/$card-* 2>/dev/null | xargs -n1 basename | tr '\n' ' ')"
# the EDID: the kayfabe guest's DVI-D monitor (1920x1080@60, 148.5 MHz), base64 from the kayfabe run
mkdir -p /lib/firmware/edid
base64 -d > /lib/firmware/edid/kf.bin <<'EOF'
AP///////wAswgEAAQAAACckAQShGRl4Bu6Ro1RMmSYPUFQhCACBgKnAswABAQEBAQEBAQEBAjqAGHE4LUBYLEUA/B0RAAAeAAAA/QAYSw+gEQAKICAgICAgAAAA/ABrYXlmYWJlCiAgICAgAAAAEAAAAAAAAAAAAAAAAAAAAIM=
EOF
conn=""
for pref in HDMI-A DVI-D DP; do
    conn=$(ls -d /sys/class/drm/$card-$pref-* 2>/dev/null | head -1)
    [ -n "$conn" ] && break
done
cname=$(basename "$conn"); cname=${cname#"$card"-}
echo "$cname:edid/kf.bin" > /sys/module/drm/parameters/edid_firmware 2>>"$OUT/force.log"
echo on > "$conn/status" 2>>"$OUT/force.log"
sleep 2
say "FORCED $cname status=$(cat "$conn/status") enabled=$(cat "$conn/enabled") edid_bytes=$(wc -c < "$conn/edid") modes=$(head -3 "$conn/modes" | tr '\n' ' ') err=$(tr '\n' ' ' < "$OUT/force.log")"
gcc -O2 -Wall -o "$OUT/kfplanes" "$HERE/kfplanes.c" $(pkg-config --cflags --libs libdrm) -lm 2>"$OUT/build.log" || { say "BUILD_FAIL $(head -3 "$OUT/build.log")"; exit 3; }
"$OUT/kfplanes" list "/dev/dri/$card" > "$OUT/list.log" 2>&1
grep '^KFPLANES_' "$OUT/list.log" | grep -v 'KFPLANES_PLANE' | sed 's/^/HW_/' | tee -a "$OUT/summary.log"
grep 'KFPLANES_PLANE' "$OUT/list.log" | head -4 | sed 's/^/HW_/' | tee -a "$OUT/summary.log"
d0=$(dmesg | wc -l)
timeout 900 "$OUT/kfplanes" all "/dev/dri/$card" "${PLANES_N:-120}" > "$OUT/all.log" 2>&1
say "ALL rc=$?"
grep '^KFPLANES_RESULT\|^KFPLANES_ENABLE\|^KFPLANES_DISABLE\|^KFPLANES_TIMEOUT\|^KFPLANES_CYCLE\|^KFPLANES_DPMS\|^KFPLANES_DONE\|^KFPLANES_FAIL\|^KFPLANES_MODESET' "$OUT/all.log" | sed 's/^/HW_/' | tee -a "$OUT/summary.log"
dmesg | tail -n +$((d0 + 1)) > "$OUT/dmesg.log"
say "DMESG flip_event_timeouts=$(grep -c 'Flip event timeout' "$OUT/dmesg.log") lut_timeouts=$(grep -c 'LUT notifier timeout' "$OUT/dmesg.log") xid=$(grep -c Xid "$OUT/dmesg.log") lines=$(wc -l < "$OUT/dmesg.log")"
say "EXIT $(date -Is)"
