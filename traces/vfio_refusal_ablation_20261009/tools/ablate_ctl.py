#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later
"""ablate_ctl.py -- copy of click_ctl.py (README section 18 of the Windows record) for the 2026-10-09 refusal ablation.
Changes: --gesture none (idle only: no input at all, then --record-secs of observation), --yend (drag end y on the
tablet axis 0..32767, default 9000 as the ablation brief says; click_ctl used 5461), x-gsp-refuse lines in events.tsv.

click_ctl.py -- host side controller of one scripted-input experiment (README section 18).

  click_ctl.py MODE RUNDIR QPID OUTDIR [--gesture drag|click] [--lock-secs 20] [--idle-secs 20] [--record-secs 90]
               [--lock-max 300]

MODE kf3  : lock screen detected from the QMP screendumps only (device kf0); NO QGA polling at all.
MODE vfio : the guest's screen is on the physical monitor, which the host cannot capture, so the lock screen is detected
            by a light QGA probe (LogonUI.exe running) every 4 s UNTIL the first positive answer, then QGA is not touched
            again (screendumps of the std VGA are taken and kept, but are not the criterion).
One persistent QMP connection does everything (screendumps, the input events), so nothing contends for the socket.

Timeline: boot -> lock screen continuously non-black for LOCK_SECS (kf3) / LogonUI present for LOCK_SECS (vfio)
-> IDLE_SECS more -> ONE gesture through QMP input-send-event on the usb-tablet (absolute axes 0..32767):
  drag : (1) move to the centre (16384,16384); (2) left button PRESS, held 300 ms; (3) while held, ~12 small absolute
         steps UP over ~0.65 s (y 16384 -> 5461, a third of the screen height = 360 px of 1080); (4) RELEASE.
  click: (1) move to the centre; (2) PRESS; 40 ms; (4) RELEASE (no move).
-> RECORD_SECS of screendumps (every ~0.4 s) after T_press.
Writes OUTDIR/click.txt (key=value, UTC with ms, CLOCK_REALTIME and CLOCK_MONOTONIC ns per phase), frames.tsv, frames/*.png
(640x360), events.tsv (qemu.log lines of interest with arrival time), status.tsv (the kf3 status line each ~1 s), qlog-offsets.tsv
(qemu.log byte offset by time, to date any qemu.log line), qmp-events.jsonl.
Exit code 0 = clicked; 3 = IDLE-DEATH (died before the click); 4 = no lock screen within LOCK_MAX.
"""
import base64, datetime, json, os, random, re, socket, struct, sys, threading, time, zlib

mode, rundir, qpid, out = sys.argv[1], sys.argv[2], int(sys.argv[3]), sys.argv[4]
opt = dict(zip(sys.argv[5::2], sys.argv[6::2]))
GESTURE = opt.get('--gesture', 'drag')
LOCK_SECS = float(opt.get('--lock-secs', 20))
IDLE_SECS = float(opt.get('--idle-secs', 20))
RECORD_SECS = float(opt.get('--record-secs', 90))
LOCK_MAX = float(opt.get('--lock-max', 300))
Y_END = int(opt.get('--yend', 9000))
HOLD_QMP = opt.get('--hold-qmp', '0') == '1'   # vfio: default OFF -- the QMP socket has ONE client slot; the coordinator/owner
# must be able to attach input devices at any time, and the std VGA's screendump is not the display anyway.
THRESH = float(opt.get('--thresh', 0.1))   # non-black fraction that counts as a drawn lock screen
os.makedirs(out + '/frames', exist_ok=True)
T0 = time.time()
qlog = rundir + '/qemu.log'
CLOCK_MONO = time.CLOCK_MONOTONIC


def utc(t=None):
    t = time.time() if t is None else t
    return datetime.datetime.fromtimestamp(t, datetime.timezone.utc).strftime('%Y-%m-%dT%H:%M:%S.%f')[:-3] + 'Z'


def log(*a):
    line = f'{utc()} +{time.time() - T0:6.1f}s ' + ' '.join(str(x) for x in a)
    print(line, flush=True)
    open(out + '/ctl.log', 'a').write(line + '\n')


def kv(k, v):
    open(out + '/click.txt', 'a').write(f'{k}={v}\n')


def stamp(label):
    """record a phase: UTC ms, realtime ns, monotonic ns (read back-to-back)."""
    r = time.time_ns(); m = time.clock_gettime_ns(CLOCK_MONO)
    kv(label, utc(r / 1e9))
    kv(label + '_realtime_ns', r)
    kv(label + '_monotonic_ns', m)
    return r / 1e9


def alive():
    try:
        os.kill(qpid, 0)
        return True
    except OSError:
        return False


