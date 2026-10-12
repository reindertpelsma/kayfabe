#!/usr/bin/env bash
# Caller owns shared GPU lock; fresh disposable baseline child only.
set -euo pipefail
REV=${1:?full source revision}
TREE=$(git rev-parse --show-toplevel)
W=/var/lib/kf-windows-20261005
ROOT=$W/codex-vfio-ring-p0-20261012
HERE=$TREE/traces/windows_matrix_20261011/gravity-vulkan/vfio-ring-p0
REF=$HERE/vfio_launch.sh
MEDIA=$W/codex-supplement-20261011/full-image-a61af1c3/kfapps_supplement.img
export VDR_DIR=$ROOT/vfio VDR_TRACE_BAR0=0
export VDR_QEMU=$ROOT/build/qemu-system-x86_64
unset VDR_REFUSE WIN_PRE_GUEST_CMD WIN_POST_GUEST_CMD
START=$(date -u '+%Y-%m-%d %H:%M:%S.%6N UTC')
test "$(git rev-parse HEAD)" = "$REV"
test -z "$(git status --porcelain --untracked-files=no)"
test "$(git -C /data/kf-managed-host rev-parse HEAD)" = 72741e2bf6f8ede05e830d9b1267202f070f4542
! pgrep qemu-system
test "$(cat /sys/kernel/iommu_groups/11/type)" = DMA-FQ
test "$(df -B1 --output=avail /var/lib | tail -1)" -gt 107374182400
test ! -e "$ROOT"
mkdir -p "$ROOT/logs"
echo "$START" > "$ROOT/START"
git rev-parse HEAD > "$ROOT/source-revision"
detached=0
finish() {
 rc=$?; trap - EXIT; set +e
 for boot in "$VDR_DIR"/boot[0-9]*; do
  test -f "$boot/qemu.pid" || continue
  pid=$(cat "$boot/qemu.pid")
  test -r "/proc/$pid/cmdline" || continue
  if tr '\0' ' ' < "/proc/$pid/cmdline" | rg -Fq -- "$boot/qmp.sock"; then
   bash "$REF" stop "${boot##*/boot}" >> "$ROOT/logs/stop.log" 2>&1
  fi
 done
 if pgrep -a qemu-system > "$ROOT/logs/post-qemu.txt"; then
  echo VFIO_RING_UNSAFE_TO_RESTORE; rc=1
 elif test "$detached" = 1; then
  bash "$REF" restore >> "$ROOT/logs/restore.log" 2>&1 || rc=1
  if test "$(cat /sys/kernel/iommu_groups/11/type)" != DMA-FQ; then
   bash "$W/pti-iommu_nogdm.sh" DMA-FQ >> "$ROOT/logs/iommu.log" 2>&1 || rc=1
  fi
 fi
 date -Is > "$ROOT/END"
 journalctl -k --since "$START" --utc --no-pager -o short-iso-precise > "$ROOT/logs/kernel-window.txt"
 echo "$?" > "$ROOT/logs/kernel-query-exit"
 rg 'NVRM: Xid' "$ROOT/logs/kernel-window.txt" > "$ROOT/logs/kernel-xids.txt"
 cat /sys/kernel/iommu_groups/11/type > "$ROOT/logs/post-iommu.txt"
 df -B1 /var/lib > "$ROOT/logs/post-disk.txt"
 test "$(cat "$ROOT/logs/post-iommu.txt")" = DMA-FQ || rc=1
 echo "$rc" > "$ROOT/EXIT"
 echo "VFIO_RING_REFERENCE_EXIT=$rc at=$(date -Is)"
 exit "$rc"
}
trap finish EXIT
echo "VFIO_RING_REFERENCE_START=$(date -Is) source=$REV"
echo 'HYPOTHESIS bounded committed fn103 tails recover complete original channel params while same native Gravity workload passes.'
echo 'FALSIFIER malformed/missing params or native workload fails; allocation evidence does not imply migration cursor semantics.'
sha256sum "$W/qemu-10.2.4.tar.xz" > "$ROOT/logs/qemu-tar.sha256"
tar -xJf "$W/qemu-10.2.4.tar.xz" -C "$ROOT"
python3 "$TREE/tools/vfio-gsp-observer/apply.py" "$ROOT/qemu-10.2.4" > "$ROOT/logs/apply.log" 2>&1
mkdir "$ROOT/build"
cd "$ROOT/build"
../qemu-10.2.4/configure --target-list=x86_64-softmmu --disable-docs --disable-tools \
 --disable-guest-agent --disable-werror --enable-slirp --enable-vnc --enable-pixman \
 --disable-gtk --disable-sdl --disable-curses --disable-libssh --disable-vde \
 --enable-tpm --without-default-features --enable-kvm --enable-system > "$ROOT/logs/configure.log" 2>&1
ninja -j12 qemu-system-x86_64 > "$ROOT/logs/build.log" 2>&1
python3 "$TREE/tools/vfio-gsp-observer/tests/test_writer.py" "$ROOT/build" > "$ROOT/logs/writer-test.log" 2>&1
sha256sum "$VDR_QEMU" > "$ROOT/logs/qemu.sha256"
test "$(df -B1 --output=avail /var/lib | tail -1)" -gt 107374182400
cd "$TREE"
detached=1
bash "$REF" detach > "$ROOT/logs/detach.log" 2>&1
IFS= read -r KF_GUEST_PW < "$W/windows-desktop/secrets/win_password" || test -n "$KF_GUEST_PW"
export KF_GUEST_PW WIN_DISABLE_MPO=1
python3 -I "$HERE/winapps_vfio.py" --run-dir "$ROOT/lane" \
 --broker "$REF" --w-dir "$W" --kf3-rev reference-vfio-580.88-alloc-tail --run-base 1 \
 --iso "$MEDIA" --image-manifest "$(dirname "$MEDIA")/manifest.json" \
 --override-dir "$W/codex-matrix-focus-tools-20261011" \
 --apps gravitymark_vk --tier 3 --per-guest 1 --no-isolate > "$ROOT/logs/lane.log" 2>&1
unset KF_GUEST_PW
