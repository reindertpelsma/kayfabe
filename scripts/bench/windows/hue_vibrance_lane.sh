#!/usr/bin/env bash
# SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later
# hue_vibrance_lane.sh — the owner's manual PR #92 check, automated: on the Windows 11 guest
# (580.88, production display flags) move the NVIDIA app's / NVIDIA Control Panel's Hue and Digital
# Vibrance sliders, and after each setting assert that
#   * the console still updates (the test pattern's frame counter changes between two QMP
#     screendumps; run 4500 froze at 0 fps),
#   * no `scanout REFUSED SDR colour program` (the Fault::Console of run 4500) and no display STALL /
#     GPU fault line appears, no host Xid, and the guest logs no nvlddmkm 153 / Display 4101 (TDR),
#   * the screendump's test-pattern patches equal the reference math applied to the coefficients the
#     driver really programmed (kf-qemu's `colour program #N` line), within ±2 levels,
# for HV_HOLD_MIN minutes per setting (samples every 15 s).
#
# Usage (trusted GPU host only; strictly one VM at a time):
#   scripts/bench/windows/hue_vibrance_lane.sh --rev <kf3-rev8> --run <N> [--out DIR] [--hold-min M]
#        [--settings "vibrance:0 vibrance:100 hue:30 hue:120 hue:210 hue:330 hue:0 vibrance:50"]
#        [--discover-only] [--wait SECONDS]
#
# The setting route is UI Automation (Microsoft's documented accessibility API) on the vendor's own
# user control, run in the auto-logon user's session; there is NO supported CLI/API for these two
# controls: the public NVAPI SDK (github.com/NVIDIA/nvapi, MIT) has no DVC/HUE entry points
# (MEASURED 2026-10-24: grep of its nvapi.h), and nothing here uses private NvAPI ids, the registry
# or driver internals (no reverse engineering). The slider names and navigation are INFERRED until a
# run's discover dump (`$OUT/discover.json`) confirms them; when the sliders are not found the
# verdict is ROUTE_UNAVAILABLE (exit 4), never PASS.
#
# Preconditions (each refusal is a verdict, not a crash): no /tmp/kf-host-quiet, $KF_LOCK free
# (flock -w WAIT, default 0), NO other qemu-system process (the owner's or another lane's: the kfagent
# queue is shared and host RAM is 29 GiB), MemAvailable >= 16 GiB, the owner overlay not open, the kf3
# binary for --rev installed. The disk is a THROW-AWAY child overlay of the owner's overlay (never
# the owner's own file); the VM is stopped by its own run number; GDM is not touched.
#
# Result: $OUT/result.json {"lane":"hue_vibrance","verdict":PASS|FAIL|BUSY|ERROR|ROUTE_UNAVAILABLE|
# INCONCLUSIVE, "settings":[...]} and RESULT lines; start marker HV_LANE_START, exit line HV_LANE_EXIT.
# Exit: 0 PASS, 1 FAIL, 2 ERROR, 3 INCONCLUSIVE, 4 ROUTE_UNAVAILABLE, 75 BUSY.
set -uo pipefail
HERE="$(cd "$(dirname "$0")" && pwd)"
REPO="$(cd "$HERE/../../.." && pwd)"
W=${HV_W:-/var/lib/kf-windows-20261005}
RUNNER=${HV_RUNNER:-/data/claude-gap/broker_gap_io.sh}
KFAGENT=${HV_KFAGENT:-/data/kfagent_server.py}
D=${HV_IMAGE_DIR:-$W/win11-full-b}
OWNER_OVERLAY=${HV_OWNER_OVERLAY:-/data/claude-owner-overlay}
LOCK=${KF_LOCK:-/tmp/kayfabe-fastguest.lock}
REV="" N="" OUT="" HOLD_MIN=${HV_HOLD_MIN:-2} WAIT=0 DISCOVER_ONLY=0
SETTINGS="vibrance:0 vibrance:100 hue:30 hue:120 hue:210 hue:330 hue:0 vibrance:50"
while [ $# -gt 0 ]; do
    case "$1" in
        --rev) REV=$2; shift 2 ;;
        --run) N=$2; shift 2 ;;
        --out) OUT=$2; shift 2 ;;
        --hold-min) HOLD_MIN=$2; shift 2 ;;
        --settings) SETTINGS=$2; shift 2 ;;
        --discover-only) DISCOVER_ONLY=1; shift ;;
        --wait) WAIT=$2; shift 2 ;;
        *) echo "usage: hue_vibrance_lane.sh --rev REV --run N [--out DIR] [--hold-min M] [--settings S] [--discover-only] [--wait S]" >&2; exit 2 ;;
    esac
