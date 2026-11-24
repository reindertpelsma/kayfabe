#!/bin/bash
# Provision a fresh Vast KVM box for the kf3 Linux propimage test: packages, rust, host driver (open module), pinned QEMU, kf3 build.
# Run ON THE BOX from the checkout: bash scripts/bench/linux/matlab_gpu_prov.sh > /root/prov.log 2>&1. Linear glue; every stage prints an RC line.
set -euo pipefail
export DEBIAN_FRONTEND=noninteractive
cd "$(dirname "${BASH_SOURCE[0]}")/../../.."
REPO=$PWD; BENCH=/workspace/bench; mkdir -p "$BENCH"
echo "PROV_START $(date -Is) rev=$(git rev-parse --short=9 HEAD)"
systemctl mask --now apt-daily.timer apt-daily-upgrade.timer >/dev/null 2>&1 || true
while fuser /var/lib/dpkg/lock-frontend >/dev/null 2>&1; do sleep 5; done
apt-get update -qq
apt-get install -y -qq $(for p in full bench; do bash scripts/bench/provision_box.sh --packages $p; done | sort -u | tr '\n' ' ') >/dev/null
echo "APT_RC=$?"
# 2026-11-19 (standalone trust): rustup-init through kf_trust.py with the official .sha256 (as provision_box.sh), never `curl | sh`.
if ! command -v cargo >/dev/null; then
  RUSTUP_DIR=$(mktemp -d)
  python3 -I scripts/ci/kf_trust.py fetch rustup-init-1.29.3 --out "$RUSTUP_DIR/rustup-init"
  chmod 0700 "$RUSTUP_DIR/rustup-init"
  "$RUSTUP_DIR/rustup-init" -y --default-toolchain stable --profile minimal >/dev/null 2>&1
fi
. "$HOME/.cargo/env"
PIN=$(sed -n 's/^channel = "\(.*\)"/\1/p' rust-toolchain.toml)
rustup toolchain install "$PIN" --profile minimal >/dev/null 2>&1
rustup target add --toolchain "$PIN" x86_64-unknown-linux-musl x86_64-unknown-uefi >/dev/null 2>&1
echo "RUST_RC=$?"
set +e; bash scripts/bench/provision_host_driver.sh > /root/prov_driver.log 2>&1; echo "DRIVER_RC=$?"; set -e
nvidia-smi --query-gpu=name,driver_version,memory.total --format=csv,noheader
grep -q "Open Kernel Module" /proc/driver/nvidia/version && echo HOST_OPEN_MODULE=yes
V=$(python3 -I scripts/ci/qemu_source.py version)
python3 -I scripts/ci/qemu_source.py fetch "$BENCH" >/dev/null
[ -d "$BENCH/qemu-$V" ] || tar -C "$BENCH" -xf "$BENCH/qemu-$V.tar.xz"
bash scripts/bench/build_kf3.sh "$BENCH/qemu-$V" "$BENCH/qemu-build-kf3" > /root/prov_kf3.log 2>&1; echo "KF3_RC=$?"
ls -la "$BENCH"/kf3-bins/*/qemu-system-x86_64 | cut -c1-200
echo "PROV_DONE $(date -Is)"
