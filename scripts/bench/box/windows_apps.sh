#!/bin/bash
# windows_apps.sh <tag> [<prov-dir>] -- the Windows APPS stage: the Windows version of the app matrix, run in the
# Windows 11 LTSC guest that the Windows gate just passed, on the same box. Run ON a provisioned box, from the
# checkout, AFTER `windows_gate.sh <tag>` ended in WINDOWS_GATE_PASS (vast_payload.py calls it as the stage
# `windows_apps`, opt-in, KF_WINAPPS=small|mid|full from the Worker):
#   bash scripts/bench/box/windows_apps.sh <tag>              log: <prov-dir>/<tag>_winapps.log
#
# STATUS: DESIGN + CODE, 2026-11-07 (docs/design/V3_WINDOWS_APPS_GATE.md). GPU-free tests only
# (scripts/ci/test_vast_windows_apps.py, `--selftest`); the first hardware run is traces/winapps_20261107/.
#
# What it does: boots FRESH guests, each a new qcow2 overlay of the gate's overlay (driver, GSP forced, autologon and
# the one-time device enable are in it; the gate's overlay is sealed read-only first), with the read-only app disk
# (kfapps.iso, label KFAPPS) on the VM's own AHCI bus and the guest OFFLINE (QEMU user networking restrict=on; only the
# harness's ssh forward). appmatrix/winapps.py runs the profile's apps one after another (detached, supervised,
# TDR/WER accounting, fresh guest after a reset), re-runs every non-PASS app alone in a fresh guest, and
# appmatrix/grade.py applies the verdict rules. This script adds the same-box host GPU reference before and after.
#
# Inputs (never from a vendor: DOCS: docs/DOWNLOADS.md "Windows app bundle"):
#   KF_WINAPPS_ISO=<file>   an app disk that is already on the box (by hand); or
#   the pinned R2 bundle `winapps-open.iso` (name in ci/vast-broker/src/benchdata.js), fetched by
#   fetch_bench_data.sh through the Worker with this run's token; the sha256 pin is
#   scripts/bench/windows/appmatrix/bundle.pin. Without a pin the stage is RETRY class=MEDIA, never a pass.
#
# Verdict lines. THE GATE IS THE GUEST PASSING (owner, 2026-11-01): exit 0 only for WINAPPS_PASS. A host fault, a
# harness fault, a missing input or a rental fault is RETRY (exit 3, "re-run on a fresh box"), never a pass and never
# an excuse. Fail closed: a log without WINAPPS_EXIT, or with any other last verdict, is not a pass.
#   WINAPPS_PASS rev=<rev> gpu="<name>" profile=<p> apps=<n> pass=<n> ...   exit 0  every expected app PASSed in the guest,
#                                                                                   host reference green before and after,
#                                                                                   no new host Xid
#   WINAPPS_FAIL diag=KAYFABE profile=<p> failed=<ids> ...                   exit 1  at least one app FAILed/TIMEd out alone in a
#                                                                                   fresh guest (or a guest did not boot) with the
#                                                                                   host reference green before and after
#   WINAPPS_RETRY action=rerun-on-a-fresh-box class=<c> ...                  exit 3  PROVIDER, MEDIA (no or wrong app disk),
#                                                                                   OPERATOR (our script: NOTRUN rows, budget ended,
#                                                                                   driver died), HOSTREF, BAR1_EXCEPTION
#   WINAPPS_REFUSED why=<w>                                                  exit 2  a precondition (not root, no gate pass, ...)
#   WINAPPS_EXIT rc=<n> <utc>                                                always last
# Knobs: KF_WINAPPS_PROFILE (small | mid | full | ids:a,b,c; default small), KF_WINAPPS_BUDGET_S (per profile default),
# KF_WINAPPS_PER_GUEST (8), KF_WINAPPS_MAX_TDR (3), KF_WINAPPS_GATE_TAG (the gate's overlay tag), WINVM_ROOT, BENCH.
set -uo pipefail
SELFTEST=0
if [ "${1:-}" = --selftest ]; then SELFTEST=1; set -- selftest "$(mktemp -d)"; fi
T=${1:?usage: windows_apps.sh <tag> [<prov-dir>]}
[[ "$T" =~ ^[a-zA-Z0-9][a-zA-Z0-9._-]*$ ]] || { echo "WINAPPS_REFUSED why=invalid-tag"; exit 2; }
PROV=${2:-${KF_PROV_DIR:-/root/prov}}
SOURCE=$(cd "$(dirname "$(readlink -f "${BASH_SOURCE[0]}")")/../../.." && pwd)
WIN=$SOURCE/scripts/bench/windows
APPM=$WIN/appmatrix
WV=$WIN/win_vm.sh
LOG=$PROV/${T}_winapps.log
EV=$PROV/${T}_winapps
mkdir -p "$PROV"
[ ! -e "$LOG" ] || { echo "WINAPPS_REFUSED why=would-overwrite-$LOG"; exit 2; }
: > "$LOG"
say() { echo "$*" | tee -a "$LOG"; }

