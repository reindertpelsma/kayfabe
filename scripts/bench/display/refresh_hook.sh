#!/usr/bin/env bash
# POST_CAPTURE_HOOK for boot_capture.sh, with display=on,gop=on. Add
# -qmp unix:/workspace/bench/run_<tag>.qmp,server=on,wait=off to QEMU's args.
# Exercise the real HMP nested wait and QMP dispatcher against fresh fbcon pixels.
set -euo pipefail
TAG=${1:?tag}
HERE=$(cd "$(dirname "$0")" && pwd)
BENCH=${BENCH_DIR:-/workspace/bench}
OUT=$BENCH/display/$TAG
mkdir -p "$OUT"
G=$HERE/../gssh_nv
gq() { timeout 60 "$G" "$@"; }
echo "REFRESH_START tag=$TAG $(date -Is)"
trap 'rc=$?; echo "REFRESH_EXIT rc=$rc $(date -Is)"' EXIT
gq 'sudo systemctl stop lightdm 2>/dev/null; sudo modprobe nvidia-drm modeset=1 fbdev=1; sudo chvt 1; sleep 3'
# Flush before the negative control: a wedged HMP may require killing that QEMU.
gq 'sudo sync'
for spec in hmp:41:red qmp:44:blue hmp:42:green; do
    IFS=: read -r interface ansi name <<<"$spec"
    gq "sudo sh -c 'printf \"\\033[${ansi}m\\033[2J\" > /dev/tty1'"
    sleep 2
    socket=$BENCH/run_$TAG.mon
    [ "$interface" != qmp ] || socket=$BENCH/run_$TAG.qmp
    timeout 15 python3 "$HERE/refresh_probe.py" "$interface" "$socket" \
        "$OUT/${interface}_${name}.ppm" | tee "$OUT/${interface}_${name}.json"
done
python3 - "$OUT" <<'PY'
import json
from pathlib import Path
import sys
root = Path(sys.argv[1])
rows = [(json.loads((root / name).read_text()), channel) for name, channel in [
    ('hmp_red.json', 0), ('qmp_blue.json', 2), ('hmp_green.json', 1)]]
for report, channel in rows:
    assert report['ok'], report
    rgb = bytes.fromhex(report['dominant'][0][0])
    assert rgb[channel] > 0 and all(rgb[channel] > c for i, c in enumerate(rgb) if i != channel), report
    assert report['dominant'][0][1] > report['width'] * report['height'] // 2, report
assert len({r['sha256'] for r, _ in rows}) == 3, rows
print('REFRESH_VERDICT PASS: HMP/QMP completed with three fresh fbcon colours')
PY
gq 'sudo sh -c '\''printf "\033[0m\033[2J" > /dev/tty1'\'''
