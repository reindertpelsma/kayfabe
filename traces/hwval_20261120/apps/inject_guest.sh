#!/usr/bin/env bash
# inject_guest.sh - the first half of scripts/apps/provision_guest_apps.sh (hwval_20261120): boot the powered-off fat
# guest on stock QEMU over slirp, copy /workspace/apps/bundle.tgz in and unpack it to /opt/apps, power off. It does NOT
# run setup_side.sh (torch, blender, models): the mini bundle is CUDA samples only. Run ON the box from a checkout
# (argument 1, default /root/kayfabe).
set -uo pipefail
REPO=${1:-/root/kayfabe}
APPS=/workspace/apps; BENCH=/workspace/bench
# a qcow2 overlay on the bench image: the base must never be written while another overlay (guest-<v>.qcow2) exists
IMG=${KF_GUEST_IMG:-$BENCH/guest-apps.qcow2}
[ -f "$IMG" ] || qemu-img create -q -f qcow2 -b "$BENCH/guest.qcow2" -F qcow2 "$IMG" || { echo "overlay create failed"; exit 2; }
say(){ echo "[$(date -Is)] $*"; }
pgrep -x qemu-system-x86 >/dev/null && { say "a QEMU is already running; the bench is serialized, refusing"; exit 2; }
[ -s "$APPS/bundle.tgz" ] || { say "no $APPS/bundle.tgz"; exit 2; }
qemu-system-x86_64 -enable-kvm -cpu host -m 12G -smp 8 -display none \
  -drive if=virtio,file="$IMG",format=qcow2 \
  -netdev user,id=n0,hostfwd=tcp::2222-:22 -device virtio-net-pci,netdev=n0 \
  -serial file:"$BENCH/appsprov_serial.log" -daemonize -pidfile "$BENCH/appsprov.pid"
trap '[ -s "$BENCH/appsprov.pid" ] && kill "$(cat "$BENCH/appsprov.pid")" 2>/dev/null; exit 143' TERM INT
GS="ssh -i $BENCH/guest_key -p 2222 -o StrictHostKeyChecking=no -o UserKnownHostsFile=/dev/null -o LogLevel=ERROR -o ConnectTimeout=8 -o ServerAliveInterval=20 ubuntu@127.0.0.1"
SCP="scp -i $BENCH/guest_key -P 2222 -o StrictHostKeyChecking=no -o UserKnownHostsFile=/dev/null -o LogLevel=ERROR"
for i in $(seq 1 60); do $GS true >/dev/null 2>&1 && break; sleep 5; done
$GS true >/dev/null 2>&1 || { say "guest never answered ssh"; exit 3; }
for i in $(seq 1 40); do case "$($GS 'cloud-init status 2>/dev/null | head -1' 2>/dev/null)" in *done*) break;; esac; sleep 5; done
say "guest up; root fs: $($GS 'df -h / | tail -1')"
$GS "sudo growpart /dev/vda 1 >/dev/null 2>&1; sudo resize2fs /dev/vda1 >/dev/null 2>&1; df -h / | tail -1"
$SCP "$APPS/bundle.tgz" ubuntu@127.0.0.1:/var/tmp/ >/dev/null || { say "scp failed"; exit 4; }
$GS "sudo mkdir -p /opt/apps && sudo tar xzf /var/tmp/bundle.tgz -C /opt/apps && sudo rm -f /var/tmp/bundle.tgz && echo BUNDLE_IN=\$(du -sh /opt/apps/bundle | cut -f1) && ls /opt/apps/bundle/samples | head -40 | tr '\n' ' '"
$GS "uname -r; modinfo -F version nvidia 2>&1"
$GS "sudo poweroff" >/dev/null 2>&1
for i in $(seq 1 30); do pgrep -x qemu-system-x86 >/dev/null || break; sleep 3; done
pgrep -x qemu-system-x86 >/dev/null && { say "provisioning QEMU still up, killing"; kill "$(cat $BENCH/appsprov.pid)"; }
say "GUEST_INJECT_DONE"
