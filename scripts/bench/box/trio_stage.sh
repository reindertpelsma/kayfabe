#!/bin/bash
# trio_stage.sh <tag> <gate-tag> [<prov-dir>] -- two guests on one GPU, then the trio (host CUDA + guest 1 +
# guest 2), on a box where windows_gate.sh <gate-tag> already ran (its overlay holds a Windows 11 guest with the
# 580.88 driver, GSP forced, a signed-in desktop) and merge_check.sh built the kf3 QEMU and the fast guest.
#
# STATUS: DRAFT, 2026-11-05 (lane trio, traces/trio_vast_20261105/). Written for the first measurement; what
# of it ran end to end, and on which box, is in that directory's README. Not wired into any CI stage.
#
# Phases (log: <prov>/<tag>_trio.log; evidence: <prov>/<tag>_trio/, text only, GPU UUIDs scrubbed):
#   host_solo   the host CUDA known-answer loop alone (KF_TRIO_HOST_SOLO_S, default 120 s): the ratio baseline
#   win_boot    Windows from the gate overlay (--once: a guest reset ends the run, it is never restarted),
#               its own gpu-uuid (GPU-<VM_UUID>), signed-in desktop, nvidia-smi -L
#   win_solo    KF_TRIO_WIN_SOLO_ROUNDS rounds of cup8 + d3d11 in the guest: the ratio baseline
#   A           Windows idle at its desktop + the Linux fast suite (KF_TRIO_ARMS, default all 30)
#   B           the trio: host loop + a Windows probe loop + the Linux fast suite, all at once
#   end         Windows reset counters since boot, adapter state, graceful stop, host Xid delta, verdicts
# Verdict lines: TRIO_PHASE_A PASS|FAIL ..., TRIO_PHASE_B PASS|FAIL|NOTRUN ..., TRIO_FOOTPRINT ..., TRIO_TIMING ...,
# and last TRIO_EXIT rc=N (0 only when A and B both PASS). A missing TRIO_EXIT is a dead run.
# What gates a phase: every result correct (host cup8/cup8bench verdict PASS, Windows cup8 byte exact and d3d11
# readback OK, Linux FAST_SUITE_PASS=N of N), forward progress (each workload completed at least one unit in every
# KF_TRIO_PROGRESS_S window while it ran), Windows alive (QEMU up, ssh answers, no GUEST_PANICKED), no Windows reset
# counter > 0, and no host Xid naming the Windows QEMU or the host loop. Throughput ratios are recorded, never gated.
#
# Harness settings that differ from a solo run (none is a product change): the two VMs take DIFFERENT GPU lock files
# (KF_GPU_LOCK for win_vm.sh, KF_LOCK for run_fast_guest.sh; the shared default lock exists to keep GPU work serial,
# which is exactly what this stage does not want), and KF_GPU_IDLE_MIB is raised so run_fast_guest.sh does not wait
# 30 s per arm for a GPU that the Windows guest keeps busy by design.
# Knobs: KF_TRIO_SOURCE (the checkout; default: this script's), KF_TRIO_QEMU, KF_TRIO_ARMS (space-separated --arms),
# KF_TRIO_HOST_SOLO_S (120), KF_TRIO_WIN_SOLO_ROUNDS (3), KF_TRIO_HOST_FRAC (0.5 of the smallest free VRAM seen in A),
# KF_TRIO_HOST_NMAX (16384), KF_TRIO_PROGRESS_S (600), KF_TRIO_BUDGET (180, per fast arm), KF_TRIO_SKIP_B (0).
set -uo pipefail
SELFTEST=0
if [ "${1:-}" = --selftest ]; then SELFTEST=1; set -- selftest selftest "$(mktemp -d)"; fi
T=${1:?usage: trio_stage.sh <tag> <gate-tag> [<prov-dir>]}
GT=${2:?usage: trio_stage.sh <tag> <gate-tag> [<prov-dir>]}
for x in "$T" "$GT"; do [[ "$x" =~ ^[a-zA-Z0-9][a-zA-Z0-9._-]*$ ]] || { echo "TRIO_REFUSED why=invalid-tag"; exit 2; }; done
PROV=${3:-${KF_PROV_DIR:-/root/prov}}
SOURCE=${KF_TRIO_SOURCE:-$(cd "$(dirname "$(readlink -f "${BASH_SOURCE[0]}")")/../../.." && pwd)}
WIN=$SOURCE/scripts/bench/windows
WV=$WIN/win_vm.sh
LOG=$PROV/${T}_trio.log
EV=$PROV/${T}_trio
export BENCH=${BENCH:-/workspace/bench}
export BENCH_DIR=${BENCH_DIR:-$BENCH}
export WINVM_ROOT=${WINVM_ROOT:-/workspace/winvm} KF_GUEST_USER=${KF_GUEST_USER:-kf}
export WINVM_SSH_CONNECT_TIMEOUT=${WINVM_SSH_CONNECT_TIMEOUT:-100} WINVM_VM_IDENTITY=${WINVM_VM_IDENTITY:-1}
export KF_DEVICE=kf3
VM=$WINVM_ROOT/${WINVM_NAME:-kfwin}
OVERLAY=gate_$(printf %s "$GT" | tr -c 'A-Za-z0-9_' '_' | cut -c1-24)
WTAG=trio_$(printf %s "$T" | tr -c 'A-Za-z0-9_' '_' | cut -c1-24)
REV=$(git -C "$SOURCE" rev-parse --short=8 HEAD 2> /dev/null || echo norev)
QEMU=${KF_TRIO_QEMU:-$BENCH/kf3-bins/$REV/qemu-system-x86_64}
FLAGS=${KF_WINGATE_FLAGS:-$WIN/kf3_flags.sample.env}
GDRV=${KF_WINGATE_GUEST_DRIVER:-580.65.06}
ARMS_STR=${KF_TRIO_ARMS:-}
HOST_SOLO_S=${KF_TRIO_HOST_SOLO_S:-120}
WIN_SOLO_ROUNDS=${KF_TRIO_WIN_SOLO_ROUNDS:-3}
HOST_FRAC=${KF_TRIO_HOST_FRAC:-0.5}
HOST_NMAX=${KF_TRIO_HOST_NMAX:-16384}
PROGRESS_S=${KF_TRIO_PROGRESS_S:-600}
FAST_BUDGET=${KF_TRIO_BUDGET:-180}
T0=$(date +%s)
TIMING=()
STAGE=start
VA=NOTRUN VB=NOTRUN WHY_A="" WHY_B=""
WIN_UP=0
BOOT_UTC=""

