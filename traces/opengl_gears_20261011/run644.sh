#!/usr/bin/env bash
# Caller takes /tmp/kayfabe-fastguest.lock with flock -o.
set -euo pipefail
W=/var/lib/kf-windows-20261005
REV=18bfb458
export PATH="$HOME/.cargo/bin:$PATH" CARGO_TARGET_DIR=/data/kf-glcrash-target
trap 'rc=$?; echo RUN644_EXIT=$rc; rm -f "$W/kf3-bins/$REV"' EXIT
if pgrep qemu-system-x86; then exit 1; fi
test "$(cat /sys/kernel/iommu_groups/11/type)" = DMA-FQ
cd /data/kf-gl730101-host
test "$(git rev-parse --short=8 HEAD)" = "$REV"
echo RUN644_START="$(date -Is)" SOURCE="$(git rev-parse HEAD)"
cargo test -p kf-abi -p kf-rm --test gsp_static_info > "$W/codex-run644-tests.log" 2>&1
bash scripts/bench/build_kf3.sh "$W/readtrace/qemu-10.2.4" "$W/readtrace/qemu-build-kf3" > "$W/codex-run644-build.log" 2>&1
test ! -e "$W/kf3-bins/$REV"
ln -s "$W/readtrace/kf3-bins/$REV" "$W/kf3-bins/$REV"
sha256sum "$W/kf3-bins/$REV/qemu-system-x86_64"
test "$(df -B1 --output=avail /var/lib | tail -1)" -gt 111669149696
IFS= read -r KF_GUEST_PW < "$W/windows-desktop/secrets/win_password" || test -n "$KF_GUEST_PW"
export KF_GUEST_PW KF3_REV_BIN=$REV HOLD_SECS=10 DWM_OVERLAY_OFF=1 GLTEST=1 SKIP_SHORTS=1 RUN_WIN_FLAGS=KF3_RPC_TRACE=1
bash scripts/bench/windows/glcrash/tdr-run29.sh 644
