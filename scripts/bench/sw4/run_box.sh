#!/usr/bin/env bash
set -euo pipefail
mkdir -p /root/sw4/evidence /workspace/bench
exec > /root/sw4/evidence/controller.log 2>&1
finish() {
    rc=$?
    echo "SW4_EXIT rc=$rc utc=$(date -u +%FT%TZ)"
    printf '%s\n' "$rc" > /root/sw4/evidence/controller.rc
    journalctl -k -b -o short-iso-precise --no-pager > /root/sw4/evidence/kernel_final.txt
    dmesg > /root/sw4/evidence/dmesg_final.txt
}
trap finish EXIT
trap 'exit 143' TERM INT
python3 /root/sw4/network_guard.py "$$" &
network_guard_pid=$!
for _ in 1 2 3 4 5; do
    [ ! -e /root/sw4/evidence/NETWORK_STOP ]
    kill -0 "$network_guard_pid"
    [ ! -s /root/sw4/evidence/network_armed.json ] || break
    sleep 1
done
[ -s /root/sw4/evidence/network_armed.json ]
rev=714ec29d3fdca0a013d9d478755346b0f1af6fa7
echo "SW4_START utc=$(date -u +%FT%TZ) rev=$rev"
df -h /
[ "$(df -BG --output=avail / | tail -1 | tr -dc 0-9)" -ge 100 ]
[ -c /dev/kvm ]
[ "$(nproc)" -ge 8 ]
[ "$(awk '/MemTotal:/{print int($2/1024/1024)}' /proc/meminfo)" -ge 16 ]
[ -c /dev/nvidiactl ] && [ -c /dev/nvidia0 ]
nvidia-smi --query-gpu=name,driver_version,pci.device_id --format=csv,noheader
nvidia-smi -q > /root/sw4/evidence/smi_initial.txt
journalctl -k -b -o short-iso-precise --no-pager > /root/sw4/evidence/kernel_initial.txt
systemctl mask --now apt-daily.timer apt-daily-upgrade.timer || true
export DEBIAN_FRONTEND=noninteractive
apt-get -o DPkg::Lock::Timeout=600 update -qq
apt-get -o DPkg::Lock::Timeout=600 install -y -qq --no-install-recommends \
    build-essential pkg-config libssl-dev git curl ca-certificates clang lld python3 \
    python3-venv python3-pip python3-tomli kmod pciutils cpio zstd psmisc busybox-static \
    ninja-build meson flex bison libglib2.0-dev libpixman-1-dev libslirp-dev musl-tools \
    linux-headers-"$(uname -r)"
git init -q /root/kayfabe
git -C /root/kayfabe remote add origin https://github.com/reindertpelsma/kayfabe.git
git -C /root/kayfabe fetch -q --depth 1 origin "$rev"
git -C /root/kayfabe checkout -q --detach FETCH_HEAD
cd /root/kayfabe
[ "$(git rev-parse HEAD)" = "$rev" ]
# Public benchmark guard files, recorded separately from the production source.
tar -xf /root/sw4/instruments.tar -C /root/kayfabe
export PATH="$HOME/.cargo/bin:$PATH"
if ! command -v cargo >/dev/null; then
    curl -fsS https://sh.rustup.rs | sh -s -- -y --profile minimal
fi
pin=$(sed -n 's/^channel = "\([^"]*\)"/\1/p' rust-toolchain.toml)
rustup toolchain install "$pin" --profile minimal
rustup target add --toolchain "$pin" x86_64-unknown-linux-musl x86_64-unknown-uefi
export CARGO_TARGET_DIR=/workspace/shared-target CARGO_BUILD_JOBS=8
export HOST_DRIVER=595.91.07
bash scripts/bench/provision_host_driver.sh > /root/sw4/evidence/driver.log 2>&1
grep -q 'Open Kernel Module' /proc/driver/nvidia/version
[ "$(nvidia-smi --query-gpu=driver_version --format=csv,noheader)" = "$HOST_DRIVER" ]
nvidia-smi -q > /root/sw4/evidence/smi_native.txt
export BENCH_DIR=/workspace/bench KF_DEVICE=kf3 KF_FROM_HOST=1
export KF3_FB_MB=8192
export KF_PAYLOAD_ROOT=/root/kayfabe
export KF_PAYLOAD_XID_ALLOWED=/root/sw4/evidence/xid_allowed.jsonl
export KF_PAYLOAD_XID_FAILED=/root/sw4/evidence/XID_STOP
export KF_PAYLOAD_CONTROL_DIR=/root/sw4/evidence/native-controls
export KF_PAYLOAD_NATIVE_RECEIPT=/root/sw4/evidence/native-receipt.json
export BASH_ENV=/root/sw4/guard_hooks.sh
: > "$KF_PAYLOAD_XID_ALLOWED"
[ "$(df -BG --output=avail / | tail -1 | tr -dc 0-9)" -ge 4 ]
echo "SW4_PHASE native-build $(date -u +%FT%TZ)"
cargo build --release -j8 -p kf-harness --bins -p kayfabe-rm-ladder \
    --bin kayfabe-rm-ladder > /root/sw4/evidence/native_build.log 2>&1
