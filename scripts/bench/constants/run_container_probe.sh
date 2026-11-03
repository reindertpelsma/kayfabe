#!/bin/bash
# Rent ONE cheap container box (unprivileged by default: exactly kayfabe's tier-2 position), run the tier-2 kit,
# bring the text back, destroy the box, verify. Lane C, 2026-10-14.
# Usage: run_container_probe.sh <offer_id> <label> <kit_dir> <out_file>
# Rules (scripts/bench/box/README.md, night plan): own ids only; never print instance_api_key; arm the auto-destroy
# right after create; no secrets on the box; only text comes back; destroy as soon as the evidence is in.
set -u
offer=$1; label=$2; kit=$3; out=$4
img=${PROBE_IMAGE:-nvidia/cuda:12.4.1-base-ubuntu22.04}
log() { echo "[$(date +%H:%M:%S)] $*" >&2; }
id=$(vastai create instance "$offer" --image "$img" --disk 8 --ssh --direct --label "$label" --raw 2>/dev/null \
  | python3 -I -c 'import json,sys; d=json.loads(sys.stdin.read()); print(d.get("new_contract") if d.get("success") else "")')
[ -n "$id" ] || { log "create failed for offer $offer"; exit 2; }
log "created $id (offer $offer)"
/root/night/arm_autodestroy.sh "$id" 45 >&2
cleanup() {
  vastai destroy instance "$id" -y >/dev/null 2>&1
  sleep 5
  if vastai show instances --raw 2>/dev/null | python3 -I -c "import json,sys; sys.exit(0 if not any(str(i['id'])=='$id' for i in json.load(sys.stdin)) else 1)"; then
    log "destroyed $id (verified)"; [ -f "/root/night/autodestroy-$id.pid" ] && kill "$(cat /root/night/autodestroy-$id.pid)" 2>/dev/null
  else
    log "destroy of $id NOT verified yet (auto-destroy stays armed)"
  fi
}
trap cleanup EXIT
host=""; port=""
for i in $(seq 1 60); do
  read -r st host port < <(vastai show instance "$id" --raw 2>/dev/null | python3 -I -c '
import json,sys
d=json.load(sys.stdin)
print(d.get("actual_status"), d.get("ssh_host") or "-", d.get("ssh_port") or "-")')
  [ "$st" = "running" ] && [ "$host" != "-" ] && break
  sleep 10
done
[ "$st" = "running" ] || { log "$id never reached running ($st)"; exit 3; }
log "$id running at $host:$port"
S="ssh -o StrictHostKeyChecking=no -o UserKnownHostsFile=/dev/null -o ConnectTimeout=15 -o LogLevel=ERROR -p $port root@$host"
for i in $(seq 1 20); do $S true 2>/dev/null && break; sleep 10; done
scp -q -o StrictHostKeyChecking=no -o UserKnownHostsFile=/dev/null -o LogLevel=ERROR -P "$port" -r "$kit" "root@$host:/root/kit" || { log "scp failed"; exit 4; }
$S 'sh /root/kit/tier2_probe_box.sh' > "$out" 2>&1
log "probe rc=$? lines=$(wc -l < "$out")"
grep -q BOX_PROBE_END "$out" || log "WARNING: no BOX_PROBE_END line"
exit 0
