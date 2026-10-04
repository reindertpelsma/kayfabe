#!/usr/bin/env bash
# D4 follow-up: measure presentation slope separately from Vulkan startup.
# A 480-frame wall time exceeded the old 17.5s estimate on this box.
set -euo pipefail
tag=${1:?tag}
G=/root/kayfabe/scripts/bench/gssh_nv
out=/workspace/bench/display/$tag
mkdir -p "$out"
gq() { timeout 90 "$G" "$@"; }
gq 'sudo systemctl stop lightdm 2>/dev/null; sudo modprobe nvidia-drm modeset=1; sudo pkill -x Xorg || true'
gq "sudo sh -c 'nohup Xorg :0 -nolisten tcp -noreset -ac -logfile /var/log/Xorg.8.log >/tmp/xfifo.log 2>&1 &'"
ready=0
for _ in $(seq 30); do
    if gq 'sudo -u ubuntu env DISPLAY=:0 xset q >/dev/null 2>&1'; then ready=1; break; fi
    sleep 1
done
test "$ready" = 1
gq 'sudo -u ubuntu env DISPLAY=:0 xrandr' > "$out/xrandr.log"
gq 'sudo -u ubuntu env DISPLAY=:0 python3 -' <<'PY' | tee "$out/fifo_timing.log"
import json,subprocess,time
rows=[]
for count in [1,480,960]:
    start=time.monotonic()
    run=subprocess.run(['vkcube','--c',str(count),'--present_mode','2'],
                       capture_output=True,text=True,timeout=60)
    elapsed=time.monotonic()-start
    row=dict(frames=count,seconds=round(elapsed,6),rc=run.returncode,
             output=run.stdout+run.stderr)
    print('FIFO_TIMING '+json.dumps(row),flush=True)
    assert run.returncode==0, row
    rows.append(row)
slope=(rows[2]['seconds']-rows[1]['seconds'])/480
hz=1/slope
startup=rows[1]['seconds']-480*slope
print(f'FIFO_SLOPE hz={hz:.4f} startup_seconds={startup:.4f}',flush=True)
assert 28.5<=hz<=30.5, rows
assert abs(rows[0]['seconds']-(startup+slope))<1.0, rows
print('FIFO_TIMING_VERDICT PASS (480-to-960 frame slope, independent 1-frame startup check)',flush=True)
PY
gq 'sudo pkill -x Xorg; sudo sync'
