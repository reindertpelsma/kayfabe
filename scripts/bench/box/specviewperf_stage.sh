#!/bin/bash
# specviewperf_stage.sh <tag> <prov-dir> -- the SPECviewperf lane of the Vast gate: the lane's viewsets run in the
# Windows 11 LTSC guest that the Windows gate just passed, on the same box. Run ON a provisioned box, from the
# checkout, AFTER windows_gate.sh <tag> ended in WINDOWS_GATE_PASS. scripts/ci/vast_stage_specviewperf.sh calls it
# (the wrapper keeps this script's whole output in a private file and lets only the status lines through):
#
#   KF_SPEC_LANE=lanea|laneb|both KF_SPEC_DEADLINE_EPOCH=<epoch s> bash scripts/bench/box/specviewperf_stage.sh <tag> <prov-dir>
#
# STATUS: DESIGN + CODE, 2026-11-07 (docs/design/V3_SPEC_GATE.md). GPU-free tests only
# (scripts/ci/test_vast_spec_stage.py, scripts/ci/spec_stage_mutants.py: a fake win_vm.sh and a fake fetch script).
# NO BOX HAS RUN IT. DISABLED BY DEFAULT: nothing starts it until the Worker sends KF_STAGES=specviewperf with KF_SPEC.
#
# What it does, strictly serial:
#   1. preflight: root, /dev/kvm, the gate's PASS and overlay, the lane, the deadline, disk and RAM; the sha256 pin
#      (scripts/bench/box/specviewperf.pin: without it RETRY class=MEDIA);
#   2. starts the download of the private copy through the Worker's /bench-data route (fetch_bench_data.sh, run token
#      only; the Worker mints a presigned URL; the sha256 is checked before anything is used) in the background;
#   3. the same-box host GPU reference BEFORE (as the gate);
#   4. boot A: the gate's overlay, sealed, with a fresh child overlay, WITHOUT kf3 (std VGA: 94 GB of tree through the
#      kf3 guest would only add risk), 16 GB RAM; the tree is streamed in
#        zstd -d | tarfilter.py (this lane's viewsets only) | ssh ... tar -xf -
#      and compared with the filter's manifest; write access and the shipped vcredist are set; the guest stops;
#   5. boot B: the child overlay WITH kf3 (the gate's flags and guest driver), the desktop, then the settle wait (no
#      CLI run sooner than settle_s after the boot: the first run within about 2 minutes of a reboot fails with
#      `Unable to gather system details`, MEASURED natively);
#   6. each viewset of the lane, one CLI run each (1 iteration, FullHD), detached and polled; the time guard decides
#      before every viewset whether it may start and with which budget (lane.py timeout); a fresh boot (and settle)
#      follows a viewset that took the guest down, at most max_guest_boots times;
#   7. the host reference AFTER and the host Xid delta; the verdict (lane.py decide); the teardown deletes the copy
#      (the archive and the child overlay that holds the tree) before this script ends.
#
# Output (stdout; the wrapper lets only these through, strict patterns, no score exists in them):
#   SPEC_STAGE_LIST lane=<l> n=<n>
#   SPEC_ROW name=<viewset> result=PASS|FAIL|NOT_RUN subtests=<done>/<total> reason=<WORD|-> failed=<names|->   (one per
#       viewset, printed as soon as it is known: a killed run still reports what finished)
#   <viewset_with_underscores> --- FAILED                                    (the payload's FINAL SUMMARY lists these)
#   SPEC_STAGE_COUNTS pass=<n> fail=<n> not_run=<n> of=<n>
#   SPECVIEWPERF_STAGE_PASS | SPECVIEWPERF_STAGE_FAIL | SPECVIEWPERF_STAGE_RETRY class=<A-Z_>
# THE GATE IS THE GUEST PASSING: exit 0 only for PASS. Missing media, a bad sha256, a transport fault, a rental
# shortfall, the time running out: RETRY (exit 3), never a pass and never an excuse. A viewset that fails, is refused
# by SPEC's own validation, hangs past its budget or resets the guest is FAIL (exit 1) unless the same-box host GPU
# reference was red (RETRY class=HOSTREF) or kf3 refused a BAR1 size (RETRY class=BAR1_EXCEPTION, the documented Vast
# exception).
# Scores: this script reads status lines only. The vendor result directory stays in the guest (never opened, never
# copied) and dies with the box. The evidence kept under <prov-dir>/<tag>_spec/ is the status-only extract of the CLI
# logs (lane.py scrub), the rows, the host reference and the notes.
# Knobs (all optional): KF_SPEC_BUDGET_S (instead of the deadline), KF_SPEC_STORE (/workspace/spec), KF_SPEC_SETTLE_S
# (lanes.json settle_s), KF_SPEC_GUEST_RAM_MB (16384), WINVM_ROOT, BENCH, KF_WINGATE_FLAGS, KF_WINGATE_GUEST_DRIVER,
# KF_WINGATE_ARM, KF_SPEC_GATE_TAG (the gate's overlay tag).
set -uo pipefail
USAGE_LINE="SPECVIEWPERF_STAGE_RETRY class=USAGE"
[ "$#" -eq 2 ] || { echo "$USAGE_LINE"; exit 2; }
T=$1
PROV=$2
[[ "$T" =~ ^[A-Za-z0-9][A-Za-z0-9_]*$ ]] || { echo "$USAGE_LINE"; exit 2; }
LANE=${KF_SPEC_LANE:-}
case "$LANE" in lanea | laneb | both) ;; *) echo "$USAGE_LINE"; exit 2 ;; esac
SOURCE=$(cd "$(dirname "$(readlink -f "${BASH_SOURCE[0]}")")/../../.." && pwd)
WIN=$SOURCE/scripts/bench/windows
WV=$WIN/win_vm.sh
SPECD=$SOURCE/scripts/bench/specviewperf
FETCH=$SOURCE/scripts/bench/box/fetch_bench_data.sh
PINFILE=$SOURCE/scripts/bench/box/specviewperf.pin
lane_py() { python3 -I "$SPECD/lane.py" "$@"; }
OBJECT=specviewperf-15.0.0.tar.zst

