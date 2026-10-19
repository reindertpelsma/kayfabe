#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later
"""MEASUREMENT ONLY (trap experiments 2026-10-19): one Windows arm = fresh overlay, boot, tiny probe, Steel Nomad xN, health.
Run on the trusted host UNDER the GPU lock:  flock /tmp/kayfabe-fastguest.lock python3 arm.py --arm control1 --rev 02911cbb-ctl --flags "KF3_LIFECYCLE_COUNT=1 ..."
Score -> fps: P1 measured app fps = score/100 (score 2967 = 29.68 fps); this script reports score and score/100, nothing else."""
import argparse, base64, json, os, re, statistics, subprocess, sys, time
from pathlib import Path

ROOT = Path('/data/claude-trapexp-20261019'); HERE = Path(__file__).resolve().parent
OWNER = Path('/var/lib/kf-windows-20261005/win11-full-b')
AG = ['python3', '/data/kfagent_server.py', 'run']

def sh(cmd, **k):
    return subprocess.run(cmd, shell=isinstance(cmd, str), text=True, capture_output=True, **k)

def agent(text, t=90):
    r = subprocess.run(AG + [text, str(t)], text=True, capture_output=True)
    return r.stdout

def push(name):
    data = (HERE / name).read_bytes(); b = base64.b64encode(data).decode()
    agent("New-Item -ItemType Directory -Force C:\\kf\\TE | Out-Null; Remove-Item C:\\kf\\TE\\%s.b64 -ErrorAction SilentlyContinue" % name, 60)
    for i in range(0, len(b), 24000):
        agent("Add-Content -NoNewline -Path C:\\kf\\TE\\%s.b64 -Value '%s'" % (name, b[i:i + 24000]), 60)
    out = agent("[IO.File]::WriteAllBytes('C:\\kf\\TE\\%s',[Convert]::FromBase64String((Get-Content -Raw C:\\kf\\TE\\%s.b64))); (Get-Item C:\\kf\\TE\\%s).Length" % (name, name, name), 60)
    assert str(len(data)) in out, (name, out)

def it(script, args, wait=60, log='C:\\kf\\TE\\ui.txt'):
    return agent("& C:\\kf\\TE\\sl_it.ps1 -Script C:\\kf\\TE\\%s -ScriptArgs '%s' -Wait %d -Log %s" % (script, args, wait, log), wait + 60)

def xid():
    return int(sh("journalctl -k --no-pager | grep -c 'NVRM: Xid'").stdout.strip() or 0)

def results():
    return re.findall(r'RESULT (\S+) (\S+)', agent((HERE / 'list_results.ps1').read_text(), 90))

def score_of(name):
    m = re.search(r'SteelNomad-(\d+)-', name); return int(m.group(1)) if m else None

def med(a):
    return statistics.median(a) if a else None

