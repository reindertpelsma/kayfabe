#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later
"""Execute the real PowerShell health gate with deterministic Windows observations."""
import json
import os
from pathlib import Path
import shutil
import subprocess
import tempfile
import unittest

HEALTH = Path(__file__).resolve().parents[1] / 'diesweep/desktop/health.ps1'
PWSH = os.environ.get('KF_PWSH') or shutil.which('pwsh')
if not PWSH and Path('/root/am/pwsh/pwsh').is_file():
    PWSH = '/root/am/pwsh/pwsh'

BOOT = '2026-10-22T04:25:15Z'
DESKTOP = '2026-10-22T04:26:00Z'


@unittest.skipUnless(PWSH, 'PowerShell required to execute the production gate')
class DesktopHealthTests(unittest.TestCase):
    def run_gate(self, events=(), explorer=True, user='FIXTURE\\kf', log_readable=True):
        fixture = dict(events=list(events), explorer=explorer, user=user,
                       log_readable=log_readable, boot=BOOT, desktop=DESKTOP)
        wrapper = r'''
$f=Get-Content -Raw $env:HEALTH_FIXTURE | ConvertFrom-Json
function Get-CimInstance {
 param($ClassName)
 switch($ClassName) {
  'Win32_OperatingSystem' { [pscustomobject]@{LastBootUpTime=[datetime]$f.boot} }
  'Win32_ComputerSystem' { [pscustomobject]@{UserName=$f.user} }
  'Win32_VideoController' { [pscustomobject]@{PNPDeviceID='PCI\VEN_10DE';DriverVersion='32.0.15.8088';ConfigManagerErrorCode=0;CurrentHorizontalResolution=1920;CurrentVerticalResolution=1080} }
 }
}
function Get-WinEvent {
 param($LogName,$MaxEvents,$FilterHashtable,$ErrorAction)
 if(!$f.log_readable){throw 'access denied'}
 if($MaxEvents){return [pscustomobject]@{Id=1}}
 foreach($e in $f.events){[pscustomobject]@{Id=$e.Id;ProviderName=$e.ProviderName;TimeCreated=[datetime]$e.TimeCreated;Message='fixture'}}
}
function Get-Process {
 param($Name,$ErrorAction)
 if($Name -eq 'dwm' -or ($Name -eq 'explorer' -and $f.explorer)) {
  [pscustomobject]@{Name=$Name;Id=50;SessionId=1;StartTime=[datetime]$f.desktop}
 }
}
& $env:HEALTH_SCRIPT
'''
        with tempfile.TemporaryDirectory() as tmp:
            path = Path(tmp)
            (path / 'fixture.json').write_text(json.dumps(fixture))
            (path / 'run.ps1').write_text(wrapper)
            env = dict(os.environ, HEALTH_FIXTURE=str(path / 'fixture.json'),
                       HEALTH_SCRIPT=str(HEALTH))
            p = subprocess.run([PWSH, '-NoProfile', '-NonInteractive', '-File', str(path / 'run.ps1')],
                               env=env, capture_output=True, text=True, timeout=30)
        rows = [json.loads(x.split(' ', 1)[1]) for x in p.stdout.splitlines()
                if x.startswith('S2_HEALTH ')]
        return p, rows[0] if rows else {}

    @staticmethod
    def event(provider, event_id=153, when='2026-10-22T04:25:20Z'):
        return dict(Id=event_id, ProviderName=provider, TimeCreated=when)

    def test_boot_153_before_desktop_does_not_refuse(self):
        p, h = self.run_gate([self.event('Microsoft-Windows-Kernel-Boot')])
        self.assertEqual(p.returncode, 0, p.stdout + p.stderr)
        self.assertEqual(h['events153'], 0)

    def test_boot_153_is_retained_with_its_epoch(self):
        p, h = self.run_gate([self.event('Microsoft-Windows-Kernel-Boot')])
        self.assertEqual(h.get('boot153_pre_desktop'), 1, p.stdout + p.stderr)
        self.assertEqual(h.get('desktop_start'), DESKTOP.replace('Z', '.0000000Z'))

    def test_nvidia_153_before_and_after_desktop_refuses(self):
        for when in ('2026-10-22T04:25:20Z', '2026-10-22T04:27:20Z'):
            p, h = self.run_gate([self.event('nvlddmkm', when=when)])
            self.assertNotEqual(p.returncode, 0, p.stdout)
            self.assertEqual(h['events153'], 1)

    def test_boot_provider_after_desktop_refuses(self):
        p, h = self.run_gate([self.event('Microsoft-Windows-Kernel-Boot', when='2026-10-22T04:27:20Z')])
        self.assertNotEqual(p.returncode, 0, p.stdout)
        self.assertEqual(h.get('unclassified153'), 1)

    def test_display_4101_and_bugcheck_refuse(self):
        for provider, event_id in (('Display', 4101), ('Microsoft-Windows-WER-SystemErrorReporting', 1001)):
            p, _ = self.run_gate([self.event(provider, event_id)])
            self.assertNotEqual(p.returncode, 0, p.stdout)

    def test_missing_explorer_refuses_even_with_dwm(self):
        p, _ = self.run_gate(explorer=False)
        self.assertNotEqual(p.returncode, 0, p.stdout)

    def test_no_interactive_user_refuses(self):
        p, _ = self.run_gate(user='')
        self.assertNotEqual(p.returncode, 0, p.stdout)

    def test_null_interactive_user_refuses(self):
        p, _ = self.run_gate(user=None)
        self.assertNotEqual(p.returncode, 0, p.stdout)

    def test_late_boot_event_never_claims_health_pass(self):
        p, _ = self.run_gate([self.event('Microsoft-Windows-Kernel-Boot', when='2026-10-22T04:28:00Z')])
        self.assertNotIn('S2_HEALTH_PASS ', p.stdout)
        self.assertNotEqual(p.returncode, 0)

    def test_unreadable_log_refuses(self):
        p, _ = self.run_gate(log_readable=False)
        self.assertNotEqual(p.returncode, 0, p.stdout)

    def test_clean_desktop_passes(self):
        p, _ = self.run_gate()
        self.assertEqual(p.returncode, 0, p.stdout + p.stderr)


if __name__ == '__main__':
    unittest.main()
