#!/bin/sh
# Reproduce docs/design/V3_API_BOUNDARIES_20261014.md end to end.  Needs: git, python3 (3.9+), network once.
# usage: tools/api_boundaries/run_all.sh <kayfabe-tree> <work-dir> <out-dir>
#   <kayfabe-tree>  a checkout (or `git archive` extract) of kayfabe containing crates/kf-*  (the doc used
#                   claude/constants-night-20261014 @ 93ff3aa0f: it has kf-ctl-probe and the consttab)
#   <work-dir>      scratch; the blobless ogkm clone goes to <work-dir>/ogkm (~60 MB with the sparse set)
#   <out-dir>       per-tag JSONs (<work-dir>/tags) and the derived tables (<out-dir>)
# Windows control censuses (inputs of windows_controls.py) are named in the doc; pass them as
#   WINDOWS_CENSUS="label=path ..."  to regenerate windows_controls.tsv, else the committed one is used.
set -eu
kf=${1:?kayfabe tree}; work=${2:?work dir}; out=${3:?out dir}
here=$(cd "$(dirname "$0")" && pwd)
mkdir -p "$work" "$out"
if [ -n "${WINDOWS_CENSUS:-}" ]; then
  # shellcheck disable=SC2086
  python3 -I "$here/windows_controls.py" --ids "$here/windows_ids.txt" $WINDOWS_CENSUS > "$here/windows_controls.tsv"
fi
python3 -I "$here/derive_served.py" "$kf" "$here/extra_ids.txt" "$here/windows_ids.txt" > "$work/served.json"
python3 -I "$here/kf_idents.py" "$kf" > "$work/idents.txt"
"$here/sweep_setup.sh" "$work/ogkm"
python3 -I "$here/sweep.py" "$work/ogkm" "$work/served.json" "$work/tags"
python3 -I "$here/analyze.py" "$work/tags" "$work/served.json" "$work/idents.txt" \
  "$here/measured_points.tsv" "$here/windows_controls.tsv" "$out"
