#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later
"""pp.py: Premiere Pro actions on the running guest of ppsession.py (runs on the pc; guest transport = the drv.py helpers of adobe_addon_20261121_driver).

  pp.py setup                         elevated: WER LocalDumps, the Premiere Startup stub (PR #588 mechanism), pp.jsx and pp_run.ps1 into C:\\kfhw\\pp
  pp.py run CLIP MODE TAG [HOLD] [keys]  one attempt of action A in the console session; prints result.json
  pp.py crash SINCE_UTC               crash evidence since SINCE: Premiere SentryIO records (time, size, renderer tag, crashing thread),
                                      Application 1000/1001/1002, WER report headers, LocalDumps sizes, display-driver events
  pp.py shot NAME                     console screenshot (stays on the pc)
"""
import os
import sys

sys.path.insert(0, "/workspace/bench/adobe_addon_20261121_driver")
import drv  # noqa: E402

HERE = os.path.dirname(os.path.abspath(__file__))
STUB_B64 = ("dHJ5IHsKICB2YXIgaiA9IG5ldyBGaWxlKCJDOi9rZmh3L0BBUFBAX2pvYi50eHQiKTsKICBpZiAoai5leGlzdHMpIHsKICAgIGoub3BlbigiciIpOyB2YXIgcCA9IGoucmVh"
            "ZCgpOyBqLmNsb3NlKCk7IGoucmVtb3ZlKCk7CiAgICB2YXIgbCA9IG5ldyBGaWxlKCJDOi9rZmh3L0BBUFBAX3N0dWIubG9nIik7IGwub3BlbigiYSIpOyBsLndyaXRlbG4obmV3"
            "IERhdGUoKS5nZXRUaW1lKCkgKyAiIGpvYiAiICsgcCArICIgc2NoZWR1bGVUYXNrPSIgKyAodHlwZW9mIGFwcC5zY2hlZHVsZVRhc2spKTsgbC5jbG9zZSgpOwogICAgJC5zbGVl"
            "cCg4MDAwKTsKICAgIHRyeSB7ICQuZXZhbEZpbGUobmV3IEZpbGUocCkpOyB9IGNhdGNoIChlKSB7IHZhciBmID0gbmV3IEZpbGUoIkM6L2tmaHcvQEFQUEBfc3R1Yi5sb2ciKTsg"
            "Zi5vcGVuKCJhIik7IGYud3JpdGVsbigiRVJSICIgKyBlKTsgZi5jbG9zZSgpOyB9CiAgICB2YXIgbDIgPSBuZXcgRmlsZSgiQzova2Zody9AQVBQQF9zdHViLmxvZyIpOyBsMi5v"
            "cGVuKCJhIik7IGwyLndyaXRlbG4obmV3IERhdGUoKS5nZXRUaW1lKCkgKyAiIGpvYiByZXR1cm5lZCIpOyBsMi5jbG9zZSgpOwogIH0KfSBjYXRjaCAoZSkgeyB9Cg==")
STARTUP = r"C:\Program Files\Adobe\Adobe Premiere Pro 2027\Scripts\Startup"

SETUP = r"""
New-Item -Force -ItemType Directory C:\kfhw\pp, C:\kfapps\dumps | Out-Null
$k = 'HKLM:\SOFTWARE\Microsoft\Windows\Windows Error Reporting\LocalDumps'; New-Item -Force -Path $k | Out-Null
New-ItemProperty -Path $k -Name DumpFolder -PropertyType ExpandString -Value 'C:\kfapps\dumps' -Force | Out-Null
New-ItemProperty -Path $k -Name DumpType -PropertyType DWord -Value 1 -Force | Out-Null
New-ItemProperty -Path $k -Name DumpCount -PropertyType DWord -Value 10 -Force | Out-Null
$t = [Text.Encoding]::UTF8.GetString([Convert]::FromBase64String('%(stub)s')).Replace('@APP@', 'pp')
if (-not (Test-Path '%(startup)s')) { New-Item -Force -ItemType Directory '%(startup)s' | Out-Null }
Set-Content -Path '%(startup)s\kf_job.jsx' -Value $t
'setup ok: ' + (Get-ChildItem '%(startup)s' | ForEach-Object { $_.Name }) -join ','
""" % dict(stub=STUB_B64, startup=STARTUP)