# ---------------------------------------------------------------- QMP (one persistent connection)
class Qmp:
    def __init__(self, path):
        self.path = path
        self.s = socket.socket(socket.AF_UNIX, socket.SOCK_STREAM)
        self.s.settimeout(20)
        self.s.connect(path)
        self.f = self.s.makefile('rwb', buffering=0)
        self.greet = json.loads(self.f.readline())
        self.cmd('qmp_capabilities')

    def cmd(self, name, args=None):
        m = {'execute': name}
        if args is not None:
            m['arguments'] = args
        self.f.write(json.dumps(m).encode() + b'\n')
        while True:
            line = self.f.readline()
            if not line:
                raise EOFError('qmp closed')
            r = json.loads(line)
            if 'event' in r:
                open(out + '/qmp-events.jsonl', 'a').write(json.dumps(dict(r, host_utc=utc())) + '\n')
                continue
            return r


qmp = None


def qmp_connect():
    global qmp
    for _ in range(240):
        if not alive():
            log('qemu gone before QMP')
            sys.exit(5)
        try:
            qmp = Qmp(rundir + '/qmp.sock')
            return qmp
        except OSError:
            time.sleep(0.5)
    log('no QMP socket')
    sys.exit(5)


def qmp_release():
    global qmp
    if qmp is not None:
        try:
            qmp.s.close()
        except OSError:
            pass
        qmp = None


qmp_connect()
log('QMP connected; mode', mode, 'gesture', GESTURE, 'lock_secs', LOCK_SECS, 'idle_secs', IDLE_SECS, 'record_secs', RECORD_SECS,
    'hold_qmp', HOLD_QMP)
if mode == 'vfio' and not HOLD_QMP:
    qmp_release()   # connect again only for the gesture; no screendumps (the std VGA is not the display on VFIO)
kv('mode', mode); kv('gesture', GESTURE); kv('qemu_pid', qpid)
kv('clock_offset_realtime_minus_monotonic_ns', time.time_ns() - time.clock_gettime_ns(CLOCK_MONO))


# ---------------------------------------------------------------- frames
def ppm_read(p):
    b = open(p, 'rb').read()
    i = 0; hdr = []
    for _ in range(3):
        j = b.index(b'\n', i); hdr.append(b[i:j]); i = j + 1
    w, h = map(int, hdr[1].split())
    return w, h, b[i:]


def png_write(w, h, rgb, path, k=3):
    w2, h2 = w // k, h // k
    raw = bytearray()
    for y in range(h2):
        row = rgb[(y * k) * w * 3:(y * k + 1) * w * 3]
        o = bytearray(w2 * 3)
        o[0::3] = row[0::3 * k][:w2]; o[1::3] = row[1::3 * k][:w2]; o[2::3] = row[2::3 * k][:w2]
        raw.append(0); raw += o

    def ch(t, d):
        return struct.pack('>I', len(d)) + t + d + struct.pack('>I', zlib.crc32(t + d) & 0xffffffff)
    open(path, 'wb').write(b'\x89PNG\r\n\x1a\n' + ch(b'IHDR', struct.pack('>IIBBBBB', w2, h2, 8, 2, 0, 0, 0))
                           + ch(b'IDAT', zlib.compress(bytes(raw), 6)) + ch(b'IEND', b''))


frames = open(out + '/frames.tsv', 'a')
fn = 0
last_frac = None


def shot(tag=''):
    """one screendump -> PNG 640x360 + nonblack fraction. Returns (utc_before_request, frac) or None."""
    global fn, last_frac
    if qmp is None:
        return None
    fn += 1
    t = time.time()
    ppm = f'{out}/frames/_tmp.ppm'
    args = {'filename': ppm}
    if mode == 'kf3':
        args['device'] = 'kf0'
    try:
        r = qmp.cmd('screendump', args)
    except (EOFError, OSError, ValueError) as e:
        log('screendump failed', e)
        return None
    t1 = time.time()
    if 'error' in r or not os.path.exists(ppm):
        frames.write(f'{utc(t)}\t-\t-\t{tag}\t{str(r)[:80]}\n'); frames.flush()
        return None
    try:
        w, h, d = ppm_read(ppm)
    except Exception as e:
        log('ppm read', e); return None
    px = d[::997]
    frac = sum(1 for x in px if x > 16) / max(1, len(px))
    name = f'f{fn:04d}-{utc(t)[11:23].replace(":", "")}.png'
    png_write(w, h, d, f'{out}/frames/{name}')
    os.unlink(ppm)
    frames.write(f'{utc(t)}\t{utc(t1)}\t{name}\t{frac:.3f}\t{tag}\n'); frames.flush()
    last_frac = frac
    return t, frac