export PATH="$PATH:$HOME/.cargo/bin"
export BENCH=${BENCH:-/workspace/bench}
export WINVM_ROOT=${WINVM_ROOT:-/workspace/winvm}
export WINVM_VM_IDENTITY=${WINVM_VM_IDENTITY:-1}
export WINVM_SSH_CONNECT_TIMEOUT=${WINVM_SSH_CONNECT_TIMEOUT:-100}
export WINVM_LOCK_WAIT_S=${WINVM_LOCK_WAIT_S:-120}
export KF_GUEST_USER=${KF_GUEST_USER:-kf}
unset KF_GUEST_PW                       # a win_vm.sh guest signs in by autologon: no password goes through the harness
PROFILE=${KF_WINAPPS_PROFILE:-small}
PER_GUEST=${KF_WINAPPS_PER_GUEST:-8}
MAX_TDR=${KF_WINAPPS_MAX_TDR:-3}
GATE_TAG=${KF_WINAPPS_GATE_TAG:-gate_$(printf %s "$T" | tr -c 'A-Za-z0-9_' '_' | cut -c1-24)}
FLAGS=${KF_WINGATE_FLAGS:-$WIN/kf3_flags.sample.env}
GDRV=${KF_WINGATE_GUEST_DRIVER:-580.65.06}
ARM=${KF_WINGATE_ARM:-B}
VM=$WINVM_ROOT/${WINVM_NAME:-kfwin}
REV=$(git -C "$SOURCE" rev-parse --short=8 HEAD 2> /dev/null || echo norev)
REVDIR=$REV
[ -z "$(git -C "$SOURCE" status --porcelain --untracked-files=no 2> /dev/null)" ] || REVDIR=$REV-dirty
export WINVM_QEMU=${WINVM_QEMU:-$BENCH/kf3-bins/$REVDIR/qemu-system-x86_64}
T0=$(date +%s)
VERDICT=""
GPU_NAME=unknown
HOSTREF_BEFORE=unrun HOSTREF_AFTER=unrun
STAGE=start
GUEST_STARTED=0
rc_final=0
IDS=""
NIDS=0

verdict() { # KIND CLASS WHY...   KIND: PASS|FAIL|RETRY
    local kind=$1 class=$2 why total
    shift 2
    why=$*
    VERDICT=$kind
    total=$(($(date +%s) - T0))
    case "$kind" in
        PASS) say "WINAPPS_PASS rev=$REV gpu=\"$GPU_NAME\" profile=$PROFILE apps=$NIDS hostref=$HOSTREF_BEFORE/$HOSTREF_AFTER secs=$total $why"; rc_final=0 ;;
        FAIL) say "WINAPPS_FAIL diag=$class profile=$PROFILE stage=$STAGE why=\"$why\" rev=$REV gpu=\"$GPU_NAME\" hostref=$HOSTREF_BEFORE/$HOSTREF_AFTER secs=$total"; rc_final=1 ;;
        *) say "WINAPPS_RETRY action=rerun-on-a-fresh-box class=$class profile=$PROFILE stage=$STAGE why=\"$why\" rev=$REV gpu=\"$GPU_NAME\" hostref=$HOSTREF_BEFORE/$HOSTREF_AFTER secs=$total"; rc_final=3 ;;
    esac
}
refuse() { say "WINAPPS_REFUSED why=$1"; VERDICT=REFUSED; rc_final=2; exit 2; }

