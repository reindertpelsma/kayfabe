#!/bin/bash
# windows_gate.sh <tag> [<prov-dir>] -- the OPT-IN Windows stage of the merge bar. Run ON a provisioned box,
# from the checkout. merge_check.sh calls it for KF_MERGE_WINDOWS=1:
#   KF_MERGE_WINDOWS=1 bash scripts/bench/box/merge_check.sh <branch> <tag>
# or by hand (a Windows lease: windows_lane.sh provisions and installs, this runs the gate):
#   bash scripts/bench/box/windows_gate.sh <tag>                  log: <prov-dir>/<tag>_windows.log
#
# STATUS: LIVE, 2026-11-01 (docs/design/V3_WINDOWS_GATE.md). Owner, 2026-10-31: no next -> master merge until
# Windows is verified; the master gate is "small and fast", the soak is a separate release lane.
#
# What it proves, in ONE bounded run (a stable-enough check, not a soak; no performance claim):
#   Windows 11 Enterprise LTSC 2024 (Microsoft 90-day evaluation, build 26100) installs ONCE per box, boots on
#   kayfabe's kf3 device with NVIDIA's stock 580.88 driver and GSP forced on (EnableGpuFirmware, class + service
#   key), reaches a signed-in desktop (dwm + explorer), the adapter reads ConfigManagerErrorCode 0, nvidia-smi
#   runs, a CUDA matmul (cup8, byte exact) and a D3D11 compute+draw+readback (kf_dxprobe) pass, a bounded idle
#   window (KF_WINGATE_IDLE_S, default 300 s) ends with zero resets (Display 4101, nvlddmkm 153, Kernel-Power 41,
#   6008, BugCheck 1001, WER LiveKernelEvent) and the same two probes pass AGAIN after the idle; then a graceful stop.
#
# Verdict lines (fail closed: anything else, or no EXIT line, is not a pass):
#   WINDOWS_GATE_PASS rev=<rev> gpu="<name>" ...                    exit 0
#   WINDOWS_GATE_FAIL class=KAYFABE stage=<s> why=<w> ...           exit 1  a guest-side defect WITH both host references green
#   WINDOWS_GATE_UNATTRIBUTED class=<c> stage=<s> why=<w> ...       exit 3  not kayfabe's: PROVIDER (rental, network), MEDIA (a pinned
#                                                                           file changed), OPERATOR (our script or configuration),
#                                                                           HOSTREF (the same-box host GPU reference was not green),
#                                                                           BAR1_EXCEPTION (the Vast BAR1 limit: counts only if the
#                                                                           windows-on-vast native reference also fails)
#   WINDOWS_GATE_REFUSED why=<w>                                    exit 2  a precondition (not root, no /dev/kvm, little disk)
#   WINDOWS_GATE_EXIT rc=<n> <utc>                                  always last (an EXIT trap)
# The owner rule (2026-10-26): a failure on a rented box is kayfabe's only if the SAME box's host-side GPU
# reference (nvidia-smi plus a CUDA matmul on the Linux host) is green before AND after the guest run.
#
# Knobs (all optional): KF_WINGATE_IDLE_S (300), KF_WINGATE_BUDGET_S (9000: the wall for everything after the
# install; the install itself is bounded by WINVM_INSTALL_WAIT_S, 4200), KF_WINGATE_FLAGS (the kf3 env file,
# scripts/bench/windows/kf3_flags.sample.env), KF_WINGATE_GUEST_DRIVER (580.65.06: the guest ABI of the
# Windows 580.88 driver), KF_WINGATE_ARM (B), WINVM_ROOT (/workspace/winvm), BENCH (/workspace/bench).
# Secrets: the Windows password and the ssh key stay in $WINVM_ROOT/kfwin/secrets; nothing from there, from tpm/
# or from firmware/ is copied to the evidence directory, and the evidence is scanned for the password.
set -uo pipefail
# --selftest: the verdict classification, with stubs and no box (GPU-free; scripts/ci/test_vast_windows_gate.py)
SELFTEST=0
if [ "${1:-}" = --selftest ]; then SELFTEST=1; set -- selftest "$(mktemp -d)"; fi
T=${1:?usage: windows_gate.sh <tag> [<prov-dir>]}
[[ "$T" =~ ^[a-zA-Z0-9][a-zA-Z0-9._-]*$ ]] || { echo "WINDOWS_GATE_REFUSED why=invalid-tag"; exit 2; }
PROV=${2:-${KF_PROV_DIR:-/root/prov}}
SOURCE=$(cd "$(dirname "$(readlink -f "${BASH_SOURCE[0]}")")/../../.." && pwd)
WIN=$SOURCE/scripts/bench/windows
WV=$WIN/win_vm.sh
LOG=$PROV/${T}_windows.log
EV=$PROV/${T}_windows
mkdir -p "$PROV"
[ ! -e "$LOG" ] || { echo "WINDOWS_GATE_REFUSED why=would-overwrite-$LOG"; exit 2; }
: > "$LOG"
say() { echo "$*" | tee -a "$LOG"; }

