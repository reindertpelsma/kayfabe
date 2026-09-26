#!/usr/bin/env bash
# provision.sh — install the headless-graphics TEST SET into the powered-off fat guest image
# (/workspace/bench/guest.qcow2, or $KF_GUEST_IMG), on the stock hypervisor over slirp — the pattern of
# provision_guest_gfx.sh / provision_guest_llm.sh. The bare-metal baseline runs the SAME image's
# userspace through hostroot.sh, so everything the items need lives in the image under /opt/gfxset.
#   needs: scripts/bench/provision_guest_gfx.sh already run (Khronos side + VirtualGL + the gfx sources),
#          /workspace/video/ff/bin/ffmpeg (the static BtbN n8.1 build of V3_VIDEO_ENGINES.md §5).
# Idempotent (a re-run keeps what is already there). Receipt: $BENCH/gfxset.receipt.
set -uo pipefail
BENCH=/workspace/bench; HERE="$(cd "$(dirname "$0")" && pwd)"
IMG=${KF_GUEST_IMG:-$BENCH/guest.qcow2}
say(){ echo "[gset-prov $(date -Is)] $*"; }
FF=${GSET_FF:-/workspace/video/ff/bin/ffmpeg}
BLENDER_URL=${BLENDER_URL:-https://download.blender.org/release/Blender4.5/blender-4.5.0-linux-x64.tar.xz}
VKPEAK_URL=${VKPEAK_URL:-https://github.com/nihui/vkpeak/releases/download/20250531/vkpeak-20250531-ubuntu.zip}
BOD_URL=${BOD_URL:-https://download.blender.org/release/BlenderBenchmark2.0/launcher/benchmark-launcher-cli-3.3.0-linux.tar.gz}
GB_URLS=${GB_URLS:-"https://cdn.geekbench.com/Geekbench-7.0.0-Linux.tar.gz https://cdn.geekbench.com/Geekbench-7.0.1-Linux.tar.gz https://cdn.geekbench.com/Geekbench-6.5.0-Linux.tar.gz https://cdn.geekbench.com/Geekbench-6.4.0-Linux.tar.gz"}
# Ubuntu 24.04: compositors + clients + capture, the build deps of the probes and of glmark2 2023.01
PKGS="weston sway swaybg grim xwayland mesa-utils mesa-utils-bin wayland-utils vulkan-tools \
libwayland-dev wayland-protocols libwayland-egl-backend-dev libgbm-dev libdrm-dev libegl-dev libgles-dev \
libopengl-dev libgl-dev libvulkan-dev libx11-dev libjpeg-dev libpng-dev libudev-dev glslang-tools \
meson ninja-build g++ gcc git make pkg-config xvfb x11-utils python3 unzip xz-utils curl ca-certificates \
ocl-icd-libopencl1 clinfo libxi6 libxxf86vm1 libxfixes3 libxrender1 libxkbcommon0 libsm6 libice6 libgl1 libegl1"
pgrep -x qemu-system-x86 >/dev/null && { say "⊘ a QEMU is already running — the bench is serialized, refusing"; exit 2; }
# the static BtbN n8.1 build of V3_VIDEO_ENGINES.md §5 (sha 5d3a9e6b… there): fetched once per box
if [ ! -x "$FF" ]; then
  mkdir -p /workspace/video && curl -fsSL -o /workspace/video/ff.tar.xz https://github.com/BtbN/FFmpeg-Builds/releases/download/latest/ffmpeg-n8.1-latest-linux64-gpl-8.1.tar.xz \
    && rm -rf /workspace/video/ff && mkdir -p /workspace/video/ff && tar -xJf /workspace/video/ff.tar.xz -C /workspace/video/ff --strip-components=1
fi
[ -x "$FF" ] || { say "⊘ no static ffmpeg at $FF"; exit 2; }
say "static ffmpeg sha256 $(sha256sum "$FF" | cut -c1-16)"
if [ ! -f "$IMG.gset_resized" ]; then qemu-img resize "$IMG" +16G && touch "$IMG.gset_resized"; fi
qemu-system-x86_64 -enable-kvm -cpu host -m 8G -smp "${GSET_PROV_SMP:-8}" -display none \
  -drive if=virtio,file="$IMG",format=qcow2 \
  -netdev user,id=n0,hostfwd=tcp::2222-:22 -device virtio-net-pci,netdev=n0 \
  -serial file:"$BENCH/gsetprov_serial.log" -daemonize -pidfile "$BENCH/gsetprov.pid"
GS="ssh -i $BENCH/guest_key -p 2222 -o StrictHostKeyChecking=no -o UserKnownHostsFile=/dev/null -o LogLevel=ERROR -o ConnectTimeout=8 -o ServerAliveInterval=20 ubuntu@127.0.0.1"
for i in $(seq 1 60); do $GS true >/dev/null 2>&1 && break; sleep 5; done
$GS true >/dev/null 2>&1 || { say "⊘ guest never answered ssh"; kill "$(cat $BENCH/gsetprov.pid)"; exit 3; }
for i in $(seq 1 40); do case "$($GS 'cloud-init status 2>/dev/null | head -1' 2>/dev/null | tr -d '\r')" in *done*) break;; esac; sleep 5; done
$GS "sudo growpart /dev/vda 1 >/dev/null 2>&1; sudo resize2fs /dev/vda1 >/dev/null 2>&1; df -h / | tail -1"
# ⊘ pin the guest kernel BEFORE apt (the nvidia module is built for the running kernel; w383)
$GS "sudo systemctl mask --now unattended-upgrades apt-daily.service apt-daily.timer apt-daily-upgrade.service apt-daily-upgrade.timer >/dev/null 2>&1; \
     sudo apt-mark hold linux-generic linux-image-generic linux-headers-generic >/dev/null 2>&1"
KB=$($GS 'uname -r' | tr -d '\r')
say "guest up, kernel $KB"
# ⊘ one unknown package name aborts the whole apt transaction — install what exists, NAME what does not
$GS "sudo apt-get update -qq; ok=''; miss=''; for p in $PKGS; do if apt-cache show \$p >/dev/null 2>&1; then ok=\"\$ok \$p\"; else miss=\"\$miss \$p\"; fi; done; \
     echo MISSING_PKGS=\$miss; sudo DEBIAN_FRONTEND=noninteractive apt-get install -y -qq --no-install-recommends \$ok 2>&1 | tail -3"
$GS "sudo usermod -aG video,render ubuntu; sudo mkdir -p /opt/gfxset && sudo chown ubuntu:ubuntu /opt/gfxset"
# ⊘ the NVIDIA OpenCL ICD file comes from the driver .run — assert it rather than assume it (V3 app harness)
$GS "[ -f /etc/OpenCL/vendors/nvidia.icd ] || { sudo mkdir -p /etc/OpenCL/vendors && echo libnvidia-opencl.so.1 | sudo tee /etc/OpenCL/vendors/nvidia.icd >/dev/null; echo OPENCL_ICD_WRITTEN; }"
# the same static ffmpeg the video lane and the host use
$GS 'mkdir -p /opt/gfxset/ff/bin && cat > /opt/gfxset/ff/bin/ffmpeg && chmod +x /opt/gfxset/ff/bin/ffmpeg' < "$FF"
# upstream artefacts, fetched INSIDE the image (both sides run these exact files)
$GS "cd /opt/gfxset && { [ -x blender/blender ] || { mkdir -p blender && curl -fsSL --retry 3 '$BLENDER_URL' | tar xJ -C blender --strip-components=1; }; }; \
     { [ -x vkpeak/vkpeak ] || { curl -fsSL -o /tmp/vkpeak.zip '$VKPEAK_URL' && mkdir -p vkpeak && unzip -o -q /tmp/vkpeak.zip -d /tmp/vkp \
          && cp \$(find /tmp/vkp -name vkpeak -type f | head -1) vkpeak/vkpeak && chmod +x vkpeak/vkpeak; }; }; \
     ls -la blender/blender vkpeak/vkpeak 2>&1 | tail -2"
# Blender Open Data (nvkvm-pv docs/reference/blender-opendata.md): launcher 3.3.0, Blender 4.5.0, 3 scenes
$GS "mkdir -p /opt/gfxset/bod && cd /opt/gfxset/bod && { [ -x benchmark-launcher-cli ] || curl -fsSL --retry 3 '$BOD_URL' | tar xz; } \
     && export HOME=/opt/gfxset/bod && ./benchmark-launcher-cli blender download 4.5.0 >/tmp/bod_dl.log 2>&1; \
     for s in monster junkshop classroom; do ./benchmark-launcher-cli scenes download --blender-version 4.5.0 \$s >>/tmp/bod_dl.log 2>&1; done; \
     ./benchmark-launcher-cli scenes list --blender-version 4.5.0 2>&1 | tail -6; sha256sum benchmark-launcher-cli | cut -c1-16; tail -2 /tmp/bod_dl.log"
# Geekbench (nvkvm-pv ran GB 7's GPU Vulkan backend): the newest of the listed releases that downloads
$GS "mkdir -p /opt/gfxset/geekbench && cd /opt/gfxset/geekbench && if ! ls geekbench[0-9]* >/dev/null 2>&1; then for u in $GB_URLS; do \
       curl -fsSL \"\$u\" | tar xz --strip-components=1 && { echo GEEKBENCH_FROM=\$u; break; }; done; fi; ls | tr '\n' ' '"
# the workloads, built in the image from the tree's sources (gfx/ = the v3-gfx lane, gfxset/src = this set)
tar -C "$HERE/.." -cf - gfx gfxset | $GS 'rm -rf /opt/gfxset/src && mkdir -p /opt/gfxset/src && tar -xf - -C /opt/gfxset/src'
CTRL=$($GS 'S=/opt/gfxset/src; B=/opt/gfxset/gfxbin; export GSET_HOME=/opt/gfxset; bash $S/gfx/build_gfx.sh $B | tail -1;
  gcc -O2 -o $B/egl_offscreen $S/gfxset/src/egl_offscreen.c -lEGL -lGLESv2 && echo BUILD_egl_offscreen=ok;
  gcc -O2 -o $B/gl_limits $S/gfxset/src/gl_limits.c -lEGL -lOpenGL && echo BUILD_gl_limits=ok;
  bash $S/gfxset/src/build_extra.sh $B;
  /opt/gfxset/ff/bin/ffmpeg -hide_banner -version | head -1; /opt/gfxset/blender/blender --version 2>/dev/null | head -1;
  command -v weston sway grim es2gears_wayland weston-screenshooter vulkaninfo eglinfo vglrun Xvfb | tr "\n" " "; df -h / | tail -1' 2>&1 | tr -d '\r')
echo "$CTRL" | sed 's/^/  control: /'
KA=$($GS 'uname -r' | tr -d '\r')
nfail=$(echo "$CTRL" | grep -c 'BUILD_.*=FAIL')
cat > "$BENCH/gfxset.receipt" <<RCPT
GSET_PROVISIONED=$(echo "$CTRL" | grep -q 'BUILD_GFX_OK' && [ "$nfail" -eq 0 ] && echo "$CTRL" | grep -q 'Blender 4.5' && echo yes || echo no)
date=$(date -Is)
image=$IMG
build_failures=$nfail
kernel_before=$KB
kernel_after=$KA
provisioner_rev=$(git -C "$HERE" rev-parse --short HEAD 2>/dev/null || echo unknown)
RCPT
cat "$BENCH/gfxset.receipt"
$GS "sync; sudo poweroff" >/dev/null 2>&1
for i in $(seq 1 40); do pgrep -x qemu-system-x86 >/dev/null || break; sleep 3; done
pgrep -x qemu-system-x86 >/dev/null && { say "⚠ provisioning QEMU still up — killing"; kill "$(cat $BENCH/gsetprov.pid)"; }
[ "$KB" = "$KA" ] || { say "⊘⊘ guest kernel moved $KB -> $KA — the nvidia module will not load"; exit 6; }
grep -q '^GSET_PROVISIONED=yes' "$BENCH/gfxset.receipt" && say "GSET_PROVISION_DONE rc=0" || { say "GSET_PROVISION_DONE rc=5 (a control failed)"; exit 5; }
