#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later
"""The one probe a coordinator tick may run on the pc (STATUS: LIVE, 2026-11-24).

    ssh pc python3 -I /path/to/scripts/bench/pc_safe_probe.py          # uptime, PC_ACTIVE tail, bind state
    ssh pc python3 -I /path/to/scripts/bench/pc_safe_probe.py --gpu    # adds the nvidia-smi line, ONLY when no bind is active

A host tool that touches the nvidia driver (`nvidia-smi`, `nvidia-modprobe`, `modprobe nvidia*`, `lspci -k` loops) while the GPU is
on vfio-pci loads the module and can start the udev load/unload loop of 71-nvidia.rules. This probe decides from the GPU lock,
the PC_ACTIVE.log lines, the bind's modprobe block and the GPU's sysfs driver link (pc_guard.bind_active, no driver call) and,
when a bind may be active, prints only uptime, the log tail and the reasons. `--gpu` is refused (exit 3) in that case.
It never loads a module and never runs a tool in the bind case.
"""

from __future__ import annotations

import argparse
import subprocess
import sys
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))
import pc_guard as G  # noqa: E402

NVIDIA_TOOLS = ("nvidia-smi", "nvidia-modprobe", "nvidia-settings", "nvidia-debugdump", "modprobe", "lspci")


def plan(active, want_gpu):
    """The commands the probe may run, as argv lists. Pure: the tests check that no nvidia tool is ever planned while active."""
    cmds = [["uptime"]]
    if want_gpu and not active:
        cmds.append(["nvidia-smi", "--query-gpu=name,driver_version,temperature.gpu,utilization.gpu", "--format=csv,noheader"])
    return cmds


def touches_nvidia(argv):
    return Path(argv[0]).name in NVIDIA_TOOLS


def main(argv=None, facts=None, runner=None, out=print):
    ap = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    ap.add_argument("--gpu", action="store_true", help="also query the GPU (refused while a bind may be active)")
    ap.add_argument("--tail", type=int, default=8, help="PC_ACTIVE.log lines to print")
    a = ap.parse_args(argv)
    if facts is None:
        lines = G.read_active_tail()
        facts = dict(active_lines=lines, lock_held=G.lock_held(), block_file_present=G.block_present(), gpu_driver=G.gpu_driver())
    runner = runner or (lambda c: G.run_cmd(c, 20))
    active, reasons = G.bind_active(**facts)
    out("bind_active=%s" % ("yes" if active else "no"))
    for r in reasons:
        out("  reason: %s" % r)
    rc = 0
    for c in plan(active, a.gpu):
        if active and touches_nvidia(c):
            raise AssertionError("refusing an nvidia tool while a bind may be active: %s" % c)
        code, text = runner(c)
        out("$ %s (rc=%s)\n%s" % (" ".join(c[:1]), code, str(text).rstrip()))
    if a.gpu and active:
        out("REFUSED: --gpu while a bind may be active (no nvidia tool is run)")
        rc = 3
    out("PC_ACTIVE.log tail:")
    for line in facts["active_lines"][-a.tail:]:
        out("  " + line)
    return rc


if __name__ == "__main__":
    sys.exit(main())
