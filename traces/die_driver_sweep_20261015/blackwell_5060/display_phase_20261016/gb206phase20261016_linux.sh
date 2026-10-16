#!/usr/bin/env bash
set -euo pipefail
mkdir -p /root/prov /workspace/bench
exec >/root/prov/gb206phase_linux.log 2>&1
trap 'rc=$?; echo "GB206PHASE_LINUX_EXIT rc=$rc $(date -u -Is)"' EXIT
echo "GB206PHASE_LINUX_START $(date -u -Is)"
export KAYFABE_BRANCH=codex/sweep-blackwell-20261015 CARGO_BUILD_JOBS=4 HOST_DRIVER=595.91.07 KF_HOST_DRIVER=595.91.07
bash /root/provision_box.sh >/root/prov/gb206phase_provision.log 2>&1
export PATH=/root/.cargo/bin:$PATH CARGO_TARGET_DIR=/root/kayfabe/target
cd /root/kayfabe
git checkout --detach f8485f6db
git rev-parse HEAD >/root/prov/gb206phase_source
bash scripts/bench/provision_host_driver.sh >/root/prov/gb206phase_driver.log 2>&1
nvidia-smi --query-gpu=name,driver_version,memory.total --format=csv,noheader
cargo build --release -p kf-harness --bin kf-ctl-probe >/root/prov/gb206phase_probe_build.log 2>&1
install -m755 target/release/kf-ctl-probe /tmp/adaref-probe
runuser -u nobody -- env KF_PROBE_I_KNOW=1 KF_PROBE_EXTRA_ONLY=1 /tmp/adaref-probe >/root/prov/gb206phase_die.log 2>&1
cat /root/prov/gb206phase_die.log
grep -q "PROBE_DIE arch=0x1b0 impl=0x6 die=GB206" /root/prov/gb206phase_die.log
bdf=$(python3 - <<'PY'
from pathlib import Path
rows=[p for p in Path('/sys/bus/pci/devices').iterdir() if (p/'vendor').read_text().strip()=='0x10de' and (p/'class').read_text().strip().startswith('0x03')]
assert len(rows)==1
print(rows[0].name)
PY
)
python3 tools/read_display_caps_page.py "$bdf" >/root/prov/gb206phase_native_caps.txt 2>/root/prov/gb206phase_native_caps.err || echo NATIVE_CAPS_READ_FAILED=1
export DEBIAN_FRONTEND=noninteractive
apt-get -o DPkg::Lock::Timeout=300 update -qq
apt-get -o DPkg::Lock::Timeout=300 install -y -qq ninja-build meson python3-venv python3-pip python3-tomli libglib2.0-dev libpixman-1-dev flex bison libslirp-dev libzstd-dev qemu-utils ovmf p7zip-full
curl --fail --location --retry 3 https://download.qemu.org/qemu-10.2.4.tar.xz -o /workspace/bench/qemu-10.2.4.tar.xz
sha256sum /workspace/bench/qemu-10.2.4.tar.xz
tar -C /workspace/bench -xf /workspace/bench/qemu-10.2.4.tar.xz
bash scripts/bench/v3_gates.sh /root/prov/gb206phase_gates.log >/root/prov/gb206phase_gates_wrapper.log 2>&1
bash scripts/bench/build_kf3.sh /workspace/bench/qemu-10.2.4 /workspace/bench/qemu-build-kf3 >/root/prov/gb206phase_build.log 2>&1
cargo build --release -p kayfabe-rm-ladder --bin kayfabe-rm-ladder >/root/prov/gb206phase_raw_build.log 2>&1
touch /workspace/bench/guest.empty
KF_FROM_HOST=1 bash scripts/fastguest/build_fast_guest.sh /workspace/bench/guest.empty /workspace/bench/fastguest595 >/root/prov/gb206phase_fast_build.log 2>&1
git rev-parse HEAD >/root/prov/gb206phase_source
echo "GB206PHASE_BUILD_READY $(date -u -Is)"
