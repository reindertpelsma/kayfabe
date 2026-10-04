#!/usr/bin/env python3
"""Real guest EDID after refresh-only events from the broker test backend.

This drives the actual broker/relay/hotplug path, not a physical host monitor.
The display size stays 1920x1080 throughout; every duplicate must be suppressed.
"""
import hashlib
import json
import os
from pathlib import Path
import subprocess
import sys
import time

tag = sys.argv[1]
gssh = '/root/kayfabe/scripts/bench/gssh_nv'
qlog = Path(f'/workspace/bench/run_{tag}_qemu.log')
out = Path(f'/workspace/bench/display/{tag}')
out.mkdir(parents=True, exist_ok=True)


def guest(command):
    return subprocess.check_output([gssh, command], timeout=60, text=True).strip()


def edid():
    # A sysfs EDID read returns cached bytes. Enumerate connector modes as a
    # userspace hotplug consumer would, then inspect the refreshed cache.
    modes = guest('sudo modetest -M nvidia-drm -c')
    (out / 'modetest_latest.txt').write_text(modes+'\n')
    raw = guest('for e in /sys/class/drm/card*-*/edid; do '
                'if [ "$(wc -c < "$e")" -gt 0 ]; then xxd -p "$e"; break; fi; done')
    data = bytes.fromhex(raw)
    assert len(data) >= 128 and sum(data[:128]) % 256 == 0, raw
    d = data[54:72]
    width, hblank = d[2] | (d[4] & 0xf0) << 4, d[3] | (d[4] & 15) << 8
    height, vblank = d[5] | (d[7] & 0xf0) << 4, d[6] | (d[7] & 15) << 8
    hz = int.from_bytes(d[:2], 'little') * 10000 / ((width+hblank)*(height+vblank))
    return data, dict(width=width, height=height, hz=hz,
                      sha256=hashlib.sha256(data).hexdigest())


def send(rate):
    with open(os.environ['R6_FIFO'], 'w') as fifo:
        fifo.write(f'm 1920 1080 {rate * 1000}\n')


guest('sudo systemctl stop lightdm 2>/dev/null; sudo modprobe nvidia-drm modeset=1 fbdev=1; sudo chvt 1')
guest('sudo sh -c \"nohup timeout 150 udevadm monitor --kernel --property --subsystem-match=drm > /tmp/r6_udev.log 2>&1 < /dev/null &\"')
time.sleep(5)
reports = []
for rate in [30, 50, 60]:
    send(rate)
    time.sleep(2)
    deadline = time.monotonic() + 25
    while True:
        data, row = edid()
        if row['width'] == 1920 and row['height'] == 1080 and abs(row['hz']-rate) < 0.5:
            break
        assert time.monotonic() < deadline, (rate, row)
        time.sleep(1)
    event = f'broker window is now 1920x1080 at {rate*1000} mHz'
    before = qlog.read_text(errors='replace').count(event)
    assert before >= 1, event
    send(rate)
    time.sleep(3)
    assert qlog.read_text(errors='replace').count(event) == before, 'duplicate reached relay'
    row['requested_hz'] = rate
    reports.append(row)
    (out / f'edid_{rate}.hex').write_text(data.hex()+'\n')
    (out / f'modetest_{rate}.txt').write_text((out / 'modetest_latest.txt').read_text())
    (out / 'udev.txt').write_text(guest('cat /tmp/r6_udev.log')+'\n')
    print('R6_EDID '+json.dumps(row), flush=True)
assert len({r['sha256'] for r in reports}) == 3, reports
(out/'refresh_hints.json').write_text(json.dumps(reports, indent=2)+'\n')
print('R6_VERDICT PASS refresh-only=3 duplicate=3 size=1920x1080', flush=True)
