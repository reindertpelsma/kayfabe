#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later
"""Run the existing PowerShell app matrix on one already-owned, signed-in VM."""
import argparse
import datetime
import json
import os
from pathlib import Path
import sys
import time

AM = Path(__file__).resolve().parents[2] / 'windows/appmatrix'
sys.path.insert(0, str(AM))
import winapps as w


def attach_app_disk(qmp, iso):
    return qmp.attach_cdrom(iso, xhci_bus='xhci.0')


def attach_media(vm, q, iso, guest_iso, expected):
    if guest_iso:
        r=q.powershell(r'C:\kf\s4_mount_apps.ps1',
                       ['-ExpectedSha256',expected['sha256'],'-ExpectedSize',str(expected['size'])],timeout=180)
        if r.exitcode != 0 or 'S4_APPDISK_MOUNTED' not in r.text():
            raise RuntimeError('guest app-disk mount failed: '+r.text()[-400:])
        return 'guest-mounted ISO'
    return vm.attach_disk(iso)


def stage_nvrtc(q, expected):
    r=q.powershell(r'C:\kf\s4_stage_nvrtc.ps1',
                   ['-ExpectedSha256',expected['sha256'],'-ExpectedSize',str(expected['size'])],timeout=180)
    if r.exitcode != 0 or 'S4_NVRTC_STAGED ' not in r.text():
        raise RuntimeError('NVRTC pinned supplement refused: '+r.text()[-400:])
    return r.text()


class ObservationBudgetExhausted(RuntimeError):
    """App dispatch must stop while its reserved stability/export budget remains."""


def budget_items(apps, end, skipped, now=lambda:datetime.datetime.now(datetime.timezone.utc)):
    partial = False
    for row in apps:
        if now() + datetime.timedelta(seconds=row['timeout_s']+690) >= end:
            partial = True
            skipped(row)
            continue
        yield row
    if partial:
        raise ObservationBudgetExhausted('lease observation/export budget exhausted')


def run_plan(session, apps, record, health, hold):
    health()
    failed = False
    partial = False
    try:
        for app in apps:
            result = session.run_app(app)
            record(result)
            health()
            if result['restart']:
                raise RuntimeError(result['restart'])
            failed |= result['verdict'] != 'PASS'
    except ObservationBudgetExhausted:
        partial = True
        health()
    if hold() < 600:
        raise RuntimeError('incomplete stability hold')
    health()
    return 2 if partial else int(failed)


class AttachedVm(w.BrokerVm):
    def __init__(self, root, qlog):
        self.root, self.qlog = Path(root), Path(qlog)
        self.qpid = int((self.root / 'run/qemu.pid').read_text())
        self.start = Path(f'/proc/{self.qpid}/stat').read_text().rsplit(')', 1)[1].split()[19]

    def run_dir(self):
        return str(self.root / 'run')

    def alive(self):
        try:
            return Path(f'/proc/{self.qpid}/stat').read_text().rsplit(')', 1)[1].split()[19] == self.start
        except OSError:
            return False

    def attach_disk(self, iso):
        return attach_app_disk(self._qmp(), iso)

    def gsp_cycles(self):
        return self.qlog.read_text(errors='replace').count('GSP phase Running -> Suspending')

    def screendump(self, png_path):
        ppm = png_path + '.ppm'
        try:
            self._qmp().cmd('screendump', {'filename':ppm})
            w.qgamod.ppm_to_png(ppm, png_path)
            return True
        except (OSError, w.qgamod.QgaError):
            return False
        finally:
            Path(ppm).unlink(missing_ok=True)