done
[ -n "$REV" ] && [ -n "$N" ] || { echo "hue_vibrance_lane: --rev and --run are required" >&2; exit 2; }
OUT=${OUT:-/data/claude-autochk92/hv-$N-$(date +%Y%m%d-%H%M%S)}
mkdir -p "$OUT" || exit 2
LOG="$OUT/lane.log"
RUN=$W/boundary-kayfabe-$N
VERDICT=ERROR REASON="" STARTED_VM=0
SET_JSON=()
say() { echo "$*" | tee -a "$LOG"; }
finish() {
    if [ "$STARTED_VM" = 1 ]; then
        say "HV_STOP run=$N"
        bash "$RUNNER" stop "$N" >> "$LOG" 2>&1 || say "HV_STOP_FAILED run=$N (see lane.log)"
        [ "${HV_KEEP_DISK:-0}" = 1 ] || rm -f "$RUN/windows.qcow2"
    fi
    local rc
    case "$VERDICT" in PASS) rc=0 ;; FAIL) rc=1 ;; INCONCLUSIVE) rc=3 ;; ROUTE_UNAVAILABLE) rc=4 ;; BUSY) rc=75 ;; *) rc=2 ;; esac
    local xid1
    xid1=$(journalctl -k --no-pager 2>/dev/null | grep -c 'NVRM: Xid')
    local IFS=,
    printf '{"lane":"hue_vibrance","classification":"MEASURED","verdict":"%s","reason":"%s","rev":"%s","run":"%s","checkout":"%s","hold_min":%s,"host_xid_delta":%s,"settings":[%s],"out":"%s","at":"%s"}\n' \
        "$VERDICT" "${REASON//\"/\'}" "$REV" "$N" "$(git -C "$REPO" rev-parse HEAD 2>/dev/null)" "$HOLD_MIN" \
        "$(( ${xid1:-0} - ${XID0:-${xid1:-0}} ))" "${SET_JSON[*]}" "$OUT" "$(date -Is)" > "$OUT/result.json"
    unset IFS
    say "RESULT lane=hue_vibrance verdict=$VERDICT rev=$REV run=$N reason=$REASON"
    say "HV_LANE_EXIT rc=$rc $(date -Is)"
    echo "HV_LANE_EXIT rc=$rc $(date -Is)" > "$OUT/EXIT"
    exit $rc
}
trap finish EXIT
refuse() { VERDICT=$1; REASON=$2; say "HV_REFUSED $1: $2"; exit 0; }
say "HV_LANE_START rev=$REV run=$N checkout=$(git -C "$REPO" rev-parse --short=9 HEAD 2>/dev/null) out=$OUT $(date -Is)"
echo "HV_LANE_START rev=$REV run=$N $(date -Is)" > "$OUT/START"