export CLIENT=$CARGO_TARGET_DIR/release/kayfabe-rm-ladder
export KF_LADDER=$CLIENT
"$CARGO_TARGET_DIR/release/kf-timer-probe" > /root/sw4/evidence/die_probe.log 2>&1
# The native RM's architecture/implementation tuple decides the die.
nvidia-smi --query-gpu=name,driver_version,pci.device_id --format=csv,noheader \
    > /root/sw4/evidence/binding.csv
"$CARGO_TARGET_DIR/release/kf-timer-probe" > /root/sw4/evidence/binding_probe.log 2>&1
python3 /root/sw4/prepare_binding.py
python3 /root/sw4/die_binding.py /root/sw4/evidence/binding_probe.log \
    /root/sw4/evidence/binding.csv /root/sw4/derived-source \
    /root/sw4/evidence/binding_plan.json > /root/sw4/evidence/binding.json
echo "SW4_PHASE gates $(date -u +%FT%TZ)"
bash scripts/bench/host_xid_watch.sh sw4_gates -- \
    bash scripts/bench/v3_gates.sh /root/sw4/evidence/gates.log
[ ! -e "$KF_PAYLOAD_XID_FAILED" ]
echo "SW4_PHASE bare $(date -u +%FT%TZ)"
bash scripts/fastguest/bare_metal_suite.sh sw4_native 180 > /root/sw4/evidence/bare.log 2>&1
grep -Fx 'BARE_SUITE_PASS=30 BARE_SUITE_FAIL=0 BARE_SUITE_CRASH=0 ARMS=30' \
    /root/sw4/evidence/bare.log
[ ! -e "$KF_PAYLOAD_XID_FAILED" ]
[ -s "$KF_PAYLOAD_NATIVE_RECEIPT" ]
echo "SW4_NATIVE_PASS $(date -u +%FT%TZ)"
[ "$(df -BG --output=avail / | tail -1 | tr -dc 0-9)" -ge 4 ]
echo "SW4_PHASE kf3-build $(date -u +%FT%TZ)"
curl -fLSs --connect-timeout 20 --max-time 600 \
    https://download.qemu.org/qemu-10.2.4.tar.xz -o /workspace/bench/qemu.tar.xz
tar -C /workspace/bench -xf /workspace/bench/qemu.tar.xz
bash scripts/bench/build_kf3.sh /workspace/bench/qemu-10.2.4 \
    /workspace/bench/qemu-build-kf3 > /root/sw4/evidence/kf3_build.log 2>&1
export QEMU_BIN=/workspace/bench/kf3-bins/714ec29d/qemu-system-x86_64
[ -x "$QEMU_BIN" ]
sha256sum "$QEMU_BIN" "$CLIENT" > /root/sw4/evidence/binary_sha256.txt
touch /workspace/bench/unused-host-mode.qcow2
bash scripts/fastguest/build_fast_guest.sh /workspace/bench/unused-host-mode.qcow2 \
    /workspace/bench/fastguest > /root/sw4/evidence/fastguest_build.log 2>&1
echo "SW4_PHASE fast-suite $(date -u +%FT%TZ)"
bash scripts/fastguest/fast_suite.sh sw4_guest 180 > /root/sw4/evidence/fast.log 2>&1
[ ! -e "$KF_PAYLOAD_XID_FAILED" ]
bash scripts/bench/box/birth_census.sh /workspace/bench sw4_guest \
    /root/sw4/evidence/gates.log > /root/sw4/evidence/births.log 2>&1
python3 /root/sw4/final_xids.py
echo "SW4_GUEST_PASS $(date -u +%FT%TZ)"