MASK = r"""
function Mask([string]$s) {
  $s = $s -replace '[A-Za-z0-9._%+-]+@[A-Za-z0-9.-]+\.[A-Za-z]{2,}', '<email>'
  $s = $s -replace '(?i)C:\\Users\\[^\\"]+', 'C:\Users\<user>'
  $s = $s -replace '(?i)C:/Users/[^/"]+', 'C:/Users/<user>'
  $s = $s -replace '\b[0-9a-f]{32,}\b', '<hex>'
  $s = $s -replace '\b[0-9A-Fa-f]{8}-[0-9A-Fa-f]{4}-[0-9A-Fa-f]{4}-[0-9A-Fa-f]{4}-[0-9A-Fa-f]{12}\b', '<uuid>'
  $s = $s -replace '(?i)("(user|username|email|name|id|ip_address|device_id|machine_id)"\s*:\s*)"[^"]*"', '$1"<masked>"'
  return $s
}
"""

CRASH = r"""
$t = [datetime]::Parse('SINCE', $null, [Globalization.DateTimeStyles]::AdjustToUniversal -bor [Globalization.DateTimeStyles]::AssumeUniversal)
'== now ' + (Get-Date).ToUniversalTime().ToString('s') + 'Z'
'== Premiere SentryIO records written since ' + $t.ToString('s') + 'Z'
foreach ($db in (Get-ChildItem 'C:\Users\*\AppData\Local\Temp\Adobe\Premiere Pro' -Recurse -Directory -Filter SentryIO-db -Force -ErrorAction SilentlyContinue)) {
  $lc = Join-Path $db.FullName 'last_crash'
  if (Test-Path $lc) { 'last_crash = ' + (Get-Content $lc -Raw).Trim() }
  foreach ($f in (Get-ChildItem $db.FullName -Recurse -File -Force -ErrorAction SilentlyContinue | Where-Object { $_.LastWriteTimeUtc -ge $t } | Sort-Object LastWriteTimeUtc)) {
    '-- {0:yyyy-MM-ddTHH:mm:ss}Z {1,10} {2}' -f $f.LastWriteTimeUtc, $f.Length, (Mask ($f.FullName.Substring($db.FullName.Length)))
    if ($f.Length -le 64MB) {
      $s = [Text.Encoding]::GetEncoding(28591).GetString([IO.File]::ReadAllBytes($f.FullName))
      foreach ($m in [regex]::Matches($s, '"gpu-renderer"\s*:\s*"[^"]*"')) { '   tag ' + $m.Value }
      foreach ($m in [regex]::Matches($s, '"(gpu-name|gpu-driver|gpu-vendor|os|exception\.type|type|value|module|function|thread_name|crashed)"\s*:\s*("[^"]{0,160}"|true|false)')) { if ($m.Value -notmatch 'breadcrumb') { '   kv ' + (Mask $m.Value) } }
      foreach ($m in [regex]::Matches($s, 'Unhandled exception[^\x00]{0,260}')) { '   ue ' + (Mask (($m.Value -replace '[^\x20-\x7e]', '.'))) }
      foreach ($m in [regex]::Matches($s, '(EXCEPTION_[A-Z_]+|0x[cC]0000[0-9a-fA-F]{3}|0x[eE]06d7363)')) { '   ex ' + $m.Value }
    }
  }
}
'== Application 1000/1001/1002 since'
Get-WinEvent -FilterHashtable @{LogName='Application'; StartTime=$t.ToLocalTime(); Id=1000,1001,1002} -ErrorAction SilentlyContinue | Select-Object -First 30 |
  ForEach-Object { $m = Mask ($_.Message -replace '\s+', ' '); '{0:yyyy-MM-ddTHH:mm:ss}Z {1} {2}' -f $_.TimeCreated.ToUniversalTime(), $_.Id, $m.Substring(0, [Math]::Min(900, $m.Length)) }
'== WER reports since'
foreach ($w in (Get-ChildItem 'C:\ProgramData\Microsoft\Windows\WER\ReportArchive', 'C:\ProgramData\Microsoft\Windows\WER\ReportQueue', 'C:\Users\*\AppData\Local\Microsoft\Windows\WER\ReportArchive', 'C:\Users\*\AppData\Local\Microsoft\Windows\WER\ReportQueue' -Directory -ErrorAction SilentlyContinue | Where-Object { $_.LastWriteTimeUtc -ge $t })) {
  '-- ' + $w.Name
  $r = Join-Path $w.FullName 'Report.wer'
  if (Test-Path $r) { Get-Content $r -Encoding Unicode | Where-Object { $_ -match '^(EventType|AppName|Sig\[\d+\]\.(Name|Value)|EventTime|FriendlyEventName)=' } | ForEach-Object { '   ' + (Mask $_) } }
  Get-ChildItem $w.FullName -File | ForEach-Object { '   file {0} {1}' -f $_.Name, $_.Length }
}
'== LocalDumps since (size only; the dumps stay in the guest)'
Get-ChildItem C:\kfapps\dumps -File -ErrorAction SilentlyContinue | Where-Object { $_.LastWriteTimeUtc -ge $t } | ForEach-Object { '{0:yyyy-MM-ddTHH:mm:ss}Z {1} {2}' -f $_.LastWriteTimeUtc, $_.Length, $_.Name }
'== System display-driver events since'
Get-WinEvent -FilterHashtable @{LogName='System'; StartTime=$t.ToLocalTime()} -ErrorAction SilentlyContinue | Where-Object { $_.Id -in 4101,4116 -or $_.ProviderName -match 'nvlddmkm|Display' } | Select-Object -First 20 |
  ForEach-Object { $m = ($_.Message -replace '\s+', ' '); '{0:yyyy-MM-ddTHH:mm:ss}Z {1} {2} {3}' -f $_.TimeCreated.ToUniversalTime(), $_.Id, $_.ProviderName, $m.Substring(0, [Math]::Min(240, $m.Length)) }
'== LiveKernelReports since'
Get-ChildItem C:\Windows\LiveKernelReports -Recurse -File -ErrorAction SilentlyContinue | Where-Object { $_.LastWriteTimeUtc -ge $t } | ForEach-Object { '{0:yyyy-MM-ddTHH:mm:ss}Z {1} {2}' -f $_.LastWriteTimeUtc, $_.FullName, $_.Length }
'== guest GPU'
Get-CimInstance Win32_VideoController | ForEach-Object { '{0} driver {1} status {2}' -f $_.Name, $_.DriverVersion, $_.Status }
"""


