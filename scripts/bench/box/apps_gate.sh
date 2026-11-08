#!/bin/bash
# apps_gate.sh — the Linux app matrix as a release/middle-tier GATE stage, in the fat guest.
# DRAFT, DISABLED BY DEFAULT (2026-11-07, lane appslinux): scripts/ci/vast_payload.py appends the
# stage `apps` only when APPS_STAGE_ENABLED is True there (the owner flips it) and the tier is
# middle or large. Design and the measurement behind the list: traces/apps_linux_20261107/README.md.
#
#   apps_gate.sh <tag> <prov_dir> [full|middle]
#
# What it does (strictly serial, one GPU job at a time, results written as they arrive):
#   1. reads the gate list (scripts/apps/gate_list.txt: apps that passed on bare metal AND in the guest),
#   2. refuses (RETRY class=NOT_BUILT) when the app bundle and the app guest image are not on the box:
#      a CI box has no CUDA toolkit apt source (the setup cache is the only apt) and building the
#      bundle costs tens of minutes, so it must arrive prebuilt. KF_APPS_PROVISION=1 builds it by
#      hand on a lease (build_bundle.sh, setup_side.sh, provision_guest_apps.sh),
#   3. runs `apps_matrix.sh guest <tag> <list>` under its own time budget (KF_APPS_BUDGET_S), with
#      bounded per-app timeouts, and prints one `APP_TRY` line per batched result AS IT ARRIVES,
#   4. prints one `APP_ROW` line per gate app (the verdict of record: the isolated re-run when the
#      batched run failed it), then exactly one verdict line and one exit line:
#        APPS_STAGE_PASS | APPS_STAGE_FAIL | APPS_STAGE_RETRY class=<A-Z_>
#        APPS_STAGE_EXIT rc=<n>
# PASS needs every gate app PASS, nothing unrun. A guest FAIL of a gate app is a kayfabe finding
# (FAIL). Missing bundle, budget exhausted, a boot that never produced a row: RETRY (never a verdict
# on the guest, never a pass).
# APP_ROW format (parsed by scripts/ci/vast_payload.py apps_rows and scripts/ci/vast_run_report.py):
#   APP_ROW name=<app> os=linux result=PASS|FAIL|NOT_RUN secs=<n|-> guest_xid=<n|-> note=<free text, last>
set -u
[ "$#" -eq 2 ] || [ "$#" -eq 3 ] || { echo "APPS_STAGE_RETRY class=USAGE"; echo "APPS_STAGE_EXIT rc=2"; exit 2; }
tag=$1 prov=$2 which=${3:-full}
case $which in full|middle) ;; *) echo "APPS_STAGE_RETRY class=USAGE"; echo "APPS_STAGE_EXIT rc=2"; exit 2 ;; esac
case $tag in ''|*[!A-Za-z0-9_]*) echo "APPS_STAGE_RETRY class=USAGE"; echo "APPS_STAGE_EXIT rc=2"; exit 2 ;; esac
here=$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)
apps=$here/../../apps
if [ "$which" = middle ]; then list=${KF_APPS_LIST:-$apps/gate_list_middle.txt}; else list=${KF_APPS_LIST:-$apps/gate_list.txt}; fi
budget=${KF_APPS_BUDGET_S:-3000}
per_boot=${KF_APPS_PER_BOOT:-100}
poll=${KF_APPS_POLL_S:-5}
root=${KF_APPS_ROOT:-/workspace/apps}
# The apps live in a COPY of the fat guest image (as in docs/design/V3_APP_MATRIX.md R2): the thin suite,
# the CUDA ladder and the Windows stage keep the pristine guest.qcow2. boot_nvkvm.sh honours KF_GUEST_IMG.
KF_GUEST_IMG=${KF_GUEST_IMG:-${BENCH_DIR:-/workspace/bench}/guest_apps.qcow2}
export KF_GUEST_IMG
results=$root/results/$tag
marker=$root/PROVISIONED
matrix=${KF_APPS_MATRIX:-$apps/apps_matrix.sh}
mkdir -p "$prov"
log=$prov/${tag}_apps_matrix.log

finish() {  # verdict-line rc
    echo "$1"
    echo "APPS_STAGE_EXIT rc=$2"
    exit "$2"
}

policy=$(python3 -I "$here/../../ci/app_policy.py" linux) || finish "APPS_STAGE_RETRY class=BAD_LIST" 3
printf '%s\n' "$policy"

[ -s "$list" ] || finish "APPS_STAGE_RETRY class=NO_LIST" 3
mapfile -t gate < <(grep -Ev '^[[:space:]]*(#|$)' "$list")
[ "${#gate[@]}" -ge 1 ] || finish "APPS_STAGE_RETRY class=NO_LIST" 3
scratch=$(mktemp -d)   # run_apps.sh makes its output directory before it lists: keep that off /opt/apps
known=$(APPS_ROOT=$scratch bash "$apps/run_apps.sh" x list)
rm -rf "$scratch"
for app in "${gate[@]}"; do
    if grep -Fq "APP_POLICY name=$app " <<< "$policy"; then
        finish "APPS_STAGE_RETRY class=BAD_LIST" 3
    fi
    case $app in *[!A-Za-z0-9_]*) finish "APPS_STAGE_RETRY class=BAD_LIST" 3 ;; esac
    case " $known " in *" $app "*) ;; *) finish "APPS_STAGE_RETRY class=BAD_LIST" 3 ;; esac