say() { echo "$*" | tee -a "$LOG"; }
stage_begin() { STAGE=$1; STAGE_T=$(date +%s); say "TRIO_STAGE $STAGE START $(date -u -Is)"; }
stage_end() { local s=$(($(date +%s) - STAGE_T)); TIMING+=("$STAGE=${s}s"); say "TRIO_STAGE $STAGE EXIT ${s}s"; }

# --------------------------------------------------------------------------------------------------
# the Windows guest (the same helpers as windows_gate.sh, with this stage's own tag and lock)
# --------------------------------------------------------------------------------------------------
g() { local t=$1; shift; timeout "$t" bash "$WV" ssh "$@" < /dev/null; }
gps() { local t=$1 f=$2; shift 2; g "$t" "powershell -NoProfile -ExecutionPolicy Bypass -File $f $*" | tr -d '\r'; }
qemu_alive() {
    local pid
    pid=$(cat "$VM/run/qemu.pid" 2> /dev/null || true)
    [ -n "$pid" ] && [ -d "/proc/$pid" ] && [ "$(awk '{print $3}' "/proc/$pid/stat" 2> /dev/null)" != Z ]
}
win_panicked() { grep -q GUEST_PANICKED "$VM"/logs/${WTAG}_b*_qmp_events.jsonl 2> /dev/null; }
win_ok() { qemu_alive && ! win_panicked; }
wait_ssh() {
    local t=$1 w=0
    while [ "$w" -lt "$t" ]; do
        qemu_alive || return 1
        g 112 'echo KF_SSH_UP' 2> /dev/null | grep -q KF_SSH_UP && return 0
        sleep 10
        w=$((w + 10))
    done
    return 1
}
win_probe() { # NAME EXE ARG OUT -> 0 when the result is correct
    local name=$1 exe=$2 arg=$3 out=$4 args="-Exe C:\\kf\\probes\\$2 -TimeoutS 180"
    [ -z "$arg" ] || args+=" -Arg $arg"
    gps 240 'C:\kf\gate_run.ps1' "$args" > "$out" 2>&1
    grep -aq '^GATE_PROBE .* rc=0 ' "$out" || return 1
    case "$name" in
        cup8) grep -aq 'CUP8 VERDICT: PASS' "$out" && grep -aq 'bad=0 maxerr=0' "$out" ;;
        d3d11) grep -aq 'KFDX RESULT OK' "$out" ;;
        *) return 1 ;;
    esac
}
win_round() { # PHASE I -> one TRIO_WIN_ITER line per probe; returns non-zero if a probe was wrong
    local ph=$1 i=$2 p rc=0 t s ok
    for p in cup8:cup8.exe: d3d11:kf_dxprobe.exe:d3d11; do
        IFS=: read -r name exe arg <<< "$p"
        t=$(date +%s)
        if win_probe "$name" "$exe" "$arg" "$EV/win_${ph}_${name}.cur"; then ok=1; else ok=0; rc=1; cp "$EV/win_${ph}_${name}.cur" "$EV/win_${ph}_${name}_fail_$i.txt"; fi
        s=$(($(date +%s) - t))
        say "TRIO_WIN_ITER phase=$ph i=$i probe=$name ok=$ok secs=$s guest_secs=$(sed -n 's/^GATE_PROBE .* secs=\([0-9]*\) .*/\1/p' "$EV/win_${ph}_${name}.cur" | tail -1) utc=$(date -u -Is)"
    done
    return $rc
}
win_stop() {
    mkdir -p "$VM/run" && touch "$VM/run/stop-requested"
    qemu_alive && timeout 420 bash "$WV" stop >> "$EV/win_stop.txt" 2>&1 < /dev/null
    WIN_UP=0
}

