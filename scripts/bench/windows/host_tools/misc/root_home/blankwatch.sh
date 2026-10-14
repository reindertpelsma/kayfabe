#!/usr/bin/env bash
# Read-only: samples the guest's output state until it blanks, then captures why.
exec > /root/blankwatch.log 2>&1
echo "START $(date -u +%FT%TZ)"
prev=""
for i in $(seq 1 120); do          # up to 2h at 1-min samples
    s=$(docker exec nvkvm-steamos-vmm-1 nvkvm-steamos-ssh \
        'printf "%s|%s|%s" "$(cat /sys/class/drm/card0/card0-Virtual-1/enabled 2>/dev/null)" \
                            "$(cat /sys/class/drm/card0/card0-Virtual-1/dpms 2>/dev/null)" \
                            "$(uptime -p)"' 2>/dev/null)
    [ -z "$s" ] && { echo "$(date -u +%H:%M:%SZ) guest unreachable"; sleep 60; continue; }
    if [ "$s" != "$prev" ]; then
        echo "$(date -u +%H:%M:%SZ) CHANGE: $s"
        case "$s" in
            disabled*|*Off*)
                echo "=== BLANKED. capturing evidence ==="
                docker exec nvkvm-steamos-vmm-1 nvkvm-steamos-ssh \
                  'journalctl -b --since "-6 min" --no-pager 2>/dev/null | grep -viE "device|mount|systemd\[1\]: Start" | tail -40' 2>&1
                echo "=== who holds inhibitors now ==="
                docker exec nvkvm-steamos-vmm-1 nvkvm-steamos-ssh 'systemd-inhibit --list 2>/dev/null | tail -5' 2>&1
                echo "END $(date -u +%FT%TZ)"; exit 0 ;;
        esac
        prev="$s"
    fi
    sleep 60
done
echo "no blank observed in 2h"; echo "END $(date -u +%FT%TZ)"
