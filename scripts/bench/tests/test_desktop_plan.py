# SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later
"""The attached Windows campaign must preserve failures and finish its stability hold."""
import importlib.util
from pathlib import Path
from types import SimpleNamespace
import unittest

PATH = Path(__file__).resolve().parents[1] / 'diesweep/desktop/matrix.py'


def load():
    spec = importlib.util.spec_from_file_location('desktop_matrix', PATH)
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


class DesktopPlanTests(unittest.TestCase):
    def run_plan(self, verdict='PASS', restart='', fail_health_at=None, seconds=600):
        m = load()
        calls = []
        count = 0
        def health():
            nonlocal count
            count += 1
            calls.append('health')
            if count == fail_health_at:
                raise RuntimeError('display fault')
        def app(a):
            calls.append(a['id'])
            return dict(app=a['id'], verdict=verdict, restart=restart)
        def hold():
            calls.append('hold')
            return seconds
        records = []
        self.calls = calls
        session = SimpleNamespace(run_app=app)
        return m.run_plan(session, [{'id':'heaven_d3d11'}, {'id':'valley_opengl'}],
                          records.append, health, hold), calls, records

    def test_unigine_rows_run_before_hold(self):
        rc, calls, records = self.run_plan()
        self.assertEqual(rc, 0)
        self.assertLess(calls.index('heaven_d3d11'), calls.index('hold'))
        self.assertLess(calls.index('valley_opengl'), calls.index('hold'))
        self.assertEqual(len(records), 2)

    def test_failure_verdict_is_not_a_pass(self):
        self.assertEqual(self.run_plan(verdict='FAIL')[0], 1)

    def test_notrun_is_not_a_pass(self):
        self.assertEqual(self.run_plan(verdict='NOTRUN')[0], 1)

    def test_timeout_does_not_start_more_gpu_work(self):
        with self.assertRaisesRegex(RuntimeError, 'app-timeout'):
            self.run_plan(verdict='TIMEOUT', restart='app-timeout')

    def test_restart_reason_refuses_a_passed_row(self):
        with self.assertRaisesRegex(RuntimeError, 'guest-rebooted'):
            self.run_plan(restart='guest-rebooted')

    def test_health_failure_before_apps_is_fatal(self):
        with self.assertRaisesRegex(RuntimeError, 'display fault'):
            self.run_plan(fail_health_at=1)

    def test_health_failure_after_app_is_fatal(self):
        with self.assertRaisesRegex(RuntimeError, 'display fault'):
            self.run_plan(fail_health_at=2)
        self.assertNotIn('valley_opengl', self.calls)
        self.assertNotIn('hold', self.calls)

    def test_health_failure_after_unigine_prevents_hold(self):
        with self.assertRaisesRegex(RuntimeError, 'display fault'):
            self.run_plan(fail_health_at=3)
        self.assertNotIn('hold', self.calls)

    def test_short_hold_is_not_accepted(self):
        with self.assertRaisesRegex(RuntimeError, 'hold'):
            self.run_plan(seconds=599)

    def test_missing_hold_is_not_accepted(self):
        with self.assertRaisesRegex(RuntimeError, 'hold'):
            self.run_plan(seconds=0)


class AppDiskTests(unittest.TestCase):
    def test_attach_does_not_hotplug_a_root_bus_controller(self):
        m = load()
        calls = []
        q = SimpleNamespace(cmd=lambda *args: calls.append(args),
                            attach_cdrom=lambda *args, **kw: calls.append(('cd', args, kw)))
        m.attach_app_disk(q, 'matrix.iso')
        self.assertEqual(len(calls), 1)
        self.assertEqual(calls[0][0], 'cd')

    def test_app_disk_uses_the_boot_controller(self):
        m = load()
        calls = []
        q = SimpleNamespace(cmd=lambda *args: calls.append(args),
                            attach_cdrom=lambda *args, **kw: calls.append(('cd', args, kw)))
        m.attach_app_disk(q, 'matrix.iso')
        self.assertEqual(len(calls), 1)
        self.assertEqual(calls[-1][2]['xhci_bus'], 'xhci.0')


class GuestIsoTests(unittest.TestCase):
    def mount(self, rc=0, marker='S4_APPDISK_MOUNTED'):
        m=load();calls=[]
        vm=SimpleNamespace(attach_disk=lambda *_: self.fail('guest ISO mode must not hotplug a USB CD'))
        q=SimpleNamespace(powershell=lambda *args,**kw: (calls.append((args,kw)) or SimpleNamespace(exitcode=rc,text=lambda:marker)))
        result=m.attach_media(vm,q,'source.iso',True,dict(sha256='a'*64,size=16))
        return result,calls

    def test_guest_iso_uses_the_existing_windows_mount_path(self):
        result,calls=self.mount();self.assertEqual(result,'guest-mounted ISO')
        self.assertEqual(calls[0][0][0],r'C:\kf\s4_mount_apps.ps1')

    def test_guest_mount_receives_the_pinned_hash_and_size(self):
        _,calls=self.mount();self.assertEqual(calls[0][0][1],['-ExpectedSha256','a'*64,'-ExpectedSize','16'])

    def test_failed_mount_cannot_start_the_matrix(self):
        with self.assertRaisesRegex(RuntimeError,'mount'):self.mount(rc=1)

    def test_missing_mount_receipt_cannot_start_the_matrix(self):
        with self.assertRaisesRegex(RuntimeError,'mount'):self.mount(marker='')

class NvrtcBootstrapTests(unittest.TestCase):
    def stage(self,rc=0,text='S4_NVRTC_STAGED {}'):
        m=load();calls=[]
        q=SimpleNamespace(powershell=lambda *args,**kw:(calls.append((args,kw)) or SimpleNamespace(exitcode=rc,text=lambda:text)))
        result=m.stage_nvrtc(q,dict(sha256='a'*64,size=314608748))
        return result,calls
    def test_bootstrap_passes_manifest_pin_to_the_real_stager(self):
        _,calls=self.stage();self.assertEqual(calls[0][0],(r'C:\kf\s4_stage_nvrtc.ps1',['-ExpectedSha256','a'*64,'-ExpectedSize','314608748']))
    def test_bootstrap_retains_its_guest_receipt(self):
        text,_=self.stage();self.assertIn('S4_NVRTC_STAGED',text)
    def test_failed_bootstrap_cannot_dispatch_apps(self):
        with self.assertRaisesRegex(RuntimeError,'NVRTC'):self.stage(rc=1)
    def test_absent_bootstrap_receipt_cannot_dispatch_apps(self):
        with self.assertRaisesRegex(RuntimeError,'NVRTC'):self.stage(text='')
