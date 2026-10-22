#!/bin/bash
set -euo pipefail
exec >/root/prov/bw580_on_evtx_fixed_controller.txt 2>&1
trap 'rc=$?; echo "EVTX_READONLY_EXIT rc=$rc $(date -u -Is)"' EXIT
grep '^LINUX_ARM_EXIT kind=raw version=595 rc=0 ' /root/prov/sweep_linux_raw595.log
grep '^LINUX_ARM_EXIT kind=cuda version=595 rc=0 ' /root/prov/sweep_linux_cuda595.log
[ -z "$(pgrep -x qemu-system-x86 || true)" ]
[ ! -e /root/prov/bw580_on_evtx_fixed.started ]
date -u -Is >/root/prov/bw580_on_evtx_fixed.started
image=/workspace/sweep-win-56788212/kfwin/disk/bw580_on.qcow2
[ -f "$image" ]
chmod 0400 "$image"
modprobe nbd max_part=16
node=/dev/nbd15
[ -b "$node" ] && [ ! -e /sys/block/nbd15/pid ]
[ "$(cat /sys/block/nbd15/size)" = 0 ]
qemu-nbd --read-only --format=qcow2 --connect "$node" "$image"
finish() {
 rc=$?; trap - EXIT
 python3 - "$image" <<'PY'
from pathlib import Path
import subprocess,sys
p=Path('/sys/block/nbd15/pid')
if p.exists():
 pid=p.read_text().strip();args=Path('/proc',pid,'cmdline').read_bytes().split(b'\0')
 assert sys.argv[1].encode() in args and b'--read-only' in args,'Not our read-only NBD server; do not disconnect'
 subprocess.run(['qemu-nbd','--disconnect','/dev/nbd15'],check=True)
PY
 echo "EVTX_READONLY_EXIT rc=$rc $(date -u -Is)"
 exit "$rc"
}
trap finish EXIT
[ "$(cat /sys/block/nbd15/ro)" = 1 ]
udevadm settle --timeout=15
lsblk -b -J -o PATH,SIZE,FSTYPE "$node" >/root/prov/bw580_on_evtx_block_layout.json
part=$(python3 /root/sweep-evtx-tools/block_layout.py /root/prov/bw580_on_evtx_block_layout.json)
mkdir -p /root/prov/bw580_on_evtx_raw
for name in System Microsoft-Windows-Kernel-PnP%4Configuration; do
 out=/root/prov/bw580_on_evtx_raw/$name.evtx
 # Read-only NTFS extraction. Never repair/recover/mount-write the guest disk.
 timeout 60 ntfscat "$part" "/Windows/System32/winevt/Logs/$name.evtx" >"$out"
 [ "$(stat -c %s "$out")" -le 67108864 ]
 (
  ulimit -f 131072
  timeout 90 evtxexport -f xml -m items "$out" >"$out.xml" 2>"$out.stderr"
 )
done
python3 /root/sweep-evtx-tools/project.py