export PATH="$PATH:$HOME/.cargo/bin"
export BENCH=${BENCH:-/workspace/bench}
export WINVM_ROOT=${WINVM_ROOT:-/workspace/winvm}
export WINVM_VM_IDENTITY=${WINVM_VM_IDENTITY:-1}
export WINVM_SSH_CONNECT_TIMEOUT=${WINVM_SSH_CONNECT_TIMEOUT:-100}
export WINVM_LOCK_WAIT_S=${WINVM_LOCK_WAIT_S:-120}
export KF_GUEST_USER=${KF_GUEST_USER:-kf}
unset KF_GUEST_PW
GUEST_RAM_MB=${KF_SPEC_GUEST_RAM_MB:-16384}   # SPEC's own check wants about 15 GB (MEASURED 2026-11-05: 8 GB fails every workload with a memory requirement)
FLAGS=${KF_WINGATE_FLAGS:-$WIN/kf3_flags.sample.env}
GDRV=${KF_WINGATE_GUEST_DRIVER:-580.65.06}
ARM=${KF_WINGATE_ARM:-B}
VM=$WINVM_ROOT/${WINVM_NAME:-kfwin}
STORE=${KF_SPEC_STORE:-/workspace/spec}
GATE_TAG=${KF_SPEC_GATE_TAG:-gate_$(printf %s "$T" | tr -c 'A-Za-z0-9_' '_' | cut -c1-24)}
TAG=spec_$(printf %s "$T" | tr -c 'A-Za-z0-9_' '_' | cut -c1-24)
OVERLAY=$TAG
ZST=$STORE/$OBJECT
EVD=$PROV/${T}_spec
# the guest's raw output (status lines, END and reset counters of each run) stays in WORK: the wrapper points it into its
# private directory, so that nothing but the status-only extracts reaches the evidence directory
WORK=${KF_SPEC_PRIVATE:-$EVD/work}
REV=$(git -C "$SOURCE" rev-parse --short=8 HEAD 2> /dev/null || echo norev)
REVDIR=$REV
[ -z "$(git -C "$SOURCE" status --porcelain --untracked-files=no 2> /dev/null)" ] || REVDIR=$REV-dirty
export WINVM_QEMU=${WINVM_QEMU:-$BENCH/kf3-bins/$REVDIR/qemu-system-x86_64}
# the lane table's constants and this lane's viewsets, in one call (SETTLE_S TAIL_S GATHER_WAIT_S MAX_BOOTS NEED_GB RELEASE IDS)
VARS=$(lane_py vars "$LANE") || { echo "$USAGE_LINE"; exit 2; }
eval "$VARS"
SETTLE_S=${KF_SPEC_SETTLE_S:-$SETTLE_S}
IFS=, read -r -a LANE_IDS <<< "$IDS"

NOW=$(date +%s)
if [ -n "${KF_SPEC_DEADLINE_EPOCH:-}" ]; then
    DEADLINE=$KF_SPEC_DEADLINE_EPOCH
elif [ -n "${KF_SPEC_BUDGET_S:-}" ]; then
    DEADLINE=$((NOW + KF_SPEC_BUDGET_S))
else
    echo "$USAGE_LINE"
    exit 2
fi
case "$DEADLINE" in '' | *[!0-9]*) echo "$USAGE_LINE"; exit 2 ;; esac
[ "$DEADLINE" -gt "$NOW" ] || { echo "SPECVIEWPERF_STAGE_RETRY class=TIME"; exit 3; }
umask 077
mkdir -p "$EVD" "$WORK" "$STORE" 2> /dev/null || { echo "SPECVIEWPERF_STAGE_RETRY class=OPERATOR"; exit 3; }
chmod 700 "$EVD" "$WORK"
LOG=$EVD/stage.log
: > "$LOG"
say() { echo "$*" | tee -a "$LOG"; }
note() { echo "SPEC_NOTE $*" | tee -a "$LOG"; }