# --------------------------------------------------------------------------------------------------
# the host CUDA known-answer loop (cup8: one NxN fp32 matmul, closed-form answer, byte exact; cup8bench: poisoned
# C before every launch, every iteration verified), sized from free VRAM, until a stop file appears
# --------------------------------------------------------------------------------------------------
host_loop() { # PHASE N STOPFILE -> TRIO_HOST_ITER lines in $EV/host_<phase>.log
    local ph=$1 n=$2 stop=$3 i=0 t s rc ok
    while [ ! -e "$stop" ]; do
        t=$(date +%s.%N)
        CUP8_N=$n timeout 900 "$EV/bin/cup8" > "$EV/host_${ph}.cur" 2>&1
        rc=$?
        ok=0
        [ "$rc" = 0 ] && grep -aq 'CUP8 VERDICT: PASS' "$EV/host_${ph}.cur" && ok=1
        [ "$ok" = 1 ] || cp "$EV/host_${ph}.cur" "$EV/host_${ph}_cup8_fail_$i.txt"
        s=$(awk -v a="$t" -v b="$(date +%s.%N)" 'BEGIN{printf "%.2f", b-a}')
        echo "TRIO_HOST_ITER phase=$ph i=$i tool=cup8 n=$n rc=$rc ok=$ok secs=$s utc=$(date -u -Is)" >> "$EV/host_$ph.log"
        [ ! -e "$stop" ] || break
        t=$(date +%s.%N)
        BENCH_SIZES=1024,2048 BENCH_ITERS=10 BENCH_BATCH=10 timeout 900 "$EV/bin/cup8bench" > "$EV/host_${ph}.cur" 2>&1
        rc=$?
        ok=0
        [ "$rc" = 0 ] && grep -aq 'BENCH_VERDICT: PASS' "$EV/host_${ph}.cur" && ok=1
        [ "$ok" = 1 ] || cp "$EV/host_${ph}.cur" "$EV/host_${ph}_bench_fail_$i.txt"
        s=$(awk -v a="$t" -v b="$(date +%s.%N)" 'BEGIN{printf "%.2f", b-a}')
        echo "TRIO_HOST_ITER phase=$ph i=$i tool=cup8bench sizes=1024,2048 rc=$rc ok=$ok secs=$s utc=$(date -u -Is)" >> "$EV/host_$ph.log"
        i=$((i + 1))
    done
    rm -f "$EV/host_${ph}.cur"
}
win_loop() { # PHASE STOPFILE
    local ph=$1 stop=$2 i=0
    while [ ! -e "$stop" ]; do
        win_ok || { say "TRIO_NOTE the Windows guest is gone during the probe loop of phase $ph"; break; }
        win_round "$ph" "$i"
        i=$((i + 1))
    done
}
heartbeat() { # PHASE STOPFILE -> TRIO_WIN_BEAT lines (QEMU alive, no panic, ssh answers) every 60 s
    local ph=$1 stop=$2 ssh
    while [ ! -e "$stop" ]; do
        ssh=0
        g 100 'echo KF_SSH_UP' 2> /dev/null | grep -q KF_SSH_UP && ssh=1
        echo "TRIO_WIN_BEAT phase=$ph qemu=$(qemu_alive && echo 1 || echo 0) panicked=$(win_panicked && echo 1 || echo 0) ssh=$ssh utc=$(date -u -Is)" >> "$EV/beat_$ph.log"
        for _ in $(seq 1 12); do [ -e "$stop" ] && break; sleep 5; done
    done
}

# --------------------------------------------------------------------------------------------------
# footprints: VRAM and BAR1 (nvidia-smi), compute processes, host RAM (MemAvailable) and the PSS of each QEMU
# --------------------------------------------------------------------------------------------------
pss_kib() { # PID... -> summed Pss of these processes and their children (KiB)
    local p sum=0 v
    for p in "$@"; do
        for c in $p $(pgrep -P "$p" 2> /dev/null); do
            v=$(awk '/^Pss:/{print $2}' "/proc/$c/smaps_rollup" 2> /dev/null)
            sum=$((sum + ${v:-0}))
        done
    done
    echo "$sum"
}
snapshot() { # LABEL -> one TRIO_SNAP line
    local label=$1 used total b1u b1t avail wp fp
    read -r used total < <(nvidia-smi --query-gpu=memory.used,memory.total --format=csv,noheader,nounits | head -1 | tr -d ',')
    b1t=$(nvidia-smi -q -d MEMORY 2> /dev/null | awk '/BAR1 Memory Usage/{f=1} f&&/Total/{print $3; exit}')
    b1u=$(nvidia-smi -q -d MEMORY 2> /dev/null | awk '/BAR1 Memory Usage/{f=1} f&&/Used/{print $3; exit}')
    avail=$(awk '/^MemAvailable:/{print int($2/1024)}' /proc/meminfo)
    wp=$(cat "$VM/run/qemu.pid" 2> /dev/null || true)
    [ -n "$wp" ] && [ -d "/proc/$wp" ] || wp=""
    fp=$(pgrep -f '[k]f-fastguest' | tr '\n' ' ')
    echo "TRIO_SNAP label=$label utc=$(date -u -Is) vram_used_mib=$used vram_total_mib=$total bar1_used_mib=${b1u:-?} bar1_total_mib=${b1t:-?} mem_avail_mib=$avail win_pss_mib=$(($(pss_kib $wp) / 1024)) fast_pss_mib=$(($(pss_kib $fp) / 1024)) compute_apps=\"$(nvidia-smi --query-compute-apps=pid,used_memory --format=csv,noheader,nounits 2> /dev/null | tr -d ' ' | tr '\n' ';')\"" >> "$EV/snap.log"
}
sampler() { # STOPFILE LABEL -> a snapshot every 15 s
    while [ ! -e "$1" ]; do snapshot "$2"; sleep 15; done
}

