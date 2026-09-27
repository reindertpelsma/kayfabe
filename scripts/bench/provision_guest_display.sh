#!/usr/bin/env bash
# ★ Install the DISPLAY userspace into the fat guest image (docs/design/V3_DISPLAY.md §5).
# Pattern: provision_guest_gfx.sh — powered-off guest, stock QEMU, slirp, one ssh session per step.
#   usage: provision_guest_display.sh [base|desktop|all]      (default: all)
#     base    — libdrm tools (modetest), kmscube, weston, the lane's own probe (kfdisp_probe), gcc
#     desktop — Xorg + lightdm + Cinnamon (Mint's desktop, from Ubuntu's archive), xdotool, imagemagick,
#               mpv; then RE-RUNS the NVIDIA .run (the same full install) if the X driver (nvidia_drv.so,
#               libglxserver_nvidia.so) is missing, now that an X server exists
# Run after provision_bench_tree.sh and provision_guest_gfx.sh. Writes $BENCH/display_lane.receipt.
# ⊘ The desktop is NOT enabled to start at boot here: the lane starts it explicitly, so a guest that
#   boots the display-off device (the default) never runs a display manager against no display.
set -uo pipefail
BENCH=/workspace/bench
SRC_DIR="$(cd "$(dirname "$0")" && pwd)"
PHASE=${1:-all}
RUN=${NV_RUN:-/root/NVIDIA-Linux-x86_64-580.159.04.run}
BASE_PKGS="gcc make pkg-config libdrm-dev libdrm-tests kmscube weston drm-info"
DESK_PKGS="xserver-xorg-core xserver-xorg-input-libinput xinit x11-xserver-utils x11-utils xterm \
lightdm lightdm-gtk-greeter cinnamon-core nemo dbus-x11 xdotool imagemagick mpv mesa-utils vulkan-tools"
say(){ echo "[$(date -Is)] $*"; }

pgrep -x qemu-system-x86 >/dev/null && { say "⊘ a QEMU is already running — the bench is serialized, refusing"; exit 2; }
qemu-system-x86_64 -enable-kvm -cpu host -m 6G -smp 8 -display none \
  -drive if=virtio,file="$BENCH/guest.qcow2",format=qcow2 \
  -netdev user,id=n0,hostfwd=tcp::2222-:22 -device virtio-net-pci,netdev=n0 \
  -serial file:"$BENCH/dispprov_serial.log" -daemonize -pidfile "$BENCH/dispprov.pid"
GS="ssh -i $BENCH/guest_key -p 2222 -o StrictHostKeyChecking=no -o UserKnownHostsFile=/dev/null -o LogLevel=ERROR -o ConnectTimeout=8 ubuntu@127.0.0.1"
for i in $(seq 1 40); do $GS true >/dev/null 2>&1 && break; sleep 5; done
$GS true >/dev/null 2>&1 || { say "⊘ guest never answered ssh"; exit 3; }
KB=$($GS 'uname -r' | tr -d '\r')
say "guest up, kernel $KB — phase $PHASE"
# ⊘ keep the kernel pinned (provision_guest_llm.sh [measured w383]): the nvidia module does not follow it.
$GS "sudo systemctl mask --now unattended-upgrades apt-daily.service apt-daily.timer apt-daily-upgrade.service apt-daily-upgrade.timer 2>/dev/null; \
     sudo apt-mark hold linux-generic linux-image-generic linux-headers-generic 2>/dev/null" >/dev/null 2>&1
$GS "sudo apt-get update -qq" >/dev/null 2>&1
if [ "$PHASE" = base ] || [ "$PHASE" = all ]; then
  $GS "sudo DEBIAN_FRONTEND=noninteractive apt-get install -y -qq $BASE_PKGS 2>&1 | tail -2"
  tar -C "$SRC_DIR" -cf - display | $GS 'rm -rf ~/display && tar -xf - -C ~'
  $GS 'gcc -O2 -Wall -o ~/display/kfdisp_probe ~/display/kfdisp_probe.c $(pkg-config --cflags --libs libdrm) && echo PROBE_BUILD_OK' | tr -d '\r'
fi
if [ "$PHASE" = desktop ] || [ "$PHASE" = all ]; then
  $GS "sudo DEBIAN_FRONTEND=noninteractive apt-get install -y -qq --no-install-recommends $DESK_PKGS 2>&1 | tail -2"
  $GS "sudo systemctl disable lightdm 2>/dev/null; sudo systemctl set-default multi-user.target" >/dev/null 2>&1
  # the X driver: the bench's .run was installed with no X server present
  if ! $GS 'test -e /usr/lib/xorg/modules/drivers/nvidia_drv.so' 2>/dev/null; then
    [ -s "$RUN" ] || { say "⊘ missing $RUN (needed to install the NVIDIA X driver)"; }
    scp -i "$BENCH/guest_key" -P 2222 -o StrictHostKeyChecking=no -o UserKnownHostsFile=/dev/null \
        -o LogLevel=ERROR "$RUN" ubuntu@127.0.0.1:/var/tmp/nv.run >/dev/null 2>&1
    # ⊘ the SAME full install provision_bench_tree.sh ran (kernel-open modules included): a
    #   userspace-only re-run first UNINSTALLS the previous installation, modules and all.
    $GS "sudo sh /var/tmp/nv.run --silent --no-nouveau-check --no-questions -m=kernel-open -j8; echo NVRUN_X_RC=\$?; rm -f /var/tmp/nv.run" 2>&1 | tail -3
  fi
  $GS "sudo usermod -aG video,render,input ubuntu"
fi
CTRL=$($GS 'command -v modetest kmscube weston Xorg lightdm cinnamon-session xdotool import mpv glxinfo vkcube | tr "\n" " "; \
            test -x ~/display/kfdisp_probe && echo PROBE=yes || echo PROBE=no; \
            ls /usr/lib/xorg/modules/drivers/nvidia_drv.so /usr/lib/x86_64-linux-gnu/nvidia/xorg/libglxserver_nvidia.so* 2>/dev/null | tr "\n" " "' | tr -d '\r')
KA=$($GS 'uname -r' | tr -d '\r')
cat > "$BENCH/display_lane.receipt" <<RCPT
DISPLAY_LANE_PROVISIONED=$(echo "$CTRL" | grep -q PROBE=yes && echo yes || echo no)
phase=$PHASE
date=$(date -Is)
control=$CTRL
kernel_before=$KB
kernel_after=$KA
provisioner_rev=$(git -C "$SRC_DIR" rev-parse --short HEAD 2>/dev/null || echo unknown)
RCPT
cat "$BENCH/display_lane.receipt"
$GS "sudo poweroff" >/dev/null 2>&1 &
for i in $(seq 1 30); do [ -e "/proc/$(cat "$BENCH/dispprov.pid" 2>/dev/null)" ] || break; sleep 2; done
[ "$KB" = "$KA" ] || { say "⊘⊘ guest kernel moved $KB -> $KA — the nvidia module will not load"; exit 6; }
echo "$CTRL" | grep -q PROBE=yes && say "GUEST_DISPLAY_DONE rc=0" || { say "GUEST_DISPLAY_DONE rc=5 (probe build failed)"; exit 5; }