# ── preconditions ─────────────────────────────────────────────────────────────────────────────
[ -e /tmp/kf-host-quiet ] && refuse BUSY "host quiet window: $(cat /tmp/kf-host-quiet)"
exec 9>"$LOCK"
flock -w "$WAIT" 9 || refuse BUSY "$LOCK is held by another GPU job"
export KF_LOCKED=1
others=$(pgrep -a qemu-system | cut -c1-160)
[ -z "$others" ] || refuse BUSY "another QEMU runs (one VM at a time): $others"
avail=$(awk '/MemAvailable/ {print int($2/1048576)}' /proc/meminfo)
[ "${avail:-0}" -ge 16 ] || refuse BUSY "MemAvailable ${avail} GiB < 16 GiB"
[ -x "$W/kf3-bins/$REV/qemu-system-x86_64" ] || refuse ERROR "no kf3 binary $W/kf3-bins/$REV (build_kf3.sh)"
[ -x "$RUNNER" ] || [ -f "$RUNNER" ] || refuse ERROR "no runner $RUNNER"
[ -f "$KFAGENT" ] || refuse ERROR "no kfagent server $KFAGENT"
ss -ltn 2>/dev/null | grep -q '127.0.0.1:8765' || refuse ERROR "the kfagent server is not listening on 127.0.0.1:8765"
for f in "$OWNER_OVERLAY/windows-overlay.qcow2" "$D/windows11.qcow2"; do
    [ -f "$f" ] || refuse ERROR "missing $f"
    fuser -s "$f" 2>/dev/null && refuse BUSY "$f is open (the owner's VM?)"
done
[ -e "$RUN" ] && refuse ERROR "$RUN exists (choose a fresh --run)"
XID0=$(journalctl -k --no-pager 2>/dev/null | grep -c 'NVRM: Xid')
say "HV_PRECONDITIONS ok mem_avail=${avail}GiB xid_before=$XID0"

# ── the VM: the owner launcher's production profile on a throw-away child overlay ───────────────
mkdir -m 0700 "$RUN" || refuse ERROR "mkdir $RUN"
qemu-img create -q -f qcow2 -F qcow2 -b "$OWNER_OVERLAY/windows-overlay.qcow2" "$RUN/windows.qcow2" || refuse ERROR "qemu-img"
cp "$OWNER_OVERLAY/OVMF_VARS.fd" "$RUN/OVMF_VARS.fd" || refuse ERROR "OVMF vars"
# shellcheck disable=SC1091
. "$D/identity.env"
while IFS= read -r name; do unset "$name"; done < <(compgen -e | sed -n '/^KF3_/p')
export BROKER_BIN=${HV_BROKER_BIN:-/data/claude-cursor9/nvkvm-display-broker}
export WIN_CPU="host,-vmx,migratable=off,+invtsc,hv_relaxed,hv_vapic,hv_spinlocks=0x1fff,hv_vpindex,hv_runtime,hv_time,hv_synic,hv_stimer,hv_tlbflush,hv_ipi,hv_frequencies"
export WIN_NET_RESTRICT=off WIN_MAC=$VM_MAC WIN_GPU_UUID=$GPU_UUID
export KF3_REV=$REV WIN_TRACE=0 WIN_GSP_OBSERVER=0 WIN_FLAGS="${HV_WIN_FLAGS:-KF3_LIFECYCLE_COUNT=1 KF3_LIFECYCLE_PERIOD_S=5}" \
    PROD_EXTRA_FLAGS='' WIN_DROP="KF3_TIMER_MAP" WIN_FB_MB=8192 WIN_RAM_MB=12288 WIN_BAR1=8589934592 WIN_REUSE=1
export WIN_QEMU_EXTRA="-uuid $VM_UUID -smbios type=1,manufacturer=KayfabeLab,product=KF-Guest,serial=$VM_SERIAL"
export WIN_TITLE="kayfabe hue/vibrance lane (run $N, $REV)" WIN_MAX_SECONDS=0
STARTED_VM=1
bash "$RUNNER" run "$N" >> "$LOG" 2>&1 || refuse ERROR "the runner failed to start run $N"
QPID=$(sed -n 's/^QPID=//p' "$W/windows-broker-run$N.state")
Q="$RUN/qemu.log"
say "HV_VM_UP qpid=$QPID run=$RUN"
grep -aq 'KF3_DISPLAY_LUT_MIRROR=1' "$Q" 2>/dev/null || sleep 5
for flag in KF3_DISPLAY_SDR_COLOR KF3_DISPLAY_LUT_MIRROR KF3_DISPLAY_ILUT_OFFSET_256; do
    grep -aq "\"$flag\"\|$flag=1\|$flag" "$RUN/command.json" 2>/dev/null || refuse ERROR "production flag $flag missing from command.json"
