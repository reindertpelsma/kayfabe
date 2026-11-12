#!/usr/bin/env bash
# SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later
# propimage_run.sh MODE: start swtpm and QEMU for the proprietary Windows image, as the invoking user (ubuntu).
# STATUS: DESIGN-ONLY, 2026-11-12. Glue only: every decision (ids, argv, CPU profile, preflight) is propimage.py.
#   PROP_DIR (default /workspace/bench/propimage_20261112): state.json, image.qcow2, firmware/, tpm/, media/
#   PROP_BIN: propimage.py directory; PROP_QEMU for kf3 modes; MODE is setup|install|offline|ci
# setup mode needs the GNOME session: it sets XDG_RUNTIME_DIR and WAYLAND_DISPLAY itself.
set -euo pipefail
MODE=${1:?mode: setup|install|offline|ci}
D=${PROP_DIR:-/workspace/bench/propimage_20261112}
BIN=${PROP_BIN:-$D/bin}
PY=(python3 -B -I "$BIN/propimage.py")
ST=$D/state.json
[ -r "$ST" ] || { echo "no $ST" >&2; exit 2; }
for d in tpm firmware run; do install -d -m 0700 "$D/$d"; done
extra=()
case "$MODE" in
  setup)
    for f in "$D"/media/win.iso "$D"/media/virtio-win.iso "$D"/media/tools.iso; do
      [ -r "$f" ] || { echo "missing $f" >&2; exit 2; }; extra+=(--cdrom "$f"); done ;;
  install|offline) extra+=(--broker-sock "${PROP_BROKER_SOCK:?broker socket}" --gop-efi "${PROP_GOP_EFI:?signed gop efi}" --uid "$(id -u)") ;;
  ci) extra+=(--display off) ;;
esac
DISK=$D/image.qcow2
[ "$MODE" != ci ] || { DISK=$D/run/clone.qcow2; rm -f "$DISK"; qemu-img create -q -f qcow2 -b "$D/image.qcow2" -F qcow2 "$DISK"; }
mapfile -t OUT < <("${PY[@]}" argv "$ST" "$MODE" --disk "$DISK" --workdir "$D" "${extra[@]}" ${PROP_QEMU:+--qemu "$PROP_QEMU"})
QCMD=${OUT[0]}
echo "CMD: $QCMD" > "$D/run/last_cmd.txt"
# swtpm: one process per QEMU start, state under $D/tpm (restored from state.json for ci/offline)
rm -f "$D/swtpm.sock" "$D/run/swtpm.pid"
swtpm socket --tpm2 --tpmstate "dir=$D/tpm,mode=0600" --ctrl "type=unixio,path=$D/swtpm.sock,mode=0600" \
  --log "file=$D/run/swtpm.log,level=20" --pid "file=$D/run/swtpm.pid" --terminate --daemon
for _ in $(seq 50); do [ -S "$D/swtpm.sock" ] && break; sleep 0.2; done
[ -S "$D/swtpm.sock" ] || { echo "swtpm did not start (see $D/run/swtpm.log; AppArmor?)" >&2; exit 3; }
if [ "$MODE" = setup ]; then
  export XDG_RUNTIME_DIR=/run/user/$(id -u) WAYLAND_DISPLAY=${WAYLAND_DISPLAY:-wayland-0} GDK_BACKEND=wayland
  export DBUS_SESSION_BUS_ADDRESS=unix:path=$XDG_RUNTIME_DIR/bus
fi
echo "START $MODE $(date -Is) pid $$" >> "$D/run/marker.txt"
eval "exec $QCMD"
