#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later
"""Box side, stdlib only: observation helpers for a guest that propimage_check.py holds up (`--hold-until-file`), used with adobe_part2/drive.py.
They only read state, except `dns-on`, which turns on the Windows DNS client's operational event log (an observation channel).

  probe.py --work DIR now                         guest UTC and local time
  probe.py --work DIR dns-on                      enable Microsoft-Windows-DNS-Client/Operational
  probe.py --work DIR logs SINCE_UTC OUT          every file under the Adobe/Creative Cloud/temp log roots written since SINCE (path, size, mtime) and
                                                  the lines of the small text files among them that carry a reason word; e-mail addresses and
                                                  user names in JSON fields are masked in the guest before the text leaves it
  probe.py --work DIR dns SINCE_UTC OUT           DNS client events since SINCE (query name, status, process) and the DNS cache, deduplicated
  probe.py --work DIR events SINCE_UTC OUT        System/Application events that matter for a GPU or application failure since SINCE
  probe.py --work DIR procs                       Adobe processes (pid, responding, cpu, title)
"""
import argparse
import importlib.util
import os
import sys
from pathlib import Path

WIN = Path(os.environ.get("KF_WIN", str(Path(__file__).resolve().parents[1])))
sys.path.insert(0, str(WIN / "adobe_part2"))

ROOTS = [
    r"$env:LOCALAPPDATA\Adobe", r"$env:APPDATA\Adobe", r"$env:LOCALAPPDATA\Temp", r"C:\ProgramData\Adobe",
    r"$env:USERPROFILE\AppData\LocalLow\Adobe", r"C:\Program Files\Common Files\Adobe\OOBE", r"C:\Windows\Temp",
]
WORDS = ("model|download|entitle|licen|profile|token|sign|auth|network|http|connect|offline|fail|error|expire|clock|denied|refus|unable|manifest|"
         "sensei|decrypt|cert|ngl|subscription|feature")

MASK = r"""
function Mask([string]$s) {
  $s = $s -replace '[A-Za-z0-9._%+-]+@[A-Za-z0-9.-]+\.[A-Za-z]{2,}', '<email>'
  $s = $s -replace '(?i)("?(first_?name|last_?name|display_?name|user_?name|screen_?name|full_?name|name|email|account_?id|user_?id|userId|guid|authId)"?\s*[:=]\s*)"[^"]*"', '$1"<masked>"'
  $s = $s -replace '(?i)(Bearer|access_token|refresh_token|device_token|id_token)([\s:="]+)[A-Za-z0-9._\-]{12,}', '$1$2<token>'
  $s = $s -replace '(?i)\b(user_?id|authId|device_?id|machine_?id|profile_?id|account_?id|owner_?id|deviceToken|ngl_?id)(\s*[=:]\s*)[^\s,;"]+', '$1$2<masked>'
  $s = $s -replace '\b[0-9a-f]{32,}\b', '<hex>'
  $s = $s -replace '\b[0-9A-Fa-f]{8}-[0-9A-Fa-f]{4}-[0-9A-Fa-f]{4}-[0-9A-Fa-f]{4}-[0-9A-Fa-f]{12}\b', '<uuid>'
  return $s
}
"""


def load(name, path):
    spec = importlib.util.spec_from_file_location(name, path)
    mod = importlib.util.module_from_spec(spec)
    sys.modules[name] = mod
    spec.loader.exec_module(mod)
    return mod


def since_ps(since):
    return "$t = [datetime]::Parse('%s', $null, [Globalization.DateTimeStyles]::AdjustToUniversal -bor [Globalization.DateTimeStyles]::AssumeUniversal)\n" % since


def logs_script(since):
    roots = "@(" + ",".join('"%s"' % r for r in ROOTS) + ")"
    return since_ps(since) + MASK + r"""
$files = foreach ($r in %s) { if (Test-Path $r) { Get-ChildItem -LiteralPath $r -Recurse -File -Force -ErrorAction SilentlyContinue | Where-Object { $_.LastWriteTimeUtc -ge $t } } }
'== FILES written since ' + $t.ToString('s') + 'Z'
foreach ($f in ($files | Sort-Object LastWriteTimeUtc)) { '{0:yyyy-MM-ddTHH:mm:ss}Z {1,12} {2}' -f $f.LastWriteTimeUtc, $f.Length, (Mask ($f.FullName -replace [regex]::Escape($env:USERPROFILE), '%%USERPROFILE%%')) }
foreach ($f in ($files | Where-Object { $_.Length -le 20MB -and $_.Extension -match '^\.(log|txt|json|xml|csv|ini|dat|lst|out)$' -or ($_.Length -le 20MB -and $_.Extension -eq '') } | Sort-Object LastWriteTimeUtc)) {
  $hits = @(Select-String -LiteralPath $f.FullName -Pattern '(?i)(%s)' -ErrorAction SilentlyContinue | Select-Object -Last 120)
  if ($hits.Count) {
    '== ' + (Mask ($f.FullName -replace [regex]::Escape($env:USERPROFILE), '%%USERPROFILE%%')) + ' (' + $hits.Count + ' matching lines, last 120)'
    foreach ($h in $hits) { $l = Mask $h.Line; if ($l.Length -gt 400) { $l = $l.Substring(0, 400) + ' ...' }; '{0}: {1}' -f $h.LineNumber, $l }
  }
}
""" % (roots, WORDS)