def ps_file(script, name, timeout=600):
    """Multi-line PowerShell does not survive the stdin transport of drv.ps: hand it over as a file and run that."""
    drv.ps("New-Item -Force -ItemType Directory C:\\kfhw\\pp | Out-Null", 60)
    local = os.path.join(HERE, name)
    with open(local, "w") as f:
        f.write(script)
    if not drv.put(local, "C:\\kfhw\\pp\\" + name):
        return None, "", "put failed"
    return drv.ps("& 'C:\\kfhw\\pp\\%s'" % name, timeout)


def main():
    cmd = sys.argv[1]
    if cmd == "setup":
        rc, o, e = ps_file(SETUP, "setup.ps1", 120)
        print(rc, o, e[-300:])
        for f in ("pp.jsx", "pp_run.ps1"):
            print(f, drv.put(os.path.join(HERE, f), "C:\\kfhw\\pp\\" + f))
    elif cmd == "run":
        clip, mode, tag = sys.argv[2:5]
        hold = sys.argv[5] if len(sys.argv) > 5 else "45"
        keys = " -Keys" if "keys" in sys.argv[6:] else ""
        o, e = drv.run_ui("..\\pp\\pp_run.ps1", args="-Clip '%s' -Mode %s -Tag %s -Hold %s%s" % (clip, mode, tag, hold, keys), timeout=1200)
        print(o, e)
    elif cmd == "crash":
        rc, o, e = ps_file(MASK + CRASH.replace("SINCE", sys.argv[2]), "crash.ps1")
        print(o)
        if rc != 0:
            print("RC", rc, e[-600:])
    elif cmd == "shot":
        print(drv.shot(sys.argv[2]))
    elif cmd == "ps":
        rc, o, e = drv.ps(sys.stdin.read(), 600)
        print(o)
        print("RC", rc, e[-400:])
    else:
        sys.exit(__doc__)


if __name__ == "__main__":
    main()
