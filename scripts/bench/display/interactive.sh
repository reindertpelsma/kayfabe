#!/usr/bin/env bash
# SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later
# interactive.sh — one command for the owner: a Linux guest on kf3 shown in a display-broker window
# on the host's live desktop, with keyboard, absolute (tablet) and relative (mouse) pointer
# (docs/design/V3_DISPLAY.md §8.17). Run as root, from an ssh session or a terminal, while the
# desktop session is logged in; nothing of the session is stopped, restarted or reconfigured.
#
#   interactive.sh            boot the guest; the window appears on the desktop (Ctrl-C stops it)
#   interactive.sh stop       stop a running guest (ACPI powerdown, then kill after 60 s) and its broker
#   interactive.sh prep       once: build the broker into /opt/nvkvm-broker and make the guest disk
#                             (a qcow2 overlay of /workspace/bench/guest.qcow2 with a desktop installed)
#   interactive.sh broker     only (re)start the broker in the desktop session (QEMU reconnects to it)
#
# What shows, in order: OVMF/TianoCore (kf3's GOP option ROM), the grub menu (5 s), the EFI stub and
# kernel console, then nvidia-drm's console and the Cinnamon desktop (Xorg, the stock NVIDIA driver).
#
# env: KF3_REV (kf3 binary revision under $BENCH/kf3-bins; default: the newest built ancestor of this
#   checkout's HEAD), KF_IMG (the guest disk; default $WORK/desktop.qcow2), KF_RAM_MB (8192),
#   KF_SMP (6), KF_EXTRA_QEMU (more QEMU args), BROKER_ARGS (more broker flags), KF_BROKER_BACKEND
#   (auto: wayland for a Wayland session, x11 otherwise; or test, which reads input from a FIFO —
#   input_proof.sh uses it), KF_NVPV_REV (prep: the nvkvm-pv revision, default badf2d7), KF_VNC=1
#   (also a QEMU VNC server on 127.0.0.1:5907, view-only use).
# Logs: $WORK/run-<stamp>/ (qemu.log, serial.log, broker.log, qemu.mon). Each run writes a start
#   marker and an EXIT line (CLAUDE.md, "a killed job and a running job look the same").
set -uo pipefail
HERE="$(cd "$(dirname "$0")" && pwd)"; REPO="$(cd "$HERE/../../.." && pwd)"
BENCH=${BENCH_DIR:-/workspace/bench}
WORK=${KF_INTERACTIVE_DIR:-/var/lib/kf-windows-20261005/broker-interactive}
B=${BROKER_BIN:-/opt/nvkvm-broker/nvkvm-display-broker}
IMG=${KF_IMG:-$WORK/desktop.qcow2}
LOCK=${KF_LOCK:-/tmp/kayfabe-fastguest.lock}
PIDF=$WORK/qemu.pid
cmd=${1:-run}
mkdir -p "$WORK"
say(){ printf '[interactive] %s\n' "$*"; }
die(){ printf '[interactive] ★ %s\n' "$*" >&2; exit 2; }
[ "$(id -u)" = 0 ] || die "run as root (QEMU opens /dev/nvidia*, the tap and the GPU lock)"

# ── the desktop session: seat0's active graphical session, its user, its display ──────────────
session() {
    local s
    s=$(loginctl show-seat seat0 -p ActiveSession --value 2>/dev/null)
    [ -n "$s" ] || s=$(loginctl list-sessions --no-legend | awk '$4 == "seat0" {print $1; exit}')
    [ -n "$s" ] || return 1
    SU=$(loginctl show-session "$s" -p Name --value); SUID=$(id -u "$SU" 2>/dev/null) || return 1
    STYPE=$(loginctl show-session "$s" -p Type --value); SID=$s
    SRUN=/run/user/$SUID
    WD=""; for w in "$SRUN"/wayland-[0-9]; do [ -S "$w" ] && { WD=$(basename "$w"); break; }; done
    XD=""; XA=""
    local xp; xp=$(pgrep -u "$SU" -o -x Xwayland || pgrep -o -x Xorg)
    if [ -n "$xp" ]; then
        XD=$(tr '\0' '\n' < "/proc/$xp/cmdline" | grep -m1 '^:[0-9]')
        XA=$(tr '\0' '\n' < "/proc/$xp/cmdline" | grep -A1 -m1 '^-auth$' | tail -1)
    fi
    return 0
}
SOCK_OF(){ echo "/run/user/$SUID/nvkvm/display.sock"; }

