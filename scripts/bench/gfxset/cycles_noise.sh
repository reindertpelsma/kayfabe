#!/usr/bin/env bash
# cycles_noise.sh "<bare dir>..." "<guest dir>..." [item...] — the spread evidence for the NONDET image
# items (default: blender_cycles_cuda blender_cycles_optix): every bare-metal image (<dir>/<item>.host_art.tar)
# and every guest image (<dir>/<item>.guest_art.tar, iso/ preferred) through imgnoise.py --matrix --nbare:
# the full pairwise table of differing values, then each image's spread (its mean number of differing
# values to the bare-metal images). A guest image whose spread exceeds every bare image's is an outlier;
# guest images that are outliers AND agree with each other would be a systematic difference.
# env CYCLES_SPARSE=<prefix>: also write each image as its sparse difference from the first bare image.
set -uo pipefail
HERE="$(cd "$(dirname "$0")" && pwd)"
BARE=${1:?bare dirs}; GUEST=${2:?guest dirs}; shift 2
ITEMS=${*:-blender_cycles_cuda blender_cycles_optix}
T=$(mktemp -d); trap 'rm -rf "$T"' EXIT
for it in $ITEMS; do
    b=(); g=()
    for d in $BARE; do
        n=$(echo "$d" | sed 's#/workspace/gfxset/results/##; s#/#_#g')
        tar -xOf "$d/$it.host_art.tar" ./out.png > "$T/$n.png" 2>/dev/null && [ -s "$T/$n.png" ] && b+=("$T/$n.png")
    done
    for d in $GUEST; do
        n=$(echo "$d" | sed 's#/workspace/gfxset/results/##; s#/#_#g')_GUEST
        a=$d/$it.guest_art.tar; [ -f "$d/iso/$it.guest_art.tar" ] && a=$d/iso/$it.guest_art.tar
        tar -xOf "$a" ./out.png > "$T/$n.png" 2>/dev/null && [ -s "$T/$n.png" ] && g+=("$T/$n.png")
    done
    echo "== $it: ${#b[@]} bare-metal image(s), ${#g[@]} guest image(s)"
    python3 "$HERE/imgnoise.py" --matrix --nbare "${#b[@]}" "${b[@]}" "${g[@]}"
    # CYCLES_SPARSE=<prefix>: every image also written as its differences from the first bare-metal image
    # (<prefix>_<item>.txt, lossless for R, G, B) — the measurement's data, small enough for the repository
    [ -n "${CYCLES_SPARSE:-}" ] && [ "${#b[@]}" -ge 1 ] && \
        python3 "$HERE/imgnoise.py" --sparse "${b[0]}" "${b[@]}" "${g[@]}" > "${CYCLES_SPARSE}_$it.txt"
    rm -f "$T"/*.png
done