export PATH="$PATH:$HOME/.cargo/bin"
export BENCH=${BENCH:-/workspace/bench}
export WINVM_ROOT=${WINVM_ROOT:-/workspace/winvm}
export WINVM_VM_IDENTITY=${WINVM_VM_IDENTITY:-1}
export WINVM_INSTALL_WAIT_S=${WINVM_INSTALL_WAIT_S:-4200}
export WINVM_SSH_CONNECT_TIMEOUT=${WINVM_SSH_CONNECT_TIMEOUT:-100}
export WINVM_LOCK_WAIT_S=${WINVM_LOCK_WAIT_S:-120}
export KF_GUEST_USER=${KF_GUEST_USER:-kf}
IDLE_S=${KF_WINGATE_IDLE_S:-300}
BUDGET_S=${KF_WINGATE_BUDGET_S:-9000}
FLAGS=${KF_WINGATE_FLAGS:-$WIN/kf3_flags.sample.env}
GDRV=${KF_WINGATE_GUEST_DRIVER:-580.65.06}
ARM=${KF_WINGATE_ARM:-B}
VM=$WINVM_ROOT/${WINVM_NAME:-kfwin}
TAG=gate_$(printf %s "$T" | tr -c 'A-Za-z0-9_' '_' | cut -c1-24)
REV=$(git -C "$SOURCE" rev-parse --short=8 HEAD 2> /dev/null || echo norev)
# build_kf3.sh installs the binary under kf3-bins/<rev>[-dirty] (a modified tracked file adds the suffix)
REVDIR=$REV
[ -z "$(git -C "$SOURCE" status --porcelain --untracked-files=no 2> /dev/null)" ] || REVDIR=$REV-dirty
export WINVM_QEMU=${WINVM_QEMU:-$BENCH/kf3-bins/$REVDIR/qemu-system-x86_64}
T0=$(date +%s)
DEADLINE=0                      # set after the install: the wall for every later wait
TIMING=()
GPU_NAME=unknown
VERDICT=""                      # set by verdict(); the EXIT trap turns a missing one into UNATTRIBUTED
HOSTREF_BEFORE=unrun HOSTREF_AFTER=unrun
STAGE=start
GUEST_STARTED=0                 # 1 once a kf3 QEMU was started: only then can a failure be kayfabe's
ENABLE_UTC=""
rc_final=0
FAILMODE=none FAILWHY="" FAILSTAGE=""
STAGE_T=0

# ------------------------------------------------------------------------------------------------
# verdicts
# ------------------------------------------------------------------------------------------------
verdict() { # KIND CLASS WHY...   KIND: PASS|FAIL|UNATTRIBUTED
    local kind=$1 class=$2 why total
    shift 2
    why=$*
    VERDICT=$kind
    total=$(($(date +%s) - T0))
    case "$kind" in
        PASS) say "WINDOWS_GATE_PASS rev=$REV gpu=\"$GPU_NAME\" idle_s=$IDLE_S hostref=$HOSTREF_BEFORE/$HOSTREF_AFTER secs=$total $why"; rc_final=0 ;;
        FAIL) say "WINDOWS_GATE_FAIL class=$class stage=$STAGE why=\"$why\" rev=$REV gpu=\"$GPU_NAME\" hostref=$HOSTREF_BEFORE/$HOSTREF_AFTER secs=$total"; rc_final=1 ;;
        *) say "WINDOWS_GATE_UNATTRIBUTED class=$class stage=$STAGE why=\"$why\" rev=$REV gpu=\"$GPU_NAME\" hostref=$HOSTREF_BEFORE/$HOSTREF_AFTER secs=$total"; rc_final=3 ;;
    esac
}
refuse() { say "WINDOWS_GATE_REFUSED why=$1"; VERDICT=REFUSED; rc_final=2; exit 2; }

# A failure of the GUEST (it did not boot, reach the desktop, pass a probe, stay quiet) is kayfabe's only with the
# same-box host reference green before AND after the run. A failure of the harness (a script step, an scp, a
# registry write that did not read back) is OPERATOR: never kayfabe's. BAR1 realize refusals are the documented
# Vast exception. The caller names the cause; conclude() decides the class after the evidence is collected.
fail_op() { FAILMODE=op FAILWHY=$*; FAILSTAGE=$STAGE; conclude; }
fail_guest() { FAILMODE=guest FAILWHY=$*; FAILSTAGE=$STAGE; conclude; }
ssh_seen() { grep -q 'SSH_UP' "$EV/ssh_seen" 2> /dev/null; }
bar1_refusal() {
    local q=$VM/logs/${TAG}_b1_qemu.log
    [ -s "$q" ] || return 1
    grep -qiE 'bar1.*(too large|exceed|cannot|refus|invalid|power of)|(too large|exceed|refus).*bar1' "$q" && ! ssh_seen
}