brokers_down() {
    pkill -f "^[^ ]*nvkvm-display-broker --socket $1( |\$)" 2>/dev/null
    pkill -9 -f "^runuser -u [a-z0-9_-]* -- env .*nvkvm-display-broker --socket $1( |\$)" 2>/dev/null
    sleep 1
    pkill -9 -f "^[^ ]*nvkvm-display-broker --socket $1( |\$)" 2>/dev/null
}

# start the broker as the session's user, in the session; $1 = log file
broker_up() {
    local log=$1 backend=${KF_BROKER_BACKEND:-auto} sock; sock=$(SOCK_OF)
    [ -x "$B" ] || die "no broker at $B — run: $0 prep"
    if [ "$backend" = auto ]; then
        if [ "$STYPE" = wayland ] && [ -n "$WD" ]; then backend=wayland; else backend=x11; fi
    fi
    brokers_down "$sock"
    install -d -o "$SU" -m 0700 "$(dirname "$sock")"; rm -f "$sock"
    if [ "$backend" = test ]; then
        # input_proof.sh: the broker's display-less backend, input scripted through a FIFO
        local fifo=${KF_BROKER_FIFO:?KF_BROKER_FIFO names the test backend input FIFO}
        # shellcheck disable=SC2086  # BROKER_ARGS is a flag list
        runuser -u "$SU" -- env "$B" --socket "$sock" --backend test --persist --verbose \
            ${BROKER_ARGS:-} < "$fifo" > "$log" 2>&1 &
    else
        # shellcheck disable=SC2086
        runuser -u "$SU" -- env XDG_RUNTIME_DIR="$SRUN" WAYLAND_DISPLAY="$WD" DISPLAY="$XD" \
            XAUTHORITY="$XA" DBUS_SESSION_BUS_ADDRESS="unix:path=$SRUN/bus" \
            "$B" --socket "$sock" --backend "$backend" --persist --verbose --title "kayfabe guest" \
            ${BROKER_ARGS:-} > "$log" 2>&1 &
    fi
    BPID=$!
    for _ in $(seq 50); do [ -S "$sock" ] && break; sleep 0.2; done
    [ -S "$sock" ] || { tail -5 "$log" >&2; die "the broker did not create $sock (log: $log)"; }
    say "broker: pid $BPID, backend $backend, socket $sock, $(cat "$(dirname "$B")/REV" 2>/dev/null || echo 'rev unknown') ($B)"
}

# the kf3 binary: KF3_REV, else this checkout's HEAD, else its newest built ancestor
pick_qemu() {
    local r
    if [ -n "${KF3_REV:-}" ]; then
        QBIN=$BENCH/kf3-bins/$KF3_REV/qemu-system-x86_64
        [ -x "$QBIN" ] || die "no kf3 binary at $QBIN (scripts/bench/build_kf3.sh builds one)"
        QREV=$KF3_REV; return
    fi
    for r in $(git -C "$REPO" rev-list --max-count=200 HEAD 2>/dev/null); do
        r=${r:0:8}
        if [ -x "$BENCH/kf3-bins/$r/qemu-system-x86_64" ]; then
            QBIN=$BENCH/kf3-bins/$r/qemu-system-x86_64; QREV=$r
            local d; d=$(git -C "$REPO" diff --stat "$r" HEAD -- crates qemu cuda | tail -1)
            [ -n "$d" ] && say "⚠ kf3 binary $r is older than HEAD and the product code differs: $d"
            return
        fi
    done
    die "no kf3 binary for any of the last 200 commits under $BENCH/kf3-bins (build_kf3.sh)"
}

