#!/usr/bin/env bash
# SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later
# push_image.sh — D4 of docs/design/V3_DIE_SWEEP_20261014.md: copy the Windows LTSC baseline from the owner
# host to a rented box WITHOUT staging it on the relay (the relay is this dev machine; it only pipes).
#   push_image.sh <box-ssh-alias> [jobs=3]
# 1 GiB chunks, each: host bounded `pread | zstd -3` -> relay -> box `zstd -d | dd conv=notrunc seek=` into /root/win/base.qcow2.
# Every pipeline must succeed before a chunk marker is written. A final full-file SHA256 comparison
# also checks legacy resume markers and refuses a truncated or otherwise different image.
# Also copies OVMF_CODE_4M.fd (distro) and the baseline's OVMF_VARS.fd. Needs ssh to $HOST and to the box.
set -euo pipefail
BOX=${1:?box ssh alias}; JOBS=${2:-3}
HOST=${IMG_HOST:-root@172.22.1.20}
SRC=${IMG_SRC:-/var/lib/kf-windows-20261005/baseline}
HERE=$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)
printf -v SRC_FILE_Q '%q' "$SRC/windows.qcow2"
printf -v RANGE_READER_Q '%q' "$(cat "$HERE/stream_image_range.py")"
SZ=$(ssh -o BatchMode=yes "$HOST" "stat -c %s $SRC_FILE_Q") || exit 2
[[ "$SZ" =~ ^[0-9]+$ && "$SZ" -gt 0 ]] || { echo 'PUSH_REFUSED image size'; exit 2; }
N=$(( (SZ + 1073741823) / 1073741824 ))
echo "PUSH_START $(date -Is) size=$SZ chunks=$N jobs=$JOBS box=$BOX"
ssh "$BOX" 'mkdir -p /root/win/done /root/win/secrets; [ -e /root/win/base.qcow2 ] || truncate -s '"$SZ"' /root/win/base.qcow2'
# firmware: code from the owner host's distro (matches the varstore), vars from the baseline
ssh -o BatchMode=yes "$HOST" "cat /usr/share/OVMF/OVMF_CODE_4M.fd" | ssh "$BOX" 'cat > /root/win/OVMF_CODE_4M.fd'
ssh -o BatchMode=yes "$HOST" "cat $SRC/OVMF_VARS.fd" | ssh "$BOX" 'cat > /root/win/OVMF_VARS.fd'
one() {
    # xargs launches a fresh bash; the parent's pipefail setting is not inherited.
    set -o pipefail
    i=$1; n=$(printf %02d "$i")
    offset=$((i * 1073741824)); bytes=$((SZ - offset))
    [ "$bytes" -le 1073741824 ] || bytes=1073741824
    ssh "$BOX" "[ -e /root/win/done/c$n ]" && return 0
    for try in 1 2 3 4 5 6; do
        if ssh -o BatchMode=yes "$HOST" "set -o pipefail; nice -n 19 python3 -c $RANGE_READER_Q $SRC_FILE_Q $offset $bytes | nice -n 19 zstd -3 -T2 -q -c" \
           | ssh "$BOX" "set -o pipefail; zstd -d -q -c | dd of=/root/win/base.qcow2 bs=1M seek=$((i * 1024)) conv=notrunc status=none"; then
            # The sender's exit status is known only after the whole relay pipeline completes.
            ssh "$BOX" "touch /root/win/done/c$n" || return 1
            echo "CHUNK $n ok try=$try $(date +%H:%M:%S)"; return 0
        fi
        echo "CHUNK $n FAILED try=$try $(date +%H:%M:%S)"; sleep 5
    done
    return 1
}
export -f one; export BOX HOST SRC SZ SRC_FILE_Q RANGE_READER_Q
seq 0 $((N - 1)) | xargs -P "$JOBS" -I{} bash -c 'one {}'
done_n=$(ssh "$BOX" 'ls /root/win/done | wc -l')
echo "PUSH_EXIT $(date -Is) chunks_done=$done_n of $N"
[ "$done_n" = "$N" ]
source_hash=$(ssh -o BatchMode=yes "$HOST" "nice -n 19 sha256sum $SRC_FILE_Q")
box_hash=$(ssh "$BOX" 'nice -n 19 sha256sum /root/win/base.qcow2')
source_hash=${source_hash%% *}; box_hash=${box_hash%% *}
[[ "$source_hash" =~ ^[0-9a-f]{64}$ && "$box_hash" = "$source_hash" ]] || {
    echo 'PUSH_INTEGRITY FAIL: source and destination differ; verify chunks before resuming' >&2
    exit 2
}
echo "PUSH_INTEGRITY PASS sha256=$source_hash"
