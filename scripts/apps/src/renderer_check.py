#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later
"""renderer_check.py <app> <logfile> -- STATUS: LIVE, 2026-11-12.

Post-check of the graphics rows of the Linux apps matrix (run by run_apps.sh after a row would PASS).
A row that "ran" on a software rasterizer (llvmpipe, lavapipe, swrast, SwiftShader, pocl CPU) says nothing
about the GPU, so it must FAIL even if its own pass regex matched. Fail closed: a log that names no device,
or an empty one, fails. The rules are per app because each tool names its device differently (measured
formats: traces/apps_linux_20261107/r1/*.guest.log).

  egl_offscreen  the "[egl] renderer:" line (also checked inside the C program)
  vulkaninfo     GPU0 of the summary (vkpeak 0 and every default consumer pick device 0)
  vkpeak         the "device =" line
  clinfo         the first "Platform #0" / "Device #0" pair of `clinfo -l` (pocl as platform #1 is normal)
  clpeak         the first "Platform:" and "Device:" lines
Exit 0 = ok, 1 = software or no device named, 2 = usage. Prints one RENDERER_CHECK line.
"""
import re
import sys

SOFTWARE = re.compile(r"llvmpipe|lavapipe|swrast|softpipe|swiftshader|software|portable computing|pocl|"
                      r"PHYSICAL_DEVICE_TYPE_CPU|DRIVER_ID_MESA_LLVMPIPE", re.I)
APPS = ("egl_offscreen", "vulkaninfo", "vkpeak", "clinfo", "clpeak")


def real_gpu(name):
    """A device/renderer string names the NVIDIA GPU: contains NVIDIA and no software-rasterizer marker."""
    return bool(name) and "nvidia" in name.lower() and not SOFTWARE.search(name)


def _first(pattern, text):
    m = re.search(pattern, text, re.M)
    return m[1].strip() if m else ""


def check(app, text):
    """(ok, reason) for the log text of one app row."""
    if app not in APPS:
        return True, "not a graphics row"
    if not text.strip():
        return False, "empty log"
    if app == "egl_offscreen":
        names = [_first(r"^\[egl\] renderer: (.*)$", text)]
    elif app == "vulkaninfo":
        m = re.search(r"^GPU0:\n((?:[ \t]+.*\n?)+)", text, re.M)
        block = m[1] if m else ""
        names = [_first(r"deviceName\s*=\s*(.*)", block)]
        if block and SOFTWARE.search(block):
            return False, "GPU0 is a software device: " + _first(r"deviceName\s*=\s*(.*)", block)
    elif app == "vkpeak":
        names = [_first(r"^device\s*=\s*(.*)$", text)]
    elif app == "clinfo":
        names = [_first(r"^Platform #0: (.*)$", text), _first(r"^ `-- Device #0: (.*)$", text)]
    else:
        names = [_first(r"^Platform: (.*)$", text), _first(r"^\s+Device: (.*)$", text)]
    for n in names:
        if not n:
            return False, "no device/renderer string found"
        if not real_gpu(n):
            return False, "not the NVIDIA GPU: " + n[:80]
    return True, "device: " + names[-1][:80]


def main(argv):
    if len(argv) != 3 or argv[1] not in APPS:
        print("usage: renderer_check.py <%s> <logfile>" % "|".join(APPS), file=sys.stderr)
        return 2
    try:
        text = open(argv[2], encoding="utf-8", errors="replace").read()
    except OSError as e:
        text = ""
        print("RENDERER_CHECK app=%s FAIL unreadable log: %s" % (argv[1], e))
        return 1
    ok, why = check(argv[1], text)
    print("RENDERER_CHECK app=%s %s %s" % (argv[1], "ok" if ok else "FAIL", why))
    return 0 if ok else 1


if __name__ == "__main__":
    sys.exit(main(sys.argv))