VERDICT=""
rc_final=0
STAGE=start
GPU_NAME=unknown
HOSTREF_BEFORE=unrun HOSTREF_AFTER=unrun
GUEST_STARTED=0
KF3_SSH_SEEN=0   # 1 once a guest booted WITH kf3 answered ssh (then a BAR1 refusal cannot be the cause of a failure)
BOOTS=0          # every boot of this stage (copy boot included): names the run log of each
RUN_BOOTS=0      # kf3 boots (the lane's runs)
BOOT_UP=0
FIRST_RUN_TAG=""
ABORT=""
ROWS=$EVD/rows.txt
: > "$ROWS"
FETCH_PID=""

left() { local n=$((DEADLINE - $(date +%s))); [ "$n" -gt 0 ] || n=0; echo "$n"; }
cap() { # WANT -> min(WANT, left), at least 1 (`timeout 0` would mean no timeout at all)
    local l
    l=$(left)
    [ "$l" -ge 1 ] || l=1
    if [ "$1" -le "$l" ]; then echo "$1"; else echo "$l"; fi
}
stage() { STAGE=$1; note "stage=$1 $(date -u -Is) left_s=$(left)"; }

# ------------------------------------------------------------------------------------------------
# guest helpers (every call bounded; ssh goes through win_vm.sh's own key, port and known_hosts)
# ------------------------------------------------------------------------------------------------
g() { local t=$1; shift; timeout "$t" bash "$WV" ssh "$@" < /dev/null; }
gspec() { # SECONDS ACTION [ARGS...]
    local t=$1 a=$2
    shift 2
    g "$t" "powershell -NoProfile -ExecutionPolicy Bypass -File C:\\kf\\spec\\spec_guest.ps1 -Action $a $*" 2> /dev/null | tr -d '\r'
}
qemu_alive() {
    local pid
    pid=$(cat "$VM/run/qemu.pid" 2> /dev/null || true)
    [ -n "$pid" ] && [ -d "/proc/$pid" ] && [ "$(awk '{print $3}' "/proc/$pid/stat" 2> /dev/null)" != Z ]
}
guest_answers() { g 60 'echo KF_SSH_UP' 2> /dev/null | grep -q KF_SSH_UP; }
wait_qemu_start() { # RUN_TAG SECONDS
    local tag=$1 t=$2 w=0 rl=$VM/logs/${1}_run.log
    while ! grep -q "QEMU_START tag=$tag boot=1 " "$rl" 2> /dev/null; do
        if grep -q "WINVM_RUN_END tag=$tag \|REFUSED: \|die: " "$rl" 2> /dev/null; then
            note "the run loop of $tag ended before QEMU started: $(tail -1 "$rl" 2> /dev/null)"
            return 1
        fi
        if [ "$w" -ge "$t" ]; then
            note "no QEMU_START of $tag within ${t}s: $(tail -1 "$rl" 2> /dev/null)"
            return 1
        fi
        sleep 1
        w=$((w + 1))
    done
}
WAIT_KF3=0
wait_ssh() { # SECONDS  (WAIT_KF3=1 while a kf3 guest is awaited)
    local t=$1 w=0 per=$((WINVM_SSH_CONNECT_TIMEOUT + 12))
    while [ "$w" -lt "$t" ]; do
        qemu_alive || { note "QEMU is gone while waiting for ssh (after ${w}s)"; return 1; }
        if g "$per" 'echo KF_SSH_UP' 2> /dev/null | grep -q KF_SSH_UP; then
            [ "$WAIT_KF3" != 1 ] || KF3_SSH_SEEN=1
            note "ssh up after ${w}s"
            return 0
        fi
        sleep 10
        w=$((w + 10))
    done
    return 1
}
# a run loop of THIS checkout's win_vm.sh (the full path: another checkout's loop is not ours to wait for)
run_loops_alive() { pgrep -f -- "$WV run " > /dev/null; }
teardown_guest() {
    local w=0
    mkdir -p "$VM/run" && touch "$VM/run/stop-requested"
    while [ "$w" -le 180 ]; do
        if qemu_alive; then
            note "graceful stop of the guest"
            timeout 420 bash "$WV" stop >> "$EVD/stop_cleanup.txt" 2>&1 < /dev/null || note "stop returned non-zero (stop_cleanup.txt)"
        fi
        run_loops_alive || return 0
        sleep 3
        w=$((w + 4))
    done
    note "a win_vm.sh run loop is still alive 180 s after the stop; not killed"
}
boot_guest() { # copy|run  -> 0 ssh up, 1 no qemu, 2 no ssh
    local kind=$1 tag args
    BOOTS=$((BOOTS + 1))
    tag=${TAG}_${kind}${BOOTS}
    [ "$kind" != run ] || RUN_BOOTS=$((RUN_BOOTS + 1))
    [ -n "$FIRST_RUN_TAG" ] || [ "$kind" != run ] || FIRST_RUN_TAG=$tag
    args=(run --detach --once --overlay "$OVERLAY" --arm "$ARM" --tag "$tag")
    if [ "$kind" = copy ]; then
        args+=(--no-kf3)
    else
        args+=(--guest-driver "$GDRV" --kf3-extra "gpu-uuid=GPU-$VMUUID" --env-file "$FLAGS")
    fi
    GUEST_STARTED=1
    WAIT_KF3=0
    [ "$kind" != run ] || WAIT_KF3=1
    if ! WINVM_RAM_MB=$GUEST_RAM_MB bash "$WV" "${args[@]}" > "$WORK/launch_$tag.txt" 2>&1 < /dev/null; then
        note "win_vm.sh run did not start ($tag): $(tail -2 "$WORK/launch_$tag.txt" | tr '\n' ' ')"
        return 1
    fi
    wait_qemu_start "$tag" "$(cap 300)" || return 1
    wait_ssh "$(cap 900)" || return 2
    BOOT_UP=$(date +%s)
    return 0
}
wait_desktop() { # 0 once dwm and explorer run for the console user
    local w=0 st
    while [ "$w" -lt "$(cap 420)" ]; do
        st=$(g 120 "powershell -NoProfile -ExecutionPolicy Bypass -File C:\\kf\\gate_state.ps1 -Mode desktop" 2> /dev/null | tr -d '\r')
        if grep -qE "^GATE_DESKTOP user='[^']*\\\\$KF_GUEST_USER' dwm=[1-9][0-9]* explorer=[1-9]" <<< "$st"; then return 0; fi
        sleep 10
        w=$((w + 10))
    done
    return 1
}
settle() {
    local age wait_s
    age=$(($(date +%s) - BOOT_UP))
    wait_s=$((SETTLE_S - age))
    [ "$wait_s" -gt 0 ] || wait_s=0
    wait_s=$(cap "$wait_s")
    note "settle: no CLI run sooner than ${SETTLE_S}s after the boot (waiting ${wait_s}s)"
    echo "SPEC_NOTE settle_wait_s=$wait_s" >> "$LOG"
    [ "$wait_s" -eq 0 ] || sleep "$wait_s"
}

