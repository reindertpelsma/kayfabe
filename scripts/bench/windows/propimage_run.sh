#!/usr/bin/env bash
# SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later
# propimage_run.sh MODE: start swtpm and QEMU for the proprietary Windows image.
# STATUS: DESIGN-ONLY, 2026-11-12. Glue only: every decision (ids, argv, CPU profile, preflight) is propimage.py.
#   setup:           run as the desktop user (ubuntu); distro QEMU, gtk window on the GNOME session.
#   install|offline: run as root (kf3 opens /dev/nvidia*); starts the display broker as the session user, then QEMU.
#   ci:              run as root; same, headless display (no broker).
# Env: PROP_DIR (default /workspace/bench/propimage_20261112), PROP_QEMU (kf3 qemu), PROP_GOP_EFI (signed boot GOP),
#      PROP_BROKER (broker binary), PROP_KF3_ENV (KF3_NAME=digits lines; default $PROP_DIR/bin/kf3_flags.sample.env)
set -euo pipefail
MODE=${1:?mode: setup|install|offline|ci}
D=${PROP_DIR:-/workspace/bench/propimage_20261112}
PY=(python3 -B -I "$D/bin/propimage.py")
ST=$D/state.json
[ -r "$ST" ] || { echo "no $ST" >&2; exit 2; }
for d in tpm firmware run; do install -d -m 0700 "$D/$d"; done
extra=() SU=${PROP_SESSION_USER:-ubuntu}
case "$MODE" in
  setup)
    for f in "$D"/media/win.iso "$D"/media/virtio-win.iso "$D"/media/tools.iso; do
      [ -r "$f" ] || { echo "missing $f" >&2; exit 2; }; extra+=(--cdrom "$f"); done ;;
  install|offline)
    [ "$(id -u)" = 0 ] || { echo "$MODE mode runs as root (kf3 opens /dev/nvidia*)" >&2; exit 2; }
    SUID=$(id -u "$SU"); SRUN=/run/user/$SUID; SOCK=$SRUN/nvkvm/propimage.sock
    extra+=(--broker-sock "$SOCK" --gop-efi "${PROP_GOP_EFI:-$D/firmware/kf-gop.signed.efi}" --uid "$SUID") ;;
  ci) [ "$(id -u)" = 0 ] || { echo "ci mode runs as root" >&2; exit 2; }; extra+=(--display off) ;;
esac
DISK=$D/image.qcow2
[ "$MODE" != ci ] || { DISK=$D/run/clone.qcow2; rm -f "$DISK"; qemu-img create -q -f qcow2 -b "$D/image.qcow2" -F qcow2 "$DISK"; }
mapfile -t OUT < <("${PY[@]}" argv "$ST" "$MODE" --disk "$DISK" --workdir "$D" "${extra[@]}" ${PROP_QEMU:+--qemu "$PROP_QEMU"})
QCMD=${OUT[0]}
echo "CMD: $QCMD" > "$D/run/last_cmd.txt"
as_user() { if [ "$(id -u)" = 0 ]; then runuser -u "$SU" -- "$@"; else "$@"; fi; }
# swtpm: one process per QEMU start, state under $D/tpm
rm -f "$D/swtpm.sock" "$D/run/swtpm.pid"
as_user swtpm socket --tpm2 --tpmstate "dir=$D/tpm,mode=0600" --ctrl "type=unixio,path=$D/swtpm.sock,mode=0600" \
  --log "file=$D/run/swtpm.log,level=20" --pid "file=$D/run/swtpm.pid" --terminate --daemon
for _ in $(seq 50); do [ -S "$D/swtpm.sock" ] && break; sleep 0.2; done
[ -S "$D/swtpm.sock" ] || { echo "swtpm did not start (see $D/run/swtpm.log; AppArmor?)" >&2; exit 3; }
case "$MODE" in
  setup)
    export XDG_RUNTIME_DIR=/run/user/$(id -u) WAYLAND_DISPLAY=${WAYLAND_DISPLAY:-wayland-0} GDK_BACKEND=wayland
    export DBUS_SESSION_BUS_ADDRESS=unix:path=$XDG_RUNTIME_DIR/bus ;;
  install|offline)
    # the normal broker path (docs/howto/display.md): the broker runs as the session user in the session
    B=${PROP_BROKER:-/workspace/bench/pcdisp_20261112/bin/nvkvm-display-broker}
    XD=$(tr '\0' '\n' < "/proc/$(pgrep -u "$SU" -o -x Xwayland)/cmdline" | grep -m1 '^:[0-9]' || true)
    install -d -o "$SU" -m 0700 "$SRUN/nvkvm"; rm -f "$SOCK"
    runuser -u "$SU" -- env XDG_RUNTIME_DIR="$SRUN" WAYLAND_DISPLAY=wayland-0 DISPLAY="$XD" DBUS_SESSION_BUS_ADDRESS="unix:path=$SRUN/bus" \
      "$B" --socket "$SOCK" --backend wayland --persist --verbose --title "kayfabe Windows (proprietary image)" > "$D/run/broker.log" 2>&1 < /dev/null &
    for _ in $(seq 50); do [ -S "$SOCK" ] && break; sleep 0.2; done
    [ -S "$SOCK" ] || { echo "the broker made no socket (see $D/run/broker.log)" >&2; exit 3; }
    # KF3_* flags of the gate (only KF3_NAME=digits lines are accepted)
    while IFS= read -r line; do
      [[ "$line" =~ ^[[:space:]]*(#.*)?$ ]] && continue
      [[ "$line" =~ ^KF3_[A-Z0-9_]+=[0-9]+$ ]] || { echo "refused env line: $line" >&2; exit 2; }
      export "$line"
    done < "${PROP_KF3_ENV:-$D/bin/kf3_flags.sample.env}"
    export KF3_RPC_TRACE=${KF3_RPC_TRACE:-1} ;;
esac
echo "START $MODE $(date -Is) pid $$" >> "$D/run/marker.txt"
eval "exec $QCMD"
