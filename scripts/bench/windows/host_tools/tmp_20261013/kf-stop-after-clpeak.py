import os, signal, time
from pathlib import Path
pid=3719643
res=Path('/var/lib/kf-windows-20261005/codex-matrix-full-20261011/win.res')
print('CONTROLLED_STOP_WATCH_START', time.time(), flush=True)
for _ in range(1800):
    cmd=Path(f'/proc/{pid}/cmdline')
    if not cmd.exists():
        raise SystemExit('driver exited before requested stop')
    argv=cmd.read_bytes().split(b'\0')
    assert b'/data/kf-winmatrix-host2/scripts/bench/windows/appmatrix/winapps.py' in argv
    assert b'/var/lib/kf-windows-20261005/codex-matrix-full-20261011' in argv
    lines=res.read_text().splitlines()
    completed=[s for s in lines if ' app=clpeak_cuda ' in s]
    if completed:
        print(completed[-1],flush=True)
        os.kill(pid,signal.SIGTERM)
        print('CONTROLLED_STOP_SENT',time.time(),flush=True)
        break
    time.sleep(.5)
else:
    raise SystemExit('watch deadline reached; no signal sent')