# the host side of the run, copied into the evidence directory (text only: never secrets/, tpm/ or firmware/)
collect_evidence() {
    local f lastq
    for f in "$VM/logs/${TAG}_run.log" "$VM/logs/install.log" "$VM/logs/nv_install.log" "$VM/logs/desktop_prep.log" "$VM/logs/checks.log" \
        "$VM"/logs/${TAG}_b*_qmp_events.jsonl "$VM"/logs/${TAG}_b*_cmdline.txt "$VM"/logs/${TAG}_b*_host_dmesg.txt \
        "$VM"/logs/${TAG}_b*_host_smi_before.txt "$VM"/logs/${TAG}_b*_host_smi_after.txt "$VM"/logs/nv_install_*.txt \
        "$VM"/logs/desktop_prep_*.txt "$VM/logs/stop.log" "$VM/logs/tpm_ek_history.txt"; do
        [ -f "$f" ] && cp "$f" "$EV/" 2> /dev/null
    done
    for f in "$VM"/logs/${TAG}_b*_qemu.log "$VM"/logs/${TAG}_b*_serial.log; do
        [ -f "$f" ] && gzip -c "$f" > "$EV/$(basename "$f").gz"
    done
    [ -d "$VM/logs/evidence/${TAG}_final" ] && cp -r "$VM/logs/evidence/${TAG}_final" "$EV/guest_collect" 2> /dev/null
    mkdir -p "$EV/shots"
    cp "$VM/logs/shots/${TAG}_desktop.png" "$VM/logs/shots/${TAG}_final.png" "$EV/shots/" 2> /dev/null
    lastq=$(ls -t "$VM"/logs/${TAG}_b*_qemu.log 2> /dev/null | head -1)
    {
        echo "qemu_log=$(basename "${lastq:-none}")"
        echo "gsp_lines=$(grep -ac 'GSP\|gsp' "${lastq:-/dev/null}" || true)"
        echo "refused_0x2080121f=$(grep -ac '0x2080121f' "${lastq:-/dev/null}" || true)"
        echo "gsp_refused_lines=$(grep -ac 'GSP REFUSED' "${lastq:-/dev/null}" || true)"
        echo "unloading_guest_driver=$(grep -ac 'UNLOADING_GUEST_DRIVER' "${lastq:-/dev/null}" || true)"
        echo "running_to_suspending=$(grep -ac 'Running -> Suspending' "${lastq:-/dev/null}" || true)"
        echo "host_xid_in_boots=$(cat "$VM"/logs/${TAG}_b*_host_dmesg.txt 2> /dev/null | grep -ac 'NVRM: Xid' || true)"
        grep -aE 'Cold ->|ProtectedRegionUp|Booted|GSP_SET_SYSTEM_INFO|bGspNocatEnabled|REFUSED' "${lastq:-/dev/null}" | head -60
    } > "$EV/gsp_signals.txt" 2>&1
}
pack_evidence() {
    # the Windows password must not be in the evidence (fail closed: remove it and say so)
    if [ -s "$VM/secrets/win_password" ]; then
        local pw
        pw=$(head -1 "$VM/secrets/win_password")
        if [ -n "$pw" ] && grep -rqF -- "$pw" "$EV" 2> /dev/null; then
            rm -rf "$EV"
            say "GATE_NOTE the evidence contained the guest password: it was removed (a script bug, not a result)"
            return 1
        fi
    fi
    rm -f "$EV/cup8_host"
    (cd "$EV" && find . -type f ! -name MANIFEST.sha256 -print0 | sort -z | xargs -0 -r sha256sum > MANIFEST.sha256)
    tar -C "$PROV" -czf "$PROV/${T}_windows.tgz" "${T}_windows" 2> /dev/null
    say "GATE_NOTE evidence: $EV ($(find "$EV" -type f | wc -l) files), $PROV/${T}_windows.tgz sha256=$(sha256sum "$PROV/${T}_windows.tgz" | cut -c1-16)"
}

# the end of every path that started a guest: stop it, take the host reference AFTER, collect, then the verdict
conclude() {
    local failstage=$STAGE xid_in xid_delta
    [ -z "$FAILSTAGE" ] || failstage=$FAILSTAGE
    teardown_guest
    if [ "$GUEST_STARTED" = 1 ] && [ "$HOSTREF_AFTER" = unrun ]; then
        STAGE=hostref_after
        hostref after
    fi
    [ "$GUEST_STARTED" != 1 ] || collect_evidence
    STAGE=$failstage
    xid_in=$(sed -n 's/^host_xid_in_boots=//p' "$EV/gsp_signals.txt" 2> /dev/null)
    xid_delta=$(sed -n 's/^HOSTREF_XID_DELTA //p' "$EV/hostref_after.txt" 2> /dev/null)
    case "$FAILMODE" in
        op) verdict UNATTRIBUTED OPERATOR "$FAILWHY" ;;
        guest)
            if bar1_refusal; then
                verdict UNATTRIBUTED BAR1_EXCEPTION "$FAILWHY (kf3 refused a BAR1 size: the documented Vast exception; kayfabe's only if windows-on-vast native fails too)"
            elif [ "$HOSTREF_BEFORE" != green ] || [ "$HOSTREF_AFTER" != green ]; then
                verdict UNATTRIBUTED HOSTREF "$FAILWHY (host reference before=$HOSTREF_BEFORE after=$HOSTREF_AFTER)"
            else
                verdict FAIL KAYFABE "$FAILWHY"
            fi ;;
        *)
            if [ "$HOSTREF_AFTER" != green ]; then
                STAGE=hostref_after
                verdict UNATTRIBUTED HOSTREF "the guest passed, but the host reference after the run is $HOSTREF_AFTER"
            elif [ "${xid_in:-0}" != 0 ] || [ "${xid_delta:-0}" != 0 ]; then
                STAGE=host_xid
                verdict FAIL KAYFABE "new host Xid during the Windows run (in boots: ${xid_in:-?}, delta: ${xid_delta:-?}) with both host references green"
            elif [ "$(sed -n 's/^refused_0x2080121f=//p' "$EV/gsp_signals.txt")" != 0 ]; then
                STAGE=gsp_signals
                verdict FAIL KAYFABE "the kf3 log shows the no-flags refusal 0x2080121f although the flags were exported"
            else
                STAGE="done"
                verdict PASS NONE "install=once driver=580.88 gsp=forced code=0 probes=cup8,d3d11,x2 resets=0 enable_utc=$ENABLE_UTC"
            fi ;;
    esac
    finish
}

finish() {
    if [ -d "$EV" ]; then pack_evidence || true; fi
    say "WINDOWS_GATE_TIMING total=$(($(date +%s) - T0))s ${TIMING[*]:-}"
    exit "$rc_final"
}
on_exit() {
    local rc=$?
    if [ -z "$VERDICT" ]; then
        # the script died without a verdict: never a pass
        VERDICT=UNATTRIBUTED
        say "WINDOWS_GATE_UNATTRIBUTED class=OPERATOR stage=$STAGE why=\"gate script ended without a verdict (rc=$rc)\" rev=$REV gpu=\"$GPU_NAME\" hostref=$HOSTREF_BEFORE/$HOSTREF_AFTER"
        rc_final=3
    fi
    if [ "$GUEST_STARTED" = 1 ] && [ -e "$VM/run/qemu.pid" ]; then teardown_guest; fi
    say "WINDOWS_GATE_EXIT rc=$rc_final $(date -u -Is)"
    exit "$rc_final"
}
trap on_exit EXIT