def main():
    p = argparse.ArgumentParser()
    p.add_argument('--arm', required=True); p.add_argument('--rev', required=True); p.add_argument('--flags', default='')
    p.add_argument('--sn', type=int, default=5); p.add_argument('--probe-reps', type=int, default=10)
    p.add_argument('--first-click', default='0.3786,0.5852'); p.add_argument('--n', default='4100')
    p.add_argument('--skip-sn', action='store_true')
    a = p.parse_args()
    D = ROOT / 'disk' / a.arm; A = ROOT / 'arms' / a.arm; A.mkdir(parents=True, exist_ok=True)
    res = {'arm': a.arm, 'rev': a.rev, 'flags': a.flags, 'start': time.strftime('%FT%T%z')}
    # 1. fresh throwaway overlay; the owner's disk is only a read-only backing file (it must not be in use).
    sh(['rm', '-rf', str(D)]); D.mkdir(parents=True)
    r = sh(['qemu-img', 'create', '-f', 'qcow2', '-b', str(OWNER / 'windows11.qcow2'), '-F', 'qcow2', str(D / 'windows11.qcow2')]); assert r.returncode == 0, r.stderr
    sh(['cp', str(OWNER / 'OVMF_VARS.fd'), str(OWNER / 'identity.env'), str(D) + '/'])
    res['xid_before'] = xid()
    env = dict(os.environ, REV=a.rev, N=a.n, SIDE=a.arm, WIN_FLAGS=a.flags)
    lf = open(A / 'launch.log', 'w')
    proc = subprocess.Popen(['bash', str(ROOT / 'launch.sh')], env=env, stdout=lf, stderr=subprocess.STDOUT, start_new_session=True)
    qpid = None; t0 = time.time()
    while time.time() - t0 < 600:
        txt = (A / 'launch.log').read_text(errors='replace')
        m = re.search(r'OWNER_SESSION_READY=\S+ run=\S+ rev=\S+ install=\S+ qpid=(\d+)', txt)
        if m: qpid = int(m.group(1)); break
        if proc.poll() is not None: res['error'] = 'launch exited'; break
        time.sleep(3)
    if qpid is None:
        json.dump(res, open(A / 'result.json', 'w'), indent=1); print('NO QEMU'); return 2
    res['qpid'] = qpid
    res['environ_kf3'] = sorted(x for x in open('/proc/%d/environ' % qpid, 'rb').read().decode('utf-8', 'replace').split('\0') if x.startswith('KF3_'))
    res['exe_md5'] = sh(['md5sum', '/proc/%d/exe' % qpid]).stdout.split()[0]
    # 2. wait for the guest agent
    t0 = time.time(); up = False
    while time.time() - t0 < 600:
        if 'kfadmin' in agent('whoami', 20).lower() or 'system' in agent('whoami', 20).lower(): up = True; break
        time.sleep(5)
    res['agent_up_s'] = round(time.time() - t0)
    if not up:
        res['error'] = 'agent never came up'
    else:
        for f in ('sl_it.ps1', 'sl_probe.ps1', 'sn_ui.ps1', 'sn_upload.ps1', 'health.ps1', 'list_results.ps1'): push(f)
        res['health0'] = agent((HERE / 'health.ps1').read_text(), 60).strip()
        # 3. tiny probe: reps x (DIRECT=0, COMPUTE=2, COPY=3), 2000 submits each; medians per step
        seq = ','.join('T:%d:copy:2000' % q for _ in range(a.probe_reps) for q in (0, 2, 3))
        agent('Remove-Item C:\\kf\\TE\\lat.txt -ErrorAction SilentlyContinue', 30)
        it('sl_probe.ps1', '-Out C:\\kf\\TE\\lat.txt -Seq %s' % seq, 30, 'C:\\kf\\TE\\lat.txt')
        t0 = time.time(); lat = ''
        while time.time() - t0 < 900:
            lat = agent('Get-Content C:\\kf\\TE\\lat.txt -ErrorAction SilentlyContinue', 60)
            if 'SEQ_END' in lat: break
            time.sleep(15)
        (A / 'lat.txt').write_text(lat)
        meds = [float(x) for x in re.findall(r'GP T iteration period us n=\d+ med=([\d.]+)', lat)]
        names = ['DIRECT', 'COMPUTE', 'COPY']
        res['probe_n'] = len(meds) // 3
        res['probe_median_us'] = {names[k]: med(meds[k::3]) for k in range(3)}
        res['probe_all_us'] = {names[k]: meds[k::3] for k in range(3)}
        # 4. Steel Nomad
        scores = []
        if not a.skip_sn:
            before = {n for n, _ in results()}
            it('sn_ui.ps1', '-Action launch -Out C:\\kf\\TE\\ui.txt', 120)
            fx, fy = a.first_click.split(',')
            it('sn_ui.ps1', '-Action click -FX %s -FY %s -Out C:\\kf\\TE\\ui.txt' % (fx, fy), 60)
            for i in range(a.sn):
                t0 = time.time(); new = None
                while time.time() - t0 < 480:
                    time.sleep(10)
                    cur = [n for n, _ in results() if n not in before]
                    if cur: new = cur[-1]; break
                if not new: res['error'] = 'no result for run %d' % (i + 1); break
                before.add(new); scores.append(score_of(new)); print('SN', a.arm, i + 1, new, flush=True)
                if i + 1 < a.sn: it('sn_ui.ps1', '-Action run -Out C:\\kf\\TE\\ui.txt', 60)
        res['sn_scores'] = scores; res['sn_fps_score_over_100'] = [s / 100 for s in scores if s]
        if scores:
            res['sn_median'] = med(scores); res['sn_min'] = min(scores); res['sn_max'] = max(scores)
        res['health1'] = agent((HERE / 'health.ps1').read_text(), 60).strip()
    res['xid_after'] = xid(); res['xid_delta'] = res['xid_after'] - res['xid_before']
    # 5. stop (the launcher's own stop file; never kill -9)
    (ROOT / 'STOP_VM').touch()
    t0 = time.time()
    while time.time() - t0 < 240 and proc.poll() is None: time.sleep(3)
    res['stopped'] = proc.poll() is not None
    ql = list((ROOT / 'W').glob('boundary-kayfabe-%s/qemu.log' % a.n))
    if ql:
        t = ql[0].read_text(errors='replace')
        sts = [l for l in t.splitlines() if 'irq[writes=' in l]
        res['last_irq_line'] = re.sub(r'.*(irq\[[^\]]*\]).*', r'\1', sts[-1]) if sts else None
        res['trapexp_lines'] = [l for l in t.splitlines() if 'trapexp' in l][-3:]
        res['qemu_log'] = str(ql[0])
    res['end'] = time.strftime('%FT%T%z')
    json.dump(res, open(A / 'result.json', 'w'), indent=1); print(json.dumps({k: v for k, v in res.items() if k not in ('probe_all_us',)}, indent=1))
    return 0

if __name__ == '__main__':
    sys.exit(main())