done

qmp() { # qmp <json command>: one QMP command on the run's socket
    python3 - "$RUN/qmp.sock" "$1" <<'PY'
import json, socket, sys
s = socket.socket(socket.AF_UNIX); s.settimeout(20); s.connect(sys.argv[1]); f = s.makefile("rw")
json.loads(f.readline()); f.write('{"execute":"qmp_capabilities"}\n'); f.flush()
while "return" not in json.loads(f.readline()): pass
f.write(sys.argv[2] + "\n"); f.flush()
while True:
    r = json.loads(f.readline())
    if "return" in r or "error" in r:
        print(json.dumps(r)); sys.exit(0 if "return" in r else 1)
PY
}
shot() { # shot <file.ppm>
    qmp "{\"execute\":\"screendump\",\"arguments\":{\"filename\":\"$1\",\"device\":\"kf0\"}}" > /dev/null && [ -s "$1" ]
}
agent() { # agent <powershell> [timeout]
    timeout $(( ${2:-60} + 15 )) python3 "$KFAGENT" run "$1" "${2:-60}"
}

# guest up: kfagent answers, the console is registered, the NVIDIA adapter is the display
ready=0
for _ in $(seq 1 120); do
    kill -0 "$QPID" 2>/dev/null || refuse ERROR "QEMU exited during boot"
    if grep -aq 'kf3: display console registered' "$Q" && agent 'hostname' 15 2>/dev/null | grep -q .; then ready=1; break; fi
    sleep 5
done
[ "$ready" = 1 ] || refuse ERROR "the guest never answered on kfagent (10 min)"
sleep 20 # auto-logon, desktop
LANE_T0=$(date -Is)
agent "(Get-PnpDevice -Class Display | Where-Object FriendlyName -match 'NVIDIA' | Select-Object -First 1).Status" 30 | tee -a "$LOG" | grep -q OK \
    || say "HV_WARN the NVIDIA display device is not OK (Code 43?)"

# the guest helper and a runner for the interactive session
b64() { base64 -w0 < "$1"; }
agent "New-Item -ItemType Directory -Force C:\\kf\\hv | Out-Null; [IO.File]::WriteAllBytes('C:\\kf\\hv\\hv.ps1',[Convert]::FromBase64String('$(b64 "$HERE/hue_vibrance_guest.ps1")')); 'OK'" 60 | grep -q OK \
    || refuse ERROR "could not copy the guest helper"
RUNI='param([string]$Name,[string]$Cmd,[string]$User="kfadmin")
$a=New-ScheduledTaskAction -Execute "cmd.exe" -Argument ("/c "+$Cmd)
$p=New-ScheduledTaskPrincipal -UserId $User -LogonType Interactive -RunLevel Limited
Unregister-ScheduledTask -TaskName "kfhv-$Name" -Confirm:$false -ErrorAction SilentlyContinue
Register-ScheduledTask -TaskName "kfhv-$Name" -Action $a -Principal $p | Out-Null
Start-ScheduledTask -TaskName "kfhv-$Name"; "STARTED $Name"'
agent "[IO.File]::WriteAllText('C:\\kf\\hv\\runi.ps1', [Text.Encoding]::UTF8.GetString([Convert]::FromBase64String('$(printf '%s' "$RUNI" | base64 -w0)'))); 'OK'" 30 | grep -q OK \
    || refuse ERROR "could not write the interactive runner"
interactive() { # interactive <name> <hv.ps1 args...>: run in the user's session, wait for its json
    local name=$1; shift
    agent "Remove-Item -ErrorAction SilentlyContinue C:\\kf\\hv\\$name.json, C:\\kf\\hv\\error-*.json; & C:\\kf\\hv\\runi.ps1 -Name $name -Cmd 'powershell -NoProfile -ExecutionPolicy Bypass -File C:\\kf\\hv\\hv.ps1 $*'" 30 > /dev/null
    for _ in $(seq 1 30); do
        sleep 2
        local r
        r=$(agent "if (Test-Path C:\\kf\\hv\\$name.json) { Get-Content C:\\kf\\hv\\$name.json } elseif (Test-Path C:\\kf\\hv\\error-$1.json) { 'ERROR ' + (Get-Content C:\\kf\\hv\\error-$1.json) }" 20)
        if [ -n "$r" ]; then printf '%s\n' "$r"; return 0; fi
    done
    return 1
}