# ------------------------------------------------------------------------------------------------
# the same-box host reference (as windows_gate.sh and windows_apps.sh)
# ------------------------------------------------------------------------------------------------
hostref() { # before|after
    local when=$1 f="$EVD/hostref_$1.txt" ok=1 xid cu xb state
    {
        echo "HOSTREF $when $(date -u -Is)"
        nvidia-smi --query-gpu=name,driver_version,memory.total,pstate --format=csv,noheader 2>&1 || ok=0
    } > "$f" 2>&1
    xid=$(dmesg 2> /dev/null | grep -c 'NVRM: Xid' || true)
    echo "HOSTREF_XID_COUNT $xid" >> "$f"
    if [ -x "$EVD/cup8_host" ]; then
        if timeout 180 "$EVD/cup8_host" > "$EVD/hostref_${when}_cup8.txt" 2>&1 && grep -q 'CUP8 VERDICT: PASS' "$EVD/hostref_${when}_cup8.txt"; then cu=PASS; else cu=FAIL; fi
    else cu=NOTBUILT; fi
    echo "HOSTREF_CUP8 $cu" >> "$f"
    [ "$cu" = PASS ] || ok=0
    if [ "$when" = before ]; then echo "$xid" > "$EVD/xid_before"; fi
    xb=$(cat "$EVD/xid_before" 2> /dev/null || echo 0)
    echo "HOSTREF_XID_DELTA $((xid - xb))" >> "$f"
    if [ "$ok" = 1 ]; then state=green; else state=RED; fi
    if [ "$when" = before ]; then HOSTREF_BEFORE=$state; else HOSTREF_AFTER=$state; fi
    note "hostref $when $state cup8=$cu xid_count=$xid xid_delta=$((xid - xb))"
}

