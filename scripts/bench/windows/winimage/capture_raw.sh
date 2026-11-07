#!/usr/bin/env bash
# SPDX-License-Identifier: Apache-2.0
# capture_raw.sh -- from the DEV BOX: copy the raw disk of a Windows VM that is booted into the RAM Linux (the UKI),
# over the ssh connection the dev box opened (Vast boxes cannot reach the dev box, so the box never pushes).
# The disk is made read-only on the box first (blockdev --setro), so nothing can write to the Windows partitions.
#
#   capture_raw.sh HOST PORT KEY OUTDIR [dd|dd-qcow2|ntfsclone-qcow2|ntfsclone-probe] [DISK]
#
#   dd               whole disk:  dd | zstd  ->  OUTDIR/disk.raw.zst, sha256 of the RAW bytes taken on the box in
#                    the same pass (tee into sha256sum), so the dev box can verify after decompression.
#   dd-qcow2         the same stream, decompressed on the dev box straight into OUTDIR/disk.qcow2 through qemu-nbd
#                    (a free /dev/nbdN; needs the nbd module and root). No raw file, no .zst file: when the
#                    compressed stream is 19 GB, keeping it and a raw copy next to the qcow2 would not fit on /data.
#                    The raw sha256 is taken on both ends and compared.
#   ntfsclone-qcow2  the disk as a qcow2 built from: the first and last MiB (protective MBR + both GPT copies, byte for byte),
#                    the ESP and MSR (raw), and the NTFS partition through `ntfsclone --save-image` (USED clusters only),
#                    restored on the dev box with `ntfsclone --restore-image` into a loop device on the nbd device. Use this
#                    when the free space is not zero: on a windows-on-vast disk the old Linux data is still in the free
#                    space, so `dd` carries about 1.4 times the used clusters (measured, see the design doc).
#   ntfsclone-probe  measures ntfsclone --save-image of the largest NTFS partition (bytes and speed) into /dev/null;
#                    it writes nothing; use it to decide whether a "used clusters only" copy is worth the reassembly.
#
# Prints one CAPTURE_* line per fact (bytes, seconds, MB/s) and CAPTURE_DONE / CAPTURE_FAIL. Needs bash+zstd+sha256sum
# in the UKI (build_alpine_uki.sh installs them). A start marker and an exit line go to OUTDIR/capture.log.
set -uo pipefail
if [ "${1:-}" = __chunk ]; then
  # internal: copy chunk INDEX (CHUNK_MB MiB) of DISK over its own ssh stream into the nbd device at the same offset
  shift; HOST=$1 PORT=$2 KEY=$3 DISK=$4 NBD=$5 OUT=$6 IDX=$7 CMB=$8
  SSHC=(ssh -i "$KEY" -o IdentitiesOnly=yes -o BatchMode=yes -o ConnectTimeout=15 -o ServerAliveInterval=15 -o ServerAliveCountMax=8 -o StrictHostKeyChecking=no -o UserKnownHostsFile=/dev/null -o LogLevel=ERROR -p "$PORT" "root@$HOST")
  skip=$((IDX * CMB / 4)); cnt=$((CMB / 4))      # 4 MiB blocks
  for try in 1 2 3; do
    "${SSHC[@]}" "bash -c 'dd if=$DISK bs=4M skip=$skip count=$cnt status=none | tee >(sha256sum | cut -d\" \" -f1 > /tmp/c$IDX.sha) | zstd -T0 -3 -q -c; sleep 1'" \
      | zstd -d -q -c | tee >(sha256sum | cut -d' ' -f1 > "$OUT/chunks/c$IDX.local") | dd of="$NBD" bs=4M seek=$skip conv=sparse,notrunc status=none
    rc=${PIPESTATUS[0]}
    r=$("${SSHC[@]}" "cat /tmp/c$IDX.sha" 2>/dev/null); l=$(cat "$OUT/chunks/c$IDX.local" 2>/dev/null)
    if [ "$rc" = 0 ] && [ -n "$r" ] && [ "$r" = "$l" ]; then echo "$IDX $r" > "$OUT/chunks/c$IDX.ok"; exit 0; fi
    echo "chunk $IDX try $try failed rc=$rc remote=$r local=$l" >> "$OUT/chunks/errors.txt"
  done
  exit 1