stage_begin() { STAGE=$1; STAGE_T=$(date +%s); say "GATE_STAGE $STAGE START $(date -u -Is)"; }
stage_end() { local s=$(($(date +%s) - STAGE_T)); TIMING+=("$STAGE=${s}s"); say "GATE_STAGE $STAGE EXIT ok ${s}s"; }
left() { # seconds left in the budget (never negative); before DEADLINE is set: a large number
    local n
    [ "$DEADLINE" -gt 0 ] || { echo 99999; return; }
    n=$((DEADLINE - $(date +%s)))
    [ "$n" -gt 0 ] || n=0
    echo "$n"
}
cap() { # WANT -> min(WANT, left)
    local l
    l=$(left)
    if [ "$1" -le "$l" ]; then echo "$1"; else echo "$l"; fi
}

# ------------------------------------------------------------------------------------------------
# guest helpers (every call bounded; ssh goes through win_vm.sh's own key, port and known_hosts)
# ------------------------------------------------------------------------------------------------
g() { # SECONDS CMD...  -> a guest PowerShell command, stdout to our stdout
    local t=$1
    shift
    timeout "$t" bash "$WV" ssh "$@" < /dev/null
}
qemu_alive() {
    local pid
    pid=$(cat "$VM/run/qemu.pid" 2> /dev/null || true)
    [ -n "$pid" ] && [ -d "/proc/$pid" ] && [ "$(awk '{print $3}' "/proc/$pid/stat" 2> /dev/null)" != Z ]
}
wait_ssh() { # SECONDS
    local t=$1 w=0 per=$((WINVM_SSH_CONNECT_TIMEOUT + 12))
    while [ "$w" -lt "$t" ]; do
        qemu_alive || { say "GATE_NOTE QEMU is gone while waiting for ssh"; return 1; }
        if g "$per" 'echo KF_SSH_UP' 2> /dev/null | grep -q KF_SSH_UP; then
            echo "SSH_UP after ${w}s" >> "$EV/ssh_seen"
            say "GATE_NOTE ssh up after ${w}s"
            return 0
        fi
        sleep 10
        w=$((w + 10))
    done
    return 1
}
guest_scp() { timeout 900 bash "$WV" scp-to "$1" "$2" < /dev/null; }
gps() { # SECONDS FILE-ON-GUEST ARGS... -> run a staged ps1, CRs stripped
    local t=$1 f=$2
    shift 2
    g "$t" "powershell -NoProfile -ExecutionPolicy Bypass -File $f $*" | tr -d '\r'
}
run_probe() { # NAME EXE ARG TIMEOUT OUTFILE -> echoes the GATE_PROBE line
    local name=$1 exe=$2 arg=$3 to=$4 out=$5 line args
    args="-Exe C:\\kf\\probes\\$exe -TimeoutS $to"
    [ -z "$arg" ] || args+=" -Arg $arg"
    gps $((to + 60)) 'C:\kf\gate_run.ps1' "$args" > "$out" 2>&1
    line=$(grep -a '^GATE_PROBE ' "$out" | tail -1)
    say "GATE_PROBE_RESULT $name $line" >&2
    echo "$line"
}
teardown_guest() {
    if qemu_alive; then
        say "GATE_NOTE graceful stop of the guest"
        timeout 420 bash "$WV" stop > "$EV/stop_cleanup.txt" 2>&1 < /dev/null || say "GATE_NOTE stop returned non-zero; see stop_cleanup.txt"
    fi
}

# ------------------------------------------------------------------------------------------------
# the host reference: the SAME box's Linux host, GPU healthy and usable (owner rule, 2026-10-26)
# ------------------------------------------------------------------------------------------------
hostref() { # before|after
    local when=$1 f="$EV/hostref_$1.txt" ok=1 xid cu xb state
    {
        echo "HOSTREF $when $(date -u -Is)"
        nvidia-smi --query-gpu=name,driver_version,memory.total,pstate --format=csv,noheader 2>&1 || ok=0
        nvidia-smi -q 2> /dev/null | grep -A2 -E 'BAR1 Memory Usage' | tr -s ' '
    } > "$f" 2>&1
    xid=$(dmesg 2> /dev/null | grep -c 'NVRM: Xid' || true)
    echo "HOSTREF_XID_COUNT $xid" >> "$f"
    if [ -x "$EV/cup8_host" ]; then
        if timeout 180 "$EV/cup8_host" > "$EV/hostref_${when}_cup8.txt" 2>&1 && grep -q 'CUP8 VERDICT: PASS' "$EV/hostref_${when}_cup8.txt"; then
            cu=PASS
        else cu=FAIL; fi
    else cu=NOTBUILT; fi
    echo "HOSTREF_CUP8 $cu" >> "$f"
    [ "$cu" = PASS ] || ok=0
    # The reference is "the GPU is healthy and usable". The Xid count is a baseline here (a merge-bar box carries the
    # approved negative-control Xids of the Linux suites); the DELTA over the guest run is judged by the gate itself.
    if [ "$when" = before ]; then echo "$xid" > "$EV/xid_before"; fi
    xb=$(cat "$EV/xid_before" 2> /dev/null || echo 0)
    echo "HOSTREF_XID_DELTA $((xid - xb))" >> "$f"
    if [ "$ok" = 1 ]; then state=green; else state=RED; fi
    if [ "$when" = before ]; then HOSTREF_BEFORE=$state; else HOSTREF_AFTER=$state; fi
    say "GATE_HOSTREF $when $state cup8=$cu xid_count=$xid xid_delta=$((xid - xb)) gpu=\"$(sed -n '2p' "$f")\""
}