def main():
    p = argparse.ArgumentParser(description=__doc__)
    p.add_argument('--vm-root', required=True)
    p.add_argument('--qlog', required=True)
    p.add_argument('--out', required=True)
    p.add_argument('--iso', required=True)
    p.add_argument('--manifest', required=True)
    p.add_argument('--lease', required=True)
    p.add_argument('--guest-iso', action='store_true')
    p.add_argument('--override-dir', default='')
    a = p.parse_args()
    out = Path(a.out); out.mkdir(parents=True, exist_ok=False)
    log = w.Log(str(out / 'session.log'))
    doc = json.loads((AM / 'apps.json').read_text())
    man = json.loads(Path(a.manifest).read_text())
    lease = json.loads(Path(a.lease).read_text())
    end = datetime.datetime.fromisoformat(lease['created_utc']) + datetime.timedelta(minutes=lease['auto_destroy_minutes']-20)
    vm = AttachedVm(a.vm_root, a.qlog)
    cfg = w.Cfg(iso=a.iso, image_identity=man['image']['identity'], max_tdr=1, user='kf', override_dir=a.override_dir)
    g = w.GuestSession(cfg, vm, doc, man, str(out), log, 'desktop')
    g.q = w.qgamod.Qga(vm.qga_path())
    g._wait_session()
    g._push()
    g.q.file_write(r'C:\kf\s4_mount_apps.ps1', Path(__file__).with_name('mount_app_disk.ps1').read_bytes())
    log('APP_DISK', attach_media(vm,g.q,a.iso,a.guest_iso,man['image']))
    g._setup()
    if json.loads((Path(g.dir) / 'setup.json').read_text())['image_identity'] != man['image']['identity']:
        raise RuntimeError('app disk identity mismatch')
    g.q.file_write(r'C:\kf\s4_stage_nvrtc.ps1', Path(__file__).with_name('stage_nvrtc.ps1').read_bytes())
    nvrtc=next(x for x in man['packages'] if x['id']=='cuda_nvrtc')['files'][0]
    log(stage_nvrtc(g.q,nvrtc).strip())
    initial = g.health()
    g.boot_utc, g.start_utc = initial['boot_utc'], initial['utc']
    g.q.file_write(r'C:\kf\s4_health.ps1', Path(__file__).with_name('health.ps1').read_bytes())
    boot = None
    checks = 0

    def health():
        nonlocal boot, checks
        if not vm.alive():
            raise RuntimeError('owned VM exited')
        r = g.q.powershell(r'C:\kf\s4_health.ps1', timeout=90)
        checks += 1
        (out / f'health_{checks:03}.txt').write_text(r.text() + r.err.decode(errors='replace'))
        h = w.parse_marker(r.text(), 'S2_HEALTH')
        if r.exitcode != 0 or not h or 'S2_HEALTH_PASS ' not in r.text():
            raise RuntimeError('desktop health refused')
        if boot is not None and h['boot'] != boot:
            raise RuntimeError('guest rebooted')
        boot = h['boot']

    def record(result):
        w.append(str(out / 'win.res'), [result['line']])
        log(result['line'])

    def hold():
        start = time.monotonic()
        while time.monotonic() - start < 600:
            if not vm.alive():
                raise RuntimeError('VM exited during hold')
            time.sleep(5)
        elapsed = time.monotonic() - start
        log(f'ARM_HOLD_OBSERVED seconds={int(elapsed)}')
        return elapsed

    # Cover the requested Unigine paths early, then retain every remaining matrix row.
    first = ['driver_status', 'dxgi_adapters', 'nvidia_smi', 'cup2_ladder', 'cup3_ladder',
             'cup8_ladder', 'dxprobe_d3d11', 'dxprobe_d3d12', 'heaven_d3d11',
             'heaven_opengl', 'valley_d3d11', 'valley_opengl']
    ordered = sorted(doc['apps'], key=lambda x: first.index(x['id']) if x['id'] in first else len(first))
    budget_skips=[]
    def skipped(row):
        facts={'notrun':'lease observation/export budget exhausted',
               'utc':datetime.datetime.now(datetime.timezone.utc).isoformat()}
        (out/'apps').mkdir(exist_ok=True)
        record(g._finish(row,w.V.decide(row,facts,''),'',facts,None,need_restart=''))
        budget_skips.append(row['id'])
        (out/'budget_stop.json').write_text(json.dumps({'utc':facts['utc'],'skipped_apps':budget_skips},indent=2)+'\n')
    def rows():
        yield from budget_items(ordered,end,skipped)
    rc = 7
    try:
        log(f'MATRIX_START apps={len(ordered)} source={os.environ.get("S4_SOURCE", "unknown")}')
        rc = run_plan(g, rows(), record, health, hold)
        vm.screendump(str(out / 'desktop_after.png'))
        return rc
    finally:
        g.q.close()
        log(f'MATRIX_EXIT rc={rc}')


if __name__ == '__main__':
    sys.exit(main())