xid_count() { dmesg 2> /dev/null | grep -c 'NVRM: Xid' || true; }

# --------------------------------------------------------------------------------------------------
# the Linux fast suite with its own lock (KF_LOCK) and a raised drain threshold
# --------------------------------------------------------------------------------------------------
fast_suite() { # PHASE -> $BENCH/<tag>_<phase>_suite.out; returns the suite's rc
    local ph=$1 arms=()
    [ -z "$ARMS_STR" ] || read -r -a arms <<< "$ARMS_STR"
    KF_LOCK=/tmp/kf-trio-linux.lock KF_GPU_IDLE_MIB=999999 QEMU_BIN=$QEMU \
        bash "$SOURCE/scripts/fastguest/fast_suite.sh" "${T}_$ph" "$FAST_BUDGET" "${arms[@]}" > "$EV/fast_$ph.run" 2>&1
}
suite_line() { grep -a '^FAST_SUITE_PASS=' "$BENCH/${T}_$1_suite.out" 2> /dev/null | tail -1; }
suite_green() { # PHASE -> 0 when every arm passed and none was NOTRUN
    local l n
    l=$(suite_line "$1")
    n=$(sed -n 's/.*ARMS=\([0-9]*\).*/\1/p' <<< "$l")
    [ -n "$n" ] && [[ "$l" == "FAST_SUITE_PASS=$n FAST_SUITE_FAIL=0 FAST_SUITE_CRASH=0 NOTRUN=0 ARMS=$n" ]]
}

# forward progress: in every PROGRESS_S window between START and END, at least one line of FILE (a completion) fell
progress_ok() { # FILE START_EPOCH END_EPOCH -> 0 or 1 with the first empty window printed
    local f=$1 a=$2 b=$3 w
    [ -s "$f" ] || { echo "no completion at all"; return 1; }
    for ((w = a; w + PROGRESS_S <= b; w += PROGRESS_S)); do
        if ! sed -n 's/.* utc=\([^ ]*\).*/\1/p' "$f" | while read -r u; do date -d "$u" +%s; done \
            | awk -v lo="$w" -v hi="$((w + PROGRESS_S))" '$1>=lo && $1<hi {f=1} END{exit !f}'; then
            echo "no completion between +$((w - a))s and +$((w - a + PROGRESS_S))s"
            return 1
        fi
    done
    return 0
}

# the Xid lines that appeared after LINE_NO of dmesg, and which of them name a process this stage must not fault
xid_new() { dmesg 2> /dev/null | grep 'NVRM: Xid' | tail -n "+$(($1 + 1))"; }
xid_foreign() { # BEFORE_COUNT [PID] -> the new Xid lines that name PID (the Windows QEMU) or the host loop (name=cup8*)
    local b=$1 re='name=cup8'
    [ -z "${2:-}" ] || re+="|pid=${2}[, ]"
    xid_new "$b" | grep -E "$re" || true
}

# GPU UUIDs never leave the box: every UUID-shaped string in the evidence becomes a synthetic one, distinct per value
scrub_uuids() { # DIR
    local map="$1/.uuidmap" f
    : > "$map"
    grep -rhoaiE '[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}' "$1" 2> /dev/null | tr 'A-F' 'a-f' | sort -u \
        | awk '{printf "s/%s/fa4efa4e-fa4e-4fa4-8fa4-fa4efa4e%04x/Ig\n", $1, NR}' > "$map"
    if [ -s "$map" ]; then
        find "$1" -type f ! -name .uuidmap -print0 | xargs -0 -r sed -i -f "$map"
    fi
    rm -f "$map"
}

verdict_lines() {
    say "TRIO_PHASE_A $VA ${WHY_A:+why=\"$WHY_A\" }rev=$REV gpu=\"${GPU_NAME:-?}\""
    say "TRIO_PHASE_B $VB ${WHY_B:+why=\"$WHY_B\" }rev=$REV gpu=\"${GPU_NAME:-?}\""
}
on_exit() {
    local rc=1
    [ "$SELFTEST" = 1 ] && return
    touch "$EV/stop_all" 2> /dev/null
    [ "$WIN_UP" = 0 ] || win_stop
    [ -n "${VA_SAID:-}" ] || verdict_lines
    [ "$VA" = PASS ] && [ "$VB" = PASS ] && rc=0
    say "TRIO_TIMING total=$(($(date +%s) - T0))s ${TIMING[*]:-}"
    if [ -d "$EV" ]; then
        rm -rf "${EV:?}/bin" "${EV:?}/probes"
        scrub_uuids "$EV"
        tar -C "$PROV" -czf "$PROV/${T}_trio.tgz" "${T}_trio" 2> /dev/null
    fi
    say "TRIO_EXIT rc=$rc $(date -u -Is)"
    exit "$rc"
}