# shellcheck disable=SC2054  # the commas are QEMU property lists inside one argument
qemu_args() {   # $1 = run dir; fills QARGS
    local d=$1 sock; sock=$(SOCK_OF)
    cp /usr/share/OVMF/OVMF_VARS_4M.fd "$d/ovmf_vars.fd"
    local ram=${KF_RAM_MB:-8192}
    QARGS=(-name kayfabe-interactive
        -object "memory-backend-memfd,id=ram0,size=${ram}M,share=on"
        -machine "q35,accel=kvm,memory-backend=ram0" -m "$ram" -cpu host -smp "${KF_SMP:-6}"
        -drive "if=pflash,format=raw,unit=0,readonly=on,file=/usr/share/OVMF/OVMF_CODE_4M.fd"
        -drive "if=pflash,format=raw,unit=1,file=$d/ovmf_vars.fd"
        -drive "if=virtio,file=$IMG,format=qcow2"
        -netdev tap,id=n0,ifname=nvktap0,script=no,downscript=no
        -device virtio-net-pci,netdev=n0,mac=52:54:00:12:34:56,romfile=
        -vga none
        -device "kf3-gpu,id=kf0,guest-driver=580.159.04,fb-mb=8192,bar1-size=134217728,bar2-size=33554432,display=on,gop=on,x11-dispsw=on,display-broker=$sock,display-broker-uid=$SUID${KF3_EXTRA:+,$KF3_EXTRA}"
        -device virtio-keyboard-pci
        -device virtio-tablet-pci,display=kf0,head=0
        -device virtio-mouse-pci
        -display none -msg timestamp=on
        -serial "file:$d/serial.log" -monitor "unix:$d/qemu.mon,server,nowait")
    [ "${KF_VNC:-0}" = 1 ] && QARGS+=(-vnc 127.0.0.1:7)
    # shellcheck disable=SC2206  # KF_EXTRA_QEMU is an argument list
    [ -n "${KF_EXTRA_QEMU:-}" ] && QARGS+=(${KF_EXTRA_QEMU})
    return 0
}

banner() {
    cat <<EOF

  ┌──────────────────────────────────────────────────────────────────────────────┐
  │ kayfabe guest window: "kayfabe guest" on ${SU}'s desktop
  │   pointer      hover = absolute tablet (the guest pointer follows yours)
  │   CTRL+ALT+G   GRAB: keyboard + pointer locked to the guest, relative mouse;
  │                press CTRL+ALT+G again to RELEASE (focus loss also releases)
  │   CTRL+ALT+F   fullscreen on/off
  │   grub menu    10 s after TianoCore (the window appears ~4 s after QEMU starts)
  │   desktop      Cinnamon logs in by itself as 'ubuntu' (~60-90 s)
  │   stop         Ctrl-C here, or: $0 stop  (or close the window: it asks the
  │                guest to power down)
  │   guest ssh    ssh -i $BENCH/guest_key ubuntu@192.168.77.2
  │   logs         $RUN
  └──────────────────────────────────────────────────────────────────────────────┘
EOF
}

case "$cmd" in
stop)
    if [ -s "$PIDF" ] && kill -0 "$(cat "$PIDF")" 2>/dev/null; then
        q=$(cat "$PIDF"); mon=$(ls -t "$WORK"/run-*/qemu.mon 2>/dev/null | head -1)
        [ -S "$mon" ] && printf 'system_powerdown\n' | timeout 5 socat - "UNIX-CONNECT:$mon" >/dev/null 2>&1
        for _ in $(seq 60); do kill -0 "$q" 2>/dev/null || break; sleep 1; done
        kill -0 "$q" 2>/dev/null && { kill -9 "$q"; say "QEMU $q killed after 60 s"; }
    fi
    session && brokers_down "$(SOCK_OF)"
    say "stopped"; exit 0 ;;
broker)
    session || die "no graphical session on seat0"
    broker_up "$WORK/broker-$(date +%Y%m%d-%H%M%S).log"; exit 0 ;;
