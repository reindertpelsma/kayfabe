# SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later
import json
import os
import shutil
import sys

HERE = os.path.dirname(os.path.abspath(__file__))
AM = os.path.dirname(HERE)
REPO = os.path.abspath(os.path.join(AM, "..", "..", "..", ".."))
sys.path.insert(0, AM)


def load_apps():
    return json.load(open(os.path.join(AM, "apps.json")))


def load_manifest():
    return json.load(open(os.path.join(AM, "manifest.json")))


def find_pwsh():
    """PowerShell 7 for the parse checks (KF_PWSH, PATH, or the scratch install used while writing this lane)."""
    for c in (os.environ.get("KF_PWSH"), shutil.which("pwsh"), "/root/am/pwsh/pwsh"):
        if c and os.path.exists(c):
            return c
    return None


PARSE_MANY = r'''
$bad = 0
foreach ($f in $args) { $t = $null; $e = $null
    [System.Management.Automation.Language.Parser]::ParseFile($f, [ref]$t, [ref]$e) | Out-Null
    if ($e.Count) { $bad++; "$f : $($e[0].Message) line $($e[0].Extent.StartLineNumber)" } }
"PARSED_BAD=$bad"
'''


def pwsh_parse(files):
    """Parse-check PowerShell files with pwsh; returns pwsh's stdout (contains PARSED_BAD=<n>)."""
    import subprocess
    import tempfile
    with tempfile.TemporaryDirectory() as d:
        sp = os.path.join(d, "parse_many.ps1")
        open(sp, "w").write(PARSE_MANY)
        r = subprocess.run([find_pwsh(), "-NoProfile", "-File", sp, *files], capture_output=True, text=True)
        return r.stdout + r.stderr
