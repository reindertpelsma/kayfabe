#!/usr/bin/env bash
# RUN ON .32: stop guest, inject .32-root pubkey into guest image /home/ubuntu/.ssh/authorized_keys
set -u
systemctl stop bm32phase0 2>/dev/null
pkill -9 -f "[q]emu-system-x86_64"; sleep 3
PUB=$(ssh-keygen -y -f /root/.ssh/id_ed25519)   # derive from PRIVATE key — .pub on disk may be stale/mismatched
echo "injecting: ${PUB:0:40}..."
modprobe nbd max_part=16 2>&1
QN=/opt/qemu-nvkvm/bin/qemu-nbd
$QN --disconnect /dev/nbd0 >/dev/null 2>&1
$QN --connect=/dev/nbd0 /opt/nvkvm-guest/ubuntu-24.04.qcow2 2>&1
sleep 2
echo "=== partitions ==="; lsblk /dev/nbd0 2>&1 | head
# find the root partition (largest ext4)
ROOTP=""
for p in /dev/nbd0p1 /dev/nbd0p2 /dev/nbd0p3 /dev/nbd0; do
  [ -b "$p" ] || continue
  fst=$(blkid -o value -s TYPE "$p" 2>/dev/null)
  echo "  $p type=$fst"
  [ "$fst" = ext4 ] && ROOTP="$p"
done
[ -n "$ROOTP" ] || { echo "NO ext4 root found"; $QN --disconnect /dev/nbd0; exit 1; }
echo "root partition: $ROOTP"
mkdir -p /mnt/ginj
mount "$ROOTP" /mnt/ginj
U=/mnt/ginj/home/ubuntu
mkdir -p "$U/.ssh"; chmod 700 "$U/.ssh"
touch "$U/.ssh/authorized_keys"
grep -qF "$PUB" "$U/.ssh/authorized_keys" || echo "$PUB" >> "$U/.ssh/authorized_keys"
# ubuntu uid/gid in guest is 1000
chown -R 1000:1000 "$U/.ssh"
echo "=== authorized_keys now ==="; cat "$U/.ssh/authorized_keys"
sync; umount /mnt/ginj
$QN --disconnect /dev/nbd0
echo "INJECT_DONE"
