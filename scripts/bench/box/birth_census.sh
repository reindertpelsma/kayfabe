#!/usr/bin/env bash
# SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later
# ★★★ birth_census.sh — the merge bar's channel-privilege gate (docs/design/THE_CONSTRAINTS.md §30).
#
# Every host channel kf3 creates must be born USER. kf-host reads RM's verdict from each
# channel-alloc reply and logs one line per birth:
#   kf-host: channel birth h=… engine=… reply_flags=… PRIVILEGED_CHANNEL=0 privilege=USER cap_sys_admin=…
# and kf-cuda logs, once per thread that makes CUDA calls:
#   kf-cuda: cuda thread posture thread=… cap_sys_admin=<cleared-for-thread-life|not-held>
# This script FAILS when:
#   - any birth line lacks `PRIVILEGED_CHANNEL=0 privilege=USER`, or any refusal line appears
#     (PRIVILEGED CHANNEL REFUSED, CHANNEL BIRTH REFUSED, CHANNEL CLASS REFUSED, CUDA THREAD REFUSED);
#   - an arm's QEMU log is missing or empty, or has NO birth line, or no CUDA posture line
#     (a sweep that reports zero must first report one: every suite arm births channels);
#   - the gates' log has no birth line;
#   - the suite scoreboard names fewer arms than ARMS=.
#
# usage: birth_census.sh <bench-dir> <tag> <gates.log>     (reads <bench>/<tag>_suite.out and
#        <bench>/fast_<tag>_<arm>_qemu.log; last line BIRTH_CENSUS_OK or BIRTH_CENSUS_FAIL …)
#        birth_census.sh --selftest                        (the census on planted logs: it must
#        pass a clean set and fail each planted defect; run before trusting a pass)
set -uo pipefail

census() {
    local bench=$1 tag=$2 gates=$3 out="$1/$2_suite.out"
    local bad=0 why="" arms=0 total=0 user=0 posture=0 armsn
    [ -s "$out" ] || { echo "BIRTH_CENSUS_FAIL no suite scoreboard at $out"; return 1; }
    armsn=$(grep -ao 'ARMS=[0-9]*' "$out" | tail -1 | cut -d= -f2)
    for arm in $(grep -ao 'FAST_CELL_ARM arm=[^ ]*' "$out" | cut -d= -f2); do
        arms=$((arms+1))
        local q="$bench/fast_${tag}_${arm}_qemu.log"
        if [ ! -s "$q" ]; then bad=$((bad+1)); why="$why $arm:no-qemu-log"; continue; fi
        local b u p r pb
        b=$(grep -ac 'kf-host: channel birth ' "$q")
        u=$(grep -a 'kf-host: channel birth ' "$q" | grep -ac ' PRIVILEGED_CHANNEL=0 privilege=USER ')
        p=$(grep -ac 'kf-cuda: cuda thread posture thread=' "$q")
        pb=$(grep -a 'kf-cuda: cuda thread posture thread=' "$q" | grep -acv 'cap_sys_admin=\(cleared-for-thread-life\|not-held\) ')
        r=$(grep -acE 'PRIVILEGED CHANNEL REFUSED|CHANNEL BIRTH REFUSED|CHANNEL CLASS REFUSED|CUDA THREAD REFUSED' "$q")
        echo "BIRTH_CENSUS arm=$arm births=$b user=$u refused=$r cuda_threads=$p"
        grep -a 'kf-cuda: cuda thread posture thread=' "$q" | sed "s/^/    $arm /"
        total=$((total+b)); user=$((user+u)); posture=$((posture+p))
        [ "$b" -ge 1 ] || { bad=$((bad+1)); why="$why $arm:no-births"; }
        [ "$u" -eq "$b" ] || { bad=$((bad+1)); why="$why $arm:non-user-birth"; }
        [ "$r" -eq 0 ] || { bad=$((bad+1)); why="$why $arm:refusal"; }
        [ "$p" -ge 1 ] || { bad=$((bad+1)); why="$why $arm:no-cuda-posture"; }
        [ "$pb" -eq 0 ] || { bad=$((bad+1)); why="$why $arm:cuda-posture-unknown"; }
    done
    [ "$arms" -ge 1 ] && [ "$arms" = "${armsn:-x}" ] || { bad=$((bad+1)); why="$why scoreboard:arms=$arms/ARMS=${armsn:-none}"; }
    local gb gu gr
    if [ -s "$gates" ]; then
        gb=$(grep -ac 'kf-host: channel birth ' "$gates")
        gu=$(grep -a 'kf-host: channel birth ' "$gates" | grep -ac ' PRIVILEGED_CHANNEL=0 privilege=USER ')
        gr=$(grep -acE 'PRIVILEGED CHANNEL REFUSED|CHANNEL BIRTH REFUSED|CHANNEL CLASS REFUSED|CUDA THREAD REFUSED' "$gates")
    else
        gb=0; gu=0; gr=0; why="$why gates:no-log"; bad=$((bad+1))
    fi
    [ "$gb" -ge 1 ] || { bad=$((bad+1)); why="$why gates:no-births"; }
    [ "$gu" -eq "$gb" ] || { bad=$((bad+1)); why="$why gates:non-user-birth"; }
    [ "$gr" -eq 0 ] || { bad=$((bad+1)); why="$why gates:refusal"; }
    echo "BIRTH_CENSUS_SUITE arms=$arms births=$total user=$user cuda_threads=$posture"
    echo "BIRTH_CENSUS_GATES births=$gb user=$gu refused=$gr"
    if [ "$bad" -eq 0 ]; then
        echo "BIRTH_CENSUS_OK arms=$arms suite_births=$total gate_births=$gb all PRIVILEGED_CHANNEL=0 privilege=USER"
        return 0
    fi
    echo "BIRTH_CENSUS_FAIL bad=$bad:$why"
    return 1
}