done
echo "APPS_STAGE_LIST n=${#gate[@]} budget_s=$budget per_boot=$per_boot"

if [ ! -s "$root/bundle.tgz" ] || [ ! -f "$marker" ]; then
    if [ "${KF_APPS_PROVISION:-0}" = 1 ]; then
        echo "APPS_PROVISION_START $(date -Is)"
        { bash "$apps/build_bundle.sh" \
            && { [ -f "$KF_GUEST_IMG" ] || cp "${BENCH_DIR:-/workspace/bench}/guest.qcow2" "$KF_GUEST_IMG"; } \
            && sed "s#^image=.*#image=$KF_GUEST_IMG#" "${BENCH_DIR:-/workspace/bench}/guest.qcow2.STAGED" >"$KF_GUEST_IMG.STAGED" \
            && bash "$apps/provision_guest_apps.sh" && date -Is >"$marker"; } \
            >"$prov/${tag}_apps_provision.log" 2>&1
        echo "APPS_PROVISION_END rc=$? $(date -Is)"
    fi
    if [ ! -f "$marker" ]; then
        finish "APPS_STAGE_RETRY class=NOT_BUILT" 3
    fi
fi

rm -rf "$results"
mkdir -p "$results"
start=$(date +%s)
echo "APPS_STAGE_START $(date -Is) tag=$tag"
# The matrix runs in its own session, so the budget kills the whole tree (QEMU included) and nothing
# outlives this script. APPS_APP_TMO bounds one app's SSH call above its own run_apps.sh timeout.
APPS_PER_BOOT=$per_boot APPS_APP_TMO=${KF_APPS_APP_TMO:-1000} \
    setsid timeout -k 30 "$budget" bash "$matrix" guest "$tag" "${gate[@]}" >"$log" 2>&1 &
matrix_pid=$!

seen=0
emit_tries() {  # APP_TRY lines for batched rows not printed yet (live progress, not the verdict)
    local file=$results/guest.res total line
    [ -f "$file" ] || return 0
    total=$(grep -ac '^APPRES ' "$file")
    while [ "$seen" -lt "$total" ]; do
        seen=$((seen + 1))
        line=$(grep -a '^APPRES ' "$file" | sed -n "${seen}p")
        echo "APP_TRY ${line#APPRES side=guest }" | cut -c1-300
    done
}
while kill -0 "$matrix_pid" 2>/dev/null; do
    emit_tries
    sleep "$poll"
done
wait "$matrix_pid"
matrix_rc=$?
emit_tries
elapsed=$(( $(date +%s) - start ))
echo "APPS_STAGE_MATRIX_END rc=$matrix_rc secs=$elapsed"

# The verdict of record per app: the isolated re-run when there is one, else the batched row.
passed=0 failed=0 unrun=0
for app in "${gate[@]}"; do
    row=""
    for file in "$results/guest_isolated.res" "$results/guest.res"; do
        [ -f "$file" ] || continue
        row=$(grep -a "^APPRES side=guest app=$app " "$file" | tail -n 1)
        [ -z "$row" ] || break
    done
    if [ -z "$row" ]; then
        unrun=$((unrun + 1))
        echo "APP_ROW name=$app os=linux result=NOT_RUN secs=- guest_xid=- note=no result row (budget or boot)"
        continue
    fi
    verdict=$(sed -n 's/.* verdict=\([A-Z_]*\) .*/\1/p' <<<"$row")
    secs=$(sed -n 's/.* secs=\([0-9-]*\) .*/\1/p' <<<"$row")
    xid=$(sed -n 's/.* guest_xid=\([0-9]*\).*/\1/p' <<<"$row")
    note=$(sed -n 's/.* note=\(.*\) boot=.*/\1/p' <<<"$row" | cut -c1-120)
    if [ "$verdict" = PASS ]; then
        passed=$((passed + 1))
        echo "APP_ROW name=$app os=linux result=PASS secs=${secs:--} guest_xid=${xid:--} note=${note:--}"
    elif [ "$verdict" = NOTRUN ] || [ "$verdict" = BOOT_FAIL ]; then
        unrun=$((unrun + 1))
        echo "APP_ROW name=$app os=linux result=NOT_RUN secs=${secs:--} guest_xid=${xid:--} note=$verdict ${note:--}"
    else
        failed=$((failed + 1))
        echo "APP_ROW name=$app os=linux result=FAIL secs=${secs:--} guest_xid=${xid:--} note=$verdict ${note:--}"
        echo "$app --- FAILED"   # the payload's FINAL SUMMARY lists these as failed_test rows
    fi
done
echo "APPS_STAGE_COUNTS pass=$passed fail=$failed not_run=$unrun of=${#gate[@]} secs=$elapsed"

if [ "$failed" -gt 0 ]; then
    finish "APPS_STAGE_FAIL" 1
elif [ "$unrun" -gt 0 ]; then
    if [ "$matrix_rc" = 124 ] || [ "$matrix_rc" = 137 ]; then
        finish "APPS_STAGE_RETRY class=TIME" 3
    fi
    finish "APPS_STAGE_RETRY class=BOX" 3
elif [ "$passed" -eq "${#gate[@]}" ] && [ "$matrix_rc" = 0 ]; then
    finish "APPS_STAGE_PASS" 0
fi
finish "APPS_STAGE_RETRY class=OPERATOR" 3