# ---------------------------------------------------------------- qemu.log watch
qpos = 0
qsz_seen = -1
events = open(out + '/events.tsv', 'a')
offs = open(out + '/qlog-offsets.tsv', 'a')
status_last = ''
last_status_t = 0
PAT = re.compile(rb'x-gsp-refuse|UnloadingGuestDriver|scanout REFUSED|REFUSED|birth REFUSED|PT-SNAP BEGIN|ChannelFreed|BORN Passthrough|BORN Translated|RETIRED|WTRACE.*VSYNC|LATCH window|GPU lost|Xid|panic|abort')
n_unload = 0


def watch():
    global qpos, status_last, last_status_t, qsz_seen, n_unload
    try:
        sz = os.path.getsize(qlog)
    except OSError:
        return
    now = time.time()
    if sz != qsz_seen:
        offs.write(f'{utc(now)}\t{sz}\n'); offs.flush(); qsz_seen = sz
    if sz <= qpos:
        return
    with open(qlog, 'rb') as f:
        f.seek(qpos)
        data = f.read(min(sz - qpos, 8 << 20))
    cut = data.rfind(b'\n') + 1
    if cut == 0:
        return
    base = qpos
    qpos += cut
    off = base
    for line in data[:cut].split(b'\n'):
        ln = len(line) + 1
        if b'UnloadingGuestDriver' in line:
            n_unload += 1
        if PAT.search(line) and not line.startswith(b'kf3: family'):
            if len(line) > 0:
                events.write(f'{utc(now)}\t{off}\t' + line[:300].decode('utf-8', 'replace') + '\n')
        if line.startswith(b'kf3: family'):
            status_last = line.decode('utf-8', 'replace')
        off += ln
    events.flush()
    if status_last and now - last_status_t >= 1.0:
        last_status_t = now
        m = re.search(r'phase=(\S+)', status_last)
        d = re.search(r'disp\[[^\]]*\]', status_last)
        open(out + '/status.tsv', 'a').write(f'{utc(now)}\t{m.group(1) if m else ""}\t{d.group(0) if d else status_last[:200]}\n')


# ---------------------------------------------------------------- vfio: LogonUI probe over QGA (stops at the first positive)
logonui_t = None


def qga_probe_thread():
    global logonui_t
    sock = rundir + '/qga.sock'
    script = "$p=Get-Process LogonUI -ErrorAction SilentlyContinue; if($p){'LOGONUI ' + $p[0].StartTime.ToUniversalTime().ToString('HH:mm:ss.fff')}else{'NONE'}"
    while logonui_t is None and alive() and time.time() - T0 < LOCK_MAX + 60:
        time.sleep(4)
        if not os.path.exists(sock):
            continue
        try:
            s = socket.socket(socket.AF_UNIX, socket.SOCK_STREAM); s.settimeout(3); s.connect(sock)
            f = s.makefile('rb', buffering=0)
            tok = random.randrange(1, 2**31 - 1)
            s.sendall(b'\xff' + json.dumps({'execute': 'guest-sync', 'arguments': {'id': tok}}).encode() + b'\n')
            ok = False
            end = time.time() + 4
            while time.time() < end:
                try:
                    r = json.loads(f.readline())
                except (socket.timeout, ValueError):
                    break
                if r.get('return') == tok:
                    ok = True; break
            if not ok:
                s.close(); continue
            s.settimeout(8)
            s.sendall(json.dumps({'execute': 'guest-exec', 'arguments': {'path': 'powershell.exe', 'arg': ['-NoProfile', '-Command', script], 'capture-output': True}}).encode() + b'\n')
            pid = json.loads(f.readline())['return']['pid']
            for _ in range(20):
                time.sleep(0.5)
                s.sendall(json.dumps({'execute': 'guest-exec-status', 'arguments': {'pid': pid}}).encode() + b'\n')
                st = json.loads(f.readline()).get('return', {})
                if st.get('exited'):
                    so = base64.b64decode(st.get('out-data', '')).decode('utf-8', 'replace')
                    log('QGA LogonUI probe:', so.strip()[:80])
                    if 'LOGONUI' in so:
                        logonui_t = time.time()
                    break
            s.close()
        except Exception as e:  # noqa
            log('QGA probe error', type(e).__name__, e)


if mode == 'vfio':
    threading.Thread(target=qga_probe_thread, daemon=True).start()

