#!/usr/bin/env bash
# SPDX-License-Identifier: Apache-2.0
# build_alpine_uki.sh -- build ONE UEFI executable (a UKI: systemd EFI stub + Alpine kernel + initramfs)
# that boots a RAM-only Linux with DHCP networking, sshd on port 22 (one authorized key), zstd,
# ntfs-3g/ntfsclone, dosfstools, curl and efibootmgr. It is dropped on the ESP of a Windows VM so the
# disk can be copied offline (docs/design/V3_WINDOWS_IMAGE_CAPTURE.md). Runs on the dev box, as root.
#
#   AUTH_KEY_PUB=/root/night/winimage_private/alpine_key.pub \
#   WORK=/data/winimage_skeleton_20261107/uki_work OUT=/data/winimage_skeleton_20261107/kf-alpine.efi \
#   bash build_alpine_uki.sh
#
# Env: AUTH_KEY_PUB (required), OUT, WORK, AUTORESTORE_S (default 900), EXTRA_CMDLINE, SERIAL (default
# ttyS0,115200), ALPINE_VER/ALPINE_BRANCH/ROOTFS_SHA256 (pinned below), MIRROR, PKGS_EXTRA, TEST_ONLY=1
# (run the pure-shell self-checks and exit).
#
# What it never does: no secret is baked in except the PUBLIC key; the sshd host key is generated at
# every boot; the image has no password. The recovery timer (kf.autorestore=N seconds) restores the
# Windows boot files from \EFI\kf-backup and reboots when nobody logged in over ssh in time.
set -euo pipefail

