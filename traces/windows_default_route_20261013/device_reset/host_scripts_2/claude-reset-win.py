#!/usr/bin/env python3
"""Windows reset test driver (trusted host). Boots are done by the launcher; this drives QMP resets and QGA checks.
usage: reset_win.py <run-dir> <outfile>"""
import sys, time, json, socket, subprocess, os, re
sys.path.insert(0, '/data/kf-prod-defaults-host/scripts/bench/windows/appmatrix')
from qga import Qga
run, out = sys.argv[1], sys.argv[2]
PW = open('/var/lib/kf-windows-20261005/windows-desktop/secrets/win_password').read().strip()
def log(m):
    open(out, 'a').write(time.strftime('%H:%M:%S ') + m + '\n'); print(m, flush=True)
def qmp(cmd, **args):
    s = socket.socket(socket.AF_UNIX); s.connect(run + '/qmp.sock'); f = s.makefile('rw'); f.readline()
    def c(x):
        f.write(json.dumps(x) + '\n'); f.flush()
        while True:
            l = json.loads(f.readline())
            if 'return' in l or 'error' in l: return l
    c({'execute': 'qmp_capabilities'}); r = c({'execute': cmd, 'arguments': args} if args else {'execute': cmd}); s.close(); return r
def qemu_log_tail(pat, n=3):
    try:
        return [l[:230] for l in open(run + '/qemu.log', errors='ignore') if re.search(pat, l)][-n:]
    except Exception: return []
def new_qga(): return Qga(run + '/qga.sock')
def wait_qga(t=420):
    t0 = time.time()
    while time.time() - t0 < t:
        try:
            q = new_qga()
            if q.ping(): return q
        except Exception: pass
        time.sleep(3)
    return None
PS = r'C:\Windows\System32\WindowsPowerShell\v1.0\powershell.exe'
def ps(q, script, timeout=90):
    r = q.exec(PS, ['-NoProfile', '-Command', script], timeout=timeout)
    return (r.out.decode(errors='replace') if isinstance(r.out, bytes) else str(r.out)).strip()
GPU = r"(Get-CimInstance Win32_VideoController | Where-Object {$_.Name -match 'NVIDIA'} | ForEach-Object { 'code=' + $_.ConfigManagerErrorCode + ' ' + $_.Name + ' ' + $_.DriverVersion }) -join ';'"
def signin():
    r = subprocess.run([sys.executable, '/data/kf-prod-defaults-host/scripts/bench/windows/signin.py', run + '/qmp.sock', run + '/qga.sock'],
                       env=dict(os.environ, KF_GUEST_PW=PW), capture_output=True, text=True, timeout=240)
    return r.returncode
def phase():
    l = qemu_log_tail(r'kf3: family=', 1)
    m = re.search(r'phase=(\w+)', l[0]) if l else None
    return m.group(1) if m else '?'
BOOT = r"(Get-CimInstance Win32_OperatingSystem).LastBootUpTime.ToString('s')"
def bds_count():
    try: return open(run + '/serial.log', 'rb').read().count(b'BdsDxe: starting Boot0003')
    except Exception: return 0
def wait_new_boot(old_boot, old_bds, t=480):
    t0 = time.time()
    while time.time() - t0 < t:
        if bds_count() > old_bds:
            try:
                q = new_qga()
                if q.ping():
                    b = ps(q, BOOT, 40)
                    if re.match(r'\d{4}-\d\d-\d\dT', b) and b != old_boot: return q, b
                q.close()
            except Exception: pass
        time.sleep(4)
    return None, None
def signin_ok(tries=4):
    for k in range(tries):
        rc = signin()
        if rc == 0: return True
        time.sleep(15)
    return False
verdict = []
def check(name, ok, detail=''):
    verdict.append((name, ok)); log(f"CHECK {'PASS' if ok else 'FAIL'} {name} {detail}")
q = wait_qga(600)
check('initial boot reaches the guest agent', q is not None)
if not q: sys.exit(2)
log('gpu: ' + ps(q, GPU))
log('bcdedit: ' + ps(q, r"bcdedit /set '{default}' recoveryenabled No; bcdedit /set '{default}' bootstatuspolicy ignoreallfailures", 40).replace(chr(10), ' ')[:200])
boot0 = ps(q, BOOT, 40); log('boot0 ' + boot0)
q.close()
# ---- W1: hard reset at the desktop with the GPU in use, x3
for i in range(1, 4):
    try: q.close()
    except Exception: pass
    ok = signin_ok(); check(f'W1.{i} signed in before reset', ok)
    q = new_qga()
    log(f'W1.{i}: kayfabe phase={phase()}'); time.sleep(30)
    ob = ps(q, BOOT, 40); od = bds_count(); q.close()
    log(f'W1.{i}: QMP system_reset (phase before={phase()}, boot={ob})'); log(str(qmp('system_reset')))
    q, nb = wait_new_boot(ob, od)
    check(f'W1.{i} real reboot + guest agent after hard reset', q is not None, f'new boot {nb}')
    if not q: break
    g = ps(q, GPU); log(f'W1.{i} gpu: {g}')
    check(f'W1.{i} NVIDIA adapter Code 0 after hard reset', 'code=0' in g, g)
    for l in qemu_log_tail(r'device reset #\d+: vCPUs out', 1): log('   ' + l[:200])
    time.sleep(20)
# ---- W2: Device Manager disable/enable x3 (needs a signed-in session only for display; PnP works as SYSTEM)
for i in range(1, 4):
    d = ps(q, r"$d=Get-PnpDevice -Class Display | Where-Object {$_.FriendlyName -match 'NVIDIA'}; Disable-PnpDevice -InstanceId $d.InstanceId -Confirm:$false -ErrorAction Stop; 'disabled'", 120)
    log(f'W2.{i} disable: {d} phase={phase()}'); time.sleep(6)
    e = ps(q, r"$d=Get-PnpDevice -Class Display | Where-Object {$_.FriendlyName -match 'NVIDIA'}; Enable-PnpDevice -InstanceId $d.InstanceId -Confirm:$false -ErrorAction Stop; 'enabled'", 120)
    log(f'W2.{i} enable: {e}'); time.sleep(12)
    g = ps(q, GPU); log(f'W2.{i} gpu: {g} phase={phase()}')
    check(f'W2.{i} adapter Code 0 after disable/enable', 'code=0' in g, g)
    for l in qemu_log_tail(r'UNLOADING_GUEST_DRIVER|GSP .* Cold|device reset', 2): log('   ' + l[:200])
# ---- W3: clean Restart
log('W3: Restart-Computer'); ps(q, 'Restart-Computer -Force', 30)
time.sleep(20); q = wait_qga(420)
check('W3 guest agent after clean restart', q is not None)
if q:
    g = ps(q, GPU); log(f'W3 gpu: {g}'); check('W3 adapter Code 0 after clean restart', 'code=0' in g, g)
log('SUMMARY ' + ' '.join(f"{n}={'PASS' if ok else 'FAIL'}" for n, ok in verdict))
sys.exit(0 if all(ok for _, ok in verdict) else 1)
