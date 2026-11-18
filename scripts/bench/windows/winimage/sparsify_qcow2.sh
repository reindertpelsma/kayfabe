#!/usr/bin/env bash
# SPDX-License-Identifier: Apache-2.0
# sparsify_qcow2.sh IMAGE.qcow2 -- free the clusters of a qcow2 that hold only zeros, IN PLACE (no second copy: the dev box has
# little free space). The image is attached with qemu-nbd --detect-zeroes=unmap --discard=unmap and every all-zero 1 MiB block is
# rewritten with zeros, which qemu turns into an unmap (the file gets holes). Used after ntfsclone-qcow2 on a qemu-nbd without
# detect-zeroes: ntfsclone wrote zeros for the free clusters. The VM must not be running on the image. Needs root and the nbd module.
set -euo pipefail
IMG=${1:?image.qcow2}
modprobe nbd max_part=0
NBD=
for n in /sys/block/nbd*; do [ -e "$n/pid" ] && continue; [ "$(cat "$n/size")" = 0 ] && { NBD=/dev/$(basename "$n"); break; }; done
[ -n "$NBD" ] || { echo "no free nbd device" >&2; exit 1; }
before=$(du -b "$IMG" | cut -f1)
qemu-nbd --connect="$NBD" --format=qcow2 --cache=none --discard=unmap --detect-zeroes=unmap "$IMG"
trap 'qemu-nbd -d "$NBD" >/dev/null 2>&1' EXIT
python3 -I - "$NBD" <<'EOF'
import os, sys
fd = os.open(sys.argv[1], os.O_RDWR)
size = os.lseek(fd, 0, os.SEEK_END)
B = 1 << 20
zero = bytes(B)
freed = 0
off = 0
while off < size:
    n = min(B, size - off)
    buf = os.pread(fd, n, off)
    if buf == zero[:n]:
        os.pwrite(fd, zero[:n], off)
        freed += n
    off += n
os.fsync(fd)
print("SPARSIFY zero_bytes_rewritten=%d" % freed)
EOF
qemu-nbd -d "$NBD" >/dev/null; trap - EXIT
echo "SPARSIFY_DONE before=$before after=$(du -b "$IMG" | cut -f1)"