ALPINE_VER=${ALPINE_VER:-3.24.4}
ALPINE_BRANCH=${ALPINE_BRANCH:-v3.24}
MIRROR=${MIRROR:-https://dl-cdn.alpinelinux.org/alpine}
ROOTFS_SHA256=${ROOTFS_SHA256:-cdaa38cb8de3537967ee5b6c9b6cd00cc962da2b018e6eea48696bb274923012}
AUTORESTORE_S=${AUTORESTORE_S:-900}
SERIAL=${SERIAL:-ttyS0,115200}
# kernel modules to load at boot (those that exist in the kernel package; missing ones are logged)
MODS=${MODS:-"af_packet virtio_pci virtio_net virtio_blk virtio_scsi virtio_console e1000 e1000e igb igc ixgbe vmxnet3 r8169 8139cp 8139too tg3 nvme ahci ata_piix ata_generic libata sd_mod sr_mod vfat fat nls_cp437 nls_iso8859_1 nls_utf8 ntfs3 fuse efivarfs ext4 loop crc32c_generic"}
PKGS=${PKGS:-"busybox bash kmod openssh-server openssh-sftp-server zstd ntfs-3g ntfs-3g-progs dosfstools curl util-linux lsblk sgdisk efibootmgr pv iproute2 e2fsprogs ca-certificates-bundle"}
PKGS="$PKGS ${PKGS_EXTRA:-}"

die() { echo "build_alpine_uki: $*" >&2; exit 1; }
log() { echo "build_alpine_uki: $*" >&2; }

# --- the pieces that are plain text, kept as functions so the tests can check them without a build ---
emit_init() {  # /init of the initramfs (busybox ash)
  cat <<'INIT'
#!/bin/sh
# kf-alpine /init: RAM Linux for an offline Windows disk copy. Everything goes to the console (serial).
export PATH=/usr/sbin:/usr/bin:/sbin:/bin
say() { echo "KFUKI $*"; }
mount -t proc proc /proc; mount -t sysfs sys /sys
mount -t devtmpfs dev /dev 2>/dev/null || mount -t tmpfs dev /dev
ln -sf /proc/self/fd /dev/fd; ln -sf /proc/self/fd/0 /dev/stdin; ln -sf /proc/self/fd/1 /dev/stdout; ln -sf /proc/self/fd/2 /dev/stderr   # bash process substitution needs /dev/fd
mkdir -p /dev/pts /run /tmp /mnt/esp; mount -t devpts devpts /dev/pts; mount -t tmpfs tmpfs /run; mount -t tmpfs tmpfs /tmp
say "boot kernel=$(uname -r) cmdline=$(cat /proc/cmdline)"
AUTORESTORE_S=900
DEBUG=
for w in $(cat /proc/cmdline); do case "$w" in kf.autorestore=*) AUTORESTORE_S=${w#kf.autorestore=};; kf.debug) DEBUG=1;; esac; done
# kf.debug: a root shell on the serial console (for the dev box only; never set on a Vast instance)
[ -n "$DEBUG" ] && setsid sh -c 'exec sh </dev/ttyS0 >/dev/ttyS0 2>&1' &

# modules (list baked in at build time)
for m in $(cat /etc/kf-modules); do modprobe "$m" 2>/dev/null && say "module $m ok" || say "module $m missing-or-failed"; done
mount -t efivarfs efivarfs /sys/firmware/efi/efivars 2>/dev/null
sleep 2   # let virtio/PCI devices appear

# network: DHCP on every NIC; fall back to \EFI\kf-backup\net.conf (written by esp_swap.ps1) on the ESP
ip link set lo up
for i in $(ls /sys/class/net); do [ "$i" = lo ] || ip link set "$i" up; done
sleep 3
GOT=
for i in $(ls /sys/class/net); do
  [ "$i" = lo ] && continue
  say "dhcp on $i ($(cat /sys/class/net/$i/address 2>/dev/null) driver=$(basename $(readlink /sys/class/net/$i/device/driver 2>/dev/null) 2>/dev/null))"
  if udhcpc -i "$i" -q -n -t 6 -T 3 -s /usr/share/udhcpc/default.script >/run/udhcpc.$i.log 2>&1; then GOT=$i; break; fi
  say "udhcpc on $i failed: $(tr '\n' '|' < /run/udhcpc.$i.log) link: $(ip -o link show $i)"
done
find_esp() {
  for p in $(ls /sys/class/block | grep -v '^loop'); do
    d=/dev/$p; [ -b "$d" ] || continue
    t=$(blkid -o value -s TYPE "$d" 2>/dev/null)
    [ "$t" = vfat ] || continue
    mount -t vfat -o ro,iocharset=iso8859-1 "$d" /mnt/esp 2>/dev/null || continue
    if [ -d /mnt/esp/EFI ]; then ESP_DEV=$d; return 0; fi
    umount /mnt/esp
  done; return 1
}
if [ -z "$GOT" ]; then
  say "dhcp failed on every NIC; looking for net.conf on the ESP"
  if find_esp && [ -f /mnt/esp/EFI/kf-backup/net.conf ]; then
    . /mnt/esp/EFI/kf-backup/net.conf     # NET_IF_MAC NET_IP NET_PREFIX NET_GW NET_DNS
    for i in $(ls /sys/class/net); do
      [ "$i" = lo ] && continue
      if [ -z "$NET_IF_MAC" ] || [ "$(cat /sys/class/net/$i/address)" = "$NET_IF_MAC" ]; then
        ip addr add "$NET_IP/$NET_PREFIX" dev "$i"; ip route add default via "$NET_GW" 2>/dev/null
        [ -n "$NET_DNS" ] && echo "nameserver $NET_DNS" > /etc/resolv.conf
        GOT=$i; say "static $NET_IP/$NET_PREFIX gw $NET_GW on $i"; break
      fi
    done
  fi
  umount /mnt/esp 2>/dev/null
fi
say "net: $(ip -4 -o addr show scope global | tr -s ' ' | cut -d' ' -f2,4 | tr '\n' ' ') default: $(ip route | grep default | head -1)"

# sshd: fresh host keys every boot, one authorized key, no passwords
mkdir -p /root/.ssh /var/empty /etc/ssh; chmod 700 /root/.ssh
cp /etc/kf-authorized_keys /root/.ssh/authorized_keys; chmod 600 /root/.ssh/authorized_keys
ssh-keygen -A >/dev/null 2>&1
for k in /etc/ssh/ssh_host_*_key.pub; do say "hostkey $(ssh-keygen -lf "$k")"; done
/usr/sbin/sshd -f /etc/ssh/sshd_config.kf && say "sshd up on 22" || say "sshd FAILED"
say "READY ip=$(ip -4 -o addr show scope global | tr -s ' ' | cut -d' ' -f4 | head -1) autorestore=${AUTORESTORE_S}s"

# recovery timer: no ssh login within AUTORESTORE_S -> put the Windows boot files back and reboot
(
  t=0
  while [ "$t" -lt "$AUTORESTORE_S" ]; do
    if ps 2>/dev/null | grep -q '[s]shd: root@'; then touch /run/kf-login-seen; say "ssh login seen; recovery timer disarmed"; exit 0; fi
    sleep 5; t=$((t + 5))
  done
  say "NO LOGIN in ${AUTORESTORE_S}s: automatic restore of the Windows boot files, then reboot"
  /usr/local/bin/kf-restore-esp && say "restore ok" || say "restore FAILED"
  sync; echo b > /proc/sysrq-trigger
) &

# keep PID 1 alive; a shell on the serial console for debugging, never required
while true; do sleep 3600; done
INIT
}

emit_restore_esp() {  # /usr/local/bin/kf-restore-esp : copy the backups back over both boot paths
  cat <<'RESTORE'
#!/bin/sh
# Mount the ESP read-write, restore \EFI\Microsoft\Boot\bootmgfw.efi and \EFI\BOOT\BOOTX64.EFI from
# \EFI\kf-backup, verify by sha256, leave the backup folder in place. Exit 0 only when both match.
set -u
ESP=${1:-}
mkdir -p /mnt/esp
if [ -z "$ESP" ]; then
  for p in $(ls /sys/class/block | grep -v '^loop'); do
    [ "$(blkid -o value -s TYPE /dev/$p 2>/dev/null)" = vfat ] || continue
    mount -t vfat -o ro,iocharset=iso8859-1 "/dev/$p" /mnt/esp 2>/dev/null || continue
    [ -d /mnt/esp/EFI/kf-backup ] && { ESP=/dev/$p; umount /mnt/esp; break; }
    umount /mnt/esp
  done
fi
[ -n "$ESP" ] || { echo "no ESP with EFI/kf-backup found"; exit 2; }
mount -t vfat -o rw,iocharset=iso8859-1 "$ESP" /mnt/esp || exit 3
B=/mnt/esp/EFI/kf-backup; rc=0
restore() {  # backup-name target
  if [ ! -f "$B/$1" ]; then echo "no backup $1 (the original had no such file): leaving $2 as is"; return 0; fi
  mkdir -p "$(dirname "/mnt/esp/$2")"; cp "$B/$1" "/mnt/esp/$2" && sync
  [ "$(sha256sum < "$B/$1")" = "$(sha256sum < "/mnt/esp/$2")" ] && echo "restored $2" || { echo "MISMATCH $2"; rc=1; }
}
restore bootmgfw.efi EFI/Microsoft/Boot/bootmgfw.efi
restore BOOTX64.EFI EFI/BOOT/BOOTX64.EFI
# a fallback path that did not exist before the swap must not keep pointing at the UKI
if [ -f "$B/BOOTX64.EFI.absent" ]; then rm -f /mnt/esp/EFI/BOOT/BOOTX64.EFI; echo "removed EFI/BOOT/BOOTX64.EFI (absent originally)"; fi
sync; umount /mnt/esp
exit $rc
RESTORE
}

emit_sshd_config() {
  cat <<'SSHD'
Port 22
PermitRootLogin prohibit-password
PasswordAuthentication no
KbdInteractiveAuthentication no
PubkeyAuthentication yes
AuthorizedKeysFile /root/.ssh/authorized_keys
PrintMotd no
Subsystem sftp /usr/lib/ssh/sftp-server
ClientAliveInterval 30
SSHD
}

emit_cmdline() { echo "console=tty0 console=$SERIAL rdinit=/init loglevel=4 net.ifnames=0 kf.autorestore=$AUTORESTORE_S ${EXTRA_CMDLINE:-}"; }

selfcheck() {  # TEST_ONLY=1: syntax-check every generated script with busybox/sh, no network, no root
  local d; d=$(mktemp -d); trap 'rm -rf "$d"' RETURN
  emit_init > "$d/init"; emit_restore_esp > "$d/restore"; emit_sshd_config > "$d/sshd"; emit_cmdline > "$d/cmdline"
  sh -n "$d/init" && sh -n "$d/restore" || die "generated shell does not parse"
  grep -q 'kf.autorestore' "$d/cmdline" || die "cmdline lacks the recovery timer"
  grep -q 'prohibit-password' "$d/sshd" || die "sshd config must not allow passwords"
  echo "SELFCHECK_OK"
}

if [ "${TEST_ONLY:-0}" = 1 ]; then selfcheck; exit 0; fi
case "${EMIT:-}" in restore) emit_restore_esp; exit 0;; init) emit_init; exit 0;; esac