# ---------------------------------------------------------------- phase 1: wait for the lock screen, idle, click
lock_since = None
black_since = None
seen_lock = False
death = None
t_click = None
while True:
    if not alive():
        death = 'qemu-gone'; break
    watch()
    if n_unload and death is None:
        death = 'UnloadingGuestDriver'
    r = shot('pre')
    now = time.time()
    if mode == 'kf3' and r:
        frac = r[1]
        if frac > THRESH:
            lock_since = lock_since or now
            black_since = None
            if not seen_lock and now - lock_since < 1.5:
                log(f'non-black frame (frac {frac:.2f}); lock screen candidate since {utc(lock_since)}')
        else:
            if seen_lock and frac < 0.02:
                black_since = black_since or now
                if now - black_since >= 3:
                    death = f'black-screen-after-lock (frac {frac:.3f})'
            if lock_since and frac <= THRESH and not seen_lock:
                lock_since = None
        if lock_since and not seen_lock and now - lock_since >= LOCK_SECS:
            seen_lock = True
            kv('lock_screen_first_nonblack', utc(lock_since))
            kv('lock_screen_continuous_20s_at', utc(now))
            log(f'lock screen continuously non-black for {LOCK_SECS:.0f}s since {utc(lock_since)}')
            t_idle0 = now
    if mode == 'vfio':
        if logonui_t and not seen_lock and now - logonui_t >= LOCK_SECS:
            seen_lock = True
            kv('lock_screen_first_nonblack', utc(logonui_t) + ' (LogonUI first seen by the QGA probe; not a screenshot)')
            kv('lock_screen_continuous_20s_at', utc(now))
            log('LogonUI present since', utc(logonui_t))
            t_idle0 = now
    if death:
        break
    if seen_lock and now - t_idle0 >= IDLE_SECS:
        break
    if now - T0 > LOCK_MAX:
        log('no lock screen within LOCK_MAX')
        kv('result', 'NO-LOCKSCREEN')
        sys.exit(4)
    time.sleep(0.5 if mode == 'kf3' else 1.0)

if death:
    log('IDLE-DEATH before the click:', death)
    kv('result', 'IDLE-DEATH'); kv('idle_death', death); kv('idle_death_seen_utc', utc())
    end = time.time() + 20
    while time.time() < end and alive():
        watch(); shot('post-idle-death'); time.sleep(1)
    sys.exit(3)

# ---------------------------------------------------------------- the gesture
def send(events_, label):
    t = stamp(label)
    r = qmp.cmd('input-send-event', {'events': events_})
    kv(label + '_reply', utc())
    if 'error' in r:
        log('input-send-event error', r)
    return t


def absev(x, y):
    return [{'type': 'abs', 'data': {'axis': 'x', 'value': x}}, {'type': 'abs', 'data': {'axis': 'y', 'value': y}}]


def btn(down):
    return [{'type': 'btn', 'data': {'button': 'left', 'down': down}}]


CX = CY = 16384
# one last quiet screendump so the frame just before is on disk
if GESTURE != 'none' and qmp is None:
    qmp_connect()   # only now, for the gesture
shot('pre-click')
log('GESTURE', GESTURE)
if GESTURE == 'none':
    t_press = stamp('T_idle_end_no_input')
    kv('result', 'IDLE-OK-NO-INPUT')
    end = t_press + RECORD_SECS
    while time.time() < end and alive():
        watch(); shot('post-idle'); time.sleep(1.0)
    watch()
    kv('record_end', utc())
    sys.exit(0)
send(absev(CX, CY), 'T_move_centre')
time.sleep(0.25)
t_press = send(btn(True), 'T_press')
if GESTURE == 'drag':
    time.sleep(0.3)
    steps = 12
    for i in range(1, steps + 1):
        y = int(CY - (CY - Y_END) * i / steps)
        if i == 1:
            send(absev(CX, y), 'T_move_start')
        elif i == steps:
            send(absev(CX, y), 'T_move_end')
        else:
            qmp.cmd('input-send-event', {'events': absev(CX, y)})
        time.sleep(0.65 / steps)
    send(btn(False), 'T_release')
else:
    time.sleep(0.04)
    send(btn(False), 'T_release')
kv('result', 'CLICKED')
if mode == 'vfio' and not HOLD_QMP:
    qmp_release()
kv('T_click', utc(t_press))
log('gesture done; recording', RECORD_SECS, 's from T_press')

# ---------------------------------------------------------------- phase 2: record
end = t_press + RECORD_SECS
dead_logged = False
while time.time() < end:
    if not alive():
        if not dead_logged:
            log('qemu process gone at', utc()); kv('qemu_gone_utc', utc()); dead_logged = True
        break
    watch()
    if n_unload:
        if not os.path.exists(out + '/.unload_logged'):
            open(out + '/.unload_logged', 'w').write(utc())
            kv('first_UnloadingGuestDriver_seen_utc', utc())
            log('UnloadingGuestDriver in qemu.log')
    shot('post')
    time.sleep(0.2)
watch()
kv('record_end', utc())
kv('qemu_alive_at_record_end', int(alive()))
log('record end; qemu alive =', alive())