# ------------------------------------------------------------------------------------------------
# selftest: conclude() with the box stubbed out. A guest failure is kayfabe's ONLY with both host references
# green; a harness failure never is; a pass needs green references, no Xid and no no-flags refusal.
# ------------------------------------------------------------------------------------------------
selftest() {
    local n=0 bad=0 out
    trap - EXIT
    teardown_guest() { :; }
    collect_evidence() { :; }
    hostref() { :; }
    finish() { return 0; }
    sc() { # NAME WANT_RC WANT_TEXT MODE BEFORE AFTER [BAR1 [XID [REFUSED]]]
        local name=$1 wrc=$2 wtext=$3 mode=$4 before=$5 after=$6 bar1=${7:-0} xid=${8:-0} refused=${9:-0}
        n=$((n + 1))
        out=$(
            EV=$(mktemp -d)
            LOG=$EV/log
            GUEST_STARTED=1
            HOSTREF_BEFORE=$before HOSTREF_AFTER=$after FAILMODE=$mode FAILWHY=why STAGE=probe
            printf 'refused_0x2080121f=%s\nhost_xid_in_boots=%s\n' "$refused" "$xid" > "$EV/gsp_signals.txt"
            printf 'HOSTREF_XID_DELTA 0\n' > "$EV/hostref_after.txt"
            if [ "$bar1" = 1 ]; then bar1_refusal() { return 0; }; fi
            conclude
            echo "RC=$rc_final"
        )
        if [[ "$out" == *"$wtext"* && "$out" == *"RC=$wrc"* ]] && { [ "$wrc" = 0 ] || [[ "$out" != *WINDOWS_GATE_PASS* ]]; }; then
            return 0
        fi
        bad=$((bad + 1))
        echo "WINDOWS_GATE_SELFTEST_FAIL case=$name want_rc=$wrc want=\"$wtext\" got: $out"
    }
    sc guest-both-green 1 'WINDOWS_GATE_FAIL class=KAYFABE' guest green green
    sc guest-before-red 3 'WINDOWS_GATE_UNATTRIBUTED class=HOSTREF' guest RED green
    sc guest-after-red 3 'WINDOWS_GATE_UNATTRIBUTED class=HOSTREF' guest green RED
    sc guest-after-unrun 3 'WINDOWS_GATE_UNATTRIBUTED class=HOSTREF' guest green unrun
    sc guest-bar1 3 'WINDOWS_GATE_UNATTRIBUTED class=BAR1_EXCEPTION' guest green green 1
    sc harness-both-green 3 'WINDOWS_GATE_UNATTRIBUTED class=OPERATOR' op green green
    sc pass 0 'WINDOWS_GATE_PASS' none green green
    sc pass-after-red 3 'class=HOSTREF' none green RED
    sc pass-with-xid 1 'WINDOWS_GATE_FAIL class=KAYFABE' none green green 0 2
    sc pass-with-flag-refusal 1 'WINDOWS_GATE_FAIL class=KAYFABE' none green green 0 0 1
    if [ "$bad" = 0 ]; then
        echo "WINDOWS_GATE_SELFTEST_OK cases=$n"
        exit 0
    fi
    echo "WINDOWS_GATE_SELFTEST_FAILED bad=$bad of $n"
    exit 1
}
if [ "$SELFTEST" = 1 ]; then selftest; fi

# ------------------------------------------------------------------------------------------------
# start
# ------------------------------------------------------------------------------------------------
mkdir -p "$EV"
chmod 0700 "$EV"
say "WINDOWS_GATE_START tag=$T rev=$REV source=$SOURCE $(date -u -Is) idle_s=$IDLE_S flags=$(basename "$FLAGS") guest_driver=$GDRV arm=$ARM"
say "WINDOWS_GATE_REQUESTED=1"

stage_begin preflight
[ "$(id -u)" = 0 ] || refuse not-root
[ -c /dev/kvm ] || refuse no-/dev/kvm
avail_gb=$(df -BG --output=avail "$(dirname "$WINVM_ROOT")" 2> /dev/null | tail -1 | tr -dc 0-9)
if [ -e "$VM/installed" ]; then need_gb=20; else need_gb=45; fi
[ "${avail_gb:-0}" -ge "$need_gb" ] || refuse "disk-${avail_gb:-0}G-free-need-${need_gb}G"
mem_gb=$(free -g | awk '/^Mem:/{print $2}')
[ "${mem_gb:-0}" -ge 20 ] || refuse "ram-${mem_gb:-0}G-need-20G"
[ -r "$FLAGS" ] || refuse "no-flags-file-$FLAGS"
if ! nvidia-smi -L > "$EV/nvidia_smi_L.txt" 2>&1; then
    verdict UNATTRIBUTED PROVIDER "nvidia-smi -L failed on the host: $(head -c 200 "$EV/nvidia_smi_L.txt")"
    finish
fi
GPU_NAME=$(nvidia-smi --query-gpu=name --format=csv,noheader | head -1)
say "GATE_NOTE host: gpu=\"$GPU_NAME\" driver=$(nvidia-smi --query-gpu=driver_version --format=csv,noheader | head -1) cpus=$(nproc) ram_gb=$mem_gb disk_free_gb=$avail_gb"
stage_end

# the host's CUDA matmul (cup8), built natively against the host driver: the same-box reference
stage_begin hostref_before
LIBCUDA=-lcuda
[ -e /usr/lib/x86_64-linux-gnu/libcuda.so ] || LIBCUDA=/usr/lib/x86_64-linux-gnu/libcuda.so.1
if ! gcc -O0 -I "$SOURCE/scripts/bench/cuda_min" -o "$EV/cup8_host" "$SOURCE/scripts/bench/cup8.c" $LIBCUDA -lm 2> "$EV/cup8_host_build.txt"; then
    say "GATE_NOTE the host cup8 reference did not build: $(tail -2 "$EV/cup8_host_build.txt" | tr '\n' ' ')"
fi
hostref before
if [ "$HOSTREF_BEFORE" != green ]; then
    verdict UNATTRIBUTED HOSTREF "the host GPU reference is not green BEFORE the guest (hostref_before.txt, hostref_before_cup8.txt)"
    finish
fi
stage_end

