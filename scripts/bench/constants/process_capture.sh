#!/bin/bash
# Decode one kfwc capture on the trusted host into repo-safe text (lane C, 2026-10-14).
# usage: process_capture.sh KF_CENSUS CAPTURE_DIR DECODE_TAG LABEL OUT_DIR
# Writes to OUT_DIR: <label>.controls.tsv/.functions.tsv/.anomalies.tsv (counts, sizes, statuses), <label>.bodies.tsv (bodies of the
# scrub allow-lists only: kf-census defaults + --allow-screened + traces/real_windows_ga104_20261014/allow_static_20261014.txt + the small
# group P/K/I/A and vblank-query controls named below; nothing on scrub::BOARD_DATA_CONTROLS), seqs.txt, binds.txt, cell.json,
# and the capture's sha256 (the capture itself never leaves the host).
set -eu
K=$1; D=$2; TAG=$3; L=$4; O=$5
HERE=$(cd "$(dirname "$0")" && pwd); R=$HERE/../../../traces/real_windows_ga104_20261014
mkdir -p "$O"; T=$(mktemp -d /dev/shm/proc.XXXXXX)
EXTRA="--allow 0x00731369 --allow 0x20802801 --allow 0x20802a12 --allow 0x50700709 --allow 0x50700118 --allow 0x0073014b --allow 0x2080121f --allow 0x20800a80 --allow 0x20800a87 --allow 0x20800ab8 --allow 0x0073010c --allow 0x5070020c --allow 0x50700117 --allow 0x5070020d"
$K --ogkm-tag "$TAG" --out "$O/$L" "$D/capture.kfgspcap" > "$O/$L.census.txt" 2>&1 || true
$K scrub --ogkm-tag "$TAG" --allow-screened --allow-list "$R/allow_static_20261014.txt" $EXTRA --out "$T/s.kfgspcap" "$D/capture.kfgspcap" > /dev/null 2>&1
$K --ogkm-tag "$TAG" --bodies --out "$T/b" "$T/s.kfgspcap" > /dev/null 2>&1
cp "$T/b.bodies.tsv" "$O/$L.bodies.tsv"
python3 -I "$HERE/../../../traces/real_windows_20261014/seqs.py" "$D/capture.kfgspcap" > "$O/seqs.txt"
python3 -I "$HERE/../../../traces/real_windows_20261014/binds.py" "$D/capture.kfgspcap" > "$O/binds.txt"
cp "$D/cell.json" "$O/cell.json"
sha256sum "$D/capture.kfgspcap" | sed "s#  .*#  $L/capture.kfgspcap#" > "$O/capture_sha256_host_only.txt"
rm -rf "$T"
echo "$L: $(sed -n 1p "$O/$L.census.txt" | cut -c1-110)"
