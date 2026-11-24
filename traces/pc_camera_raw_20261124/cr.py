#!/usr/bin/env python3
"""cr.py: Camera Raw control actions on the running guest (pc side; drv.py helpers of 2026-11-21).
usage: cr.py setup | open FILE | shot NAME | click X Y | dbl X Y | keys K | events SINCE_UTC | state | jsx"""
import os, sys, json, time
sys.path.insert(0, "/workspace/bench/adobe_addon_20261121_driver")
import drv
HERE = os.path.dirname(os.path.abspath(__file__))
cmd = sys.argv[1]
if cmd == "setup":
    print(drv.put(os.path.join(HERE, "cr_open.ps1"), drv.GD + "\\cr_open.ps1"))
elif cmd == "open":
    o, e = drv.run_ui("cr_open.ps1", args="-File '%s'" % sys.argv[2], timeout=900); print(o, e)
elif cmd == "shot":
    p, o = drv.shot(sys.argv[2]); print(p, o)
elif cmd in ("click", "dbl", "rclick"):
    print(drv.inp({"click": "click", "dbl": "dblclick", "rclick": "rclick"}[cmd], int(sys.argv[2]), int(sys.argv[3])))
elif cmd == "keys":
    print(drv.inp("keys", keys=sys.argv[2]))
elif cmd == "jsx":
    print("task", drv.start_ui("psjs.ps1", args="-File C:\\kfhw\\drv\\t_acr.jsx"))
elif cmd == "events":
    since = sys.argv[2]
    s = r"""
$t = [datetime]::Parse('SINCE').ToUniversalTime().ToLocalTime()
'== System 4101/4116/14/13 (display driver)'
Get-WinEvent -FilterHashtable @{LogName='System'; StartTime=$t} -ErrorAction SilentlyContinue | ? { $_.Id -in 4101,4116,13,14,153 -or $_.ProviderName -match 'nvlddmkm|Display' } | select -First 40 | % { '{0:u} {1} {2} {3}' -f $_.TimeCreated.ToUniversalTime(), $_.Id, $_.ProviderName, (($_.Message -replace '\s+',' ').Substring(0, [Math]::Min(220, ($_.Message -replace '\s+',' ').Length))) }
'== Application 1000/1001/1002 (crash, WER, hang)'
Get-WinEvent -FilterHashtable @{LogName='Application'; StartTime=$t; Id=1000,1001,1002} -ErrorAction SilentlyContinue | select -First 40 | % { '{0:u} {1} {2}' -f $_.TimeCreated.ToUniversalTime(), $_.Id, (($_.Message -replace '\s+',' ').Substring(0, [Math]::Min(300, ($_.Message -replace '\s+',' ').Length))) }
'== LiveKernelReports'
Get-ChildItem C:\Windows\LiveKernelReports -Recurse -File -ErrorAction SilentlyContinue | ? { $_.LastWriteTime -gt $t } | % { '{0:u} {1} {2}' -f $_.LastWriteTime.ToUniversalTime(), $_.FullName, $_.Length }
'== Photoshop'
Get-Process Photoshop -ErrorAction SilentlyContinue | % { 'pid {0} responding={1} cpu={2:N1}s ws={3:N0}MB title={4}' -f $_.Id, $_.Responding, $_.CPU, ($_.WorkingSet64/1MB), $_.MainWindowTitle }
""".replace("SINCE", since)
    rc, o, e = drv.ps(s, 300); print(o); print("RC", rc, e[-200:])
elif cmd == "state":
    rc, o, e = drv.ps("Get-Process Photoshop -ErrorAction SilentlyContinue | % { 'pid {0} responding={1} cpu={2:N1}s title={3}' -f $_.Id, $_.Responding, $_.CPU, $_.MainWindowTitle }; (Get-Date).ToUniversalTime().ToString('s')", 60); print(o, rc)