# the pattern, then the baseline (neutral settings) screendump
interactive pattern pattern > "$OUT/pattern.json" || refuse ERROR "the test pattern did not come up"
sleep 3
shot "$OUT/baseline.ppm" || refuse ERROR "QMP screendump failed"
interactive discover discover > "$OUT/discover.json" 2>&1
say "HV_DISCOVER $(head -c 300 "$OUT/discover.json")"
if [ "$DISCOVER_ONLY" = 1 ]; then VERDICT=INCONCLUSIVE; REASON="discover-only run"; exit 0; fi

overall=PASS
for s in $SETTINGS; do
    what=${s%%:*} value=${s#*:}
    since=$(stat -c %s "$Q")
    r=$(interactive "set-$what" set "$what" "$value")
    printf '%s\n' "$r" > "$OUT/set-$what-$value.json"
    if ! printf '%s' "$r" | grep -q '"found":true'; then
        say "HV_ROUTE_UNAVAILABLE $what=$value: $r"
        VERDICT=ROUTE_UNAVAILABLE; REASON="slider $what not found by UI Automation (see discover.json)"; exit 0
    fi
    sleep 3
    sv=PASS
    samples=$(( HOLD_MIN * 60 / 15 )); [ "$samples" -ge 1 ] || samples=1
    for i in $(seq 0 "$samples"); do
        a="$OUT/$what-$value-$i-a.ppm" b="$OUT/$what-$value-$i-b.ppm"
        if ! shot "$a" || ! { sleep 1.5; shot "$b"; }; then sv=FAIL; say "HV_SAMPLE $what=$value i=$i screendump failed"; break; fi
        g=$(python3 "$HERE/hue_vibrance_check.py" grade --baseline "$OUT/baseline.ppm" --shot "$a" --shot2 "$b" \
            --qemu-log "$Q" --since "$since" --out "$OUT/$what-$value-$i.json")
        grc=$?
        say "HV_SAMPLE $what=$value i=$i $g"
        case $grc in 0) ;; 3) [ "$sv" = PASS ] && sv=INCONCLUSIVE ;; *) sv=FAIL ;; esac
        [ "$i" -gt 0 ] && rm -f "$a" "$b" # keep the first pair of every setting as evidence
        kill -0 "$QPID" 2>/dev/null || { sv=FAIL; say "HV_SAMPLE QEMU died"; break; }
        [ "$i" -lt "$samples" ] && sleep 12
    done
    say "HV_SETTING $what=$value verdict=$sv"
    SET_JSON+=("{\"control\":\"$what\",\"value\":$value,\"verdict\":\"$sv\"}")
    case $sv in FAIL) overall=FAIL ;; INCONCLUSIVE) [ "$overall" = PASS ] && overall=INCONCLUSIVE ;; esac
done

# the guest's own view: no TDR (153 / 4101) since the lane started, and it still answers
agent "powershell -NoProfile -ExecutionPolicy Bypass -File C:\\kf\\hv\\hv.ps1 events '$LANE_T0'; Get-Content C:\\kf\\hv\\events.json" 60 > "$OUT/events.json"
say "HV_EVENTS $(cat "$OUT/events.json")"
grep -q '"nvlddmkm_153":0' "$OUT/events.json" && grep -q '"display_4101":0' "$OUT/events.json" || { overall=FAIL; REASON="guest TDR events (see events.json)"; }
xid1=$(journalctl -k --no-pager 2>/dev/null | grep -c 'NVRM: Xid')
[ "$xid1" = "$XID0" ] || { overall=FAIL; REASON="host Xid delta $((xid1 - XID0))"; }
VERDICT=$overall
[ -n "$REASON" ] || REASON="settings: $SETTINGS"
exit 0