# ------------------------------------------------------------------------------------------------
# verdict and teardown
# ------------------------------------------------------------------------------------------------
emit_row() { # a SPEC_ROW line from lane.py
    echo "$1" >> "$ROWS"
    say "$1"
    case "$1" in *" result=FAIL "*) say "$(sed -n 's/^SPEC_ROW name=\([a-z_]*\)-\([0-9]*\) .*/\1_\2/p' <<< "$1") --- FAILED" ;; esac
}
synthetic_row() { # VIEWSET RESULT REASON
    emit_row "SPEC_ROW name=$1 result=$2 subtests=0/0 reason=$3 failed=-"
}
bar1_refusal() {
    local q
    [ "$KF3_SSH_SEEN" = 0 ] || return 1
    q=$VM/logs/${FIRST_RUN_TAG}_b1_qemu.log
    [ -n "$FIRST_RUN_TAG" ] && [ -s "$q" ] || return 1
    grep -a 'realize refused' "$q" | sed 's/.*realize refused//' | grep -qi 'bar1'
}
delete_copy() {
    local gone=""
    if [ -e "$ZST" ] || [ -e "$ZST.part" ]; then rm -f -- "$ZST" "$ZST.part"; gone=archive; fi
    if [ -e "$VM/disk/$OVERLAY.qcow2" ]; then rm -f -- "$VM/disk/$OVERLAY.qcow2"; gone+="${gone:+,}overlay"; fi
    say "SPEC_COPY_DELETED removed=${gone:-nothing}"
}
collect_host_logs() {
    local f
    for f in "$VM/logs/${TAG}"_*_run.log "$VM/logs/${TAG}"_*_host_dmesg.txt "$VM/logs/${TAG}"_*_cmdline.txt; do
        [ -f "$f" ] && cp "$f" "$EVD/" 2> /dev/null
    done
    for f in "$VM/logs/${TAG}"_*_qemu.log; do
        [ -f "$f" ] && gzip -c "$f" > "$EVD/$(basename "$f").gz"
    done
    rm -f "$EVD/cup8_host"
}
conclude() {
    local xid_delta bar1=0 kind cls pass fail nrun
    stage conclude
    if [ -n "$FETCH_PID" ] && kill -0 "$FETCH_PID" 2> /dev/null; then
        kill "$FETCH_PID" 2> /dev/null
        pkill -f -- "fetch_bench_data.sh $OBJECT --sha256" 2> /dev/null
    fi
    teardown_guest
    if [ "$GUEST_STARTED" = 1 ] && [ "$HOSTREF_BEFORE" = green ] && [ "$HOSTREF_AFTER" = unrun ]; then hostref after; fi
    xid_delta=$(sed -n 's/^HOSTREF_XID_DELTA //p' "$EVD/hostref_after.txt" 2> /dev/null)
    bar1_refusal && bar1=1
    collect_host_logs
    delete_copy
    for n in "${LANE_IDS[@]}"; do
        if ! grep -q "^SPEC_ROW name=$n " "$ROWS"; then
            if [ -n "$ABORT" ]; then synthetic_row "$n" NOT_RUN SETUP; else synthetic_row "$n" NOT_RUN TIME; fi
        fi
    done
    read -r kind cls < <(lane_py decide "$LANE" --rows "$ROWS" --before "$HOSTREF_BEFORE" --after "$HOSTREF_AFTER" --bar1 "$bar1" --xid "${xid_delta:-0}" ${ABORT:+--abort "$ABORT"})
    pass=$(grep -c ' result=PASS ' "$ROWS")
    fail=$(grep -c ' result=FAIL ' "$ROWS")
    nrun=$(grep -c ' result=NOT_RUN ' "$ROWS")
    say "SPEC_STAGE_COUNTS pass=$pass fail=$fail not_run=$nrun of=${#LANE_IDS[@]}"
    (cd "$EVD" && find . -path ./work -prune -o -type f ! -name MANIFEST.sha256 -print0 | sort -z | xargs -0 -r sha256sum > "$WORK/MANIFEST.sha256" && mv "$WORK/MANIFEST.sha256" MANIFEST.sha256)
    case "$kind" in
        PASS) VERDICT=PASS; rc_final=0; say "SPECVIEWPERF_STAGE_PASS" ;;
        FAIL) VERDICT=FAIL; rc_final=1; say "SPECVIEWPERF_STAGE_FAIL" ;;
        *) VERDICT=RETRY; rc_final=3; say "SPECVIEWPERF_STAGE_RETRY class=${cls:-OPERATOR}" ;;
    esac
    exit "$rc_final"
}
abort() { # CLASS WHY...  a setup step failed: nothing that ran can be a verdict on the guest
    ABORT=$1
    shift
    note "abort class=$ABORT stage=$STAGE: $*"
    conclude
}
# shellcheck disable=SC2317  # reached through the EXIT trap
on_exit() {
    local rc=$?
    if [ -z "$VERDICT" ]; then
        VERDICT=RETRY
        trap - EXIT
        note "the stage script ended without a verdict (rc=$rc, stage=$STAGE)"
        if [ "$GUEST_STARTED" = 1 ]; then teardown_guest; fi
        delete_copy
        echo "SPECVIEWPERF_STAGE_RETRY class=OPERATOR"
        exit 3
    fi
}
trap on_exit EXIT
trap 'exit 143' TERM INT HUP

