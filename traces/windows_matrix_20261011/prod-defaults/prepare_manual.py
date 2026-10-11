#!/usr/bin/env python3
"""Prepare credentials/settings only; never sign in, send input or start an app."""
import base64
import json
import sys
import time
from pathlib import Path

sys.path.insert(0, '/data/kf-prod-defaults-host/scripts/bench/windows/appmatrix')
from qga import Qga, Qmp, ppm_to_png

w = Path('/var/lib/kf-windows-20261005')
run = w / 'boundary-kayfabe-1380'
flags = json.loads((run / 'command.json').read_text())['flags']
assert 'KF3_TIMER_MAP' not in flags, flags
assert all(v == '1' for v in flags.values()), flags
allowed = set("WIN_USER_CHANNELS_PASSTHROUGH WIN_TWIN_DEFAPI_OBJECT KERNEL_GR_CE KERNEL_GR_WORK TRANSLATED_CE_RELAY DEFERRED_API PREEMPT_BIND_PROBE DISPLAY_IMP_ENABLE DISPLAY_CTRL_PROBE ZCULL_BIND_PROBE SW_RUNLIST_HOST_OWNED ASYNC_PREEMPT GFX_POOL_PROBE SW_RUNLIST_PROBE MEMORY_LIST_PROBE DISPLAY_SDR_COLOR DISPLAY_CAPS_PROBE DISPLAY_LUT_MIRROR DISPLAY_ILUT_OFFSET_256".split())
assert set(flags) == {'KF3_' + x for x in allowed}, flags
q = Qga(str(run / 'qga.sock'))
end = time.monotonic() + 900
while not q.ping():
    if time.monotonic() >= end:
        raise SystemExit('manual preparation: guest agent deadline; no input sent')
    time.sleep(2)
password = (w / 'windows-desktop/secrets/win_password').read_text().strip()
q.call('guest-set-user-password', {'username': 'vast',
       'password': base64.b64encode(password.encode()).decode(), 'crypted': False})
password = None
ps = r'''
$ErrorActionPreference = 'Stop'
$p = 'HKLM:\SOFTWARE\Microsoft\Windows NT\CurrentVersion\Winlogon'
New-ItemProperty -Path $p -Name AutoAdminLogon -Value '0' -PropertyType String -Force | Out-Null
Remove-ItemProperty -Path $p -Name DefaultPassword -ErrorAction SilentlyContinue
New-ItemProperty -Path 'HKLM:\SOFTWARE\Microsoft\Windows\Dwm' -Name OverlayTestMode -Value 5 -PropertyType DWord -Force | Out-Null
$gpu = @(Get-CimInstance Win32_VideoController | Where-Object {$_.Name -match 'NVIDIA'} | ForEach-Object { @{name=$_.Name; code=$_.ConfigManagerErrorCode; driver=$_.DriverVersion} })
@{username='vast'; autologon=(Get-ItemProperty -Path $p).AutoAdminLogon; nvidia=$gpu; input_sent=$false; apps_started=$false} | ConvertTo-Json -Depth 5 -Compress
'''
r = q.exec(r'C:\Windows\System32\WindowsPowerShell\v1.0\powershell.exe',
           ['-NoProfile', '-Command', ps], timeout=90)
if r.exitcode != 0 or r.timed_out:
    raise SystemExit('manual preparation: settings check failed; no input sent')
d = json.loads(r.text().strip().lstrip('\ufeff'))
assert d['autologon'] == '0', d
assert len(d['nvidia']) == 1 and d['nvidia'][0]['code'] == 0, d
(run / 'manual-settings.json').write_text(json.dumps(d, indent=2) + '\n')
q.close()
m = Qmp(str(run / 'qmp.sock'))
m.attach_cdrom(str(w / 'codex-supplement-20261011/full-image-a61af1c3/kfapps_supplement.img'))
m.screendump(str(run / 'manual-login.ppm'))
ppm_to_png(str(run / 'manual-login.ppm'), str(run / 'manual-login.png'))
(run / 'manual-login.ppm').unlink()
print('MANUAL_READY username=vast input_sent=false apps_started=false')
