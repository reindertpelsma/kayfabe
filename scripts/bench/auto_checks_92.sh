#!/usr/bin/env bash
# SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later
# auto_checks_92.sh — the owner's manual checks for PR #92 (display colour path) and the broker's
# cursor ownership, as one gate (docs/CI_V3.md §"PR #92 automated checks"):
#   A  GPU-free   cargo test -p kf-disp --test color_hue_vibrance --test color_output_ptx,
#                 cargo test -p kf-qemu --lib colour_program, the graders' selftests
#                 (scripts/ci/test_auto_checks_92.py), the broker selftest (make -C broker check)
#   B  GPU        scripts/bench/display/color_gpu_lane.sh      (the kernel on the host GPU)
#   C  Windows    scripts/bench/windows/hue_vibrance_lane.sh   (Hue / Digital Vibrance on the guest)
#   D  Linux      scripts/bench/display/cursor_lane.sh         (Wayland cursor, true XOR)
#
#   scripts/bench/auto_checks_92.sh [--only "A B C D"] [--rev <kf3-rev8>] [--win-run N] [--out DIR] [--wait S]
#
# A needs nothing but cargo/python3/make. B-D run only on the trusted GPU host, strictly serially,
# each taking the fast-guest lock itself (BUSY = 75 is reported, not retried). C and D need a kf3
# binary built for --rev (scripts/bench/build_kf3.sh). Start marker AUTO92_START, exit line
# AUTO92_EXIT, $OUT/summary.json {"A":rc,"B":rc,...,"verdict":...}. Exit: 0 when every selected
# part passed, 75 when nothing failed but something was BUSY, else 1.
set -uo pipefail
HERE="$(cd "$(dirname "$0")" && pwd)"
REPO="$(cd "$HERE/../.." && pwd)"
ONLY="A B C D" REV="" WINRUN="" OUT="" WAIT=0
while [ $# -gt 0 ]; do
    case "$1" in
        --only) ONLY=$2; shift 2 ;;
        --rev) REV=$2; shift 2 ;;
        --win-run) WINRUN=$2; shift 2 ;;
        --out) OUT=$2; shift 2 ;;
        --wait) WAIT=$2; shift 2 ;;
        *) echo "usage: auto_checks_92.sh [--only P] [--rev R] [--win-run N] [--out DIR] [--wait S]" >&2; exit 2 ;;
    esac
done
OUT=${OUT:-/tmp/auto_checks_92-$(date +%Y%m%d-%H%M%S)}
mkdir -p "$OUT" || exit 2
LOG=$OUT/auto92.log
declare -A RC
say() { echo "$*" | tee -a "$LOG"; }
say "AUTO92_START checkout=$(git -C "$REPO" rev-parse HEAD) only=\"$ONLY\" rev=${REV:-none} $(date -Is)"
part() { # part <letter> <cmd...>
    local p=$1; shift
    say "AUTO92_PART_START $p"
    ( cd "$REPO" && "$@" ) > "$OUT/$p.log" 2>&1
    RC[$p]=$?
    say "AUTO92_PART_EXIT $p rc=${RC[$p]}"
}
for p in $ONLY; do
    case $p in
        A) part A bash -c 'set -e
            cargo test --locked -p kf-disp --test color_hue_vibrance --test color_output_ptx
            cargo test --locked -p kf-qemu --lib colour_program
            python3 -m unittest scripts/ci/test_auto_checks_92.py
            make -s -C broker check' ;;
        B) part B bash scripts/bench/display/color_gpu_lane.sh --out "$OUT/B" --wait "$WAIT" ;;
        C) if [ -z "$REV" ] || [ -z "$WINRUN" ]; then say "AUTO92 C needs --rev and --win-run"; RC[C]=2
           else part C bash scripts/bench/windows/hue_vibrance_lane.sh --rev "$REV" --run "$WINRUN" --out "$OUT/C" --wait "$WAIT"; fi ;;
        D) if [ -z "$REV" ]; then say "AUTO92 D needs --rev"; RC[D]=2
           else part D bash scripts/bench/display/cursor_lane.sh --rev "$REV" --out "$OUT/D" --wait "$WAIT"; fi ;;
        *) say "AUTO92 unknown part $p"; RC[$p]=2 ;;
    esac
done
verdict=PASS
for p in "${!RC[@]}"; do
    case ${RC[$p]} in 0) ;; 75) [ "$verdict" = PASS ] && verdict=BUSY ;; *) verdict=FAIL ;; esac
done
{
    printf '{"gate":"auto_checks_92","verdict":"%s","checkout":"%s","rev":"%s","parts":{' "$verdict" "$(git -C "$REPO" rev-parse HEAD)" "$REV"
    first=1
    for p in "${!RC[@]}"; do [ $first = 1 ] || printf ','; printf '"%s":%s' "$p" "${RC[$p]}"; first=0; done
    printf '},"out":"%s","at":"%s"}\n' "$OUT" "$(date -Is)"
} > "$OUT/summary.json"
say "RESULT gate=auto_checks_92 verdict=$verdict"
rc=1; [ "$verdict" = PASS ] && rc=0; [ "$verdict" = BUSY ] && rc=75
say "AUTO92_EXIT rc=$rc $(date -Is)"
exit $rc