selftest() {
    local bad=0 d
    d=$(mktemp -d)
    BENCH=$d T=st
    printf 'FAST_SUITE_PASS=30 FAST_SUITE_FAIL=0 FAST_SUITE_CRASH=0 NOTRUN=0 ARMS=30\n' > "$d/st_A_suite.out"
    printf 'FAST_SUITE_PASS=29 FAST_SUITE_FAIL=1 FAST_SUITE_CRASH=0 NOTRUN=0 ARMS=30\n' > "$d/st_B_suite.out"
    printf 'FAST_SUITE_PASS=4 FAST_SUITE_FAIL=0 FAST_SUITE_CRASH=0 NOTRUN=0 ARMS=4\n' > "$d/st_C_suite.out"
    suite_green A || { bad=$((bad + 1)); echo "TRIO_SELFTEST_FAIL suite 30/30 not green"; }
    ! suite_green B || { bad=$((bad + 1)); echo "TRIO_SELFTEST_FAIL suite 29/30 green"; }
    suite_green C || { bad=$((bad + 1)); echo "TRIO_SELFTEST_FAIL subset 4/4 not green"; }
    ! suite_green missing || { bad=$((bad + 1)); echo "TRIO_SELFTEST_FAIL missing suite green"; }
    PROGRESS_S=60
    local a=1800000000
    printf 'X utc=%s\nX utc=%s\n' "$(date -u -d @$((a + 10)) -Is)" "$(date -u -d @$((a + 70)) -Is)" > "$d/p1"
    progress_ok "$d/p1" "$a" $((a + 120)) > /dev/null || { bad=$((bad + 1)); echo "TRIO_SELFTEST_FAIL progress with a unit per window"; }
    progress_ok "$d/p1" "$a" $((a + 180)) > /dev/null && { bad=$((bad + 1)); echo "TRIO_SELFTEST_FAIL progress with an empty window"; }
    progress_ok "$d/none" "$a" $((a + 60)) > /dev/null && { bad=$((bad + 1)); echo "TRIO_SELFTEST_FAIL progress with no file"; }
    mkdir -p "$d/ev"
    printf 'kf3: guest GPU UUID GPU-0123abcd-0000-4000-8000-00000000000a\nhost GPU-0123ABCD-0000-4000-8000-00000000000A x GPU-ffffffff-0000-4000-8000-00000000000b\n' > "$d/ev/a.txt"
    scrub_uuids "$d/ev"
    if grep -qiE '0123abcd|ffffffff' "$d/ev/a.txt" || [ "$(grep -o 'fa4efa4e[0-9a-f-]*' "$d/ev/a.txt" | sort -u | wc -l)" != 2 ]; then
        bad=$((bad + 1))
        echo "TRIO_SELFTEST_FAIL uuid scrub: $(cat "$d/ev/a.txt")"
    fi
    xid_new() { printf '%s\n' 'NVRM: Xid (PCI:0000:01:00): 31, pid=4242, name=qemu-system-x86, Ch 1' 'NVRM: Xid (PCI:0000:01:00): 13, pid=77, name=cup8bench, Ch 2'; }
    [ "$(xid_foreign 0 4242 | grep -c .)" = 2 ] || { bad=$((bad + 1)); echo "TRIO_SELFTEST_FAIL xid_foreign with the Windows pid"; }
    [ "$(xid_foreign 0 | grep -c .)" = 1 ] || { bad=$((bad + 1)); echo "TRIO_SELFTEST_FAIL xid_foreign without a pid"; }
    [ "$(xid_foreign 0 999 | grep -c .)" = 1 ] || { bad=$((bad + 1)); echo "TRIO_SELFTEST_FAIL xid_foreign with another pid"; }
    rm -rf "$d"
    if [ "$bad" = 0 ]; then echo "TRIO_SELFTEST_OK"; exit 0; fi
    echo "TRIO_SELFTEST_FAILED bad=$bad"
    exit 1
}
if [ "$SELFTEST" = 1 ]; then selftest; fi

# ==================================================================================================
mkdir -p "$PROV"
[ ! -e "$LOG" ] || { echo "TRIO_REFUSED why=would-overwrite-$LOG"; exit 2; }
mkdir -p "$EV/bin" && chmod 0700 "$EV"
: > "$LOG"
trap on_exit EXIT
say "TRIO_START tag=$T gate_tag=$GT rev=$REV source=$SOURCE overlay=$OVERLAY $(date -u -Is)"