# ------------------------------------------------------------------------------------------------
# preflight
# ------------------------------------------------------------------------------------------------
stage preflight
say "SPEC_STAGE_LIST lane=$LANE n=${#LANE_IDS[@]}"
note "rev=$REV tag=$T lane=$LANE release=$RELEASE deadline_left_s=$(left)"
[ "$(id -u)" = 0 ] || abort OPERATOR "not root"
[ -c "${KF_SPEC_KVM_DEV:-/dev/kvm}" ] || abort PROVIDER "no /dev/kvm"
[ -e "$VM/installed" ] || abort OPERATOR "windows is not installed ($VM)"
grep -q '^WINDOWS_GATE_PASS ' "$PROV/${T}_windows.log" 2> /dev/null || abort OPERATOR "no WINDOWS_GATE_PASS in $PROV/${T}_windows.log"
[ -f "$VM/disk/$GATE_TAG.qcow2" ] || abort OPERATOR "no gate overlay disk/$GATE_TAG.qcow2"
[ -r "$FLAGS" ] || abort OPERATOR "no flags file $FLAGS"
[ -x "$WINVM_QEMU" ] || abort OPERATOR "no kf3 qemu $WINVM_QEMU"
[ -s "$VM/identity.env" ] || abort OPERATOR "no VM identity"
VMUUID=$(sed -n 's/^VM_UUID=//p' "$VM/identity.env" 2> /dev/null | head -1)
[[ "$VMUUID" =~ ^[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$ ]] || abort OPERATOR "no per-VM identity (VM_UUID)"
if pgrep -x qemu-system-x86 > /dev/null || run_loops_alive; then abort OPERATOR "a qemu or run loop is already running"; fi
need_gb=$NEED_GB
avail_gb=$(df -BG --output=avail "$STORE" 2> /dev/null | tail -1 | tr -dc 0-9)
[ "${avail_gb:-0}" -ge "$need_gb" ] || abort PROVIDER "disk ${avail_gb:-0}G free, need ${need_gb}G (the disk request may have been ignored)"
mem_gb=$(free -g | awk '/^Mem:/{print $2}')
[ "${mem_gb:-0}" -ge 24 ] || abort PROVIDER "ram ${mem_gb:-0}G, need 24G for a 16G guest"
if ! nvidia-smi -L > "$EVD/nvidia_smi_L.txt" 2>&1; then abort PROVIDER "nvidia-smi -L failed on the host"; fi
GPU_NAME=$(nvidia-smi --query-gpu=name --format=csv,noheader | head -1)
note "host: gpu=\"$GPU_NAME\" cpus=$(nproc) ram_gb=$mem_gb disk_free_gb=$avail_gb need_gb=$need_gb"
PIN=$(sed -n 's/^specviewperf-15\.0\.0\.tar\.zst sha256=\([0-9a-f]\{64\}\)$/\1/p' "$PINFILE" 2> /dev/null | head -1)
[ -n "$PIN" ] || abort MEDIA "no sha256 pin for $OBJECT in $PINFILE (the owner pins it)"
[ -f "$FETCH" ] || abort OPERATOR "no fetch helper"

# ------------------------------------------------------------------------------------------------
# the copy: download in the background (the Worker's /bench-data route, this run's token only)
# ------------------------------------------------------------------------------------------------
stage fetch
FETCH_TMO=$(cap 3600)
(
    timeout "$FETCH_TMO" bash "$FETCH" "$OBJECT" --sha256 "$PIN" --out "$ZST" > "$WORK/fetch.txt" 2>&1
    echo $? > "$WORK/fetch.rc"
) &
FETCH_PID=$!

stage hostref_before
LIBCUDA=-lcuda
[ -e /usr/lib/x86_64-linux-gnu/libcuda.so ] || LIBCUDA=/usr/lib/x86_64-linux-gnu/libcuda.so.1
gcc -O0 -I "$SOURCE/scripts/bench/cuda_min" -o "$EVD/cup8_host" "$SOURCE/scripts/bench/cup8.c" $LIBCUDA -lm 2> "$EVD/cup8_host_build.txt" \
    || note "the host cup8 reference did not build: $(tail -2 "$EVD/cup8_host_build.txt" | tr '\n' ' ')"
hostref before
[ "$HOSTREF_BEFORE" = green ] || abort HOSTREF "the host GPU reference is not green BEFORE the guest (hostref_before.txt)"

# ------------------------------------------------------------------------------------------------
# boot A (no kf3): the tree goes in
# ------------------------------------------------------------------------------------------------
stage overlay
# the gate's overlay is the parent of ours and must not change again; its evidence has been taken
chmod 0400 "$VM/disk/$GATE_TAG.qcow2"
if [ -e "$VM/disk/$OVERLAY.qcow2" ]; then abort OPERATOR "the overlay $OVERLAY exists already"; fi
qemu-img create -q -f qcow2 -F qcow2 -b "$VM/disk/$GATE_TAG.qcow2" "$VM/disk/$OVERLAY.qcow2" > "$WORK/overlay.txt" 2>&1 \
    || abort OPERATOR "qemu-img could not create the child overlay: $(tail -1 "$WORK/overlay.txt")"

stage boot_copy
boot_guest copy
brc=$?
[ "$brc" = 0 ] || abort TRANSPORT "the copy boot (no kf3) did not come up (rc=$brc)"
g 60 'New-Item -ItemType Directory -Force -Path C:\kf\spec | Out-Null' > /dev/null 2>&1
for f in spec_guest.ps1 spec_run.ps1; do
    timeout 300 bash "$WV" scp-to "$WIN/spec/$f" 'C:/kf/spec/' < /dev/null > "$WORK/scp_$f.txt" 2>&1 || abort TRANSPORT "scp of $f to the guest failed"
done
timeout 300 bash "$WV" scp-to "$WIN/gate_state.ps1" 'C:/kf/gate_state.ps1' < /dev/null > "$WORK/scp_gate_state.txt" 2>&1 || abort TRANSPORT "scp of gate_state.ps1 failed"
gspec 120 prep > "$WORK/prep.txt"
grep -q '^SPEC_PREP ' "$WORK/prep.txt" || abort TRANSPORT "spec_guest.ps1 -Action prep gave no answer"
note "guest prep: $(grep '^SPEC_PREP ' "$WORK/prep.txt")"

stage fetch_wait
wait "$FETCH_PID"
FETCH_PID=""
frc=$(cat "$WORK/fetch.rc" 2> /dev/null || echo 1)
note "fetch rc=$frc: $(tail -1 "$WORK/fetch.txt" 2> /dev/null | head -c 160)"
case "$frc" in
    0) ;;
    3 | 4) abort MEDIA "the private copy is missing or does not match its pin (fetch rc=$frc)" ;;
    124) abort TIME "the download did not finish inside its budget" ;;
    2) abort OPERATOR "fetch_bench_data.sh refused its arguments or environment" ;;
    *) abort PROVIDER "the download failed (fetch rc=$frc)" ;;
