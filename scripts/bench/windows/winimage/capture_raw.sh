#!/usr/bin/env bash
# SPDX-License-Identifier: Apache-2.0
# capture_raw.sh -- from the DEV BOX: copy the raw disk of a Windows VM that is booted into the RAM Linux (the UKI),
# over the ssh connection the dev box opened (Vast boxes cannot reach the dev box, so the box never pushes).
# The disk is made read-only on the box first (blockdev --setro), so nothing can write to the Windows partitions.
#
#   capture_raw.sh HOST PORT KEY OUTDIR [dd|ntfsclone-probe] [DISK]
#
#   dd               whole disk:  dd | zstd  ->  OUTDIR/disk.raw.zst, sha256 of the RAW bytes taken on the box in
#                    the same pass (tee into sha256sum), so the dev box can verify after decompression.
#   ntfsclone-probe  measures ntfsclone --save-image of the largest NTFS partition (bytes and speed) into /dev/null;
#                    it writes nothing; use it to decide whether a "used clusters only" copy is worth the reassembly.
#
# Prints one CAPTURE_* line per fact (bytes, seconds, MB/s) and CAPTURE_DONE / CAPTURE_FAIL. Needs bash+zstd+sha256sum
# in the UKI (build_alpine_uki.sh installs them). A start marker and an exit line go to OUTDIR/capture.log.
set -uo pipefail
HOST=${1:?host}; PORT=${2:?port}; KEY=${3:?private key file}; OUT=${4:?output dir}; MODE=${5:-dd}; DISK=${6:-}
mkdir -p "$OUT"; LOG=$OUT/capture.log
SSH=(ssh -i "$KEY" -o IdentitiesOnly=yes -o BatchMode=yes -o ConnectTimeout=15 -o ServerAliveInterval=15 -o ServerAliveCountMax=8 -o StrictHostKeyChecking=no -o UserKnownHostsFile=/dev/null -o LogLevel=ERROR -p "$PORT" "root@$HOST")
say() { echo "$*" | tee -a "$LOG"; }
say "CAPTURE_START $(date -u +%FT%TZ) mode=$MODE host=$HOST:$PORT out=$OUT pid=$$"
trap 'say "CAPTURE_EXIT rc=$? $(date -u +%FT%TZ)"' EXIT

"${SSH[@]}" 'lsblk -b -o NAME,SIZE,TYPE,FSTYPE,LABEL,PARTTYPENAME 2>/dev/null; echo ---; for d in /sys/block/*; do n=$(basename $d); case $n in loop*|ram*) continue;; esac; echo "$n $(cat $d/size) sectors ro=$(cat $d/ro) model=$(cat $d/device/model 2>/dev/null)"; done' 2>&1 | tee -a "$LOG"

if [ -z "$DISK" ]; then
  # the disk that holds an NTFS partition and a vfat ESP: the first match
  DISK=$("${SSH[@]}" 'for d in /sys/block/*; do n=$(basename $d); case $n in loop*|ram*) continue;; esac; [ -b /dev/$n ] || continue; if lsblk -nro FSTYPE /dev/$n | grep -q ntfs && lsblk -nro FSTYPE /dev/$n | grep -q vfat; then echo /dev/$n; break; fi; done' 2>/dev/null)
fi
[ -n "$DISK" ] || { say "CAPTURE_FAIL no disk with an NTFS and a vfat partition found"; exit 1; }
say "CAPTURE_DISK $DISK"
# nothing may be mounted from it, and it becomes read-only for the rest of the session
if "${SSH[@]}" "grep -q \"^$DISK\" /proc/mounts"; then say "CAPTURE_FAIL a partition of $DISK is mounted in the UKI"; exit 1; fi
"${SSH[@]}" "blockdev --setro $DISK; echo ro=\$(blockdev --getro $DISK) bytes=\$(blockdev --getsize64 $DISK)" | tee -a "$LOG"
BYTES=$("${SSH[@]}" "blockdev --getsize64 $DISK")
"${SSH[@]}" "sgdisk -p $DISK" 2>&1 | tee "$OUT/partition_table.txt" >/dev/null
"${SSH[@]}" "sgdisk --backup=/tmp/gpt.bin $DISK >/dev/null 2>&1; base64 /tmp/gpt.bin" 2>/dev/null | base64 -d > "$OUT/gpt_backup.bin" || true

t0=$(date +%s.%N)
case $MODE in
  dd)
    # raw sha256 on the box in the same pass; the compressed stream is the only thing sent
    "${SSH[@]}" "bash -c 'dd if=$DISK bs=4M status=none | tee >(sha256sum | cut -d\" \" -f1 > /tmp/raw.sha256) | zstd -T0 -3 -q -c; wait; sync'" > "$OUT/disk.raw.zst" 2> "$OUT/dd.err"
    rc=$?
    t1=$(date +%s.%N)
    RAWSHA=$("${SSH[@]}" 'cat /tmp/raw.sha256' 2>/dev/null)
    echo "$RAWSHA" > "$OUT/disk.raw.sha256"
    ZB=$(stat -c %s "$OUT/disk.raw.zst")
    secs=$(echo "$t1 - $t0" | bc -l)
    say "CAPTURE_DD rc=$rc raw_bytes=$BYTES zst_bytes=$ZB seconds=$(printf %.1f "$secs") raw_MBps=$(echo "$BYTES / $secs / 1000000" | bc -l | cut -c1-6) zst_MBps=$(echo "$ZB / $secs / 1000000" | bc -l | cut -c1-6) raw_sha256=$RAWSHA"
    [ "$rc" = 0 ] && [ -n "$RAWSHA" ] && [ "$ZB" -gt 1000000 ] || { say "CAPTURE_FAIL dd rc=$rc"; exit 1; }
    ;;
  ntfsclone-probe)
    P=$("${SSH[@]}" "lsblk -nrbo NAME,SIZE,FSTYPE $DISK | grep ntfs | sort -k2 -n | tail -1 | cut -d' ' -f1")
    [ -n "$P" ] || { say "CAPTURE_FAIL no NTFS partition"; exit 1; }
    "${SSH[@]}" "ntfsclone --save-image -O - /dev/$P 2>/tmp/ntfsclone.err | zstd -T0 -3 -q -c | wc -c; tail -c 600 /tmp/ntfsclone.err" 2>&1 | tee -a "$LOG"
    t1=$(date +%s.%N); say "CAPTURE_NTFSCLONE_PROBE partition=$P seconds=$(printf %.1f "$(echo "$t1 - $t0" | bc -l)")"
    ;;
  *) say "CAPTURE_FAIL unknown mode $MODE"; exit 1;;
esac
say "CAPTURE_DONE mode=$MODE"
