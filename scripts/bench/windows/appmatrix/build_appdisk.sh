#!/usr/bin/env bash
# SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later
# build_appdisk.sh — create the Windows app matrix's read-only application disk (kfapps.iso, label KFAPPS) from
# manifest.json on the GPU host. No guest internet is needed at run time: every app is on this disk.
#
#   build_appdisk.sh [--root DIR] [--tools DIR] [--max-tier N] [--only ID,ID] [--no-fetch] [--no-tools] [--dry-run]
#
#   --root DIR     staging root (default /var/lib/kf-windows-20261005/appmatrix); uses DIR/dl (downloads), DIR/pipdl
#                  (optional pre-fetched wheels, hard-linked), DIR/built (mingw test programs), DIR/image (output)
#   --tools DIR    use already-built test programs from DIR instead of DIR/built (build_tools.sh needs mingw-w64,
#                  which the GPU host does not have: build on a dev machine and copy DIR over)
#   --max-tier N   leave out packages of a higher tier (1 = no installers/big extras; default 9 = everything)
#   --dry-run      check the manifest and apps, print what would be fetched/built, change nothing
# Output: DIR/image/kfapps.iso and DIR/image/manifest.json (the manifest with the sha256 of every file on the disk,
# the image's own sha256 and its identity); the manifest is also written back next to this script when --update-repo
# is given, so the pinned hashes can be committed (never the binaries).
# Free space: refuses unless MIN_FREE_GB (default 100) stays free on the staging filesystem after the estimated need.
set -euo pipefail
HERE=$(cd "$(dirname "$(readlink -f "${BASH_SOURCE[0]}")")" && pwd)
ROOT=/var/lib/kf-windows-20261005/appmatrix; TOOLS=""; TIER=9; ONLY=""; FETCH=1; BUILDTOOLS=1; DRY=0; UPDATE=0
while [ $# -gt 0 ]; do case "$1" in
  --root) ROOT=$2; shift 2;; --tools) TOOLS=$2; shift 2;; --max-tier) TIER=$2; shift 2;; --only) ONLY=$2; shift 2;;
  --no-fetch) FETCH=0; shift;; --no-tools) BUILDTOOLS=0; shift;; --dry-run) DRY=1; shift;; --update-repo) UPDATE=1; shift;;
  -h|--help) sed -n '2,20p' "$0"; exit 0;; *) echo "unknown option $1" >&2; exit 2;; esac; done
MIN_FREE_GB=${MIN_FREE_GB:-100}
say(){ echo "[build_appdisk $(date -u +%FT%TZ)] $*"; }
M=$HERE/manifest.json
python3 -I "$HERE/appdisk.py" check --manifest "$M" --apps "$HERE/apps.json" || exit 1
need_gb=$(python3 -I - "$M" "$TIER" "$ROOT/dl" <<'PY'
import json, os, sys
m = json.load(open(sys.argv[1])); t = int(sys.argv[2]); dl = sys.argv[3]
miss = img = tree = 0
for p in m["packages"]:
    if p.get("tier", 1) > t:
        continue
    for f in p["files"]:
        sz = f.get("size") or 0
        if not os.path.exists(os.path.join(dl, p["id"], f["name"])):
            miss += sz                                   # still to download
        if p.get("image_form") == "tree":
            tree += sz * 1.7                             # extracted copy, temporary
        img += sz * (1.7 if p.get("image_form") == "tree" else 1)   # the image itself
print(int((miss + img + tree) / 1e9) + 2)
PY
)
mkdir -p "$ROOT"
free_gb=$(df -BG --output=avail "$ROOT" | tail -1 | tr -dc 0-9)
say "root=$ROOT free=${free_gb}G need~${need_gb}G keep>=${MIN_FREE_GB}G tier<=$TIER"
if [ "$DRY" = 1 ]; then
  python3 -I "$HERE/appdisk.py" layout --manifest "$M" --max-tier "$TIER" | head -5
  say "DRY-RUN ok (nothing fetched or built)"; exit 0
fi
[ $((free_gb - need_gb)) -ge "$MIN_FREE_GB" ] || { say "REFUSED: free space would drop below ${MIN_FREE_GB}G"; exit 3; }
mkdir -p "$ROOT"/{dl,pipdl,built,image,work}
ARGS=(--manifest "$M" --max-tier "$TIER"); [ -n "$ONLY" ] && ARGS+=(--only "$ONLY")
if [ "$FETCH" = 1 ]; then
  say "START fetch"; python3 -I "$HERE/appdisk.py" fetch "${ARGS[@]}" --dl "$ROOT/dl" --pipdl "$ROOT/pipdl"; say "EXIT fetch rc=$?"
fi
if [ -z "$TOOLS" ]; then
  TOOLS=$ROOT/built
  if [ "$BUILDTOOLS" = 1 ] && command -v x86_64-w64-mingw32-g++ >/dev/null; then say "START build_tools"; bash "$HERE/build_tools.sh" "$TOOLS"; fi
fi
[ -f "$TOOLS/kf_dxprobe.exe" ] || say "WARNING: no test programs in $TOOLS (dxprobe_*, glgears_wgl, cup*_ladder will be NOTRUN)"
say "START build image"
python3 -I "$HERE/appdisk.py" build "${ARGS[@]}" --dl "$ROOT/dl" --tools "$TOOLS" --out "$ROOT/image"
say "EXIT build image rc=$?"
( cd "$ROOT/image" && sha256sum kfapps.iso | tee kfapps.iso.sha256 )
[ "$UPDATE" = 1 ] && cp "$ROOT/image/manifest.json" "$M" && say "manifest written back to $M"
say "DONE"