# ------------------------------------------------------------------------------------------------
: "${AUTH_KEY_PUB:?set AUTH_KEY_PUB to the public key file to authorize}"
[ -r "$AUTH_KEY_PUB" ] || die "cannot read $AUTH_KEY_PUB"
grep -q 'PRIVATE KEY' "$AUTH_KEY_PUB" && die "$AUTH_KEY_PUB holds a private key: refusing to bake it into an image"
grep -q '^ssh-' "$AUTH_KEY_PUB" || die "$AUTH_KEY_PUB is not a public key"
[ "$(id -u)" = 0 ] || die "needs root (unshare + chroot)"
WORK=${WORK:-/data/winimage_skeleton_20261107/uki_work}
OUT=${OUT:-$WORK/kf-alpine.efi}
for t in curl cpio objcopy unshare sha256sum tar; do command -v $t >/dev/null || die "missing tool $t"; done
mkdir -p "$WORK"; R=$WORK/rootfs; DL=$WORK/dl; mkdir -p "$DL"

tgz=$DL/alpine-minirootfs-$ALPINE_VER-x86_64.tar.gz
if [ ! -f "$tgz" ]; then curl -fsSL -o "$tgz" "$MIRROR/$ALPINE_BRANCH/releases/x86_64/alpine-minirootfs-$ALPINE_VER-x86_64.tar.gz"; fi
[ "$(sha256sum < "$tgz" | cut -d' ' -f1)" = "$ROOTFS_SHA256" ] || die "minirootfs sha256 differs from the pin"
log "minirootfs sha256 ok"

