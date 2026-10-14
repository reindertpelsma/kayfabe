#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later
"""Host-side: run one PowerShell file in the guest of boundary run N through QGA guest-exec (as the agent's
account, SYSTEM) and print its output. Optional OUT appends the output there too.
usage: qga_run_ps.py RUN|desktop|DIR PSFILE [OUT]
(2026-10-08: `desktop` is windows_broker.sh's persistent overlay; a path names any run directory.)"""
import subprocess, sys
run, ps = sys.argv[1], sys.argv[2]
out = sys.argv[3] if len(sys.argv) > 3 else None
rd = (run if run.startswith('/') else '/var/lib/kf-windows-20261005/windows-desktop' if run == 'desktop'
      else f'/var/lib/kf-windows-20261005/boundary-kayfabe-{run}')
qmp = '/var/lib/kf-windows-20261005/boundary-tools/qmp.py'
pr = subprocess.run(['timeout', '280', 'python3', qmp, rd + '/qga.sock', 'qga-exec', 'powershell.exe', '-NoProfile', '-Command', open(ps).read()],
                    capture_output=True, text=True)
txt = pr.stdout + (('\n[stderr]\n' + pr.stderr) if pr.stderr.strip() else '') + f'\n[rc={pr.returncode}]\n'
print(txt)
if out:
    open(out, 'a').write(txt)