DNS = r"""
'== DNS client events'
Get-WinEvent -FilterHashtable @{LogName='Microsoft-Windows-DNS-Client/Operational'; StartTime=$t.ToLocalTime()} -ErrorAction SilentlyContinue | Where-Object { $_.Id -in 3006,3008,3009,3010,3011,3016,3018,3019,3020 } |
  ForEach-Object { $x = [xml]$_.ToXml(); $q = ($x.Event.EventData.Data | Where-Object { $_.Name -eq 'QueryName' }).'#text'; $st = ($x.Event.EventData.Data | Where-Object { $_.Name -in 'QueryStatus','Status' } | Select-Object -First 1).'#text'
    $p = $_.ProcessId; $pn = (Get-Process -Id $p -ErrorAction SilentlyContinue).ProcessName
    '{0:yyyy-MM-ddTHH:mm:ss}Z id={1} pid={2} proc={3} status={4} name={5}' -f $_.TimeCreated.ToUniversalTime(), $_.Id, $p, $pn, $st, $q } | Select-Object -Unique -Last 400
'== DNS cache'
Get-DnsClientCache -ErrorAction SilentlyContinue | ForEach-Object { '{0} type={1} status={2}' -f $_.Entry, $_.Type, $_.Status } | Select-Object -Unique
'== TCP connections not on loopback'
Get-NetTCPConnection -ErrorAction SilentlyContinue | Where-Object { $_.RemoteAddress -notmatch '^(127\.|::1|0\.0\.0\.0|::$)' } | ForEach-Object { '{0} {1}:{2} pid={3} {4}' -f $_.State, $_.RemoteAddress, $_.RemotePort, $_.OwningProcess, (Get-Process -Id $_.OwningProcess -ErrorAction SilentlyContinue).ProcessName }
"""

EVENTS = r"""
'== System (display, kernel power, TDR)'
Get-WinEvent -FilterHashtable @{LogName='System'; StartTime=$t.ToLocalTime()} -ErrorAction SilentlyContinue | Where-Object { $_.Id -in 41,1074,6006,6008,4101,4116,13,14,153,1001 -or $_.ProviderName -match 'nvlddmkm|Display' } | Select-Object -First 60 |
  ForEach-Object { $m = ($_.Message -replace '\s+', ' '); '{0:yyyy-MM-ddTHH:mm:ss}Z {1} {2} {3}' -f $_.TimeCreated.ToUniversalTime(), $_.Id, $_.ProviderName, $m.Substring(0, [Math]::Min(240, $m.Length)) }
'== Application 1000/1001/1002'
Get-WinEvent -FilterHashtable @{LogName='Application'; StartTime=$t.ToLocalTime(); Id=1000,1001,1002} -ErrorAction SilentlyContinue | Select-Object -First 40 |
  ForEach-Object { $m = ($_.Message -replace '\s+', ' '); '{0:yyyy-MM-ddTHH:mm:ss}Z {1} {2}' -f $_.TimeCreated.ToUniversalTime(), $_.Id, $m.Substring(0, [Math]::Min(320, $m.Length)) }
'== LiveKernelReports'
Get-ChildItem C:\Windows\LiveKernelReports -Recurse -File -ErrorAction SilentlyContinue | Where-Object { $_.LastWriteTimeUtc -ge $t } | ForEach-Object { '{0:yyyy-MM-ddTHH:mm:ss}Z {1} {2}' -f $_.LastWriteTimeUtc, $_.FullName, $_.Length }
"""

PROCS = r"""Get-Process -ErrorAction SilentlyContinue | Where-Object { $_.Path -match 'Adobe' -or $_.ProcessName -match 'Photoshop|Lightroom|Premiere|AfterFX|Illustrator|Sampler|Creative|Adobe|CEP|node' } |
  ForEach-Object { 'pid {0} {1} responding={2} cpu={3:N0}s ws={4:N0}MB title={5}' -f $_.Id, $_.ProcessName, $_.Responding, $_.CPU, ($_.WorkingSet64 / 1MB), $_.MainWindowTitle }
(Get-Date).ToUniversalTime().ToString('s') + 'Z'"""


def main():
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("--work", required=True)
    sub = ap.add_subparsers(dest="cmd", required=True)
    sub.add_parser("now")
    sub.add_parser("dns-on")
    sub.add_parser("procs")
    for c in ("logs", "dns", "events"):
        s = sub.add_parser(c)
        s.add_argument("since")
        s.add_argument("out")
    a = ap.parse_args()
    drive = load("kf_drive", WIN / "adobe_part2/drive.py")
    g = drive.guest_of(a.work)
    if a.cmd == "now":
        script = "(Get-Date).ToUniversalTime().ToString('s') + 'Z utc; local ' + (Get-Date).ToString('s')"
    elif a.cmd == "dns-on":
        script = "wevtutil sl Microsoft-Windows-DNS-Client/Operational /e:true /ms:20971520; wevtutil gl Microsoft-Windows-DNS-Client/Operational | Select-String enabled"
    elif a.cmd == "procs":
        script = PROCS
    elif a.cmd == "logs":
        script = logs_script(a.since)
    elif a.cmd == "dns":
        script = since_ps(a.since) + DNS
    else:
        script = since_ps(a.since) + EVENTS
    r = g.ps(script, timeout=600)
    text = r.stdout + ("\n== stderr rc=%d\n%s" % (r.returncode, r.stderr[:2000]) if r.returncode or r.stderr.strip() else "")
    if getattr(a, "out", None):
        Path(a.out).parent.mkdir(parents=True, exist_ok=True)
        Path(a.out).write_text(text)
        print("wrote", a.out, len(text.splitlines()), "lines")
    else:
        print(text)


if __name__ == "__main__":
    main()
