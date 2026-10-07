#!/usr/bin/env bash
# vast_reaper.sh — destroy THIS SESSION's idle vast boxes.
#
# Scope: only instance ids listed in $REG (one "id alias" per line). The vast
# account is shared with other sessions, so an unregistered id is NEVER touched.
#
# A box counts as BUSY when any of these hold (checked over ssh):
#   - a qemu / cargo / rustc / ffmpeg / python / vulkan / cuda workload process exists
#   - an ssh session other than this probe is connected
#   - load average (1 min) >= 0.5
#   - any file under /workspace or /root was modified in the last $IDLE_MIN minutes
# Idle for $IDLE_MIN minutes in a row (or unreachable that long) => destroyed.
#
# Env: DRY_RUN=1 (print only), IDLE_MIN (default 60), EVERY (poll seconds, default 600),
#      ONCE=1 (one pass, for tests).
set -u
DIR=$(cd "$(dirname "$0")" && pwd)
REG=${REG:-$HOME/.kayfabe/vast_boxes.reg}
STATE=${STATE:-$HOME/.kayfabe/vast_reaper.state}   # "id first_idle_epoch"
LOG=${LOG:-$HOME/.kayfabe/vast_reaper.log}
IDLE_MIN=${IDLE_MIN:-60}
EVERY=${EVERY:-600}
DRY_RUN=${DRY_RUN:-0}
mkdir -p "$(dirname "$REG")"; touch "$REG" "$STATE"

log() { echo "$(date -u +%FT%TZ) $*" | tee -a "$LOG"; }

live_ids() {
  vastai show instances --raw 2>/dev/null |
    python3 -c 'import json,sys; [print(i["id"]) for i in json.load(sys.stdin)]'
}

probe() {  # prints BUSY / IDLE / UNREACHABLE
  local alias=$1
  [ -n "${FAKE_PROBE:-}" ] && { echo "$FAKE_PROBE"; return; }   # test hook only
  timeout 40 ssh -o ConnectTimeout=15 -o BatchMode=yes "$alias" "IDLE_MIN=$IDLE_MIN bash -s" 2>/dev/null <<'EOS' || echo UNREACHABLE
busy=0; why=""
if pgrep -f '[q]emu-system|[c]argo|[r]ustc|[f]fmpeg|[v]ulkan|[n]vcc|[l]lama|[o]llama' >/dev/null; then busy=1; why="$why proc"; fi
if pgrep -af 'python' | grep -v -E 'unattended|networkd-dispatcher|vast' | grep -q .; then busy=1; why="$why python"; fi
n=$(ss -Htn state established '( sport = :22 )' 2>/dev/null | wc -l); [ "$n" -gt 1 ] && { busy=1; why="$why ssh=$n"; }
l=$(cut -d' ' -f1 /proc/loadavg); awk -v l="$l" 'BEGIN{exit !(l>=0.5)}' && { busy=1; why="$why load=$l"; }
if find /workspace /root -xdev -newermt "-${IDLE_MIN} minutes" -type f -print -quit 2>/dev/null | grep -q .; then busy=1; why="$why files"; fi
[ $busy = 1 ] && echo "BUSY$why" || echo IDLE
EOS
}

pass() {
  local live now; live=$(live_ids); now=$(date +%s)
  [ -z "$live" ] && { log "vast list empty or failed; nothing to do"; }
  while read -r id alias _; do
    [ -z "${id:-}" ] && continue
    case $id in \#*) continue;; esac
    if ! grep -qx "$id" <<<"$live"; then log "$id ($alias): not live — skip"; continue; fi
    # An alias with no ~/.ssh/config entry resolves to itself: we cannot probe it, so we must
    # not read "unreachable" as "idle" — skip it loudly instead of destroying a live box.
    if [ "$(ssh -G "$alias" 2>/dev/null | awk '/^hostname /{print $2}')" = "$alias" ]; then
      log "$id ($alias): NO SSH ALIAS configured — cannot probe; NOT counted idle (fix the alias)"
      sed -i "/^$id /d" "$STATE"; continue
    fi
    st=$(probe "$alias" | tail -1)
    first=$(awk -v i="$id" '$1==i{print $2}' "$STATE")
    if [[ $st == BUSY* ]]; then
      sed -i "/^$id /d" "$STATE"; log "$id ($alias): $st"
      continue
    fi
    [ -z "$first" ] && { first=$now; echo "$id $now" >> "$STATE"; }
    idle=$(( (now - first) / 60 ))
    log "$id ($alias): $st for ${idle} min (limit $IDLE_MIN)"
    if [ "$idle" -ge "$IDLE_MIN" ]; then
      if [ "$DRY_RUN" = 1 ]; then log "DRY_RUN: would destroy $id ($alias)"
      else
        log "DESTROY $id ($alias)"; vastai destroy instance "$id" -y >>"$LOG" 2>&1
        sed -i "/^$id /d" "$STATE"
      fi
    fi
  done < "$REG"
}

log "start DRY_RUN=$DRY_RUN IDLE_MIN=$IDLE_MIN EVERY=$EVERY reg=$REG"
while :; do pass; [ "${ONCE:-0}" = 1 ] && break; sleep "$EVERY"; done