# ------------------------------------------------------------------------------------------------
# media, install (once per box), kf3 binary, probes
# ------------------------------------------------------------------------------------------------
stage_begin prereqs
missing=0
[ -e /usr/share/OVMF/OVMF_CODE_4M.secboot.fd ] || missing=1
for c in swtpm swtpm_setup sbsign xorriso pnmtopng 7z virt-fw-vars x86_64-w64-mingw32-g++ x86_64-w64-mingw32-gcc x86_64-w64-mingw32-dlltool; do
    command -v "$c" > /dev/null || missing=1
done
if [ "$missing" = 1 ]; then
    if ! DEBIAN_FRONTEND=noninteractive timeout 900 apt-get -o DPkg::Lock::Timeout=300 install -y -qq \
        ovmf swtpm swtpm-tools sbsigntool python3-virt-firmware xorriso netpbm p7zip-full \
        gcc-mingw-w64-x86-64 g++-mingw-w64-x86-64 > "$EV/apt.txt" 2>&1; then
        verdict UNATTRIBUTED PROVIDER "apt could not install the Windows prerequisites: $(tail -2 "$EV/apt.txt" | tr '\n' ' ')"
        finish
    fi
fi
stage_end

stage_begin kf3_binary
if [ ! -x "$WINVM_QEMU" ]; then
    say "GATE_NOTE no kf3 QEMU at $WINVM_QEMU: building this revision"
    if ! (cd "$SOURCE" && bash scripts/bench/build_kf3.sh "$BENCH/qemu-10.2.4" "$BENCH/qemu-build-kf3") > "$EV/build_kf3.txt" 2>&1; then
        verdict UNATTRIBUTED OPERATOR "build_kf3.sh failed: $(tail -2 "$EV/build_kf3.txt" | tr '\n' ' ')"
        finish
    fi
fi
if [ ! -x "$WINVM_QEMU" ]; then verdict UNATTRIBUTED OPERATOR "no kf3 QEMU at $WINVM_QEMU after the build"; finish; fi
say "GATE_NOTE kf3 qemu=$WINVM_QEMU sha256=$(sha256sum "$WINVM_QEMU" | cut -c1-16) rev=$REV"
stage_end

stage_begin install
if [ -e "$VM/installed" ]; then
    say "GATE_NOTE Windows already installed on this box ($(cat "$VM/installed")): install once per lease"
else
    # media and install are not kayfabe's: no kf3 device is attached during the install
    (cd "$SOURCE" && bash "$WV" install) > "$EV/install_stdout.txt" 2>&1
    irc=$?
    if [ "$irc" != 0 ] || [ ! -e "$VM/installed" ]; then
        cls=OPERATOR
        grep -q 'sha256' "$EV/install_stdout.txt" && cls=MEDIA
        grep -qE 'curl: |Could not resolve|Connection (timed out|refused)' "$EV/install_stdout.txt" && cls=PROVIDER
        verdict UNATTRIBUTED "$cls" "win_vm.sh install rc=$irc: $(grep -aE 'REFUSED|TIMEOUT|EXIT .* rc=[1-9]' "$EV/install_stdout.txt" | tail -2 | tr '\n' ' ')"
        finish
    fi
fi
stage_end
DEADLINE=$(($(date +%s) + BUDGET_S))

stage_begin probes_build
PROBES=$(mktemp -d "$BENCH/wingate-probes.XXXXXX")   # build outputs stay out of the evidence (only their sha256 goes in)
mkdir -p "$PROBES"
if ! bash "$WIN/cuda/build_cup_win.sh" "$PROBES" > "$EV/probes_build.txt" 2>&1; then
    verdict UNATTRIBUTED OPERATOR "build_cup_win.sh failed: $(tail -2 "$EV/probes_build.txt" | tr '\n' ' ')"
    finish
fi
DXSRC=$WIN/appmatrix/tools/src/kf_dxprobe.cpp
DXFLAGS=(-O2 -std=gnu++17 -static -static-libgcc -static-libstdc++)
if ! x86_64-w64-mingw32-g++ "${DXFLAGS[@]}" "$DXSRC" -o "$PROBES/kf_dxprobe.exe" -ld3d11 -ld3d12 -ldxgi -ldxguid >> "$EV/probes_build.txt" 2>&1; then
    # Ubuntu 22.04's mingw lacks D3D_FEATURE_LEVEL_12_2 (traces/tur1_20261027/README.md)
    if ! x86_64-w64-mingw32-g++ "${DXFLAGS[@]}" -D'D3D_FEATURE_LEVEL_12_2=((D3D_FEATURE_LEVEL)0xc200)' "$DXSRC" -o "$PROBES/kf_dxprobe.exe" \
        -ld3d11 -ld3d12 -ldxgi -ldxguid >> "$EV/probes_build.txt" 2>&1; then
        verdict UNATTRIBUTED OPERATOR "kf_dxprobe did not build: $(tail -2 "$EV/probes_build.txt" | tr '\n' ' ')"
        finish
    fi
