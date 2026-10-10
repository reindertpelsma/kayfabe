#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later
"""gl-monitor.py RUN QPID OUTDIR [DEADLINE_S] -- host side, read-only collection of the Windows guest's own logs through QGA.

Moments (labels in the file names):
  A  as soon as the guest agent answers (a guest-sync probe every 3 s until then): the full collector, guest_logs_collect.ps1
  (light timeline) a short sample (guest_logs_light.ps1) every ~5 s from then on, to the end; every third sample is
     followed by an events-only sample (guest_logs_events.ps1, file E-<n>) until the death, so the guest's event log is read LIVE
     (the guest does not flush it to disk before a bugcheck)
  B  right after the guest's NVIDIA driver died: first of (1) kayfabe's log shows 'UnloadingGuestDriver' (the guest's RM client
     unloaded; it is polled every 0.5 s, also while a QGA call is in flight), (2) the NVIDIA video controller reports an error
     code or is gone after it had been healthy, (3) the guest's boot time changed (it bugchecked and rebooted). The full
     collector runs at once. An OS that is bugchecking does not answer; the call is retried until it does.
  C  60 s after B, if the OS still answers: full collector again
  D60 / D120  the light sample nearest B+60 s and B+120 s is kept as its own file (service/process state)
  R  the boot time changed again after B: a full snapshot of the new boot
  L  no death by 330 s after the monitor started: one full snapshot ('late'), sampling goes on to the deadline
The script never writes in the guest. Output: OUTDIR/snap-<label>-<HHMMSS>.txt, timeline.txt, monitor.log, light/NNN.txt, E/NNN.txt.
The QGA client here sends the 0xFF parser-reset byte before guest-sync and retries the sync every 4 s (qmp.py's qga-exec waits
30 s for the first sync after another client's connection, which cost 30 s at moment A in runs 116 and 117).
"""
import base64, datetime, json, os, random, re, socket, subprocess, sys, time

run, qpid, out = sys.argv[1], int(sys.argv[2]), sys.argv[3]
deadline_s = float(sys.argv[4]) if len(sys.argv) > 4 else 600.0
W = '/var/lib/kf-windows-20261005'
rd = f'{W}/boundary-kayfabe-{run}'
sock = rd + '/qga.sock'
here = os.path.dirname(os.path.abspath(__file__))
FULL = open(os.path.join(here, 'guest_logs_collect.ps1')).read()
LIGHT = open(os.path.join(here, 'guest_logs_light.ps1')).read()
EVENTS = open(os.path.join(here, 'guest_logs_events.ps1')).read()
os.makedirs(out + '/light', exist_ok=True)
os.makedirs(out + '/E', exist_ok=True)
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


def unloaded():
    try:
        return int(subprocess.run(['grep', '-a', '-c', 'UnloadingGuestDriver', rd + '/qemu.log'], capture_output=True, text=True).stdout.strip() or 0)
    except Exception:
        return 0


class Abort(Exception):
    pass


def _readline(f, until, abort):
    """One JSON line from the socket; None when `until` passes. Short socket timeouts so `abort()` is polled."""
    while time.time() < until:
        if abort and abort():
            raise Abort()
        try:
            line = f.readline()
        except (socket.timeout, TimeoutError):
            continue
        if not line:
            raise EOFError('closed')
        try:
            return json.loads(line)
        except ValueError:
            continue
    return None


def _sync(s, f, abort, within):
    end = time.time() + within
    while time.time() < end:
        tok = random.randrange(1, 2**31 - 1)
        s.sendall(b'\xff' + json.dumps({'execute': 'guest-sync', 'arguments': {'id': tok}}).encode() + b'\n')
        t1 = min(end, time.time() + 4)
        while time.time() < t1:
            r = _readline(f, t1, abort)
            if r is not None and r.get('return') == tok:
                return True
    return False


def qga_run(script, tmo, abort=None, sync_within=20):
    """guest-exec powershell -Command SCRIPT as SYSTEM; (rc, stdout, stderr-ish, seconds). rc 124 timeout, 97 no sync, 98 aborted, 99 error."""
    s0 = time.time()
    try:
        s = socket.socket(socket.AF_UNIX, socket.SOCK_STREAM)
        s.settimeout(1.0)
        s.connect(sock)
        f = s.makefile('rb', buffering=0)
    except OSError as e:
        return 99, '', f'connect {e}', time.time() - s0
    try:
        if not _sync(s, f, abort, sync_within):
            return 97, '', 'no guest-sync answer', time.time() - s0
        s.sendall(json.dumps({'execute': 'guest-exec', 'arguments': {'path': 'powershell.exe', 'arg': ['-NoProfile', '-Command', script], 'capture-output': True}}).encode() + b'\n')
        r = _readline(f, time.time() + 15, abort)
        if not r or 'return' not in r:
            return 99, '', f'guest-exec: {str(r)[:200]}', time.time() - s0
        pid = r['return']['pid']
        end = time.time() + tmo
        while time.time() < end:
            s.sendall(json.dumps({'execute': 'guest-exec-status', 'arguments': {'pid': pid}}).encode() + b'\n')
            st = _readline(f, time.time() + 10, abort)
            st = (st or {}).get('return', {})
            if st.get('exited'):
                so = base64.b64decode(st.get('out-data', '')).decode('utf-8', 'replace')
                se = base64.b64decode(st.get('err-data', '')).decode('utf-8', 'replace')[-300:]
                return int(st.get('exitcode', 1)), so, se, time.time() - s0
            t1 = time.time() + 0.7
            while time.time() < t1:
                if abort and abort():
                    raise Abort()
                time.sleep(0.1)
        return 124, '', 'exec timeout', time.time() - s0
    except Abort:
        return 98, '', 'aborted: unload seen', time.time() - s0
    except (OSError, EOFError) as e:
        return 99, '', f'{type(e).__name__} {e}', time.time() - s0
    finally:
        try:
            s.close()
        except OSError:
            pass


