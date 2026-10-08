#!/usr/bin/env bash
# SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later
# screen_sampler.sh RUN_DIR [SECONDS] [EVERY] — 2026-10-08: QMP `screendump device=kf0` of a running
# Windows guest (windows_broker.sh; this QEMU build has no libpng, so PPM then pnmtopng) every EVERY s for SECONDS s into RUN_DIR/shots/tNNNN.png, plus the
# host's NVRM Xid line count beside each sample (two kf3 VMs share the GPU: a new Xid is a stop sign).
# The PNG is kf3's own console surface (what the broker window shows), never a photo of a screen.
R=${1:?run dir}; S=${2:-900}; E=${3:-15}
Q=/var/lib/kf-windows-20261005/boundary-tools/qmp.py
mkdir -p "$R/shots"
for i in $(seq 0 "$E" "$S"); do
    t=$(printf %04d "$i")
    python3 "$Q" "$R/qmp.sock" cmd screendump "{\"filename\":\"$R/shots/t$t.ppm\",\"device\":\"kf0\"}" \
        > /dev/null 2>&1 && pnmtopng "$R/shots/t$t.ppm" > "$R/shots/t$t.png" 2>/dev/null && rm -f "$R/shots/t$t.ppm" \
        || echo "t$t screendump failed" >> "$R/shots/errors.txt"
    echo "t$t $(date +%T) xid=$(dmesg | grep -c 'NVRM: Xid')" >> "$R/shots/xid.txt"
    sleep "$E"
done
