#!/usr/bin/env bash
# SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later
# push_image.sh — D4 of docs/design/V3_DIE_SWEEP_20261014.md: copy the Windows LTSC baseline from the owner
# host to a rented box WITHOUT staging it on the relay (the relay is this dev machine; it only pipes).
#   push_image.sh <box-ssh-alias> [jobs=3]
# 1 GiB chunks, each: host `dd | zstd -3` -> relay -> box `zstd -d | dd conv=notrunc seek=` into /root/win/base.qcow2.
# zstd's frame checksum is verified by the decompressor (a bad chunk exits non-zero and is retried); a chunk's
# marker /root/win/done/cNN is written only after that. Re-running skips the finished chunks (resumable).
# Also copies OVMF_CODE_4M.fd (distro) and the baseline's OVMF_VARS.fd. Needs ssh to $HOST and to the box.
set -uo pipefail
BOX=${1:?box ssh alias}; JOBS=${2:-3}
HOST=${IMG_HOST:-root@172.22.1.20}
SRC=${IMG_SRC:-/var/lib/kf-windows-20261005/baseline}
SZ=$(ssh -o BatchMode=yes "$HOST" "stat -c %s $SRC/windows.qcow2") || exit 2
N=$(( (SZ + 1073741823) / 1073741824 ))
echo "PUSH_START $(date -Is) size=$SZ chunks=$N jobs=$JOBS box=$BOX"
ssh "$BOX" 'mkdir -p /root/win/done /root/win/secrets; [ -e /root/win/base.qcow2 ] || truncate -s '"$SZ"' /root/win/base.qcow2'
# firmware: code from the owner host's distro (matches the varstore), vars from the baseline
ssh -o BatchMode=yes "$HOST" "cat /usr/share/OVMF/OVMF_CODE_4M.fd" | ssh "$BOX" 'cat > /root/win/OVMF_CODE_4M.fd'
ssh -o BatchMode=yes "$HOST" "cat $SRC/OVMF_VARS.fd" | ssh "$BOX" 'cat > /root/win/OVMF_VARS.fd'
one() {
    i=$1; n=$(printf %02d "$i")
    ssh "$BOX" "[ -e /root/win/done/c$n ]" && return 0
    for try in 1 2 3 4 5 6; do
        if ssh -o BatchMode=yes "$HOST" "nice -n 19 dd if=$SRC/windows.qcow2 bs=1M skip=$((i * 1024)) count=1024 status=none | nice -n 19 zstd -3 -T2 -q -c" \
           | ssh "$BOX" "zstd -d -q -c | dd of=/root/win/base.qcow2 bs=1M seek=$((i * 1024)) conv=notrunc status=none && touch /root/win/done/c$n"; then
            echo "CHUNK $n ok try=$try $(date +%H:%M:%S)"; return 0
        fi
        echo "CHUNK $n FAILED try=$try $(date +%H:%M:%S)"; sleep 5
    done
    return 1
}
export -f one; export BOX HOST SRC
seq 0 $((N - 1)) | xargs -P "$JOBS" -I{} bash -c 'one {}'
done_n=$(ssh "$BOX" 'ls /root/win/done | wc -l')
echo "PUSH_EXIT $(date -Is) chunks_done=$done_n of $N"
[ "$done_n" = "$N" ]
