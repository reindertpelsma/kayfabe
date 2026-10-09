#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later
"""gl-monitor.py RUN QPID OUTDIR [DEADLINE_S] -- host side, read-only collection of the Windows guest's own logs through QGA.

Moments (labels in the file names):
  A  as soon as the guest agent answers (pinged every 5 s until then): the full collector, guest_logs_collect.ps1
  (light timeline) a short sample (guest_logs_light.ps1) every ~6 s from then on, to the end
  B  right after the guest's NVIDIA driver died: first of (1) kayfabe's log shows 'UnloadingGuestDriver' (the guest's RM client
     unloaded), (2) the NVIDIA video controller reports an error code (43) or is gone after it had been healthy.
     The full collector runs at once.
  C  60 s after B, if the OS still answers: full collector again
  D60 / D120  the light sample nearest B+60 s and B+120 s is kept as its own file (service/process state)
  L  no death by 330 s after the monitor started: one full snapshot ('late'), sampling goes on to the deadline
The script never writes in the guest. Output: OUTDIR/snap-<label>-<HHMMSS>.txt, timeline.txt, monitor.log, light/NNN.txt.
"""
import os, re, subprocess, sys, time, datetime

run, qpid, out = sys.argv[1], int(sys.argv[2]), sys.argv[3]
deadline_s = float(sys.argv[4]) if len(sys.argv) > 4 else 600.0
W = '/var/lib/kf-windows-20261005'
rd = f'{W}/boundary-kayfabe-{run}'
sock = rd + '/qga.sock'
qmp = W + '/boundary-tools/qmp.py'
here = os.path.dirname(os.path.abspath(__file__))
FULL = open(os.path.join(here, 'guest_logs_collect.ps1')).read()
LIGHT = open(os.path.join(here, 'guest_logs_light.ps1')).read()
os.makedirs(out + '/light', exist_ok=True)
t0 = time.time()


def now():
    return datetime.datetime.now(datetime.timezone.utc).strftime('%H:%M:%S.%f')[:-3]


def log(*a):
    line = f'{now()} +{time.time() - t0:5.1f}s ' + ' '.join(str(x) for x in a)
    print(line, flush=True)
    open(out + '/monitor.log', 'a').write(line + '\n')


def alive():
    try:
        os.kill(qpid, 0)
        return True
    except OSError:
        return False


def ping():
    try:
        return subprocess.run(['timeout', '8', 'python3', qmp, sock, 'qga-ping'], capture_output=True).returncode == 0
    except Exception:
        return False


def qexec(script, tmo):
    s = time.time()
    try:
        pr = subprocess.run(['timeout', str(tmo), 'python3', qmp, sock, 'qga-exec', 'powershell.exe', '-NoProfile', '-Command', script],
                            capture_output=True, text=True, timeout=tmo + 15)
        return pr.returncode, pr.stdout, pr.stderr[-400:], time.time() - s
    except Exception as e:
        return 99, '', f'{type(e).__name__}', time.time() - s


def unloaded():
    try:
        return int(subprocess.run(['grep', '-a', '-c', 'UnloadingGuestDriver', rd + '/qemu.log'], capture_output=True, text=True).stdout.strip() or 0)
    except Exception:
        return 0


def full(label, tries=3):
    for k in range(tries):
        st = now()
        rc, so, se, dur = qexec(FULL, 280)
        ok = '=== END' in so
        name = f'{out}/snap-{label}-{st.replace(":", "")[:6]}{"" if ok else "-FAILED"}.txt'
        open(name, 'w').write(f'# moment {label}  host_utc_start={st}  duration={dur:.1f}s  rc={rc}  complete={ok}  stderr={se!r}\n' + so)
        log(f'FULL {label} try{k + 1} rc={rc} dur={dur:.1f}s bytes={len(so)} complete={ok} -> {os.path.basename(name)}')
        if ok or not alive():
            return ok
        time.sleep(3)
    return False


log(f'monitor start run={run} qpid={qpid} deadline={deadline_s}s unload_lines_at_start={unloaded()}')
tl = open(out + '/timeline.txt', 'a')
# --- wait for the first QGA answer (moment A)
while alive() and time.time() - t0 < deadline_s:
    if os.path.exists(sock) and ping():
        break
    time.sleep(5)
else:
    log('QGA never answered' if alive() else 'qemu gone before QGA answered')
    sys.exit(0)
log('QGA answered; moment A')
full('A')
seen_ok = False
tB = None
doneC = False
doneL = False
d60 = d120 = False
n = 0
fails = 0
unload0 = unloaded()
while alive() and time.time() - t0 < deadline_s:
    n += 1
    st = now()
    rc, so, se, dur = qexec(LIGHT, 60)
    ls = next((l for l in so.splitlines() if l.startswith('LS ')), '')
    open(f'{out}/light/{n:03d}.txt', 'w').write(f'# host_utc_start={st} duration={dur:.1f}s rc={rc} stderr={se!r}\n' + so)
    tl.write(f'{st} +{time.time() - t0:5.1f}s dur={dur:4.1f}s rc={rc} unload_lines={unloaded()} {ls if ls else "NO-ANSWER " + se}\n')
    tl.flush()
    fails = 0 if ls else fails + 1
    gpu = re.search(r'gpu=(\S*)', ls)
    gpu = gpu.group(1) if gpu else None
    if gpu and gpu.startswith('code0'):
        seen_ok = True
    err = bool(gpu) and ((gpu not in ('none',) and not gpu.startswith('code0')) or (gpu == 'none' and seen_ok))
    ul = unloaded() > unload0
    if tB is None and (err or ul):
        tB = time.time()
        log(f'DEATH detected: videocontroller_error={err} gpu={gpu} unloading_in_qemu_log={ul}')
        full('B')
    elif tB is not None:
        el = time.time() - tB
        if not doneC and el >= 60:
            doneC = True
            full('C')
        if not d60 and el >= 60 and ls:
            d60 = True
            open(f'{out}/snap-D60-light.txt', 'w').write(f'# light sample at B+{el:.0f}s host_utc={st}\n' + so)
        if not d120 and el >= 120 and ls:
            d120 = True
            open(f'{out}/snap-D120-light.txt', 'w').write(f'# light sample at B+{el:.0f}s host_utc={st}\n' + so)
        if d120 and doneC:
            log('B+120 s reached, stopping the collection')
            break
    elif not doneL and time.time() - t0 >= 330:
        doneL = True
        log('no death by 330 s: late full snapshot')
        full('L')
    if fails >= 4 and tB is None:
        log(f'QGA silent for {fails} samples (OS gone or hung?)')
    time.sleep(3)
log('monitor end alive=%s B=%s C=%s' % (alive(), tB is not None, doneC))