prep)
    REV=${KF_NVPV_REV:-badf2d7}; NVPV=${NVPV_DIR:-/root/nvkvm-pv}
    # the broker: an export of the pinned revision (the clone's checkout is never touched), built
    # world-readable — the desktop user cannot traverse /root
    git -C "$NVPV" cat-file -e "$REV^{commit}" 2>/dev/null || git -C "$NVPV" fetch -q origin \
        || die "nvkvm-pv $REV not in $NVPV"
    S=/opt/nvkvm-broker/src-$REV; rm -rf "$S"; mkdir -p "$S"
    git -C "$NVPV" archive "$REV" src | tar -x -C "$S"
    make -C "$S/src/broker" report nvkvm-display-broker nvkvm-broker-testclient > "/opt/nvkvm-broker/make-$REV.log" 2>&1 \
        || die "broker build failed: /opt/nvkvm-broker/make-$REV.log"
    install -m 0755 "$S/src/broker/nvkvm-display-broker" "$S/src/broker/nvkvm-broker-testclient" /opt/nvkvm-broker/
    git -C "$NVPV" rev-parse --short=12 "$REV" > /opt/nvkvm-broker/REV; chmod -R a+rX /opt/nvkvm-broker
    say "broker $(cat /opt/nvkvm-broker/REV): $(grep -A4 'backends:' "/opt/nvkvm-broker/make-$REV.log" | tr -s ' ' | tr '\n' ' ')"
    if [ -e "$IMG" ]; then say "guest disk $IMG exists — kept (delete it to provision again)"; exit 0; fi
    session || die "no graphical session on seat0 (needed for the display-broker-uid)"
    qemu-img create -q -f qcow2 -b "$BENCH/guest.qcow2" -F qcow2 "$IMG" || die "qemu-img"
    pick_qemu
    RUN=$WORK/prep-$(date +%Y%m%d-%H%M%S); mkdir -p "$RUN"
    qemu_args "$RUN"
    exec 9>"$LOCK"; flock -w 600 9 || die "the GPU lock $LOCK is held"
    say "provisioning boot, kf3 $QREV ($QBIN)"
    "$QBIN" "${QARGS[@]}" > "$RUN/qemu.log" 2>&1 & q=$!
    G=(ssh -i "$BENCH/guest_key" -o StrictHostKeyChecking=no -o UserKnownHostsFile=/dev/null -o LogLevel=ERROR -o ConnectTimeout=5 ubuntu@192.168.77.2)
    for _ in $(seq 100); do "${G[@]}" true 2>/dev/null && break; kill -0 $q || die "QEMU died: $RUN/qemu.log"; sleep 3; done
    "${G[@]}" 'sudo bash -s' > "$RUN/provision.log" 2>&1 <<'GUEST'
set -x
export DEBIAN_FRONTEND=noninteractive
apt-get update -q
apt-get install -y -q cinnamon-core lightdm lightdm-gtk-greeter xserver-xorg-core \
    xserver-xorg-input-libinput gnome-terminal evtest libinput-tools xdotool xinput \
    x11-xserver-utils x11-utils
echo "APT_RC=$?"
cat > /etc/default/grub.d/99-kf-interactive.cfg <<'EOF'
# kayfabe interactive.sh: a visible grub menu, and nvidia-drm's console after the driver loads
GRUB_TIMEOUT_STYLE=menu
GRUB_TIMEOUT=10
GRUB_RECORDFAIL_TIMEOUT=10
GRUB_CMDLINE_LINUX_DEFAULT="console=tty1 console=ttyS0 nvidia-drm.modeset=1 nvidia-drm.fbdev=1"
EOF
update-grub
echo nvidia_drm > /etc/modules-load.d/kf-nvidia-drm.conf
mkdir -p /etc/lightdm/lightdm.conf.d
cat > /etc/lightdm/lightdm.conf.d/50-kf-autologin.conf <<'EOF'
[Seat:*]
autologin-user=ubuntu
autologin-user-timeout=0
autologin-session=cinnamon
user-session=cinnamon
EOF
systemctl set-default graphical.target
systemctl enable lightdm
# ⊘ [measured 2026-10-08, run proof-p2] without an xorg.conf X picked modesetting, whose glamor
# fails on nvidia-drm ("modeset(0): Failed to create pixmap"), and lightdm restarted X forever
cat > /etc/X11/xorg.conf <<'EOF'
# kayfabe interactive.sh: the stock NVIDIA X driver on kf3 (guest PCI 00:02.0)
Section "ServerLayout"
    Identifier "kf3"
    Screen 0 "kf3-screen"
EndSection
Section "Device"
    Identifier "kf3-gpu"
    Driver "nvidia"
    BusID "PCI:0:2:0"
EndSection
Section "Screen"
    Identifier "kf3-screen"
    Device "kf3-gpu"
EndSection
EOF
usermod -aG input ubuntu
# the desktop must not lock or blank while the owner looks away
sudo -u ubuntu dbus-launch gsettings set org.cinnamon.desktop.screensaver lock-enabled false || true
sudo -u ubuntu dbus-launch gsettings set org.cinnamon.desktop.session idle-delay 0 || true
echo PROVISION_DONE
GUEST
    say "provision: $(grep -E 'APT_RC|PROVISION_DONE' "$RUN/provision.log" | tr '\n' ' ') (log $RUN/provision.log)"
    "${G[@]}" 'sudo systemctl poweroff' >/dev/null 2>&1
    for _ in $(seq 90); do kill -0 $q 2>/dev/null || break; sleep 1; done
    kill -0 $q 2>/dev/null && kill -9 $q
    say "INTERACTIVE_PREP_EXIT $(date -Is)"; exit 0 ;;