stage_begin preflight
[ "$(id -u)" = 0 ] || { WHY_A="not root"; exit 2; }
[ -c /dev/kvm ] || { WHY_A="no /dev/kvm"; exit 2; }
[ -x "$QEMU" ] || { WHY_A="no kf3 QEMU at $QEMU"; exit 2; }
[ -e "$VM/disk/$OVERLAY.qcow2" ] || { WHY_A="no gate overlay $VM/disk/$OVERLAY.qcow2 (run windows_gate.sh $GT first)"; exit 2; }
[ -s "$BENCH/fastguest/initrd.cpio.gz" ] || { WHY_A="no fast guest at $BENCH/fastguest"; exit 2; }
GPU_NAME=$(nvidia-smi --query-gpu=name --format=csv,noheader | head -1)
say "TRIO_NOTE host gpu=\"$GPU_NAME\" driver=$(nvidia-smi --query-gpu=driver_version --format=csv,noheader | head -1) cpus=$(nproc) ram_gib=$(free -g | awk '/^Mem:/{print $2}') qemu_sha256=$(sha256sum "$QEMU" | cut -c1-16)"
nvidia-smi -q -d MEMORY > "$EV/host_smi_memory_idle.txt" 2>&1
LIBCUDA=-lcuda
[ -e /usr/lib/x86_64-linux-gnu/libcuda.so ] || LIBCUDA=/usr/lib/x86_64-linux-gnu/libcuda.so.1
gcc -O0 -I "$SOURCE/scripts/bench/cuda_min" -o "$EV/bin/cup8" "$SOURCE/scripts/bench/cup8.c" $LIBCUDA -lm > "$EV/host_build.txt" 2>&1 \
    && gcc -O2 -o "$EV/bin/cup8bench" "$SOURCE/scripts/bench/cup8bench.c" $LIBCUDA -lm >> "$EV/host_build.txt" 2>&1 \
    || { WHY_A="host tools did not build: $(tail -2 "$EV/host_build.txt" | tr '\n' ' ')"; exit 2; }
XID0=$(xid_count)
snapshot idle
stage_end

stage_begin host_solo
rm -f "$EV/stop_host_solo"
( sleep "$HOST_SOLO_S"; touch "$EV/stop_host_solo" ) &
host_loop solo 4096 "$EV/stop_host_solo"
say "TRIO_NOTE host solo: $(grep -c ' ok=1 ' "$EV/host_solo.log") correct units, $(grep -c ' ok=0 ' "$EV/host_solo.log") wrong"
stage_end

