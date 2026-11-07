#!/bin/bash
# vast_stage_specviewperf.sh — the release tier's SPECviewperf stage on a Vast box (PLUMBING,
# 2026-11-05, wtier). scripts/ci/vast_payload.py runs it last, only when the Worker sends
# KF_STAGES=specviewperf (release tier only, disabled by default: ci/vast-broker defaults.json
# releaseStages is empty).
#
#   vast_stage_specviewperf.sh <tag> <prov_dir>
#
# The lane itself is scripts/bench/box/specviewperf_stage.sh, owned by the SPECviewperf lane; it
# fetches the media with scripts/bench/box/fetch_bench_data.sh (the Worker's presigned /bench-data
# route, run token only) and must print exactly one line
#   SPECVIEWPERF_STAGE_PASS | SPECVIEWPERF_STAGE_FAIL | SPECVIEWPERF_STAGE_RETRY class=<A-Z_>
# Until that lane exists this wrapper prints SPECVIEWPERF_STAGE_RETRY class=NOT_BUILT (never a
# pass).
#
# Scores never leave the box through this stage: the lane's whole output goes to a private file
# (mode 0600 in a 0700 directory that is not streamed), and only the verdict line, matched by a
# strict pattern with no digits, is printed to the streamed log (docs/SPEC_RESULTS.md).
set -u
[ "$#" -eq 2 ] || { echo "SPECVIEWPERF_STAGE_RETRY class=USAGE"; exit 2; }
tag=$1 prov=$2
case $tag in ''|*[!A-Za-z0-9_]*) echo "SPECVIEWPERF_STAGE_RETRY class=USAGE"; exit 2 ;; esac
here=$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)
lane=${KF_SPECVIEWPERF_LANE:-$here/../bench/box/specviewperf_stage.sh}
private=${KF_CI_PRIVATE_DIR:-/workspace/kf-ci-private}
if [ ! -f "$lane" ]; then
    echo "SPECVIEWPERF_STAGE_RETRY class=NOT_BUILT"
    echo "SPECVIEWPERF_STAGE_EXIT rc=3"
    exit 3
fi
umask 077
mkdir -p "$private" && chmod 700 "$private"
log=$private/specviewperf_$tag.log
bash "$lane" "$tag" "$prov" >"$log" 2>&1
rc=$?
verdict=$(grep -E '^SPECVIEWPERF_STAGE_(PASS|FAIL|RETRY)( class=[A-Z_]{1,32})?$' "$log" | tail -n 1)
echo "${verdict:-SPECVIEWPERF_STAGE_RETRY class=NO_VERDICT}"
echo "SPECVIEWPERF_STAGE_EXIT rc=$rc"
exit "$rc"