# The class of a non-zero outcome from the grade (grade.py), the same-box host reference and the BAR1 exception.
# $1 = grade kind (PASS|FAIL|RETRY), $2 = host reference before, $3 = after, $4 = 1 when kf3 refused a BAR1 size and
# no guest ever answered ssh (the documented Vast exception), $5 = new host Xid count over the run.
classify() {
    local kind=$1 hb=$2 ha=$3 bar1=$4 xid=$5
    case "$kind" in
        FAIL)
            if [ "$bar1" = 1 ]; then echo "RETRY BAR1_EXCEPTION"
            elif [ "$hb" != green ] || [ "$ha" != green ]; then echo "RETRY HOSTREF"
            else echo "FAIL KAYFABE"; fi ;;
        RETRY) echo "RETRY OPERATOR" ;;
        PASS)
            if [ "$ha" != green ]; then echo "RETRY HOSTREF"
            elif [ "${xid:-0}" != 0 ]; then echo "FAIL KAYFABE"
            else echo "PASS NONE"; fi ;;
        *) echo "RETRY OPERATOR" ;;
    esac
}

run_loop_alive_any() { pgrep -f "win_vm\.sh run " > /dev/null; }
teardown_all() {
    local w=0
    mkdir -p "$VM/run" && touch "$VM/run/stop-requested"
    while [ "$w" -le 180 ]; do
        if [ -f "$VM/run/qemu.pid" ] && [ -d "/proc/$(cat "$VM/run/qemu.pid" 2> /dev/null || echo 0)" ]; then timeout 420 bash "$WV" stop >> "$EV/stop_cleanup.txt" 2>&1 < /dev/null; fi
        run_loop_alive_any || return 0
        sleep 3
        w=$((w + 4))
    done
    say "APPS_NOTE a win_vm.sh run loop is still alive after 180 s of the stop; not killed"
}
on_exit() {
    local rc=$?
    if [ -z "$VERDICT" ]; then
        VERDICT=RETRY
        say "WINAPPS_RETRY action=rerun-on-a-fresh-box class=OPERATOR profile=$PROFILE stage=$STAGE why=\"apps script ended without a verdict (rc=$rc)\" rev=$REV gpu=\"$GPU_NAME\" hostref=$HOSTREF_BEFORE/$HOSTREF_AFTER"
        rc_final=3
    fi
    if [ "$GUEST_STARTED" = 1 ]; then teardown_all; fi
    say "WINAPPS_EXIT rc=$rc_final $(date -u -Is)"
    exit "$rc_final"
}
trap on_exit EXIT
stage() { STAGE=$1; say "APPS_STAGE $1 $(date -u -Is)"; }

selftest() {
    local n=0 bad=0 out
    cs() { # NAME WANT KIND HB HA BAR1 XID
        n=$((n + 1))
        out=$(classify "$3" "$4" "$5" "$6" "$7")
        [ "$out" = "$2" ] && return 0
        bad=$((bad + 1))
        echo "WINAPPS_SELFTEST_FAIL case=$1 want=\"$2\" got=\"$out\""
    }
    cs pass-green 'PASS NONE' PASS green green 0 0
    cs pass-hostref-red-after 'RETRY HOSTREF' PASS green RED 0 0
    cs pass-new-xid 'FAIL KAYFABE' PASS green green 0 3
    cs fail-green 'FAIL KAYFABE' FAIL green green 0 0
    cs fail-hostref-before 'RETRY HOSTREF' FAIL RED green 0 0
    cs fail-hostref-after 'RETRY HOSTREF' FAIL green RED 0 0
    cs fail-bar1 'RETRY BAR1_EXCEPTION' FAIL green green 1 0
    cs retry-harness 'RETRY OPERATOR' RETRY green green 0 0
    cs unknown-kind-never-pass 'RETRY OPERATOR' MAYBE green green 0 0
    # a log can only pass through verdict(): the exit code and the line agree
    n=$((n + 1))
    out=$(
        LOG=$(mktemp)
        verdict PASS NONE ok > /dev/null
        echo "RC=$rc_final"
        verdict FAIL KAYFABE x > /dev/null
        echo "RC=$rc_final"
        verdict RETRY OPERATOR x > /dev/null
        echo "RC=$rc_final"
        rm -f "$LOG"
    )
    [ "$out" = "$(printf 'RC=0\nRC=1\nRC=3')" ] || { bad=$((bad + 1)); echo "WINAPPS_SELFTEST_FAIL case=verdict-rc got: $out"; }
    if [ "$bad" = 0 ]; then echo "WINAPPS_SELFTEST_OK cases=$n"; rc_final=0; exit 0; fi
    echo "WINAPPS_SELFTEST_FAILED bad=$bad of $n"
    rc_final=1
    exit 1
}
if [ "$SELFTEST" = 1 ]; then VERDICT=selftest; selftest; fi