fi
rm -f "$PROBES/libnvcuda.a"
(cd "$PROBES" && sha256sum ./*.exe > "$EV/probes.sha256")
say "GATE_NOTE probes built: $(cut -c1-12 "$EV/probes.sha256" | tr '\n' ' ')"
stage_end

# ------------------------------------------------------------------------------------------------
# boot 1: the installed disk on kf3, no NVIDIA driver yet; the driver goes in with GSP forced, device left disabled
# ------------------------------------------------------------------------------------------------
stage_begin boot1
if ! bash "$WV" sign > "$EV/sign.txt" 2>&1; then
    verdict UNATTRIBUTED OPERATOR "win_vm.sh sign failed: $(tail -2 "$EV/sign.txt" | tr '\n' ' ')"
    finish
fi
GUEST_STARTED=1
if ! bash "$WV" run --detach --overlay "$TAG" --arm "$ARM" --guest-driver "$GDRV" --env-file "$FLAGS" --tag "$TAG" > "$EV/run_launch.txt" 2>&1 < /dev/null; then
    fail_op "win_vm.sh run did not start: $(tail -2 "$EV/run_launch.txt" | tr '\n' ' ')"
fi
sleep 5
wait_ssh "$(cap 900)" || fail_guest "the guest never answered ssh after boot 1 (900 s); run log: $(tail -2 "$VM/logs/${TAG}_run.log" 2> /dev/null | tr '\n' ' ')"
stage_end

stage_begin check_kf3
bash "$WV" check --kf3 > "$EV/check_before_driver.txt" 2>&1
if ! grep -q 'WIN_CHECK_DONE fail=0' "$EV/check_before_driver.txt"; then
    fail_op "check.ps1 --kf3 before the driver: $(grep -a ' FAIL ' "$EV/check_before_driver.txt" | head -3 | tr '\n' ' ')"
fi
say "GATE_NOTE check --kf3 (before the driver): $(grep -c ' PASS ' "$EV/check_before_driver.txt") PASS lines, WIN_CHECK_DONE fail=0"
stage_end

stage_begin nv_install
bash "$WV" nv-install --no-enable > "$EV/nv_install.txt" 2>&1
grep -a 'GSP_ON' "$EV/nv_install.txt" | head -20 | while read -r l; do say "  $l"; done
rb=$(grep -ac 'readback .*EnableGpuFirmware=1' "$EV/nv_install.txt")
if [ "$rb" -lt 2 ]; then
    fail_op "the GSP keys did not read back as 1 (class and service key; got $rb): $(grep -a 'readback\|REFUSED\|WARN' "$EV/nv_install.txt" | head -4 | tr '\n' ' ')"
fi
stage_end

stage_begin desktop_prep
if ! bash "$WV" desktop-prep > "$EV/desktop_prep.txt" 2>&1; then
    fail_op "desktop-prep: $(grep -a 'DESKTOP_PREP' "$EV/desktop_prep.txt" | grep -v ' OK$' | head -3 | tr '\n' ' ')"
fi
grep -a 'DESKTOP_PREP' "$EV/desktop_prep.txt" | while read -r l; do say "  $l"; done
stage_end

# ------------------------------------------------------------------------------------------------
# boot 2: a fresh QEMU (kf3 has no reset path); autologon brings the desktop up; then the enable, once
# ------------------------------------------------------------------------------------------------
stage_begin reboot
g 60 'shutdown /r /t 0' > /dev/null 2>&1
w=0
until grep -q "QEMU_START tag=$TAG boot=2" "$VM/logs/${TAG}_run.log" 2> /dev/null; do
    sleep 5
    w=$((w + 5))
    [ "$w" -lt "$(cap 420)" ] || fail_guest "the guest did not restart within 420 s of shutdown /r (no second QEMU)"
done
say "GATE_NOTE second QEMU started after ${w}s"
wait_ssh "$(cap 900)" || fail_guest "the guest never answered ssh after the reboot (900 s)"
stage_end

stage_begin desktop
guest_scp "$WIN/gate_state.ps1" 'C:/kf/gate_state.ps1' || fail_op "scp of gate_state.ps1 failed"
guest_scp "$WIN/gate_run.ps1" 'C:/kf/gate_run.ps1' || fail_op "scp of gate_run.ps1 failed"
ok=0
w=0
while [ "$w" -lt "$(cap 420)" ]; do
    gps 120 'C:\kf\gate_state.ps1' -Mode desktop > "$EV/desktop_state.txt" 2>&1
    if grep -aqE "^GATE_DESKTOP user='[^']*\\\\$KF_GUEST_USER' dwm=[1-9][0-9]* explorer=[1-9]" "$EV/desktop_state.txt"; then ok=1; break; fi
    sleep 10
    w=$((w + 10))
done
grep -a '^GATE_' "$EV/desktop_state.txt" | while read -r l; do say "  $l"; done
[ "$ok" = 1 ] || fail_guest "no signed-in desktop for $KF_GUEST_USER (dwm + explorer) after ${w}s"
bash "$WV" shot "${TAG}_desktop" > "$EV/shot_desktop.txt" 2>&1 || true
stage_end

stage_begin enable
# the enable instant is stamped IN THE GUEST (its clock decides the event window); the enable is issued once
guest_scp "$WIN/gate_enable.ps1" 'C:/kf/gate_enable.ps1' || fail_op "scp of gate_enable.ps1 failed"
gps 180 'C:\kf\gate_enable.ps1' > "$EV/enable_once.txt" 2>&1
ENABLE_UTC=$(sed -n 's/^GATE_ENABLE utc=\([0-9TZ:-]*\) .* rc=0$/\1/p' "$EV/enable_once.txt" | tail -1)
[ -n "$ENABLE_UTC" ] || fail_op "the one-time device enable was not issued: $(tail -2 "$EV/enable_once.txt" | tr '\n' ' ')"
say "GATE_NOTE device enabled once at $ENABLE_UTC (guest clock)"
code=""
w=0
while [ "$w" -lt "$(cap 240)" ]; do
    gps 120 'C:\kf\gate_state.ps1' -Mode display > "$EV/display_after_enable.txt" 2>&1
    code=$(sed -n "s/^GATE_DISPLAY name='[^']*' driver=[^ ]* code=\([0-9]*\) .*/\1/p" "$EV/display_after_enable.txt" | tail -1)
    [ "$code" = 0 ] && break
    sleep 10
    w=$((w + 10))
done
grep -a '^GATE_DISPLAY' "$EV/display_after_enable.txt" | while read -r l; do say "  $l"; done
[ "$code" = 0 ] || fail_guest "the NVIDIA adapter did not reach ConfigManagerErrorCode 0 within ${w}s of the enable (code=${code:-none})"
grep -aq "driver=32.0.15.8088" "$EV/display_after_enable.txt" || fail_op "the adapter driver is not 32.0.15.8088 (580.88): $(grep -a GATE_DISPLAY "$EV/display_after_enable.txt" | head -1)"
stage_end

# ------------------------------------------------------------------------------------------------
# acceptance: nvidia-smi, the probes, a bounded idle, the probes again, the reset counters
# ------------------------------------------------------------------------------------------------
smi() { # SUFFIX
    g 180 'C:\Windows\System32\nvidia-smi.exe -L' 2>&1 | tr -d '\r' > "$EV/nvidia_smi_L_guest$1.txt"
    g 180 'C:\Windows\System32\nvidia-smi.exe -q' 2>&1 | tr -d '\r' > "$EV/nvidia_smi_q_guest$1.txt"
    grep -aq 'GPU 0:' "$EV/nvidia_smi_L_guest$1.txt" && grep -aqE 'Driver Version +: 580\.88' "$EV/nvidia_smi_q_guest$1.txt"
}
probes_round() { # SUFFIX  -> sets PROBE_BAD (graded: cup8 + d3d11; reported: cup3 + d3d12)
    local sfx=$1 line
    PROBE_BAD=""
    if [ -z "$sfx" ]; then
        g 60 'New-Item -ItemType Directory -Force -Path C:\kf\probes | Out-Null' > /dev/null 2>&1
        guest_scp "$PROBES/." 'C:/kf/probes/' > /dev/null 2>&1 || { PROBE_BAD="the-scp-of-the-probes "; return; }
    fi
    line=$(run_probe cup8 cup8.exe '' 180 "$EV/probe_cup8$sfx.txt")
    if ! { grep -aq 'CUP8 VERDICT: PASS' "$EV/probe_cup8$sfx.txt" && grep -aq 'bad=0 maxerr=0' "$EV/probe_cup8$sfx.txt" && [[ "$line" == *"rc=0 "* ]]; }; then PROBE_BAD+="cup8 "; fi
    line=$(run_probe d3d11 kf_dxprobe.exe d3d11 180 "$EV/probe_d3d11$sfx.txt")
    if ! { grep -aq 'KFDX RESULT OK' "$EV/probe_d3d11$sfx.txt" && [[ "$line" == *"rc=0 "* ]]; }; then PROBE_BAD+="d3d11 "; fi
    run_probe cup3 cup3.exe '' 120 "$EV/probe_cup3$sfx.txt" > /dev/null
    run_probe d3d12 kf_dxprobe.exe d3d12 180 "$EV/probe_d3d12$sfx.txt" > /dev/null
}

stage_begin smi_probes
smi "" || fail_guest "nvidia-smi in the guest: no GPU 0, or not driver 580.88 (nvidia_smi_L_guest.txt)"
say "GATE_NOTE guest nvidia-smi: $(head -1 "$EV/nvidia_smi_L_guest.txt")"
probes_round ""
[ -z "$PROBE_BAD" ] || fail_guest "graded probes failed: ${PROBE_BAD}(probe_*.txt)"
stage_end

stage_begin idle
w=0
miss=0
last_ssh=0
while [ "$w" -lt "$IDLE_S" ] && [ "$(left)" -gt 120 ]; do
    sleep 15
    w=$((w + 15))
    qemu_alive || fail_guest "QEMU exited during the idle window after ${w}s"
    if grep -q GUEST_PANICKED "$VM/logs/${TAG}_b2_qmp_events.jsonl" 2> /dev/null; then fail_guest "GUEST_PANICKED during the idle window after ${w}s"; fi
    if [ $((w - last_ssh)) -ge 60 ]; then
        last_ssh=$w
        if g 100 'echo KF_SSH_UP' 2> /dev/null | grep -q KF_SSH_UP; then
            miss=0
        else
            miss=$((miss + 1))
            say "GATE_NOTE ssh miss $miss at ${w}s of the idle window"
        fi
        [ "$miss" -lt 3 ] || fail_guest "the guest stopped answering ssh during the idle window (3 misses, ${w}s)"
    fi
done
say "GATE_NOTE idle window observed ${w}s of ${IDLE_S}s"
[ "$w" -ge "$IDLE_S" ] || fail_op "the budget ended the idle window early (${w}s of ${IDLE_S}s)"
stage_end

stage_begin post_idle
smi "_post" || fail_guest "nvidia-smi after the idle window failed (nvidia_smi_*_guest_post.txt)"
probes_round "_post"
[ -z "$PROBE_BAD" ] || fail_guest "graded probes failed AFTER the idle window: ${PROBE_BAD}(probe_*_post.txt)"
gps 300 'C:\kf\gate_state.ps1' -Mode tdr -Since "$ENABLE_UTC" > "$EV/tdr_counts.txt" 2>&1
grep -a '^GATE_TDR' "$EV/tdr_counts.txt" | while read -r l; do say "  $l"; done
grep -aq '^GATE_TDR .* total=0$' "$EV/tdr_counts.txt" || fail_guest "reset counters are not all zero since the enable: $(grep -a '^GATE_TDR' "$EV/tdr_counts.txt" | head -1)"
gps 120 'C:\kf\gate_state.ps1' -Mode display > "$EV/display_final.txt" 2>&1
grep -aq "code=0 " "$EV/display_final.txt" || fail_guest "the adapter is not at code 0 at the end: $(grep -a GATE_DISPLAY "$EV/display_final.txt" | head -1)"
bash "$WV" shot "${TAG}_final" > "$EV/shot_final.txt" 2>&1 || true
bash "$WV" check --kf3 > "$EV/check_after_driver.txt" 2>&1 || true
say "GATE_NOTE check --kf3 (after the driver): $(grep -a 'WIN_' "$EV/check_after_driver.txt" | grep -c ' FAIL ') FAIL line(s), $(grep -a 'WIN_CHECK_DONE' "$EV/check_after_driver.txt" | tr -d '\r' | tail -1)"
stage_end

stage_begin collect_stop
bash "$WV" collect "${TAG}_final" > "$EV/collect.txt" 2>&1 || say "GATE_NOTE collect returned non-zero (the guest evidence may be partial)"
stage_end
STAGE="done"
conclude
