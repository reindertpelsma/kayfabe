#!/usr/bin/env bash
# imgcmp.sh <resdir> [ffmpeg] — the CONTENT grade for image items whose digest is NONDET (differs between
# the two bare-metal runs, e.g. Cycles path tracing: GPU atomics make the sample accumulation order vary).
# For each such item it extracts the image from the bare-metal run 1, run 2 and guest artefacts and computes
#   floor = PSNR(bare metal 1, bare metal 2)   — how far bare metal is from ITSELF
#   guest = PSNR(bare metal 1, guest)
# and prints GSET_IMG item=<i> key=<file> floor=<dB> guest=<dB> verdict=MATCH|DIFF, where MATCH means the
# guest is within 3 dB of bare metal's own run-to-run noise (or both are identical: inf).
# ⊘ Only for NONDET digests: an item whose digest is stable on bare metal is graded byte for byte.
set -uo pipefail
R=${1:?resdir}; FF=${2:-/workspace/video/ff/bin/ffmpeg}
psnr(){ "$FF" -hide_banner -nostats -i "$1" -i "$2" -lavfi psnr -f null - 2>&1 | grep -o 'average:[^ ]*' | tail -1 | cut -d: -f2; }
T=$(mktemp -d); trap 'rm -rf "$T"' EXIT
for it in $(sed -n 's/^GSET_DIG side=host item=\([^ ]*\) key=png .*/\1/p' "$R/host.dig" | sort -u); do
    h1=$(grep -a "item=$it key=png " "$R/host.dig" | tail -1 | sed 's/.*val=//')
    h2=$(grep -a "item=$it key=png " "$R/host2.dig" 2>/dev/null | tail -1 | sed 's/.*val=//')
    [ -n "$h2" ] && [ "$h1" != "$h2" ] || continue          # stable on bare metal: byte-graded elsewhere
    g=$R/$it.guest_art.tar; [ -f "$R/iso/$it.guest_art.tar" ] && g=$R/iso/$it.guest_art.tar
    tar -xOf "$R/$it.host_art.tar" ./out.png > "$T/h1.png" 2>/dev/null
    tar -xOf "$R/h2/$it.host_art.tar" ./out.png > "$T/h2.png" 2>/dev/null
    tar -xOf "$g" ./out.png > "$T/g.png" 2>/dev/null
    if [ ! -s "$T/h1.png" ] || [ ! -s "$T/h2.png" ] || [ ! -s "$T/g.png" ]; then
        echo "GSET_IMG item=$it key=out.png floor=- guest=- verdict=ABSENT"; continue
    fi
    f=$(psnr "$T/h1.png" "$T/h2.png"); q=$(psnr "$T/h1.png" "$T/g.png")
    v=$(awk -v f="$f" -v q="$q" 'BEGIN{ if (q=="inf") {print "MATCH"; exit} if (f=="inf") {print (q+0>=60)?"MATCH":"DIFF"; exit} print (q+0 >= f-3.0)?"MATCH":"DIFF" }')
    echo "GSET_IMG item=$it key=out.png floor=${f}dB guest=${q}dB verdict=$v"
done
