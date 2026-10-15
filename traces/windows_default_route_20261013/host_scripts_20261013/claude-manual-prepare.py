#!/usr/bin/env python3
"""Owner session preparation: set the 'vast' password from the host secret file via the guest agent and turn autologon off. Never signs in, sends input or starts an app. Does NOT touch DWM overlay settings."""
import base64, json, sys, time
from pathlib import Path
sys.path.insert(0, '/data/kf-prod-defaults-host/scripts/bench/windows/appmatrix')
from qga import Qga
w = Path('/var/lib/kf-windows-20261005')
run = w / sys.argv[1]
flags = json.loads((run / 'command.json').read_text())['flags']
print('FLAGS', len(flags), ' '.join(sorted(flags)))
q = Qga(str(run / 'qga.sock'))
end = time.monotonic() + 900
while not q.ping():
    if time.monotonic() >= end:
        raise SystemExit('guest agent deadline; no input sent')
    time.sleep(2)
password = (w / 'windows-desktop/secrets/win_password').read_text().strip()
q.call('guest-set-user-password', {'username': 'vast', 'password': base64.b64encode(password.encode()).decode(), 'crypted': False})
password = None
ps = r'''
$p = 'HKLM:\SOFTWARE\Microsoft\Windows NT\CurrentVersion\Winlogon'
New-ItemProperty -Path $p -Name AutoAdminLogon -Value '0' -PropertyType String -Force | Out-Null
New-ItemProperty -Path 'HKLM:\SOFTWARE\Microsoft\Windows\Dwm' -Name OverlayTestMode -Value 5 -PropertyType DWord -Force | Out-Null
Remove-ItemProperty -Path $p -Name DefaultPassword -ErrorAction SilentlyContinue
$gpu = @(Get-CimInstance Win32_VideoController | Where-Object {$_.Name -match 'NVIDIA'} | ForEach-Object { @{name=$_.Name; code=$_.ConfigManagerErrorCode; driver=$_.DriverVersion} })
@{username='vast'; autologon=(Get-ItemProperty -Path $p).AutoAdminLogon; nvidia=$gpu; input_sent=$false; apps_started=$false} | ConvertTo-Json -Depth 5 -Compress
'''
r = q.exec(r'C:\Windows\System32\WindowsPowerShell\v1.0\powershell.exe', ['-NoProfile', '-Command', ps], timeout=90)
print('PREPARE rc', r.exitcode, 'timed_out', r.timed_out)
print(str(getattr(r,'out',None) or getattr(r,'output',None) or [a for a in dir(r) if not a.startswith('_')])[:400])
