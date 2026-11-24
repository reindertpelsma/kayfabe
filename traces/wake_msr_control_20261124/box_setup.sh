#!/bin/bash
# SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later
# Box setup for the S3 wake MSR control: stock and wake-hunk QEMU 10.2.4, the Ubuntu 24.04 cloud base, the NoCloud seed.
# Usage: box_setup.sh KAYFABE_CHECKOUT WORKDIR. Linear glue; the build flags are build_qemu.py's CONFIGURE_FLAGS.
set -euo pipefail
K=$1; W=$2
mkdir -p "$W" && cd "$W"
DEBIAN_FRONTEND=noninteractive apt-get install -y -q ninja-build pkg-config python3-venv libglib2.0-dev libpixman-1-dev \
  libslirp-dev libzstd-dev flex bison qemu-utils cloud-image-utils >/dev/null
TARBALL=$(python3 -I "$K/scripts/ci/qemu_source.py" fetch "$W")
FLAGS=$(cd "$K/scripts/release" && python3 -I -c 'import build_qemu; print(" ".join(build_qemu.CONFIGURE_FLAGS))')
for v in stock patched; do
  rm -rf "src-$v" && mkdir "src-$v" && tar xJf "$TARBALL" -C "src-$v" --strip-components=1
done
python3 -I "$K/scripts/release/build_qemu.py" wake-hunk "$W/src-patched"
for v in stock patched; do
  (mkdir -p "build-$v" && cd "build-$v" && "../src-$v/configure" $FLAGS "--with-pkgversion=wakemsr-$v" >configure.log 2>&1 \
     && ninja -j"$(( $(nproc) / 2 ))" qemu-system-x86_64 >ninja.log 2>&1 && echo "BUILD_OK $v") &
done
wait
IMG=noble-server-cloudimg-amd64.img
curl -sSfLO "https://cloud-images.ubuntu.com/noble/current/$IMG"
curl -sSfL https://cloud-images.ubuntu.com/noble/current/SHA256SUMS | grep " \*$IMG\$" | sha256sum -c -
qemu-img convert -O qcow2 "$IMG" base.qcow2 && qemu-img resize -q base.qcow2 8G
[ -f guestkey ] || ssh-keygen -q -t ed25519 -N '' -f guestkey
cat > user-data <<EOF
#cloud-config
users:
  - name: ubuntu
    sudo: ALL=(ALL) NOPASSWD:ALL
    shell: /bin/bash
    ssh_authorized_keys: [$(cat guestkey.pub)]
runcmd:
  - echo 'GRUB_CMDLINE_LINUX_DEFAULT="console=ttyS0 mem=960M mem_sleep_default=deep no_console_suspend"' > /etc/default/grub.d/99-wakemsr.cfg
  - update-grub
  - systemctl disable --now unattended-upgrades apt-daily.timer apt-daily-upgrade.timer snapd.service || true
power_state: {mode: poweroff, condition: true}
EOF
printf 'instance-id: wakemsr\nlocal-hostname: wakemsr\n' > meta-data
cloud-localds cidata.iso user-data meta-data
echo SETUP_DONE