# ------------------------------------------------------------------------------------------------
mkdir -p "$EV"
chmod 0700 "$EV"
say "WINAPPS_START tag=$T rev=$REV source=$SOURCE $(date -u -Is) profile=$PROFILE gate_tag=$GATE_TAG per_guest=$PER_GUEST"
say "WINAPPS_REQUESTED=1"
python3 -I "$SOURCE/scripts/ci/app_policy.py" windows || refuse policy-unreadable

stage preflight
[ "$(id -u)" = 0 ] || refuse not-root
[ -c /dev/kvm ] || refuse no-/dev/kvm
[ -e "$VM/installed" ] || refuse "windows-not-installed-($VM)"
grep -q '^WINDOWS_GATE_PASS ' "$PROV/${T}_windows.log" 2> /dev/null || refuse "no-WINDOWS_GATE_PASS-in-$PROV/${T}_windows.log"
[ -f "$VM/disk/$GATE_TAG.qcow2" ] || refuse "no-gate-overlay-disk/$GATE_TAG.qcow2"
[ -r "$FLAGS" ] || refuse "no-flags-file-$FLAGS"
[ -x "$WINVM_QEMU" ] || refuse "no-kf3-qemu-$WINVM_QEMU"
[ -s "$VM/identity.env" ] || refuse "no-vm-identity"
if pgrep -x qemu-system-x86 > /dev/null || run_loop_alive_any; then refuse "a-qemu-or-run-loop-is-already-running"; fi
avail_gb=$(df -BG --output=avail "$(dirname "$WINVM_ROOT")" 2> /dev/null | tail -1 | tr -dc 0-9)
[ "${avail_gb:-0}" -ge 15 ] || refuse "disk-${avail_gb:-0}G-free-need-15G"
if ! nvidia-smi -L > "$EV/nvidia_smi_L.txt" 2>&1; then
    verdict RETRY PROVIDER "nvidia-smi -L failed on the host: $(head -c 200 "$EV/nvidia_smi_L.txt")"
    exit 3
fi
GPU_NAME=$(nvidia-smi --query-gpu=name --format=csv,noheader | head -1)
# the profile's app list: one source of truth (appmatrix/gate_profiles.json), checked against apps.json
if ! IDS=$(python3 -I "$APPM/gate_profiles.py" ids "$PROFILE" 2> "$EV/profile_err.txt"); then
    refuse "profile-$PROFILE-$(head -c 120 "$EV/profile_err.txt" | tr '\n ' '__')"
fi
NIDS=$(printf %s "$IDS" | tr ',' '\n' | grep -c .)
BUDGET_S=${KF_WINAPPS_BUDGET_S:-$(python3 -I "$APPM/gate_profiles.py" budget "$PROFILE")}
say "APPS_NOTE host: gpu=\"$GPU_NAME\" cpus=$(nproc) ram_gb=$(free -g | awk '/^Mem:/{print $2}') disk_free_gb=$avail_gb apps=$NIDS budget_s=$BUDGET_S"
say "APPS_NOTE app ids: $IDS"

