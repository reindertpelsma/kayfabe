#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later
"""Box side: elevated, in the held guest, install WER LocalDumps and the ExtendScript Startup stubs (the harness rows do this before each row).
usage: pp_setup.py WORKDIR"""
import importlib.util
import os
import sys
from pathlib import Path

WIN = Path(os.environ.get("KF_WIN", str(Path(__file__).resolve().parents[1])))
sys.path.insert(0, str(WIN / "adobe_part2"))


def load(name, path):
    spec = importlib.util.spec_from_file_location(name, path)
    mod = importlib.util.module_from_spec(spec)
    sys.modules[name] = mod
    spec.loader.exec_module(mod)
    return mod


drive = load("kf_drive", WIN / "adobe_part2/drive.py")
ab = load("propimage_apps_b", WIN / "propimage_apps_b.py")
g = drive.guest_of(sys.argv[1])
r = g.ps(ab.LOCALDUMPS_SETUP + ab.startup_stub_setup())
print("setup rc", r.returncode, r.stderr[:200])
r = g.ps("Get-ChildItem 'C:\\Program Files\\Adobe\\Adobe Premiere Pro 2027\\Scripts\\Startup' | ForEach-Object { $_.Name }")
print(r.stdout.strip())
