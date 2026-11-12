#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later
"""The Windows guest side of the explicit broker stage (docs/design/V3_BROKER_IN_CI.md).

STATUS: PLUMBING 2026-11-12. NOT RUN on any box. The control commands are the ones the PC display run used
(/workspace/bench/pcdisp_20261112: win_vm.sh ssh/scp-to, qmp.py screendump, runi.ps1, kf_inlog, res.ps1); only the
orchestration is new. All shell-outs go through one injectable `runner(argv, timeout) -> (rc, text)`.
"""

import os
import re
import subprocess
import sys
import time
from pathlib import Path

import broker_ci

DISPLAY_DIR = Path("scripts/bench/display/win")


def real_runner(argv, timeout=120, env=None):
    try:
        p = subprocess.run(argv, capture_output=True, text=True, timeout=timeout, stdin=subprocess.DEVNULL, env=env)
    except (OSError, subprocess.SubprocessError) as e:
        return 255, str(e)
    return p.returncode, (p.stdout or "") + (p.stderr or "")


def ppm_stats(data):
    """(mean, standard deviation) of a binary PPM (P6, maxval 255), over all samples scaled to 0..1."""
    m = re.match(rb"P6\s+(\d+)\s+(\d+)\s+255\s", data)
    if not m:
        raise ValueError("not a P6 255 ppm")
    body = data[m.end():]
    if not body:
        raise ValueError("empty ppm")
    step = max(1, len(body) // 200000)  # a sample of at most ~200k bytes is plenty for blank-or-not
    sample = body[::step]
    n = len(sample)
    mean = sum(sample) / n / 255.0
    var = sum((b / 255.0 - mean) ** 2 for b in sample) / n
    return mean, var ** 0.5


class WinGuest:
    def __init__(self, root, winvm, qmp_sock, work, runner=real_runner, sleep=time.sleep, env=None):
        self.root, self.winvm, self.qmp, self.work = Path(root), winvm, qmp_sock, Path(work)
        self.run, self.sleep, self.env = runner, sleep, env
        self.sources = self.root / DISPLAY_DIR

    def _wv(self, *args, timeout=120):
        return self.run(["bash", self.winvm, *args], timeout)

    def build_logger(self):
        exe = self.work / "kf_inlog.exe"
        rc, out = self.run(["x86_64-w64-mingw32-gcc", "-O2", "-o", str(exe), str(self.sources / "kf_inlog.c"),
                            "-luser32"], 120)
        return exe if rc == 0 and exe.is_file() else None

    def copy_tools(self, exe):
        for src, dst in ((exe, "C:/kf/"), (self.sources / "runi.ps1", "C:/kf/"), (self.sources / "res.ps1", "C:/kf/")):
            rc, _ = self._wv("scp-to", str(src), dst)
            if rc != 0:
                return False
        return True

    def screendump_stats(self):
        ppm = self.work / "first.ppm"
        ppm.unlink(missing_ok=True)
        self.run(["python3", str(self.root / "scripts/bench/windows/qmp.py"), self.qmp, "cmd", "screendump",
                  '{"filename":"%s","device":"kf0"}' % ppm], 60)
        for _ in range(10):
            if ppm.is_file() and ppm.stat().st_size > 0:
                break
            self.sleep(0.5)
        return ppm_stats(ppm.read_bytes())

    def set_resolution(self, w, h):
        self._wv("ssh", r'powershell -NoProfile -ExecutionPolicy Bypass -File C:\kf\runi.ps1 -Name res -Cmd '
                 r'"powershell -NoProfile -ExecutionPolicy Bypass -File C:\kf\res.ps1 %d %d"' % (w, h))
        self.sleep(8)

    def logger_start(self):
        self._wv("ssh", r'powershell -NoProfile -ExecutionPolicy Bypass -File C:\kf\runi.ps1 -Name inlog -Cmd '
                 r'"C:\kf\kf_inlog.exe C:\kf\t\inlog.log 40"')
        for _ in range(20):
            if "KFIN START" in self.logger_text():
                return
            self.sleep(0.5)

    def logger_text(self):
        return self._wv("ssh", r"type C:\kf\t\inlog.log")[1]


def explicit_main(args, root):
    """`broker_ci.py explicit --tag T --prov DIR`: boot a fresh guest (a child overlay of the gate's) with the broker,
    run the three checks, stop. Prints exactly one BROKER_EXPLICIT line and exits 0 only when all three pass. Any
    failure of the broker side is a FAIL line, never a retry; the harness steps that cannot run at all (no logger
    build, no guest) are ALSO a FAIL (broker_start / broker_input), because the ruling leaves no excusing class."""
    opts = dict(zip(args[::2], args[1::2]))
    tag, prov = opts.get("--tag", ""), Path(opts.get("--prov", "/root/prov"))
    if not re.fullmatch(r"[A-Za-z0-9][A-Za-z0-9._-]*", tag):
        return 2
    env = os.environ
    winvm = str(Path(root) / "scripts/bench/windows/win_vm.sh")
    vm_root = Path(env.get("WINVM_ROOT", "/workspace/winvm")) / env.get("WINVM_NAME", "kfwin")
    gate_tag = env.get("KF_BROKER_GATE_TAG", "gate_" + re.sub(r"[^A-Za-z0-9_]", "_", tag)[:24])
    work = prov / (tag + "_broker")
    work.mkdir(parents=True, exist_ok=True)
    binary, built = broker_ci.ensure_binary(root)
    session = broker_ci.BrokerSession(binary, broker_ci.SOCKET_ROOT, "explicit")
    failure = None
    rows = []
    try:
        if not built or not session.start():
            raise broker_ci.BrokerFailure("broker_build" if not built else "broker_start")
        session.mark_guest_started()
        runtag = "brk_" + re.sub(r"[^A-Za-z0-9]", "", tag)[:20]
        uuid = ""
        try:
            uuid = next(l.split("=", 1)[1].strip() for l in (vm_root / "identity.env").read_text().splitlines()
                        if l.startswith("VM_UUID="))
        except (OSError, StopIteration):
            raise broker_ci.BrokerFailure("broker_start")
        genv = dict(env, WINVM_OVERLAY_BACKING=gate_tag, WINVM_BROKER_SOCK=str(session.sock), WINVM_VM_IDENTITY="1")
        flags = env.get("KF_WINGATE_FLAGS", str(Path(root) / "scripts/bench/windows/kf3_flags.sample.env"))
        rc, out = real_runner(["bash", winvm, "run", "--detach", "--overlay", runtag, "--arm", env.get("KF_WINGATE_ARM", "B"),
                               "--guest-driver", env.get("KF_WINGATE_GUEST_DRIVER", "580.65.06"), "--kf3-extra",
                               "gpu-uuid=GPU-" + uuid, "--env-file", flags, "--tag", runtag], 120, genv)
        if rc != 0:
            raise broker_ci.BrokerFailure("broker_start")
        guest = WinGuest(root, winvm, str(vm_root / "run/qmp.sock"), work, env=genv)
        exe = guest.build_logger()
        # wait for the guest's ssh; a guest that never answers has also shown the broker no frame
        for _ in range(60):
            if real_runner(["bash", winvm, "ssh", "echo ok"], 30, genv)[0] == 0:
                break
            time.sleep(10)
        if exe is None or not guest.copy_tools(exe):
            raise broker_ci.BrokerFailure("broker_input", rows)
        rows = broker_ci.run_explicit(session, guest)
    except broker_ci.BrokerFailure as f:
        failure = f
    finally:
        real_runner(["bash", winvm, "stop"], 420, env)
        session.stop()
    print(broker_ci.format_explicit(rows if failure is None else failure.rows, failure), flush=True)
    return 0 if failure is None else 1