def ping():
    try:
        s = socket.socket(socket.AF_UNIX, socket.SOCK_STREAM)
        s.settimeout(1.0)
        s.connect(sock)
        f = s.makefile('rb', buffering=0)
        ok = _sync(s, f, None, 4)
        s.close()
        return ok
    except Exception:
        return False


def full(label, tries=3):
    for k in range(tries):
        st = now()
        rc, so, se, dur = qga_run(FULL, 120)
        ok = '=== END' in so
        name = f'{out}/snap-{label}-{st.replace(":", "")[:6]}{"" if ok else "-FAILED"}.txt'
        open(name, 'w').write(f'# moment {label}  host_utc_start={st}  duration={dur:.1f}s  rc={rc}  complete={ok}  stderr={se!r}\n' + so)
        log(f'FULL {label} try{k + 1} rc={rc} dur={dur:.1f}s bytes={len(so)} complete={ok} -> {os.path.basename(name)}')
        if ok or not alive():
            return ok
        time.sleep(2)
    return False


log(f'monitor start run={run} qpid={qpid} deadline={deadline_s}s unload_lines_at_start={unloaded()}')
tl = open(out + '/timeline.txt', 'a')
# --- wait for the first QGA answer (moment A)
while alive() and time.time() - t0 < deadline_s:
    if os.path.exists(sock) and ping():
        break
    time.sleep(3)
else:
    log('QGA never answered' if alive() else 'qemu gone before QGA answered')
    sys.exit(0)
log('QGA answered; moment A')
full('A')
seen_ok = False
boot0 = None
tB = None
doneC = False
doneL = False
d60 = d120 = False
n = 0
ne = 0
fails = 0
unload0 = unloaded()


def abort_on_unload():
    return tB is None and unloaded() > unload0


while alive() and time.time() - t0 < deadline_s:
    n += 1
    st = now()
    rc, so, se, dur = qga_run(LIGHT, 20, abort=None if tB is not None else abort_on_unload)
    ls = next((l for l in so.splitlines() if l.startswith('LS ')), '')
    bt = re.search(r' boot=(\S+)', so)
    bt = bt.group(1) if bt else None
    hung = '' if ls else ('HUNG ' if rc in (124, 97, 98, 99) else 'NO-ANSWER ')
    open(f'{out}/light/{n:03d}.txt', 'w').write(f'# host_utc_start={st} duration={dur:.1f}s rc={rc} stderr={se!r}\n' + so)
    tl.write(f'{st} +{time.time() - t0:5.1f}s dur={dur:4.1f}s rc={rc} unload_lines={unloaded()} boot={bt} {ls if ls else hung + se}\n')
    tl.flush()
    fails = 0 if ls else fails + 1
    gpu = re.search(r'gpu=(\S*)', ls)
    gpu = gpu.group(1) if gpu else None
    if gpu and gpu.startswith('code0'):
        seen_ok = True
    err = bool(gpu) and ((gpu not in ('none',) and not gpu.startswith('code0')) or (gpu == 'none' and seen_ok))
    ul = unloaded() > unload0
    rebooted = bool(bt) and boot0 is not None and bt != boot0
    if bt and boot0 is None:
        boot0 = bt
    if rebooted:
        boot0 = bt
    if tB is None and (err or ul or rebooted):
        tB = time.time()
        log(f'DEATH detected: videocontroller_error={err} gpu={gpu} unloading_in_qemu_log={ul} guest_rebooted={rebooted} (qemu-log unload count {unloaded()} vs {unload0} at A)')
        full('B')
    elif rebooted:
        log('guest rebooted again')
        full('R')
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
    else:
        if n % 3 == 0:
            ne += 1
            st = now()
            rc, so, se, dur = qga_run(EVENTS, 30, abort=abort_on_unload)
            open(f'{out}/E/{ne:03d}.txt', 'w').write(f'# host_utc_start={st} duration={dur:.1f}s rc={rc} complete={"=== END" in so} stderr={se!r}\n' + so)
            log(f'E{ne:03d} rc={rc} dur={dur:.1f}s bytes={len(so)}')
        if not doneL and time.time() - t0 >= 330:
            doneL = True
            log('no death by 330 s: late full snapshot')
            full('L')
    if fails >= 4 and tB is None:
        log(f'QGA silent for {fails} samples (OS gone or hung?)')
    time.sleep(2)
log('monitor end alive=%s B=%s C=%s' % (alive(), tB is not None, doneC))
