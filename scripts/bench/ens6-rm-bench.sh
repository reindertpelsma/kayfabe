#!/usr/bin/env bash
# ENS6 host RM cost benchmark: N live VA reservation objects (sparse 64 KiB / normal 2 MiB) and the
# release+re-reserve churn of the ENS5 recycling pattern. NO VM; an own unprivileged RM client.
#
#   ens6-rm-bench.sh OUT_ROOT ALLOC_BINARY CHURN_BINARY
#
# Preconditions (checked, and the script exits 3 / 4 / 5 without touching the GPU if they fail):
#   * the host lock /tmp/kayfabe-fastguest.lock is taken exactly as the lane scripts take it
#     (flock -n; the whole sequence holds it), so no VM can start meanwhile;
#   * no qemu process, no compute process on the GPU;
#   * the desktop owner is idle >= 10 min (mutter IdleMonitor, asked as the desktop user).
# Every arm runs as the desktop uid with ZERO capabilities and no new privileges, under a hard
# timeout, with the desktop watchdog (one mutter D-Bus call, < 3 s) between samples. The sequence
# aborts at the first NEW host Xid, a failed watchdog or a timeout. GPU memory before/after each
# arm is recorded (nvidia-smi). Refusals are DATA (the arm prints its first refusal), not aborts.
set -u
if [[ ${1:-} != --locked ]]; then
    [[ $# == 3 ]] || { echo "usage: $0 OUT_ROOT ALLOC_BINARY CHURN_BINARY" >&2; exit 2; }
    exec flock -n -E 3 /tmp/kayfabe-fastguest.lock "$0" --locked "$@"
fi
shift
OUT=$1 ALLOC=$2 CHURN=$3
UIDN=1000
mkdir -p "$OUT" && chown "$UIDN:$UIDN" "$OUT" || exit 2
log() { printf '%s %s\n' "$(date -Is)" "$*" | tee -a "$OUT/sequence.log"; }

pgrep -x 'qemu-system-x86_64' >/dev/null && { log "ABORT vm running"; exit 4; }
apps=$(nvidia-smi --query-compute-apps=pid --format=csv,noheader 2>/dev/null | grep -c .)
[[ $apps == 0 ]] || { log "ABORT $apps compute apps on the GPU"; exit 4; }

BUS="unix:path=/run/user/$UIDN/bus"
idle_ms() {
    setpriv --reuid=$UIDN --regid=$UIDN --init-groups env DBUS_SESSION_BUS_ADDRESS="$BUS" \
        timeout 3 gdbus call --session --dest org.gnome.Mutter.IdleMonitor \
        --object-path /org/gnome/Mutter/IdleMonitor/Core \
        --method org.gnome.Mutter.IdleMonitor.GetIdletime 2>/dev/null | grep -o '[0-9]\+' | head -1
}
idle=$(idle_ms)
[[ -n $idle && $idle -ge 600000 ]] || { log "ABORT owner not idle or compositor silent: idle_ms=${idle:-none}"; exit 5; }
log "preflight ok: no vm, no compute apps, owner idle ${idle} ms"

cat >"$OUT/watchdog.sh" <<EOF
#!/bin/sh
exec env DBUS_SESSION_BUS_ADDRESS="$BUS" timeout 2 gdbus call --session \
    --dest org.gnome.Mutter.IdleMonitor --object-path /org/gnome/Mutter/IdleMonitor/Core \
    --method org.gnome.Mutter.IdleMonitor.GetIdletime
EOF
chmod 755 "$OUT/watchdog.sh"
start_epoch=$(date +%s)
xids() { journalctl -k --since "@$start_epoch" 2>/dev/null | grep -c 'NVRM: Xid'; }
smi() { nvidia-smi --query-gpu=memory.used,memory.free --format=csv,noheader 2>&1; }
base_xids=$(xids)

run_arm() { # NAME ARGS... (the binary is the first of ARGS)
    local name=$1; shift
    local dir="$OUT/$name"
    smi >"$OUT/$name.smi-before"
    log "arm $name: $*"
    local t0=$(date +%s.%N)
    setpriv --reuid=$UIDN --regid=$UIDN --clear-groups --inh-caps=-all --bounding-set=-all \
        --no-new-privs env -i PATH=/usr/bin:/bin HOME=/nonexistent \
        timeout --signal=TERM --kill-after=10s 300s "$@" "$dir" "$OUT/watchdog.sh" \
        >"$OUT/$name.out" 2>"$OUT/$name.err"
    local rc=$?
    local t1=$(date +%s.%N)
    smi >"$OUT/$name.smi-after"
    echo "$name rc=$rc wall_s=$(awk -v a="$t0" -v b="$t1" 'BEGIN{print b-a}')" >>"$OUT/arms.txt"
    log "arm $name rc=$rc: $(grep -h 'ENS[56]_RM' "$OUT/$name.out" | head -1)"
    if (( $(xids) > base_xids )); then log "ABORT new host Xid after $name"; exit 6; fi
    if [[ $rc == 124 || $rc == 137 ]]; then log "ABORT timeout in $name"; exit 7; fi
    [[ -n $(idle_ms) ]] || { log "ABORT desktop compositor silent after $name"; exit 8; }
}

for n in 1000 8000 32000 128000; do
    for kind in sparse normal; do
        # the alloc binary takes: --run-native N KIND DIR WATCHDOG (DIR and WATCHDOG appended)
        run_arm "alloc-$kind-$n" "$ALLOC" --run-native "$n" "$kind"
    done
done
# churn: the saturated tree of 7330 (8190 live blocks) and neighbours, 2000 merge/split cycles
for n in 0 1000 8190 32000; do
    run_arm "churn-$n" "$CHURN" --run-churn "$n" 2000
done
log "sequence complete; new Xids: $(( $(xids) - base_xids ))"
