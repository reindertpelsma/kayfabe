#!/usr/bin/env bash
# Install the app matrix INTO the powered-off fat guest image (guest.qcow2), on the stock
# hypervisor over slirp (the same shape as scripts/bench/provision_guest_llm.sh).
# Needs /workspace/apps/bundle.tgz (build_bundle.sh). ~20-40 min (torch, blender, models).
set -uo pipefail
BENCH=/workspace/bench; HERE="$(cd "$(dirname "$0")" && pwd)"
say(){ echo "[$(date -Is)] $*"; }
pgrep -x qemu-system-x86 >/dev/null && { say "⊘ a QEMU is already running — the bench is serialized, refusing"; exit 2; }
[ -s /workspace/apps/bundle.tgz ] || { say "⊘ no /workspace/apps/bundle.tgz — run build_bundle.sh first"; exit 2; }
# room for torch + blender + models (the base image has ~33 GB); cloud-init growpart extends / on boot
if [ ! -f "$BENCH/.apps_resized" ]; then qemu-img resize "$BENCH/guest.qcow2" +40G && touch "$BENCH/.apps_resized"; fi
# ⚠ -cpu host is required (x86-64-v2 for numpy; see provision_guest_llm.sh)
qemu-system-x86_64 -enable-kvm -cpu host -m 12G -smp 8 -display none \
  -drive if=virtio,file="$BENCH/guest.qcow2",format=qcow2 \
  -netdev user,id=n0,hostfwd=tcp::2222-:22 -device virtio-net-pci,netdev=n0 \
  -serial file:"$BENCH/appsprov_serial.log" -daemonize -pidfile "$BENCH/appsprov.pid"
GS="ssh -i $BENCH/guest_key -p 2222 -o StrictHostKeyChecking=no -o UserKnownHostsFile=/dev/null -o LogLevel=ERROR -o ConnectTimeout=8 -o ServerAliveInterval=20 ubuntu@127.0.0.1"
SCP="scp -i $BENCH/guest_key -P 2222 -o StrictHostKeyChecking=no -o UserKnownHostsFile=/dev/null -o LogLevel=ERROR"
for i in $(seq 1 60); do $GS true >/dev/null 2>&1 && break; sleep 5; done
$GS true >/dev/null 2>&1 || { say "⊘ guest never answered ssh"; exit 3; }
for i in $(seq 1 40); do case "$($GS 'cloud-init status 2>/dev/null | head -1' 2>/dev/null)" in *done*) break;; esac; sleep 5; done
say "guest up; root fs: $($GS 'df -h / | tail -1')"
$GS "sudo growpart /dev/vda 1 >/dev/null 2>&1; sudo resize2fs /dev/vda1 >/dev/null 2>&1; df -h / | tail -1"
# ⊘ pin the guest kernel BEFORE apt (the nvidia module is built for the running kernel)
$GS "sudo systemctl mask --now unattended-upgrades apt-daily.service apt-daily.timer apt-daily-upgrade.service apt-daily-upgrade.timer >/dev/null 2>&1; \
     sudo apt-mark hold linux-generic linux-image-generic linux-headers-generic >/dev/null 2>&1; uname -r"
$SCP /workspace/apps/bundle.tgz "$HERE/setup_side.sh" "$HERE/run_apps.sh" ubuntu@127.0.0.1:/var/tmp/ >/dev/null || { say "⊘ scp failed"; exit 4; }
$GS "sudo mkdir -p /opt/apps && sudo tar xzf /var/tmp/bundle.tgz -C /opt/apps && sudo cp /var/tmp/run_apps.sh /opt/apps/bundle/ && sudo rm -f /var/tmp/bundle.tgz && echo BUNDLE_IN=\$(du -sh /opt/apps/bundle | cut -f1)"
$GS "sudo bash /var/tmp/setup_side.sh" 2>&1 | grep -E '^(SETUP_|IMPORT_OK|HF_MODEL|/dev)' | sed 's/^/  guest: /'
$GS "uname -r; modinfo -F version nvidia 2>&1"
$GS "sudo poweroff" >/dev/null 2>&1
for i in $(seq 1 30); do pgrep -x qemu-system-x86 >/dev/null || break; sleep 3; done
pgrep -x qemu-system-x86 >/dev/null && { say "⚠ provisioning QEMU still up — killing"; kill "$(cat $BENCH/appsprov.pid)"; }
say "GUEST_APPS_DONE"