rm -rf "$R"; mkdir -p "$R"; tar -xzf "$tgz" -C "$R"
printf '%s/main\n%s/community\n' "$MIRROR/$ALPINE_BRANCH" "$MIRROR/$ALPINE_BRANCH" > "$R/etc/apk/repositories"
cp /etc/resolv.conf "$R/etc/resolv.conf"
cat > "$R/tmp/inchroot.sh" <<EOS
set -e
apk update
apk add --no-cache linux-virt $PKGS
EOS
# a private mount namespace: the bind mounts vanish when it ends, the host is never touched
unshare -m --propagation private sh -c "
  mount -t proc proc '$R/proc'; mount --bind /dev '$R/dev'
  chroot '$R' /bin/sh /tmp/inchroot.sh
  umount '$R/dev' '$R/proc'" || die "apk in the chroot failed"
rm -f "$R/tmp/inchroot.sh"

KVER=$(ls "$R/lib/modules" | head -1); [ -n "$KVER" ] || die "no kernel modules installed"
VMLINUZ=$(ls "$R"/boot/vmlinuz-virt) ; [ -f "$VMLINUZ" ] || die "no vmlinuz-virt"
log "kernel $KVER"

# module closure of MODS (modprobe --show-depends resolves the dependencies from modules.dep)
unshare -m --propagation private sh -c "
  mount -t proc proc '$R/proc'; mount --bind /dev '$R/dev'
  chroot '$R' depmod -a $KVER
  umount '$R/dev' '$R/proc'"
: > "$WORK/mods.keep"; : > "$WORK/mods.list"
for m in $MODS; do
  deps=$(chroot "$R" modprobe -S "$KVER" --show-depends "$m" 2>/dev/null | sed -n 's/^insmod //p' | awk '{print $1}') || deps=
  if [ -z "$deps" ]; then
    # built into the kernel or not shipped: not an error, init logs it
    log "module $m: not a loadable module here (built in, or absent)"; echo "$m" >> "$WORK/mods.list"; continue
  fi
  echo "$m" >> "$WORK/mods.list"; echo "$deps" >> "$WORK/mods.keep"
done
sort -u "$WORK/mods.keep" -o "$WORK/mods.keep"

