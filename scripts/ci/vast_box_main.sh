#!/usr/bin/env bash
# SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later
# Runs ONLY on the disposable Vast VM. No credentials or return executable channel.
set -euo pipefail
source_root=$(cd "$(dirname "$0")/../.." && pwd)
check_results() {
  local evidence=$1
  grep -Ex 'V3_GATES_SUMMARY pass=9 fail=0 gate10=(PASS|FALLBACK)' "$evidence/gates.log"
  grep -Fx 'BARE_SUITE_PASS=30 BARE_SUITE_FAIL=0 BARE_SUITE_CRASH=0 ARMS=30' "$evidence/bare.log"
  grep -Fx 'FAST_SUITE_PASS=30 FAST_SUITE_FAIL=0 FAST_SUITE_CRASH=0 NOTRUN=0 ARMS=30' "$evidence/suite.log"
  grep -E '^BIRTH_CENSUS_OK arms=30( |$)' "$evidence/births.log"
}
if [[ "${1:-}" = --check-results ]]; then check_results "${2:?evidence}"; exit; fi
if [[ "${1:-}" = --must-fail-control ]]; then
  control_dir=$(mktemp -d)
  set +e
  bash "$source_root/scripts/bench/box/birth_census.sh" "$control_dir" missing "$control_dir/missing.log"
  control_rc=$?
  rmdir "$control_dir"
  exit "$control_rc"
fi
check_control() {
  if bash "$0" --must-fail-control; then
    echo 'MUST_FAIL_CONTROL_UNEXPECTED_PASS'
    return 1
  fi
  echo 'MUST_FAIL_CONTROL_FAILED_AS_REQUIRED'
}
if [[ "${1:-}" = --check-control ]]; then check_control; exit; fi
check_disk() {
  local available
  available=$(df -B1 --output=avail / | tail -n1 | tr -d ' ')
  if [[ ! "$available" =~ ^[0-9]{1,15}$ ]] || [[ "$available" -lt 60000000000 ]]; then
    echo 'VAST_CI_DISK_UNAVAILABLE: at least 60 GB free required'
    return 1
  fi
}
if [[ "${1:-}" = --check-disk ]]; then check_disk; exit; fi
expected_sha=${1:?verified SHA}
[[ "$expected_sha" =~ ^[0-9a-f]{40}$ ]]
cd "$source_root"
[[ "$(git rev-parse HEAD)" = "$expected_sha" ]]
check_disk
[[ -c /dev/kvm ]]
# Run the same control path exercised by the GPU-free harness tests.
check_control
export DEBIAN_FRONTEND=noninteractive
# Avoid unattended upgrades' lock without changing system kernel/reboot state.
apt-get -o DPkg::Lock::Timeout=600 update -qq
apt-get -o DPkg::Lock::Timeout=600 install -y -qq build-essential pkg-config libssl-dev \
  git curl ca-certificates clang lld python3 python3-venv python3-pip python3-tomli \
  kmod pciutils cpio zstd psmisc busybox-static ninja-build meson flex bison \
  libglib2.0-dev libpixman-1-dev libslirp-dev musl-tools linux-headers-"$(uname -r)"
if ! command -v cargo >/dev/null; then
  curl --fail --silent --show-error https://sh.rustup.rs | sh -s -- -y --profile minimal
fi
export PATH="$HOME/.cargo/bin:$PATH"
pin=$(sed -n 's/^channel = "\([^"]*\)"/\1/p' rust-toolchain.toml)
[[ -n "$pin" ]]
rustup toolchain install "$pin" --profile minimal
rustup target add --toolchain "$pin" x86_64-unknown-linux-musl x86_64-unknown-uefi
# This helper provisions the driver, never checks out the application tree.
bash scripts/bench/provision_host_driver.sh
[[ "$(git rev-parse HEAD)" = "$expected_sha" ]]
export BENCH_DIR=/workspace/bench KF_DEVICE=kf3 KF_FROM_HOST=1
export CARGO_TARGET_DIR=/workspace/kf-ci-target CARGO_BUILD_JOBS=2
mkdir -p "$BENCH_DIR" /workspace/kf-ci-evidence
evidence=/workspace/kf-ci-evidence
qemu_version=10.2.4
curl --fail --location --show-error --connect-timeout 20 --max-time 600 \
  "https://download.qemu.org/qemu-$qemu_version.tar.xz" -o "$BENCH_DIR/qemu.tar.xz"
tar -C "$BENCH_DIR" -xf "$BENCH_DIR/qemu.tar.xz"
crates=()
for manifest in crates/kf-*/Cargo.toml; do crate=${manifest#crates/}; crates+=(-p "${crate%/Cargo.toml}"); done
[[ ${#crates[@]} -ge 36 ]]
cargo test -j2 --release --no-fail-fast "${crates[@]}"
bash scripts/bench/v3_gates.sh "$evidence/gates.log"
bash scripts/bench/build_kf3.sh "$BENCH_DIR/qemu-$qemu_version" "$BENCH_DIR/qemu-build-kf3"
cargo build -j2 --release -p kayfabe-rm-ladder --bin kayfabe-rm-ladder
export CLIENT="$CARGO_TARGET_DIR/release/kayfabe-rm-ladder" KF_LADDER="$CARGO_TARGET_DIR/release/kayfabe-rm-ladder"
bash scripts/fastguest/bare_metal_suite.sh ci_host 180 | tee "$evidence/bare.log"
bash scripts/fastguest/build_fast_guest.sh "$BENCH_DIR/unused-host-mode.qcow2" "$BENCH_DIR/fastguest"
bash scripts/fastguest/fast_suite.sh ci 180
cp "$BENCH_DIR/ci_suite.out" "$evidence/suite.log"
cat "$evidence/suite.log"
bash scripts/bench/box/birth_census.sh --selftest
bash scripts/bench/box/birth_census.sh "$BENCH_DIR" ci "$evidence/gates.log" | tee "$evidence/births.log"
[[ "$(git rev-parse HEAD)" = "$expected_sha" ]]
check_results "$evidence"
echo 'KF_SUITE_COMPLETE'
