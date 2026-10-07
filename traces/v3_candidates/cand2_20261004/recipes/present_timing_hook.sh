#!/usr/bin/env bash
set -euo pipefail
tag=${1:?tag}
G=/root/kayfabe/scripts/bench/gssh_nv
out=/workspace/bench/display/$tag
mkdir -p "$out"
gq() { timeout 90 "$G" "$@"; }
gq 'cat > /tmp/vk_present_timing.c' < /root/vk_present_timing.c
gq 'gcc -shared -fPIC -O2 -Wall -Wextra -Werror -o /tmp/vk_present_timing.so /tmp/vk_present_timing.c -ldl -pthread && sha256sum /tmp/vk_present_timing.c'
gq 'sudo systemctl stop lightdm 2>/dev/null; sudo modprobe nvidia-drm modeset=1; sudo pkill -x Xorg || true; sleep 3'
gq "sudo sh -c 'nohup Xorg :0 -nolisten tcp -noreset -ac -logfile /var/log/Xorg.9.log >/tmp/xpresent.log 2>&1 &'"
ready=0
for _ in $(seq 30); do
    if gq 'sudo -u ubuntu env DISPLAY=:0 xset q >/dev/null 2>&1'; then ready=1; break; fi
    sleep 1
done
test "$ready" = 1
gq 'sudo -u ubuntu env DISPLAY=:0 xrandr' > "$out/xrandr.log"
for mode in 2 0; do
    gq "sudo -u ubuntu env DISPLAY=:0 LD_PRELOAD=/tmp/vk_present_timing.so timeout 60 vkcube --c 480 --present_mode $mode" \
        > "$out/present_$mode.log" 2>&1
    cat "$out/present_$mode.log"
done
python3 - "$out" <<'PY'
import json
from pathlib import Path
import sys
out=Path(sys.argv[1]); rows={}
for mode in [2,0]:
    text=(out/f'present_{mode}.log').read_text()
    reports=[json.loads(line.removeprefix('VK_PRESENT_TIMING '))
             for line in text.splitlines() if line.startswith('VK_PRESENT_TIMING ')]
    report=next(r for r in reports if r['count']==480)
    assert report['errors']==0, report
    rows[mode]=report
assert 28.5<=rows[2]['fps']<=30.5, rows
assert rows[0]['fps']>55, rows
print('PRESENT_TIMING_VERDICT PASS '+json.dumps(rows))
PY
gq 'sudo pkill -x Xorg; sudo sync'