# initramfs tree: the rootfs without the kernel, the full module tree, docs, apk state and firmware
I=$WORK/initramfs; rm -rf "$I"; mkdir -p "$I"
( cd "$R" && tar --exclude=./boot --exclude=./lib/modules --exclude=./usr/share/doc --exclude=./usr/share/man \
      --exclude=./usr/share/info --exclude=./var/cache --exclude=./lib/firmware --exclude=./usr/share/locale \
      --exclude=./proc/* --exclude=./dev/* -cf - . ) | tar -xf - -C "$I"
mkdir -p "$I/lib/modules/$KVER"
while read -r f; do [ -n "$f" ] || continue; mkdir -p "$I$(dirname "$f")"; cp "$R$f" "$I$f"; done < "$WORK/mods.keep"
for f in modules.order modules.builtin modules.builtin.modinfo; do cp "$R/lib/modules/$KVER/$f" "$I/lib/modules/$KVER/" 2>/dev/null || true; done
# modules.dep for the subset: the chroot's own depmod, run on a copy placed inside the chroot
mkdir -p "$R/tmp/mt/lib/modules"; cp -a "$I/lib/modules/$KVER" "$R/tmp/mt/lib/modules/"
chroot "$R" depmod -b /tmp/mt "$KVER"; cp "$R/tmp/mt/lib/modules/$KVER"/modules.* "$I/lib/modules/$KVER/"; rm -rf "$R/tmp/mt"
mkdir -p "$I/proc" "$I/sys" "$I/dev" "$I/run" "$I/tmp" "$I/mnt/esp" "$I/root" "$I/usr/local/bin" "$I/var/empty"
cp "$AUTH_KEY_PUB" "$I/etc/kf-authorized_keys"
cp "$WORK/mods.list" "$I/etc/kf-modules"
emit_init > "$I/init"; chmod 755 "$I/init"
emit_restore_esp > "$I/usr/local/bin/kf-restore-esp"; chmod 755 "$I/usr/local/bin/kf-restore-esp"
emit_sshd_config > "$I/etc/ssh/sshd_config.kf"
sed -i 's|^root:.*|root:*:0:0:root:/root:/bin/sh|' "$I/etc/passwd" 2>/dev/null || true
sed -i 's|^root:[^:]*:|root:*:|' "$I/etc/shadow" 2>/dev/null || true
grep -q '^sshd:' "$I/etc/passwd" || echo 'sshd:x:22:22:sshd:/var/empty:/sbin/nologin' >> "$I/etc/passwd"
grep -q '^sshd:' "$I/etc/group" || echo 'sshd:x:22:' >> "$I/etc/group"
rm -rf "$I/etc/ssh/ssh_host_"*   # keys are generated at every boot

# kernel support for the initramfs compression
CFG=$R/boot/config-$KVER
comp=zstd; grep -q '^CONFIG_RD_ZSTD=y' "$CFG" 2>/dev/null || comp=xz
grep -q "^CONFIG_RD_${comp^^}=y" "$CFG" 2>/dev/null || comp=gzip
log "initramfs compression: $comp"
case $comp in zstd) cz="zstd -19 -T0 -q -c";; xz) cz="xz -9 --check=crc32 -T0 -c";; gzip) cz="gzip -9 -c";; esac
( cd "$I" && find . | cpio -o -H newc --quiet ) | $cz > "$WORK/initramfs.img"
log "initramfs $(stat -c %s "$WORK/initramfs.img") bytes"

# the EFI stub (systemd's linuxx64.efi.stub, from Alpine's systemd-efistub package)
STUB=$WORK/linuxx64.efi.stub
if [ ! -f "$STUB" ]; then
  unshare -m --propagation private sh -c "
    mount -t proc proc '$R/proc'; mount --bind /dev '$R/dev'
    chroot '$R' apk add --no-cache systemd-efistub
    umount '$R/dev' '$R/proc'" || die "cannot install systemd-efistub"
  cp "$R"/usr/lib/systemd/boot/efi/linuxx64.efi.stub "$STUB" || die "no linuxx64.efi.stub in the package"
fi
emit_cmdline > "$WORK/cmdline.txt"
printf 'NAME="kf-alpine"\nID=kf-alpine\nVERSION_ID=%s\n' "$ALPINE_VER" > "$WORK/osrel.txt"
# layout per the UKI specification: sections at fixed VMAs after the stub
align() { echo $(( ($1 + 4095) / 4096 * 4096 )); }
end=$(objdump -h "$STUB" | awk 'NF==7 && $1 ~ /^[0-9]+$/ {s=strtonum("0x"$3)+strtonum("0x"$4); if (s>m) m=s} END{printf "%d", m}')
off=$(align $(( end )))
add=()
for sec in osrel:"$WORK/osrel.txt" cmdline:"$WORK/cmdline.txt" linux:"$VMLINUZ" initrd:"$WORK/initramfs.img"; do
  name=${sec%%:*}; file=${sec#*:}
  add+=(--add-section ".$name=$file" --change-section-vma ".$name=$(printf '0x%x' "$off")")
  off=$(align $(( off + $(stat -c %s "$file") )))
done
objcopy "${add[@]}" "$STUB" "$OUT"
sz=$(stat -c %s "$OUT")
echo "UKI_BUILT out=$OUT bytes=$sz sha256=$(sha256sum < "$OUT" | cut -d' ' -f1) kernel=$KVER alpine=$ALPINE_VER initramfs=$comp"
