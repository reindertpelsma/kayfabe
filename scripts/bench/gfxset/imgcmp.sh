#!/usr/bin/env bash
# imgcmp.sh <resdir> — the CONTENT grade for image items whose digest is NONDET (differs between the two
# bare-metal runs, e.g. Cycles path tracing: GPU atomics make the sample accumulation order vary).
# For each such item it extracts out.png from every bare-metal run it can find — run 1 (<resdir>), run 2
# (<resdir>/h2) and every extra bare-metal run in GSET_NOISE_DIRS (dirs holding <item>.host_art.tar, e.g.
# the nd/h<k> runs of a noise measurement) — and the guest's, and grades with imgnoise.py:
#   floor = the LOWEST PSNR among the bare-metal pairs (bare metal's two farthest runs)
#   guest = the MEDIAN PSNR of the guest to each bare-metal image; MATCH iff guest >= floor
# and prints GSET_IMG item=<i> key=out.png floor=<dB> guest=<dB> verdict=MATCH|DIFF + the detail fields
# (n_bare, PSNR ranges, how many values differ and by how much).
# ⊘ CORRECTED 2026-09-26 (gs2): the first rule was "guest within 3 dB of PSNR(run 1, run 2)" — ONE sample
#   of the spread plus an assumed margin. gs2's Cycles CUDA fell 0.3 dB outside it while differing from
#   bare metal in a handful of values by 1 LSB; the spread is now measured over every bare-metal run.
# ⊘ Only for NONDET digests: an item whose digest is stable on bare metal is graded byte for byte.
set -uo pipefail
R=${1:?resdir}; HERE="$(cd "$(dirname "$0")" && pwd)"
T=$(mktemp -d); trap 'rm -rf "$T"' EXIT
for it in $(sed -n 's/^GSET_DIG side=host item=\([^ ]*\) key=png .*/\1/p' "$R/host.dig" | sort -u); do
    h1=$(grep -a "item=$it key=png " "$R/host.dig" | tail -1 | sed 's/.*val=//')
    h2=$(grep -a "item=$it key=png " "$R/host2.dig" 2>/dev/null | tail -1 | sed 's/.*val=//')
    [ -n "$h2" ] && [ "$h1" != "$h2" ] || continue          # stable on bare metal: byte-graded elsewhere
    g=$R/$it.guest_art.tar; [ -f "$R/iso/$it.guest_art.tar" ] && g=$R/iso/$it.guest_art.tar
    bare=(); k=0
    for src in "$R" "$R/h2" ${GSET_NOISE_DIRS:-}; do
        [ -f "$src/$it.host_art.tar" ] || continue
        k=$((k+1)); tar -xOf "$src/$it.host_art.tar" ./out.png > "$T/b$k.png" 2>/dev/null
        [ -s "$T/b$k.png" ] && bare+=("$T/b$k.png")
    done
    tar -xOf "$g" ./out.png > "$T/g.png" 2>/dev/null
    if [ "${#bare[@]}" -lt 2 ] || [ ! -s "$T/g.png" ]; then
        echo "GSET_IMG item=$it key=out.png floor=- guest=- verdict=ABSENT n_bare=${#bare[@]}"; continue
    fi
    echo "GSET_IMG item=$it key=out.png $(python3 "$HERE/imgnoise.py" --bare "${bare[@]}" --guest "$T/g.png" 2>&1 | tail -1)"
    rm -f "$T"/b*.png "$T/g.png"
done