stage_begin win_boot
vmuuid=$(sed -n 's/^VM_UUID=//p' "$VM/identity.env" 2> /dev/null | head -1)
[[ "$vmuuid" =~ ^[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$ ]] || { WHY_A="no VM_UUID in $VM/identity.env"; exit 1; }
rm -f "$VM/run/stop-requested"
KF_GPU_LOCK=/tmp/kf-trio-windows.lock WINVM_QEMU=$QEMU bash "$WV" run --detach --once --overlay "$OVERLAY" --arm B \
    --guest-driver "$GDRV" --kf3-extra "gpu-uuid=GPU-$vmuuid" --env-file "$FLAGS" --tag "$WTAG" > "$EV/win_launch.txt" 2>&1 < /dev/null \
    || { WHY_A="win_vm.sh run did not start: $(tail -2 "$EV/win_launch.txt" | tr '\n' ' ')"; exit 1; }
WIN_UP=1
sleep 5
wait_ssh 900 || { WHY_A="Windows never answered ssh (900 s) on its solo boot"; exit 1; }
BOOT_UTC=$(g 60 "powershell -NoProfile -Command \"(Get-CimInstance Win32_OperatingSystem).LastBootUpTime.ToUniversalTime().ToString('yyyy-MM-ddTHH:mm:ssZ')\"" | tr -d '\r' | tail -1)
[[ "$BOOT_UTC" =~ ^20[0-9-]+T[0-9:]+Z$ ]] || { WHY_A="could not read the guest's boot time (got '${BOOT_UTC:0:40}')"; exit 1; }
ok=0
for _ in $(seq 1 40); do
    gps 120 'C:\kf\gate_state.ps1' -Mode desktop > "$EV/win_desktop.txt" 2>&1
    grep -aqE "^GATE_DESKTOP user='[^']*\\\\$KF_GUEST_USER' dwm=[1-9][0-9]* explorer=[1-9]" "$EV/win_desktop.txt" && { ok=1; break; }
    sleep 10
done
[ "$ok" = 1 ] || { WHY_A="no signed-in Windows desktop on the solo boot"; exit 1; }
g 180 'C:\Windows\System32\nvidia-smi.exe -L' 2>&1 | tr -d '\r' > "$EV/win_smi_L.txt"
grep -aq 'GPU 0:' "$EV/win_smi_L.txt" || { WHY_A="nvidia-smi -L in Windows printed no GPU 0"; exit 1; }
# the guest's GPU UUID must be its own (GPU-<VM_UUID>), never the host's
HOSTUUID=$(nvidia-smi --query-gpu=uuid --format=csv,noheader | head -1)
if grep -aqi "GPU-$vmuuid" "$EV/win_smi_L.txt" && ! grep -aqi "$HOSTUUID" "$EV/win_smi_L.txt"; then UUID_WIN=own; else UUID_WIN=WRONG; fi
say "TRIO_NOTE windows nvidia-smi -L: uuid=$UUID_WIN (own=GPU-<VM_UUID>, never the host's), boot_since=$BOOT_UTC"
for f in gate_state.ps1 gate_run.ps1; do timeout 300 bash "$WV" scp-to "$WIN/$f" "C:/kf/$f" < /dev/null > /dev/null 2>&1; done
PROBES=$EV/probes
mkdir -p "$PROBES"
bash "$WIN/cuda/build_cup_win.sh" "$PROBES" > "$EV/probes_build.txt" 2>&1 || { WHY_A="build_cup_win.sh failed"; exit 1; }
x86_64-w64-mingw32-g++ -O2 -std=gnu++17 -static -static-libgcc -static-libstdc++ "$WIN/appmatrix/tools/src/kf_dxprobe.cpp" \
    -o "$PROBES/kf_dxprobe.exe" -ld3d11 -ld3d12 -ldxgi -ldxguid >> "$EV/probes_build.txt" 2>&1 || { WHY_A="kf_dxprobe did not build"; exit 1; }
g 60 'New-Item -ItemType Directory -Force -Path C:\kf\probes | Out-Null' > /dev/null 2>&1
for f in "$PROBES/cup8.exe" "$PROBES/kf_dxprobe.exe"; do timeout 600 bash "$WV" scp-to "$f" 'C:/kf/probes/' < /dev/null > /dev/null 2>&1 || { WHY_A="scp of $(basename "$f") failed"; exit 1; }; done
snapshot win_up
stage_end

stage_begin win_solo
solo_bad=0
for i in $(seq 1 "$WIN_SOLO_ROUNDS"); do win_round solo "$i" || solo_bad=1; done
[ "$solo_bad" = 0 ] || { WHY_A="a Windows probe was wrong on the solo boot, before any second guest (win_solo_*_fail_*.txt)"; exit 1; }
stage_end

# ---- phase A: Windows idle at its desktop + the Linux fast suite --------------------------------------------------
stage_begin phase_A
STOP=$EV/stop_A
rm -f "$STOP"
xa=$(xid_count)
heartbeat A "$STOP" &
sampler "$STOP" A &
ta=$(date +%s)
fast_suite A
tb=$(date +%s)
touch "$STOP"
wait
WINPID=$(cat "$VM/run/qemu.pid" 2> /dev/null || true)
win_round A_after 0 > /dev/null 2>&1; aprobe=$?
WHY_A=""
suite_green A || WHY_A+="fast suite not green ($(suite_line A)); "
win_ok || WHY_A+="the Windows guest is gone or panicked; "
grep -q ' ssh=0 ' "$EV/beat_A.log" 2> /dev/null && WHY_A+="Windows missed $(grep -c ' ssh=0 ' "$EV/beat_A.log") ssh heartbeat(s); "
[ "$aprobe" = 0 ] || WHY_A+="a Windows probe after phase A was wrong; "
[ -z "$(xid_foreign "$xa" "$WINPID")" ] || WHY_A+="a host Xid names the Windows QEMU; "
if [ -z "$WHY_A" ]; then VA=PASS; else VA=FAIL; WHY_A=${WHY_A%; }; fi
say "TRIO_NOTE phase A: $(suite_line A) secs=$((tb - ta)) beats=$(grep -c . "$EV/beat_A.log" 2> /dev/null) xid_new=$(xid_new "$xa" | grep -c . || true)"
xid_new "$xa" > "$EV/xid_A.txt"
stage_end

# ---- phase B: the trio --------------------------------------------------------------------------------------------
if [ "${KF_TRIO_SKIP_B:-0}" = 1 ] || ! win_ok; then
    WHY_B="not run (KF_TRIO_SKIP_B=${KF_TRIO_SKIP_B:-0}, windows_ok=$(win_ok && echo 1 || echo 0))"
else
    stage_begin phase_B
    minfree=$(sed -n 's/.*label=A .* vram_used_mib=\([0-9]*\) vram_total_mib=\([0-9]*\) .*/\2 \1/p' "$EV/snap.log" | awk '{f=$1-$2; if(m==""||f<m)m=f} END{print m+0}')
    HN=$(awk -v f="$minfree" -v fr="$HOST_FRAC" -v mx="$HOST_NMAX" 'BEGIN{n=int(sqrt(f*fr*1048576/12)/1024)*1024; if(n>mx)n=mx; if(n<1024)n=1024; print n}')
    say "TRIO_NOTE phase B host load: smallest free VRAM in A ${minfree} MiB -> cup8 N=$HN ($((3 * HN * HN * 4 / 1048576)) MiB) + cup8bench 1024,2048"
    STOP=$EV/stop_B
    rm -f "$STOP"
    xb=$(xid_count)
    heartbeat B "$STOP" &
    sampler "$STOP" B &
    host_loop B "$HN" "$STOP" &
    HOSTPID=$!
    win_loop B "$STOP" > /dev/null 2>&1 &
    ta=$(date +%s)
    fast_suite B
    tb=$(date +%s)
    touch "$STOP"
    wait
    grep -a 'TRIO_WIN_ITER phase=B ' "$LOG" > "$EV/win_B.log" || true
    WHY_B=""
    suite_green B || WHY_B+="fast suite not green ($(suite_line B)); "
    grep -q ' ok=0 ' "$EV/host_B.log" 2> /dev/null && WHY_B+="host loop: $(grep -c ' ok=0 ' "$EV/host_B.log") wrong unit(s); "
    grep -q ' ok=0 ' "$EV/win_B.log" 2> /dev/null && WHY_B+="Windows probes: $(grep -c ' ok=0 ' "$EV/win_B.log") wrong; "
    m=$(progress_ok "$EV/host_B.log" "$ta" "$tb") || WHY_B+="host loop stalled: $m; "
    m=$(progress_ok "$EV/win_B.log" "$ta" "$tb") || WHY_B+="Windows probes stalled: $m; "
    win_ok || WHY_B+="the Windows guest is gone or panicked; "
    grep -q ' ssh=0 ' "$EV/beat_B.log" 2> /dev/null && WHY_B+="Windows missed $(grep -c ' ssh=0 ' "$EV/beat_B.log") ssh heartbeat(s); "
    [ -z "$(xid_foreign "$xb" "$WINPID")" ] || WHY_B+="a host Xid names the Windows QEMU or the host loop; "
    xid_new "$xb" > "$EV/xid_B.txt"
    say "TRIO_NOTE phase B: $(suite_line B) secs=$((tb - ta)) host_units=$(grep -c ' ok=1 ' "$EV/host_B.log" 2> /dev/null) win_probes_ok=$(grep -c ' ok=1 ' "$EV/win_B.log" 2> /dev/null) xid_new=$(grep -c . "$EV/xid_B.txt" || true)"
    if [ -z "$WHY_B" ]; then VB=PASS; else VB=FAIL; WHY_B=${WHY_B%; }; fi
    : "$HOSTPID"
    stage_end
fi

# ---- end: Windows reset counters since boot, then a graceful stop --------------------------------------------------
stage_begin end
if win_ok; then
    gps 300 'C:\kf\gate_state.ps1' -Mode tdr -Since "$BOOT_UTC" > "$EV/win_tdr.txt" 2>&1
    gps 120 'C:\kf\gate_state.ps1' -Mode display > "$EV/win_display_end.txt" 2>&1
    grep -a '^GATE_TDR\|^GATE_DISPLAY' "$EV/win_tdr.txt" "$EV/win_display_end.txt" | sed 's/^[^:]*://' | while read -r l; do say "  $l"; done
    if ! grep -aq '^GATE_TDR .* total=0$' "$EV/win_tdr.txt"; then
        [ "$VA" != PASS ] || { VA=FAIL; WHY_A="Windows reset counters are not all zero since boot"; }
        [ "$VB" != PASS ] || { VB=FAIL; WHY_B="Windows reset counters are not all zero since boot"; }
    fi
fi
win_stop
cp "$VM/logs/${WTAG}_run.log" "$EV/" 2> /dev/null
for f in "$VM"/logs/${WTAG}_b*_qemu.log "$VM"/logs/${WTAG}_b*_qmp_events.jsonl; do [ -f "$f" ] && gzip -c "$f" > "$EV/$(basename "$f").gz"; done
for ph in A B; do
    cp "$BENCH/${T}_${ph}_suite.out" "$EV/" 2> /dev/null
    # one realize line per fast arm QEMU: the UUID it served and the BAR1 budget it saw
    grep -ah 'guest GPU UUID\|BAR1\|realize refused' "$BENCH"/fast_${T}_${ph}_*_qemu.log 2> /dev/null | sort | uniq -c | head -40 > "$EV/fast_${ph}_realize.txt"
done
grep -ah 'guest GPU UUID\|BAR1\|realize refused' "$VM"/logs/${WTAG}_b*_qemu.log 2> /dev/null | head -20 > "$EV/win_realize.txt"
XID1=$(xid_count)
say "TRIO_NOTE host Xid lines: before=$XID0 after=$XID1"
# UUIDs, compared here (they are scrubbed from the evidence): the Windows device's, every fast arm's, the host's
uu() { grep -aohiE 'guest GPU UUID GPU-[0-9a-f-]{36}' "$@" 2> /dev/null | awk '{print tolower($4)}' | sort -u; }
WU=$(uu "$EV/win_realize.txt")
LU=$(cat "$BENCH"/fast_${T}_A_*_qemu.log "$BENCH"/fast_${T}_B_*_qemu.log 2> /dev/null | uu /dev/stdin)
say "TRIO_UUID windows_values=$(grep -c . <<< "$WU") linux_values=$(grep -c . <<< "$LU") shared=$(comm -12 <(echo "$WU") <(echo "$LU") | grep -c .) host_in_any=$(printf '%s\n%s\n' "$WU" "$LU" | grep -ci "${HOSTUUID#GPU-}")"
stage_end
snapshot final
say "TRIO_FOOTPRINT $(awk '/label=idle/{i=$0} /label=win_up/{w=$0} END{print "idle: " substr(i, index(i,"vram_used")) " | win_up: " substr(w, index(w,"vram_used"))}' "$EV/snap.log")"
verdict_lines
VA_SAID=1
