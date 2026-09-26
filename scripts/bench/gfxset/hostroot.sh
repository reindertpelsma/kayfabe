#!/usr/bin/env bash
# hostroot.sh up|down|run <cmd...> — the BARE-METAL side of the headless-graphics test set, run on
# the host GPU **inside the guest image's own userspace**.
#
# ★ Why: the bench host is Ubuntu 22.04 and the fat guest is 24.04. A render hash, a glmark2
#   validation or an encoded md5 is only a fair host-vs-guest comparison when both sides run the
#   SAME binaries against the SAME libraries (glmark2, weston, Xvfb, VirtualGL, the Vulkan loader,
#   libglvnd — and the NVIDIA 580.159.04 userspace the guest image was given from the same .run as
#   the host kernel module). So the baseline chroots into the powered-off guest image:
#     qemu-nbd --read-only  →  mount -o ro  →  overlayfs with a tmpfs upper (the image is never
#     written)  →  bind /dev /proc /sys /dev/pts /dev/shm /run/udev  →  chroot.
#   What differs from the guest is exactly what the comparison is about: the kernel + GPU path
#   (bare metal vs the kf3 device).
# ⊘ The bench is serial: refuses while a QEMU runs (the qcow2 must not be opened twice).
# ⊘ nbd partition race: poll for /dev/nbdXp1 and retry the mount (build_fast_guest.sh, w824/w825g).
set -uo pipefail
IMG=${KF_GUEST_IMG:-/workspace/bench/guest.qcow2}
NBD=${GSET_NBD:-/dev/nbd1}
R=${GSET_ROOT:-/mnt/gset}
die(){ echo "hostroot: ⊘ $*" >&2; exit 2; }
up(){
    mountpoint -q "$R/root" && { echo "hostroot: already up at $R/root"; return 0; }
    pgrep -x qemu-system-x86 >/dev/null && die "a QEMU is running — the bench is serial and $IMG may be open"
    modprobe nbd max_part=8 2>/dev/null
    mkdir -p "$R/lower" "$R/rw" "$R/root"
    qemu-nbd --read-only --connect="$NBD" -f qcow2 "$IMG" || die "qemu-nbd could not attach $IMG"
    for _ in $(seq 1 25); do [ -b "${NBD}p1" ] && break; partprobe "$NBD" 2>/dev/null || true; sleep 0.2; done
    [ -b "${NBD}p1" ] || { qemu-nbd --disconnect "$NBD" >/dev/null 2>&1; die "${NBD}p1 never appeared"; }
    udevadm settle 2>/dev/null || true
    ok=0; for _ in $(seq 1 25); do mount -o ro "${NBD}p1" "$R/lower" 2>/dev/null && { ok=1; break; }; sleep 0.2; done
    [ $ok = 1 ] || { qemu-nbd --disconnect "$NBD" >/dev/null 2>&1; die "${NBD}p1 would not mount"; }
    mount -t tmpfs -o size=${GSET_OVERLAY_SIZE:-12g} gsetrw "$R/rw" && mkdir -p "$R/rw/u" "$R/rw/w" \
      && mount -t overlay gsetroot -o "lowerdir=$R/lower,upperdir=$R/rw/u,workdir=$R/rw/w" "$R/root" \
      || { down; die "overlay mount failed"; }
    for d in dev proc sys dev/pts dev/shm run/udev; do
        mkdir -p "$R/root/$d"; mount --bind "/$d" "$R/root/$d" 2>/dev/null || echo "hostroot: ⚠ bind /$d failed"
    done
    mkdir -p "$R/root/tmp" && chmod 1777 "$R/root/tmp"
    chmod 0666 /dev/dri/card* /dev/dri/renderD* 2>/dev/null   # bench box only: see `run`
    mkdir -p "$R/root/var/tmp/gfxset" && chown -R 1000:1000 "$R/root/var/tmp/gfxset"
    # ⊘ the image's /etc/resolv.conf is a symlink into /run/systemd (not bound here) — measured: every lookup
    #   in the chroot went to [::1]:53 and failed. Replace it (in the overlay only) with the host's file; the
    #   chroot shares the host's network namespace, so the host's stub resolver is reachable as-is.
    rm -f "$R/root/etc/resolv.conf" && cp -L /etc/resolv.conf "$R/root/etc/resolv.conf"
    echo "hostroot: UP image=$IMG root=$R/root os=$(. "$R/root/etc/os-release"; echo "$PRETTY_NAME") nvlibs=$(ls "$R/root/usr/lib/x86_64-linux-gnu/" | grep -oE 'libnvidia-glcore\.so\.[0-9.]+' | head -1) hostdrv=$(cat /sys/module/nvidia/version 2>/dev/null)"
}
down(){
    for d in run/udev dev/shm dev/pts sys proc dev; do umount -l "$R/root/$d" 2>/dev/null; done
    umount -l "$R/root" 2>/dev/null; umount -l "$R/rw" 2>/dev/null; umount -l "$R/lower" 2>/dev/null
    qemu-nbd --disconnect "$NBD" >/dev/null 2>&1
    mountpoint -q "$R/lower" && echo "hostroot: ⚠ $R/lower still mounted" || echo "hostroot: DOWN"
}
case "${1:-}" in
    up) up ;;
    down) down ;;
    run) shift; mountpoint -q "$R/root" || die "not up"
        # ★ as the image's own uid 1000 (`ubuntu`, the guest's ssh user) with ITS video/render gids, so
        #   both sides run the workloads as the same unprivileged user (weston/sway refuse or behave
        #   differently as root). The host's /dev/dri nodes carry the HOST's render gid, which the image
        #   does not know — up() opens them 0666 on this (bench-only) box.
        gids=$(awk -F: '$1=="video"||$1=="render"{printf "%s%s", s, $3; s=","}' "$R/root/etc/group")
        exec chroot --userspec=1000:1000 --groups="$gids" "$R/root" /usr/bin/env -i \
            HOME=/home/ubuntu USER=ubuntu PATH=/usr/local/sbin:/usr/local/bin:/usr/sbin:/usr/bin:/sbin:/bin TERM=dumb LANG=C.UTF-8 "$@" ;;
    rootrun) shift; mountpoint -q "$R/root" || die "not up"; exec chroot "$R/root" /usr/bin/env -i \
            HOME=/root PATH=/usr/local/sbin:/usr/local/bin:/usr/sbin:/usr/bin:/sbin:/bin TERM=dumb LANG=C.UTF-8 "$@" ;;
    *) echo "usage: hostroot.sh up|down|run|rootrun <cmd...>"; exit 2 ;;
esac
