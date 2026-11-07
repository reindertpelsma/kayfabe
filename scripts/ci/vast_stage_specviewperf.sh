#!/bin/bash
# vast_stage_specviewperf.sh — the release tier's SPECviewperf stage on a Vast box. scripts/ci/vast_payload.py runs it
# last, only when the Worker sends KF_STAGES=specviewperf together with KF_SPEC=lanea|laneb|both (release tier only,
# disabled by default: ci/vast-broker defaults.json releaseStages is empty, and no Worker code sends KF_SPEC yet).
#
#   KF_SPEC_LANE=<lane> KF_SPEC_DEADLINE_EPOCH=<epoch s> vast_stage_specviewperf.sh <tag> <prov_dir>
#
# STATUS: PLUMBING 2026-11-05 (wtier); the lane exists since 2026-11-07 (docs/design/V3_SPEC_GATE.md), GPU-free tests only.
# The lane itself is scripts/bench/box/specviewperf_stage.sh; it fetches the media with
# scripts/bench/box/fetch_bench_data.sh (the Worker's presigned /bench-data route, run token only) and prints
# exactly one verdict line
#   SPECVIEWPERF_STAGE_PASS | SPECVIEWPERF_STAGE_FAIL | SPECVIEWPERF_STAGE_RETRY class=<A-Z_>
# plus status lines. Without the lane file this wrapper prints SPECVIEWPERF_STAGE_RETRY class=NOT_BUILT (never a pass).
#
# Scores never leave the box through this stage: the lane's whole output goes to a private file (mode 0600 in a 0700
# directory that is not streamed), and only lines that match one of the strict STATUS patterns below are printed to the
# streamed log AS THEY ARRIVE (a killed run still reports the viewsets that finished): a viewset name, PASS/FAIL/NOT_RUN,
# a count of completed subtests of the total, a reason word and the names of failed subtests. Nothing else passes: no
# note, no timing, no number a benchmark prints (docs/SPEC_RESULTS.md). The verdict line is printed once, at the end.
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
# the start marker (the exit line is the last line): a killed run and a running one look the same without them
case ${KF_SPEC_LANE:-} in lanea|laneb|both) startlane=$KF_SPEC_LANE ;; *) startlane=none ;; esac
echo "SPEC_STAGE_START lane=$startlane"
umask 077
mkdir -p "$private" && chmod 700 "$private"
log=$private/specviewperf_$tag.log
export KF_SPEC_PRIVATE=$private/spec_$tag   # the lane keeps the guest's raw output here (0700), never in the evidence directory
rcfile=$private/specviewperf_$tag.rc
: > "$log"
# the STATUS patterns (one alternative per line kind; anchored; no free text)
status='^SPEC_STAGE_LIST lane=(lanea|laneb|both) n=[0-9]{1,2}$'
status+='|^SPEC_ROW name=[a-z][a-z_]*-[0-9]{2} result=(PASS|FAIL|NOT_RUN) subtests=[0-9]{1,3}/[0-9]{1,3} reason=([A-Z_]{1,16}|-) failed=([A-Za-z0-9_,]{1,120}|-)$'
status+='|^[a-z][a-z_]*_[0-9]{2} --- FAILED$'
status+='|^SPEC_STAGE_COUNTS pass=[0-9]{1,2} fail=[0-9]{1,2} not_run=[0-9]{1,2} of=[0-9]{1,2}$'
status+='|^SPEC_COPY_DELETED removed=(archive|overlay|archive,overlay|nothing)$'
{ bash "$lane" "$tag" "$prov" 2>&1; echo "$?" > "$rcfile"; } | tee -a "$log" | grep --line-buffered -E "$status"
rc=$(cat "$rcfile" 2>/dev/null || echo 1)
rm -f "$rcfile"
# the class is one of a fixed list (an allowlist: nothing a benchmark printed can be a class)
verdict=$(grep -E '^SPECVIEWPERF_STAGE_(PASS|FAIL|RETRY( class=(NOT_BUILT|MEDIA|PROVIDER|TRANSPORT|TIME|HOSTREF|BAR1_EXCEPTION|OPERATOR|USAGE))?)$' "$log" | tail -n 1)
echo "${verdict:-SPECVIEWPERF_STAGE_RETRY class=NO_VERDICT}"
echo "SPECVIEWPERF_STAGE_EXIT rc=$rc"
exit "$rc"