stage app_disk
ISO=${KF_WINAPPS_ISO:-}
if [ -z "$ISO" ]; then
    PIN=$(sed -n 's/^winapps-open\.iso sha256=\([0-9a-f]\{64\}\)$/\1/p' "$APPM/bundle.pin" 2> /dev/null | head -1)
    if [ -z "$PIN" ]; then
        verdict RETRY MEDIA "no sha256 pin for winapps-open.iso in appmatrix/bundle.pin: the bundle was never packaged and pinned (docs/DOWNLOADS.md, Windows app bundle)"
        exit 3
    fi
    ISO=/workspace/winapps/kfapps.iso
    mkdir -p "$(dirname "$ISO")"
    if ! bash "$SOURCE/scripts/bench/box/fetch_bench_data.sh" winapps-open.iso --sha256 "$PIN" --out "$ISO" > "$EV/fetch_bundle.txt" 2>&1; then
        verdict RETRY MEDIA "fetch of winapps-open.iso failed (fetch_bundle.txt): $(tail -1 "$EV/fetch_bundle.txt" | head -c 160)"
        exit 3
    fi
fi
[ -f "$ISO" ] || { verdict RETRY MEDIA "no app disk at $ISO"; exit 3; }
ISO_SHA=$(sha256sum "$ISO" | cut -d' ' -f1)
say "APPS_NOTE app disk $ISO sha256=$ISO_SHA bytes=$(stat -c %s "$ISO")"
IMGMAN=${KF_WINAPPS_IMAGE_MANIFEST:-$(dirname "$ISO")/manifest.json}
[ -f "$IMGMAN" ] || IMGMAN=$APPM/manifest.json

stage hostref_before
LIBCUDA=-lcuda
[ -e /usr/lib/x86_64-linux-gnu/libcuda.so ] || LIBCUDA=/usr/lib/x86_64-linux-gnu/libcuda.so.1
gcc -O0 -I "$SOURCE/scripts/bench/cuda_min" -o "$EV/cup8_host" "$SOURCE/scripts/bench/cup8.c" $LIBCUDA -lm 2> "$EV/cup8_host_build.txt" \
    || say "APPS_NOTE the host cup8 reference did not build: $(tail -2 "$EV/cup8_host_build.txt" | tr '\n' ' ')"
hostref() { # before|after
    local when=$1 f="$EV/hostref_$1.txt" ok=1 xid cu xb state
    {
        echo "HOSTREF $when $(date -u -Is)"
        nvidia-smi --query-gpu=name,driver_version,memory.total,pstate --format=csv,noheader 2>&1 || ok=0
    } > "$f" 2>&1
    xid=$(dmesg 2> /dev/null | grep -c 'NVRM: Xid' || true)
    echo "HOSTREF_XID_COUNT $xid" >> "$f"
    if [ -x "$EV/cup8_host" ]; then
        if timeout 180 "$EV/cup8_host" > "$EV/hostref_${when}_cup8.txt" 2>&1 && grep -q 'CUP8 VERDICT: PASS' "$EV/hostref_${when}_cup8.txt"; then cu=PASS; else cu=FAIL; fi
    else cu=NOTBUILT; fi
    echo "HOSTREF_CUP8 $cu" >> "$f"
    [ "$cu" = PASS ] || ok=0
    if [ "$when" = before ]; then echo "$xid" > "$EV/xid_before"; fi
    xb=$(cat "$EV/xid_before" 2> /dev/null || echo 0)
    echo "HOSTREF_XID_DELTA $((xid - xb))" >> "$f"
    if [ "$ok" = 1 ]; then state=green; else state=RED; fi
    if [ "$when" = before ]; then HOSTREF_BEFORE=$state; else HOSTREF_AFTER=$state; fi
    say "APPS_HOSTREF $when $state cup8=$cu xid_count=$xid xid_delta=$((xid - xb))"
}
hostref before
if [ "$HOSTREF_BEFORE" != green ]; then
    verdict RETRY HOSTREF "the host GPU reference is not green BEFORE the guests (hostref_before.txt)"
    exit 3
fi

stage seal_parent
# The gate's overlay is the parent of every apps overlay: it must not change again. Sealing is a mode bit on a file of
# this disposable box; the gate's evidence has been taken.
chmod 0400 "$VM/disk/$GATE_TAG.qcow2"