esac
[ -f "$ZST" ] || abort MEDIA "no archive after a successful fetch"
note "archive sha256 verified against the pin ($(stat -c %s "$ZST") bytes)"

stage copy
set -o pipefail
zstd -d -c --long=31 "$ZST" 2> "$WORK/zstd.err" \
    | python3 -I "$SPECD/tarfilter.py" --lane "$LANE" --manifest "$WORK/manifest.txt" 2> "$WORK/tarfilter.err" \
    | timeout "$(cap 4200)" bash "$WV" ssh 'cmd /c "tar -xf - -C C:/"' > "$WORK/copy_tar.txt" 2>&1
cps=("${PIPESTATUS[@]}")
note "copy: zstd=${cps[0]} tarfilter=${cps[1]} ssh_tar=${cps[2]} $(tr '\n' ' ' < "$WORK/tarfilter.err" | head -c 120)"
[ "${cps[1]}" != 3 ] || abort MEDIA "the archive holds a member this tree never holds (tarfilter refused it)"
[ "${cps[2]}" != 124 ] || abort TIME "the copy into the guest did not finish inside its budget"
if [ "${cps[0]}" != 0 ] || [ "${cps[1]}" != 0 ] || [ "${cps[2]}" != 0 ]; then
    abort TRANSPORT "the copy into the guest failed (zstd=${cps[0]} tarfilter=${cps[1]} ssh_tar=${cps[2]})"
fi
gspec 600 verify > "$WORK/guest_view.txt"
grep -q '^SPEC_VERIFY_DONE' "$WORK/guest_view.txt" || abort TRANSPORT "the guest did not list the copied viewsets"
nview=0
while read -r l; do
    case "$l" in VIEWSET\ *) ;; *) continue ;; esac
    nview=$((nview + 1))
    grep -qxF "$l" "$WORK/guest_view.txt" || abort TRANSPORT "the guest holds a different tree than the one streamed ($l)"
done < "$WORK/manifest.txt"
[ "$nview" -ge "${#LANE_IDS[@]}" ] || abort MEDIA "the archive holds $nview viewset directories of this lane, expected ${#LANE_IDS[@]}"
note "copy verified: $nview viewset directories match the manifest"
gspec 300 post > "$WORK/post.txt"
grep -q '^SPEC_POST cli=present' "$WORK/post.txt" || abort MEDIA "no SPECviewperf-CLI.exe in the copied tree"
note "guest post: $(tr '\n' ' ' < "$WORK/post.txt" | head -c 160)"
rm -f -- "$ZST"
say "SPEC_COPY_DELETED removed=archive"
teardown_guest

# ------------------------------------------------------------------------------------------------
# boot B (kf3): the lane's viewsets
# ------------------------------------------------------------------------------------------------
GUEST_DEAD_ROW=0
boot_run() { # 0 when a kf3 guest is up, at the desktop and settled
    local rc
    boot_guest run
    rc=$?
    [ "$rc" = 0 ] || { note "kf3 boot rc=$rc"; return 1; }
    wait_desktop || { note "no signed-in desktop"; return 1; }
    settle
    return 0
}
FRESH=0   # 1 after a viewset that reset, hung or killed the guest: the next viewset gets a fresh boot (kf3 has no reset path)
ensure_guest() { # 0 ready; 1 no time or boots left (NOT_RUN); 2 the boot failed (a guest failure)
    if [ "$FRESH" = 0 ] && qemu_alive && guest_answers; then return 0; fi
    FRESH=0
    teardown_guest
    [ "$RUN_BOOTS" -lt "$MAX_BOOTS" ] || { note "max_guest_boots reached"; return 1; }
    [ "$(left)" -gt $((TAIL_S + SETTLE_S + 600)) ] || { note "no time for another boot"; return 1; }
    boot_run && return 0
    # a boot that failed because the deadline came is the time's, not the guest's
    [ "$(left)" -gt "$TAIL_S" ] || return 1
    return 2
}
poll_viewset() { # W SECONDS -> POLL=ended|expired|dead|silent
    local w=$1 secs=$2 t0 misses=0 out
    t0=$(date +%s)
    POLL=expired
    while :; do
        qemu_alive || { POLL=dead; return; }
        out=$(gspec 100 status -Workload "$w")
        if grep -q 'SPEC_STATE end=1' <<< "$out"; then POLL=ended; return; fi
        if grep -q 'SPEC_STATE end=0' <<< "$out"; then misses=0; else misses=$((misses + 1)); fi
        [ "$misses" -lt 4 ] || { POLL=silent; return; }
        [ $(($(date +%s) - t0)) -lt "$secs" ] || { POLL=expired; return; }
        sleep 20
    done
}
collect_viewset() { # W -> $EVD/<W>/{cli.log,END.txt,tdr.txt} (names fixed here: the guest output decides nothing else)
    local w=$1 d=$WORK/$1
    mkdir -p "$d"
    : > "$d/cli.log"
    : > "$d/END.txt"
    : > "$d/tdr.txt"
    gspec 200 collect -Workload "$w" > "$d/collect.txt"
    awk -v dir="$d" '
        /^SPEC_COLLECT_BEGIN (cli\.log|END\.txt|tdr\.txt)$/ { f = dir "/" $2; next }
        /^SPEC_COLLECT_END / { f = ""; next }
        f != "" { print > f }' "$d/collect.txt"
}
run_viewset() { # W SECONDS MODE(full|cut) -> GRADE_LINE, GRADE_RC
    local w=$1 secs=$2 mode=$3 d=$WORK/$1 abn="" out
    mkdir -p "$d"
    out=$(gspec 120 launch -Workload "$w")
    if grep -q '^SPEC_LAUNCH ok ' <<< "$out"; then
        poll_viewset "$w" "$secs"
    elif grep -q '^SPEC_LAUNCH fail ' <<< "$out"; then
        # our own guest script says it could not start the run: the harness, not the guest under test
        note "launch of $w failed: $(head -c 160 <<< "$out")"
        POLL=launch
    else
        note "launch of $w got no answer: the guest does not answer"
        POLL=silent
    fi
    case "$POLL" in
        ended) ;;
        launch) abn=launch ;;
        expired)
            gspec 120 kill > /dev/null
            if [ "$mode" = cut ]; then abn="cut"; else abn="full"; fi ;;
        dead) abn=dead ;;
        silent) abn=silent ;;
    esac
    if [ "$POLL" = dead ]; then
        : > "$d/cli.log"; : > "$d/END.txt"; : > "$d/tdr.txt"
    else
        collect_viewset "$w"
    fi
    # the status-only evidence (--scrub-out): nothing that is not a status line is kept
    GRADE_LINE=$(lane_py grade "$w" --cli "$d/cli.log" --end "$d/END.txt" --tdr "$d/tdr.txt" --scrub-out "$EVD/$w.cli.txt" ${abn:+--timeout "$abn"})
    grep -a '^SPECRUN_END ' "$d/END.txt" > "$EVD/$w.end.txt" 2> /dev/null
    grep -a '^GATE_TDR \|^GATE_LIVEDUMP ' "$d/tdr.txt" > "$EVD/$w.tdr.txt" 2> /dev/null
}