selftest() {
    local d; d=$(mktemp -d); trap 'rm -rf "$d"' RETURN
    local good='kf-host: channel birth h=0xcafe0020 engine=0x1 reply_flags=0x00000080 PRIVILEGED_CHANNEL=0 privilege=USER cap_sys_admin=cleared-for-call'
    local post='kf-cuda: cuda thread posture thread=kf3-vamgr cap_sys_admin=cleared-for-thread-life (…)'
    plant() {  # plant <case> <arm-a line…> — builds a two-arm suite + gates log under $d/$1
        local c=$1; shift; mkdir -p "$d/$c"
        printf 'FAST_CELL_ARM arm=a verdict=PASS\nFAST_CELL_ARM arm=b verdict=PASS\nFAST_SUITE_PASS=2 FAST_SUITE_FAIL=0 FAST_SUITE_CRASH=0 NOTRUN=0 ARMS=2\n' > "$d/$c/t_suite.out"
        printf '%s\n' "$@" > "$d/$c/fast_t_a_qemu.log"
        printf '%s\n%s\n' "$good" "$post" > "$d/$c/fast_t_b_qemu.log"
        printf '%s\n' "$good" > "$d/$c/gates.log"
    }
    local cases=0 wrong=0
    expect() {  # expect <pass|fail> <case>
        cases=$((cases+1))
        if census "$d/$2" t "$d/$2/gates.log" >/dev/null; then got=pass; else got=fail; fi
        [ "$got" = "$1" ] && echo "SELFTEST $2: $got (expected)" || { echo "SELFTEST $2: $got, EXPECTED $1"; wrong=$((wrong+1)); }
    }
    plant clean "$good" "$post";                                        expect pass clean
    plant admin "${good/PRIVILEGED_CHANNEL=0/PRIVILEGED_CHANNEL=1}" "$post"; expect fail admin
    plant noline "kf3: nothing about channels" "$post";                  expect fail noline
    plant refused "$good" "$post" 'kf-host: ⊘ PRIVILEGED CHANNEL REFUSED h=0xcafe0021'; expect fail refused
    plant noposture "$good";                                             expect fail noposture
    plant cudaheld "$good" "${post/cleared-for-thread-life/held}";       expect fail cudaheld
    plant cudarefused "$good" "$post" 'kf-cuda: ⊘ CUDA THREAD REFUSED thread=kf3-vamgr'; expect fail cudarefused
    plant nogates "$good" "$post"; : > "$d/nogates/gates.log";           expect fail nogates
    plant gateadmin "$good" "$post"; printf '%s\n' "${good/PRIVILEGED_CHANNEL=0/PRIVILEGED_CHANNEL=1}" > "$d/gateadmin/gates.log"; expect fail gateadmin
    plant missingarm "$good" "$post"; rm "$d/missingarm/fast_t_b_qemu.log"; expect fail missingarm
    plant shortboard "$good" "$post"; sed -i 's/ARMS=2/ARMS=3/' "$d/shortboard/t_suite.out"; expect fail shortboard
    echo "BIRTH_CENSUS_SELFTEST cases=$cases wrong=$wrong"
    [ "$wrong" -eq 0 ]
}

if [ "${1:-}" = --selftest ]; then selftest; exit $?; fi
census "${1:?bench dir}" "${2:?tag}" "${3:?gates log}"