stage run_matrix
GUEST_STARTED=1
RUN=$EV/run
DEADLINE=$(($(date +%s) + BUDGET_S))
python3 -I "$APPM/winapps.py" --vm winvm --run-dir "$RUN" --apps "$IDS" --tier 3 --iso "$ISO" --image-manifest "$IMGMAN" \
    --winvm-script "$WV" --winvm-root "$WINVM_ROOT" --winvm-name "${WINVM_NAME:-kfwin}" --parent-overlay "$GATE_TAG" --flags-file "$FLAGS" \
    --guest-driver "$GDRV" --arm "$ARM" --per-guest "$PER_GUEST" --max-tdr "$MAX_TDR" --deadline-epoch "$DEADLINE" > "$EV/driver.out" 2>&1
DRC=$?
say "APPS_NOTE winapps.py rc=$DRC (0 done, 3 repeated guest boot failures, 4 guest limit, 6 time budget ended)"
teardown_all
GUEST_STARTED=0

stage hostref_after
hostref after
xid_delta=$(sed -n 's/^HOSTREF_XID_DELTA //p' "$EV/hostref_after.txt" 2> /dev/null)

stage grade
python3 -I "$APPM/summarize.py" "$RUN" > "$EV/summary.md" 2> /dev/null || true
python3 -I "$APPM/grade.py" "$RUN" --expected "$IDS" > "$EV/grade.txt" 2>&1
GRC=$?
grep -a '^WINAPPS_' "$EV/grade.txt" | while read -r l; do say "$l"; done
kind=PASS
[ "$GRC" = 0 ] || { [ "$GRC" = 1 ] && kind=FAIL || kind=RETRY; }
# a driver that did not run to its end is never a pass, whatever rows exist
[ "$DRC" = 0 ] || { [ "$kind" = PASS ] && kind=RETRY; }
bar1=0
for q in "$VM"/logs/apps_*_b1_qemu.log; do
    [ -f "$q" ] || continue
    grep -a 'realize refused' "$q" | sed 's/.*realize refused//' | grep -qi 'bar1' && bar1=1
done
grep -q 'SSH_UP\|READY boot=' "$RUN/session.log" 2> /dev/null && bar1=0    # a guest answered: not the BAR1 exception
read -r KIND CLASS <<< "$(classify "$kind" "$HOSTREF_BEFORE" "$HOSTREF_AFTER" "$bar1" "$xid_delta")"
why="grade: $(grep -a '^WINAPPS_GRADE kind=' "$EV/grade.txt" | head -1 | cut -d' ' -f2-)"
for k in GUEST_FAIL BOOT_FAIL HARNESS NOT_RUN; do
    l=$(grep -a "^WINAPPS_GRADE_$k " "$EV/grade.txt" | head -1 | cut -d' ' -f2-)
    [ -z "$l" ] || why+="; ${k,,}=$l"
done
[ "$DRC" = 0 ] || why+="; driver rc=$DRC"
[ "${xid_delta:-0}" = 0 ] || why+="; host Xid delta=${xid_delta}"

stage collect
# text only: never secrets/, tpm/, firmware/, disks
for f in "$VM"/logs/apps_*_run.log "$VM"/logs/apps_*_host_smi_*.txt "$VM"/logs/apps_*_host_dmesg.txt "$VM"/logs/apps_*_cmdline.txt; do
    [ -f "$f" ] && cp "$f" "$EV/" 2> /dev/null
done
for f in "$VM"/logs/apps_*_qemu.log; do
    [ -f "$f" ] && gzip -c "$f" > "$EV/$(basename "$f").gz"
done
rm -f "$EV/cup8_host"
(cd "$EV" && find . -type f ! -name MANIFEST.sha256 -print0 | sort -z | xargs -0 -r sha256sum > MANIFEST.sha256)
tar -C "$PROV" -czf "$PROV/${T}_winapps.tgz" "${T}_winapps" 2> /dev/null
say "APPS_NOTE evidence: $EV ($(find "$EV" -type f | wc -l) files), $PROV/${T}_winapps.tgz sha256=$(sha256sum "$PROV/${T}_winapps.tgz" | cut -c1-16)"

STAGE="done"
verdict "$KIND" "$CLASS" "$why"
exit "$rc_final"
