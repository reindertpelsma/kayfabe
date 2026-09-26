#!/usr/bin/env bash
# cycles_noise.sh "<bare dir>..." "<guest dir>..." [item...] — the spread evidence for the NONDET image
# items (default: blender_cycles_cuda blender_cycles_optix): every bare-metal image (<dir>/<item>.host_art.tar)
# and every guest image (<dir>/<item>.guest_art.tar, iso/ preferred) through imgnoise.py --matrix --nbare:
# the full pairwise table of differing values, then each image's spread (its mean number of differing
# values to the bare-metal images). A guest image whose spread exceeds every bare image's is an outlier;
# guest images that are outliers AND agree with each other would be a systematic difference.
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
    rm -f "$T"/*.png
done