stage boot_run
FAILED_BOOT=0
if ! boot_run; then
    FAILED_BOOT=1
    if [ "$(left)" -le "$TAIL_S" ]; then FAILED_BOOT=2; fi   # the deadline came during the boot: time, not the guest
fi
for w in "${LANE_IDS[@]}"; do
    stage "viewset_$w"
    if [ "$FAILED_BOOT" = 2 ]; then
        synthetic_row "$w" NOT_RUN TIME
        continue
    fi
    if [ "$FAILED_BOOT" = 1 ]; then
        # the gate booted this image on kf3 minutes ago: a boot that fails now is the guest's failure (decide() attributes it)
        if [ "$GUEST_DEAD_ROW" = 0 ]; then synthetic_row "$w" FAIL GUEST; GUEST_DEAD_ROW=1; else synthetic_row "$w" NOT_RUN BOOT; fi
        continue
    fi
    for attempt in 1 2; do
        # the time guard twice: before a boot that may not be worth starting, and after it (a boot and the settle wait
        # take minutes, so the viewset's budget is computed from the time that is left NOW)
        read -r mode secs < <(lane_py timeout "$w" "$(left)")
        if [ "$mode" = skip ]; then
            note "time guard: $w not started ($(left)s left)"
            synthetic_row "$w" NOT_RUN TIME
            break
        fi
        ensure_guest
        erc=$?
        if [ "$erc" = 1 ]; then synthetic_row "$w" NOT_RUN TIME; break; fi
        if [ "$erc" = 2 ]; then
            if [ "$GUEST_DEAD_ROW" = 0 ]; then synthetic_row "$w" FAIL GUEST; GUEST_DEAD_ROW=1; else synthetic_row "$w" NOT_RUN BOOT; fi
            FAILED_BOOT=1
            break
        fi
        read -r mode secs < <(lane_py timeout "$w" "$(left)")
        if [ "$mode" = skip ]; then
            note "time guard: $w not started after the boot ($(left)s left)"
            synthetic_row "$w" NOT_RUN TIME
            break
        fi
        note "run $w (budget ${secs}s, mode $mode, left $(left)s)"
        run_viewset "$w" "$secs" "$mode"
        if [ "$attempt" = 1 ] && [[ "$GRADE_LINE" == *"reason=GATHER"* ]]; then
            # the harness trap of the first run after a boot: wait, then once more; a second GATHER stays NOT_RUN (RETRY)
            note "$w: Unable to gather system details; waiting ${GATHER_WAIT_S}s and trying once more"
            sleep "$(cap "$GATHER_WAIT_S")"
            continue
        fi
        emit_row "$GRADE_LINE"
        # a guest that reset, hung or was killed is not evidence for the next viewset: the next one gets a fresh boot
        case "$GRADE_LINE" in *"reason=RESET"* | *"reason=TIMEOUT"* | *"reason=GUEST"*) FRESH=1 ;; esac
        break
    done
done

# the guest stops, then the host reference AFTER (as the gate: the GPU is the host's again), then the verdict
conclude
