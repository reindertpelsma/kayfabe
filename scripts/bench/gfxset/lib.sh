#!/usr/bin/env bash
# lib.sh — shared helpers for the headless-graphics test set (sourced by items.sh).
#
# Line protocol (every graded line at column 0; the suite greps nothing else):
#   GSET_DIG  side=<s> item=<i> key=<k> val=<hash>     a content digest, compared host vs guest
#   GSET_VAL  side=<s> item=<i> key=<k> val=<text>     a recorded value, NOT graded by equality
#   GSET_RES  side=<s> item=<i> verdict=PASS|FAIL|TIMEOUT|NOTRUN rc=<rc> secs=<n> note=<...>
# ⊘ NOTRUN = a precondition was missing (a tool, a file) — never a statement about the GPU.
# ⊘ TIMEOUT is only a run that STARTED and exceeded its budget (it has secs=); an item that never
#   started is NOTRUN (provisioning_reports_done_without_building_qemu: "?s" is not a timeout).
gset_now(){ date +%s.%N; }
gset_md5(){ [ -s "$1" ] && md5sum "$1" | cut -c1-16 || echo EMPTY; }
# fnv-free content digest of a whole directory's files, in name order (for frame dumps)
gset_dirdig(){ ( cd "$1" 2>/dev/null && find . -type f | LC_ALL=C sort | xargs -r md5sum | md5sum | cut -c1-16 ) || echo EMPTY; }
gset_dig(){ echo "GSET_DIG side=$SIDE item=$ITEM key=$1 val=$2"; }
gset_val(){ echo "GSET_VAL side=$SIDE item=$ITEM key=$1 val=$2"; }
gset_need(){ # tool... — NOTRUN if any is missing
    local t; for t in "$@"; do command -v "$t" >/dev/null 2>&1 || [ -x "$t" ] || { echo "GSET_NEED_MISSING $t"; return 1; }; done
}
