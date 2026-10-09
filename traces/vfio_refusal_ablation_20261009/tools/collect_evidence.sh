#!/usr/bin/env bash
# SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later
# collect_evidence.sh RUNDIR OUT -- small, filtered evidence from one ablation boot (RUNDIR =
# .../runs/session-*/<label>), for `git add`. Never copies gsp.jsonl/trace.log/qemu.log whole (GBs,
# and qemu.log may carry machine paths); it greps the lines that matter and keeps them under 1 MB.
set -eu
R=${1:?rundir}; O=${2:?outdir}
mkdir -p "$O"
cp "$R/boot1/command.json" "$O/command.json" 2>/dev/null || true
cp "$R/boot1/marker.txt" "$O/marker.txt" 2>/dev/null || true
cp "$R/ctl/click.txt" "$O/click.txt" 2>/dev/null || true
cp "$R/verify.txt" "$O/verify.txt" 2>/dev/null || true
grep -aE 'x-gsp-refuse|GSP REFUSED|UnloadingGuestDriver|BugCheck|TDR|VIDEO_TDR|panic|Xid' "$R/boot1/qemu.log" 2>/dev/null \
  | head -c 900000 > "$O/qemu-log-filtered.txt" || true
grep -ac 'x-gsp-refuse: REWROTE' "$R/boot1/qemu.log" 2>/dev/null > "$O/rewrite-count.txt" || true
tail -c 4000 "$R/boot1/qemu.log" 2>/dev/null > "$O/qemu-log-tail.txt" || true
SCRUB='s/172\.22\.1\.20/<host>/g'    # never write the trusted host's own IP into committed evidence either
find "$R/guestlogs" -maxdepth 1 -name 'snap-*.txt' 2>/dev/null | while read -r f; do
  sed "$SCRUB" "$f" > "$O/$(basename "$f")"
done
[ -f "$R/guestlogs/timeline.txt" ] && sed "$SCRUB" "$R/guestlogs/timeline.txt" > "$O/guestlogs-timeline.txt"
echo "collected into $O:"
ls -la "$O"
