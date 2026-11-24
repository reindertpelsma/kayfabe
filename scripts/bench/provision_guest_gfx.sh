#!/usr/bin/env bash
# ★ Install the headless-graphics userspace INTO the fat guest image (and onto the host, for the
# bare-metal baseline).  Pattern: provision_guest_llm.sh — powered-off guest, stock QEMU, slirp.
#   usage: provision_guest_gfx.sh            (guest)   |   provision_guest_gfx.sh host
# Guest (noble): vulkan-tools, glslang, Vulkan/EGL/GL headers, mesa-utils (eglinfo/glxinfo),
#   Xvfb, VirtualGL 3.1.3 (.deb from the upstream release), then a CPU-side build of the gfx
#   workloads as the control: a build that fails here is the harness's problem, never kayfabe's.
# ⊘ The NVIDIA userspace (Vulkan ICD, EGL/GLX vendor libs, nvidia-drm.ko) already came from the
#   full 580.159.04 .run installed by provision_bench_tree.sh; this adds only the Khronos side.
set -uo pipefail
BENCH=/workspace/bench
IMG=${KF_GUEST_IMG:-$BENCH/guest.qcow2}
SRC_DIR="$(cd "$(dirname "$0")" && pwd)"
# [2026-11-17] The VirtualGL .deb is fetched on THIS side by scripts/ci/kf_trust.py (audit row 24): its sha256 is the pin in
# scripts/ci/trust_manifest.json (measured from the upstream release asset) and the file is checked before apt sees it. A `VGL_URL` the
# caller passes is a USER INPUT (docs/OWNER_RULINGS.md section AX): trusted, its sha256 and size recorded, never refused on our pin.
KF_TRUST_PY="$(cd "$(dirname "$0")/../ci" && pwd)/kf_trust.py"
vgl_deb() {  # vgl_deb OUT
  if [ -n "${VGL_URL:-}" ]; then
    python3 -I "$KF_TRUST_PY" fetch virtualgl-3.1.3-amd64-deb --out "$1" --user-source "$VGL_URL"
  else
    python3 -I "$KF_TRUST_PY" fetch virtualgl-3.1.3-amd64-deb --out "$1"
  fi
}
PKGS="vulkan-tools libvulkan-dev glslang-tools libegl-dev libopengl-dev libgl-dev libx11-dev mesa-utils xvfb x11-utils gcc"
say(){ echo "[$(date -Is)] $*"; }

if [ "${1:-}" = host ]; then
    export DEBIAN_FRONTEND=noninteractive
    apt-get update -qq && apt-get install -y -qq $PKGS 2>&1 | tail -2
    VGL_DIR=$(mktemp -d) && vgl_deb "$VGL_DIR/vgl.deb" && apt-get install -y -qq "$VGL_DIR/vgl.deb" 2>&1 | tail -1
    rm -rf "${VGL_DIR:-}"
    bash "$SRC_DIR/gfx/build_gfx.sh" /root/gfxbin && say "GFX_HOST_PROVISIONED=yes" || { say "GFX_HOST_PROVISIONED=no"; exit 4; }
    exit 0
fi

pgrep -x qemu-system-x86 >/dev/null && { say "⊘ a QEMU is already running — the bench is serialized, refusing"; exit 2; }
qemu-system-x86_64 -enable-kvm -cpu host -m 4G -smp 8 -display none \
  -drive if=virtio,file="$IMG",format=qcow2 \
  -netdev user,id=n0,hostfwd=tcp:127.0.0.1:2222-:22 -device virtio-net-pci,netdev=n0 \
  -serial file:"$BENCH/gfxprov_serial.log" -daemonize -pidfile "$BENCH/gfxprov.pid"
GS="ssh -i $BENCH/guest_key -p 2222 -o StrictHostKeyChecking=no -o UserKnownHostsFile=/dev/null -o LogLevel=ERROR -o ConnectTimeout=8 ubuntu@127.0.0.1"
for i in $(seq 1 40); do $GS true >/dev/null 2>&1 && break; sleep 5; done
$GS true >/dev/null 2>&1 || { say "⊘ guest never answered ssh"; exit 3; }
for i in $(seq 1 40); do
  case "$($GS 'cloud-init status 2>/dev/null | head -1' 2>/dev/null | tr -d '\r')" in *done*|*disabled*) break ;; esac; sleep 5
done
# ⊘ pin the kernel first (provision_guest_llm.sh, [measured w383]): the nvidia module does not follow it.
$GS "sudo systemctl mask --now unattended-upgrades apt-daily.service apt-daily.timer apt-daily-upgrade.service apt-daily-upgrade.timer 2>/dev/null; \
     sudo apt-mark hold linux-generic linux-image-generic linux-headers-generic 2>/dev/null" >/dev/null 2>&1
KB=$($GS 'uname -r' | tr -d '\r')
say "guest up, kernel $KB — installing: $PKGS + VirtualGL"
$GS "sudo apt-get update -qq && sudo DEBIAN_FRONTEND=noninteractive apt-get install -y -qq $PKGS 2>&1 | tail -2"
# A .run driver uninstall can remove the development linker symlinks even when
# dpkg still considers these packages installed. Restore their owned files.
$GS "sudo DEBIAN_FRONTEND=noninteractive apt-get install --reinstall -y -qq libegl-dev libopengl-dev libgl-dev 2>&1 | tail -2"
VGL_DIR=$(mktemp -d) && vgl_deb "$VGL_DIR/vgl.deb" \
  && $GS 'cat > /tmp/vgl.deb' < "$VGL_DIR/vgl.deb" \
  && $GS "sudo DEBIAN_FRONTEND=noninteractive apt-get install -y -qq /tmp/vgl.deb 2>&1 | tail -1"
rm -rf "${VGL_DIR:-}"
# the render node for the unprivileged ssh user (Ubuntu gives /dev/dri/renderD* to group render)
$GS "sudo usermod -aG video,render ubuntu"
tar -C "$SRC_DIR" -cf - gfx | $GS 'rm -rf ~/gfx && tar -xf - -C ~'
CTRL=$($GS 'bash ~/gfx/build_gfx.sh ~/gfxbin 2>&1 | tail -1; command -v vglrun Xvfb vulkaninfo eglinfo glslangValidator | tr "\n" " "; ls /usr/share/vulkan/icd.d/ /usr/share/glvnd/egl_vendor.d/ 2>&1 | tr "\n" " "' | tr -d '\r')
say "control: $CTRL"
KA=$($GS 'uname -r' | tr -d '\r')
cat > "$BENCH/gfx_lane.receipt" <<RCPT
GFX_LANE_PROVISIONED=$(echo "$CTRL" | grep -q BUILD_GFX_OK && echo yes || echo no)
date=$(date -Is)
control=$CTRL
kernel_before=$KB
kernel_after=$KA
provisioner_rev=$(git -C "$SRC_DIR" rev-parse --short HEAD 2>/dev/null || echo unknown)
RCPT
cat "$BENCH/gfx_lane.receipt"
$GS "sudo poweroff" >/dev/null 2>&1 &
sleep 20
[ "$KB" = "$KA" ] || { say "⊘⊘ guest kernel moved $KB -> $KA — the nvidia module will not load"; exit 6; }
echo "$CTRL" | grep -q BUILD_GFX_OK && say "GUEST_GFX_DONE rc=0" || { say "GUEST_GFX_DONE rc=5 (control build failed)"; exit 5; }