fi
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
  dd-qcow2)
    modprobe nbd max_part=0 2>/dev/null; NBD=
    for n in /sys/block/nbd*; do [ -e "$n/pid" ] && continue; [ "$(cat "$n/size" 2>/dev/null)" = 0 ] && { NBD=/dev/$(basename "$n"); break; }; done
    [ -n "$NBD" ] || { say "CAPTURE_FAIL no free /dev/nbdN (modprobe nbd)"; exit 1; }
    rm -f "$OUT/disk.qcow2"; qemu-img create -q -f qcow2 "$OUT/disk.qcow2" "$BYTES" || exit 1
    qemu-nbd --connect="$NBD" --format=qcow2 --cache=writeback --discard=unmap --detect-zeroes=unmap "$OUT/disk.qcow2" || { say "CAPTURE_FAIL qemu-nbd"; exit 1; }
    trap 'qemu-nbd -d "$NBD" >/dev/null 2>&1; say "CAPTURE_EXIT rc=$? $(date -u +%FT%TZ)"' EXIT
    say "CAPTURE_NBD $NBD"
    STREAMS=${STREAMS:-6}; CMB=${CHUNK_MB:-1024}; NCH=$(( (BYTES + CMB * 1048576 - 1) / (CMB * 1048576) ))
    [ $((BYTES % (CMB * 1048576))) = 0 ] || say "note: disk size is not a multiple of the chunk; the last chunk reads past the end and is short"
    mkdir -p "$OUT/chunks"; rm -f "$OUT/chunks/"*
    say "CAPTURE_PARALLEL streams=$STREAMS chunk_MiB=$CMB chunks=$NCH (one ssh stream per chunk: a single stream is limited by the round-trip time)"
    seq 0 $((NCH - 1)) | xargs -P "$STREAMS" -I{} bash "$0" __chunk "$HOST" "$PORT" "$KEY" "$DISK" "$NBD" "$OUT" {} "$CMB"
    rc=$?; sync; qemu-nbd -d "$NBD" >/dev/null 2>&1; t1=$(date +%s.%N)
    nok=$(ls "$OUT/chunks/"*.ok 2>/dev/null | wc -l)
    cat "$OUT/chunks/"*.ok 2>/dev/null | sort -n > "$OUT/chunk_sha256.txt"
    TOTALSHA=$(sha256sum < "$OUT/chunk_sha256.txt" | cut -d' ' -f1)
    secs=$(echo "$t1 - $t0" | bc -l); QA=$(du -b "$OUT/disk.qcow2" | cut -f1)
    say "CAPTURE_DDQCOW2 rc=$rc raw_bytes=$BYTES chunks_ok=$nok/$NCH qcow2_bytes_on_disk=$QA seconds=$(printf %.1f "$secs") raw_MBps=$(echo "$BYTES / $secs / 1000000" | bc -l | cut -c1-6) chunk_list_sha256=$TOTALSHA"
    [ "$rc" = 0 ] && [ "$nok" = "$NCH" ] || { say "CAPTURE_FAIL dd-qcow2 rc=$rc chunks_ok=$nok/$NCH (see $OUT/chunks/errors.txt)"; exit 1; }
    ;;
  ntfsclone-qcow2)
    modprobe nbd max_part=0 2>/dev/null; NBD=
    for n in /sys/block/nbd*; do [ -e "$n/pid" ] && continue; [ "$(cat "$n/size" 2>/dev/null)" = 0 ] && { NBD=/dev/$(basename "$n"); break; }; done
    [ -n "$NBD" ] || { say "CAPTURE_FAIL no free /dev/nbdN"; exit 1; }
    P=$("${SSH[@]}" "lsblk -nrbo NAME,SIZE,FSTYPE $DISK | grep ntfs | sort -k2 -n | tail -1 | cut -d' ' -f1")
    POFF=$("${SSH[@]}" "echo \$((\$(cat /sys/class/block/$P/start) * 512))"); PSIZE=$("${SSH[@]}" "blockdev --getsize64 /dev/$P")
    [ -n "$P" ] && [ -n "$POFF" ] && [ -n "$PSIZE" ] || { say "CAPTURE_FAIL cannot find the NTFS partition"; exit 1; }
    say "CAPTURE_NTFS partition=$P offset=$POFF size=$PSIZE"
    rm -f "$OUT/disk.qcow2"; qemu-img create -q -f qcow2 "$OUT/disk.qcow2" "$BYTES" || exit 1
    qemu-nbd --connect="$NBD" --format=qcow2 --cache=writeback --discard=unmap --detect-zeroes=unmap "$OUT/disk.qcow2" || { say "CAPTURE_FAIL qemu-nbd"; exit 1; }
    LOOP=
    cleanup() { [ -n "$LOOP" ] && losetup -d "$LOOP" 2>/dev/null; qemu-nbd -d "$NBD" >/dev/null 2>&1; say "CAPTURE_EXIT rc=$? $(date -u +%FT%TZ)"; }
    trap cleanup EXIT
    # 1. head (MBR + primary GPT + gap), 2. ESP+MSR (between 1 MiB and the NTFS partition), 3. tail (backup GPT): raw
    cp_range() {  # name skipMiB countMiB
      "${SSH[@]}" "dd if=$DISK bs=1M skip=$2 count=$3 status=none | tee >(sha256sum | cut -d\" \" -f1 > /tmp/r_$1.sha) | zstd -T0 -3 -q -c; sleep 1" \
        | zstd -d -q -c | tee >(sha256sum | cut -d' ' -f1 > "$OUT/r_$1.local") | dd of="$NBD" bs=1M seek=$2 conv=notrunc status=none
      local rc=${PIPESTATUS[0]}; sleep 1
      [ "$rc" = 0 ] && [ "$("${SSH[@]}" "cat /tmp/r_$1.sha")" = "$(cat "$OUT/r_$1.local")" ] && say "CAPTURE_RANGE $1 ok skipMiB=$2 countMiB=$3 sha256=$(cat "$OUT/r_$1.local")" || { say "CAPTURE_FAIL range $1"; exit 1; }
    }
    cp_range head 0 1
    cp_range espmsr 1 $(( POFF / 1048576 - 1 ))
    cp_range tail $(( BYTES / 1048576 - 1 )) 1
    LOOP=$(losetup -f --show -o "$POFF" --sizelimit "$PSIZE" "$NBD") || { say "CAPTURE_FAIL losetup"; exit 1; }
    say "CAPTURE_LOOP $LOOP over $NBD"
    t0=$(date +%s.%N)
    "${SSH[@]}" "bash -c 'ntfsclone --save-image -O - /dev/$P 2>/tmp/ntfsclone.err | tee >(sha256sum | cut -d\" \" -f1 > /tmp/img.sha) | zstd -T0 -3 -q -c; sleep 1'" \
      | zstd -d -q -c | tee >(sha256sum | cut -d' ' -f1 > "$OUT/img.sha.local") | ntfsclone --restore-image --overwrite "$LOOP" - > "$OUT/ntfsclone_restore.log" 2>&1
    rc=${PIPESTATUS[0]}; t1=$(date +%s.%N); sync; sleep 1
    RS=$("${SSH[@]}" 'cat /tmp/img.sha' 2>/dev/null); LS=$(cat "$OUT/img.sha.local" 2>/dev/null)
    secs=$(echo "$t1 - $t0" | bc -l); QA=$(du -b "$OUT/disk.qcow2" | cut -f1)
    "${SSH[@]}" 'tail -c 400 /tmp/ntfsclone.err' > "$OUT/ntfsclone_save_tail.txt" 2>&1
    say "CAPTURE_NTFSCLONE_QCOW2 rc=$rc ntfs_seconds=$(printf %.1f "$secs") qcow2_bytes_on_disk=$QA image_stream_sha256_remote=$RS local=$LS"
    [ "$rc" = 0 ] && [ -n "$RS" ] && [ "$RS" = "$LS" ] || { say "CAPTURE_FAIL ntfsclone rc=$rc sha-equal=$([ "$RS" = "$LS" ] && echo yes || echo no)"; exit 1; }
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