run) ;;
*) die "usage: $0 [run|stop|prep|broker]" ;;
esac

# ── run ─────────────────────────────────────────────────────────────────────────────────────────
session || die "no graphical session on seat0 — log in on the host's screen first"
[ -e "$IMG" ] || die "no guest disk $IMG — run: $0 prep"
pgrep -x qemu-system-x86 >/dev/null && die "a QEMU is already running ($(pgrep -x qemu-system-x86 | tr '\n' ' ')): the GPU runs one guest at a time"
pick_qemu
RUN=${KF_RUN_DIR:-$WORK/run-$(date +%Y%m%d-%H%M%S)}; mkdir -p "$RUN"
echo "INTERACTIVE_START kf3=$QREV checkout=$(git -C "$REPO" rev-parse --short=8 HEAD) $(date -Is)" | tee "$RUN/marker.txt"
exec 9>"$LOCK"; flock -n 9 || die "the GPU lock $LOCK is held by another run (fuser $LOCK)"
say "session: $SU (uid $SUID) session $SID type=$STYPE wayland=${WD:-none} x=${XD:-none}; locked=$(loginctl show-session "$SID" -p LockedHint --value)"
# ⊘ [measured 2026-10-08, kf3 0e64a960, host 595.91.07, runs ab-D/E against ab-H/I/J, V3_DISPLAY.md
# §8.17] with gop=on, a broker that is ALREADY connected when the display worker composes its first
# boot frame stalls that copy ("scanout REFUSED copy 1 did not complete in 2s") and the display
# stays dead (the guest's nvidia-modeset then times out on its core channel). A broker that
# connects 3 s or more after QEMU starts does not. So the broker is started once the worker shows
# the boot layer, plus KF_BROKER_DELAY seconds; the relay's own retry (at most every 5 s) connects.
# KF_REUSE_BROKER=1 keeps a broker that is already running (the QEMU-restart reattach measurement).
OWN_BROKER=1
if [ "${KF_REUSE_BROKER:-0}" = 1 ] && [ -S "$(SOCK_OF)" ] \
   && pgrep -f "^[^ ]*nvkvm-display-broker --socket $(SOCK_OF)( |\$)" >/dev/null; then
    OWN_BROKER=0; say "broker: reusing the running one on $(SOCK_OF) (pid $(pgrep -n -f "^[^ ]*nvkvm-display-broker --socket $(SOCK_OF)( |\$)"))"
else
    brokers_down "$(SOCK_OF)"
fi
qemu_args "$RUN"
printf '%q ' "$QBIN" "${QARGS[@]}" > "$RUN/cmdline.txt"
"$QBIN" "${QARGS[@]}" > "$RUN/qemu.log" 2>&1 &
q=$!; echo "$q" > "$PIDF"
cleanup() {
    kill -0 "$q" 2>/dev/null && { kill "$q"; sleep 2; kill -9 "$q" 2>/dev/null; }
    [ "$OWN_BROKER" = 1 ] && brokers_down "$(SOCK_OF)"; rm -f "$PIDF"
    echo "INTERACTIVE_EXIT rc=${rc:-?} $(date -Is)" | tee -a "$RUN/marker.txt"
}
trap 'rc=130; cleanup; exit 130' INT TERM
if [ "$OWN_BROKER" = 1 ]; then
    for _ in $(seq 150); do
        grep -aq 'kf3: display: +[0-9]* ms the console shows\|kf3: display worker up' "$RUN/qemu.log" 2>/dev/null && break
        kill -0 "$q" 2>/dev/null || break
        sleep 0.2
    done
    sleep "${KF_BROKER_DELAY:-3}"
    broker_up "$RUN/broker.log"
fi
say "QEMU pid $q, kf3 $QREV ($QBIN)"
banner
wait "$q"; rc=$?
say "QEMU exited rc=$rc (log $RUN/qemu.log)"
cleanup
exit "$rc"
