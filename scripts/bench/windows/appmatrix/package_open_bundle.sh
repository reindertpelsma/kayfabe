#!/usr/bin/env bash
# SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later
# package_open_bundle.sh -- build the Windows apps bundle for the private R2 copy: ONE app disk (kfapps.iso, label KFAPPS)
# from the REDISTRIBUTABLE packages only. Run ONCE by a person on a machine that may download from the vendors (never
# on a rented box, never in CI), then upload the result with the owner's R2 credentials.
#
# STATUS: DESIGN + CODE, 2026-11-07 (docs/design/V3_WINDOWS_APPS_GATE.md, section 3). GPU-free; `--dry-run` checks everything
# that needs no network. It has NOT been run to completion by a person: the measured run of 2026-11-07
# (traces/winapps_20261107/) used an app disk built by `appdisk.py` from the same manifest with a SUPERSET of packages.
#
#   package_open_bundle.sh --out DIR [--root DIR] [--dry-run]
#
#   --out DIR    result: DIR/winapps-open.iso, DIR/winapps-open.manifest.json (the manifest the image carries: the sha256 of
#                every file and the image identity), DIR/bundle.pin.line (the line for appmatrix/bundle.pin)
#   --root DIR   staging (default: DIR/stage): downloads in DIR/stage/dl, test programs in DIR/stage/built
#   --dry-run    validate gate_profiles.json against apps.json and manifest.json, list the packages that WOULD be fetched, and
#                refuse to go on if any package outside `open_packages` would be included; fetch and build nothing
#
# What it never does: upload (it prints the commands for the owner to run), fetch a package outside gate_profiles.json
# `open_packages`, or put a free-but-not-redistributable app (GravityMark, FurMark, GpuTest, Unigine), a GPL ffmpeg build or any
# licensed/account software into the image. The mingw-built test programs (kf_dxprobe, kf_glgears, the CUDA driver-API
# ladder, kf_overlayprobe) are built from this repository's own sources (needs gcc-mingw-w64-x86-64).
#
# After the upload (the owner, with R2 write credentials; the Worker only reads, ci/vast-broker/DEPLOY.md):
#   rclone copyto DIR/winapps-open.iso            r2:kayfabe-bench-enam/winapps-open/kfapps.iso
#   rclone copyto DIR/winapps-open.manifest.json  r2:kayfabe-bench-enam/winapps-open/manifest.json
# then commit DIR/bundle.pin.line as appmatrix/bundle.pin and record the packaging date and operator in docs/DOWNLOADS.md.
set -euo pipefail
HERE=$(cd "$(dirname "$(readlink -f "${BASH_SOURCE[0]}")")" && pwd)
OUT=""; ROOT=""; DRY=0
while [ $# -gt 0 ]; do case "$1" in
  --out) OUT=$2; shift 2;; --root) ROOT=$2; shift 2;; --dry-run) DRY=1; shift;;
  -h|--help) sed -n '2,28p' "$0"; exit 0;; *) echo "unknown option $1" >&2; exit 2;; esac; done
[ -n "$OUT" ] || { sed -n '2,12p' "$0"; exit 2; }
ROOT=${ROOT:-$OUT/stage}
say(){ echo "[package_open_bundle $(date -u +%FT%TZ)] $*"; }
[ -z "${CI:-}" ] && [ -z "${KF_RUN_TOKEN:-}" ] || { say "REFUSED: this is a one-time packaging step for a person, not for CI or a rented box"; exit 4; }
python3 -I "$HERE/gate_profiles.py" check || exit 1
ONLY=$(python3 -I - "$HERE" <<'PY'
import json, sys, os
here = sys.argv[1]
p = json.load(open(os.path.join(here, "gate_profiles.json")))
m = json.load(open(os.path.join(here, "manifest.json")))
need = {x["id"]: x.get("needs", []) for x in m["packages"]}
out, st = [], list(p["open_packages"])
seen = set()
while st:
    x = st.pop()
    if x in seen:
        continue
    seen.add(x)
    out.append(x)
    st += need.get(x, [])
bad = [x for x in out if x not in p["open_packages"]]
if bad:
    sys.exit("REFUSED: open packages need packages outside the bundle: " + ",".join(bad))
print(",".join(sorted(out)))
PY
)
say "packages: $ONLY"
if [ "$DRY" = 1 ]; then
  python3 -I "$HERE/appdisk.py" layout --manifest "$HERE/manifest.json" --max-tier 1 | head -3
  say "DRY-RUN ok (nothing fetched or built)"; exit 0
fi
mkdir -p "$OUT" "$ROOT"/{dl,pipdl,built,work}
say "START fetch"
python3 -I "$HERE/appdisk.py" fetch --manifest "$HERE/manifest.json" --max-tier 1 --only "$ONLY" --dl "$ROOT/dl" --pipdl "$ROOT/pipdl" --no-write
say "START build_tools"
bash "$HERE/build_tools.sh" "$ROOT/built"
say "START build image"
python3 -I "$HERE/appdisk.py" build --manifest "$HERE/manifest.json" --max-tier 1 --only "$ONLY" --dl "$ROOT/dl" --tools "$ROOT/built" --out "$ROOT/image"
mv "$ROOT/image/kfapps.iso" "$OUT/winapps-open.iso"
cp "$ROOT/image/manifest.json" "$OUT/winapps-open.manifest.json"
sha=$(sha256sum "$OUT/winapps-open.iso" | cut -d' ' -f1)
echo "winapps-open.iso sha256=$sha" > "$OUT/bundle.pin.line"
say "DONE $OUT/winapps-open.iso bytes=$(stat -c %s "$OUT/winapps-open.iso") sha256=$sha"
say "next (the owner, R2 write credentials): upload both files (see the header), commit bundle.pin.line as appmatrix/bundle.pin"
