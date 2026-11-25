#!/bin/bash
# SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later
# Box setup for the rootless proof, AS ROOT, after provision_full.sh READY and windows_gate.sh --prepare
# (traces/rootless_proof_20261125/PREREGISTRATION.md, "Box setup"). Not part of the claim: it prepares a user and a tree.
set -euo pipefail
U=kfrun
echo "SETUP_START $(date -Is)"
sudo --version | head -1
echo "== before any change"
ls -l /dev/kvm /dev/nvidia* 2>&1
echo "dmesg_restrict=$(cat /proc/sys/kernel/dmesg_restrict) protected_regular=$(cat /proc/sys/fs/protected_regular) root_memlock_kb=$(ulimit -l)"
nvidia-smi --query-gpu=name,driver_version,memory.total --format=csv,noheader
if [ ! -e /dev/nvidia-uvm ]; then echo "nvidia-uvm missing: nvidia-modprobe -c 0 -u"; nvidia-modprobe -c 0 -u; ls -l /dev/nvidia-uvm; fi
useradd --create-home --shell /bin/bash --groups kvm "$U"
id "$U"
sudo -l -U "$U" 2>&1 | tail -1 || true
REV=$(git -C /root/kayfabe rev-parse --short=8 HEAD)
echo "rev=$REV"
[ ! -d "/workspace/bench/kf3-bins/$REV" ] || mv "/workspace/bench/kf3-bins/$REV" "/workspace/bench/kf3-bins-root-$REV"
rm -rf /workspace/kf-src
git clone -q --no-hardlinks /root/kayfabe /workspace/kf-src
git -C /workspace/kf-src checkout -q "$REV"
install -d /workspace/winvm /workspace/rlprov
chown -R "$U:$U" /workspace/bench /workspace/kf-src /workspace/winvm /workspace/rlprov
[ -e /tmp/kayfabe-fastguest.lock ] || : > /tmp/kayfabe-fastguest.lock
chmod 0666 /tmp/kayfabe-fastguest.lock
touch /var/log/kfroot-sudo.log && chown "root:$U" /var/log/kfroot-sudo.log && chmod 0640 /var/log/kfroot-sudo.log
sysctl -w kernel.dmesg_restrict=0
echo "== after"
ls -l /dev/kvm /dev/nvidia* 2>&1
echo "SETUP_EXIT rc=0 $(date -Is)"
