#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later
"""Applications on someone's own Windows disk, WITHOUT a state.json (stdlib only): propimage_check.py's sibling for mode `plain`.

STATUS: DESIGN-ONLY, 2026-11-16 (phase 1 of docs/design/V3_PROPIMAGE_RUNNER.md; GPU-free tests only, never booted). It is a separate
file so that propimage_check.py (the deterministic state.json shape) stays as it is; it reuses that file's guest transport, its app
drivers and its verdict rules, and writes the same result.json / verdict.json layout that scripts/ci/propimage_apps.py reads.

What differs from the state.json shape, on purpose (`propimage.py build_argv(mode="plain")`):
  - a throwaway qcow2 overlay of the given disk (the disk itself is never booted or written), a fresh swtpm and a copy of the distro's
    OVMF variables; the guest identity (SMBIOS, MAC, GPU UUID) comes from a per-run seed: NOT pinned, so nothing proves it equals
    another run's;
  - the guest clock is the host's real UTC now (`-rtc base=utc`), the network is off by default and `--net on` gives it internet;
    clocknet_assert.py refuses every other combination;
  - the CPU is the host's (`-cpu host` plus the Hyper-V enlightenments), not the fixed profile;
  - the login (user, ssh key, port) is given on the command line; there is no identity verify, no GPU-UUID repair and no clock check.
The installed applications come from `--apps a,b` (declared) or `--apps probe` (the Uninstall registry keys read over ssh, written to
<out>/probe.json); a name no definition matches is reported in probe.json and never run.
"""
import argparse
import json
import os
import re
import secrets
import shlex
import shutil
import subprocess
import sys
import time
from contextlib import ExitStack
from pathlib import Path

HERE = Path(__file__).resolve().parent
sys.path.insert(0, str(HERE))
import clocknet_assert  # noqa: E402,F401
import propimage_registry as R  # noqa: E402
from propimage_check import (CHECKS, PI, Guest, app_classification, hold_for_release, power_fix,  # noqa: E402
                             require_swtpm, result_markdown, run, run_check, stop_process)

OVMF_VARS = "/usr/share/OVMF/OVMF_VARS_4M.ms.fd"
PROBE_PS = r"""
$k='HKLM:\SOFTWARE\Microsoft\Windows\CurrentVersion\Uninstall\*','HKLM:\SOFTWARE\WOW6432Node\Microsoft\Windows\CurrentVersion\Uninstall\*'
@(Get-ItemProperty $k -ErrorAction SilentlyContinue | Where-Object { $_.DisplayName } | ForEach-Object { $_.DisplayName } | Sort-Object -Unique) | ConvertTo-Json -Compress
"""


def parse_probe(text):
    """Program names from the probe's JSON line (an array, or one string when there is a single program). Bounded and printable."""
    line = next((l for l in reversed(text.splitlines()) if l.strip().startswith(("[", '"'))), "")
    try:
        data = json.loads(line)
    except ValueError:
        return []
    data = [data] if type(data) is str else data
    return [re.sub(r"[^\x20-\x7e]", "?", x)[:120] for x in data if type(x) is str][:512]


def launch(a, work, seed=None):
    """(qemu argv, clone command, state) for this run. Raises PropError when clocknet_assert refuses the combination."""
    state = PI.plain_state(seed or secrets.token_hex(8), a.ssh_user, a.ssh_port)
    clone = Path(work) / "run/clone.qcow2"
    argv = PI.build_argv(state, PI.PLAIN, str(clone), str(work), qemu=a.qemu, display="off", net=a.net)
    return argv, clone, state


def main(argv=None):
    ap = argparse.ArgumentParser(description=__doc__.split("\n\n")[0])
    ap.add_argument("--disk", required=True)
    ap.add_argument("--work", required=True)
    ap.add_argument("--out", required=True)
    ap.add_argument("--qemu", required=True)
    ap.add_argument("--apps", default="probe", help="a,b | probe")
    ap.add_argument("--net", choices=("off", "on"), default="off")
    ap.add_argument("--ssh-user", required=True)
    ap.add_argument("--ssh-key", required=True)
    ap.add_argument("--ssh-port", type=int, default=None)
    ap.add_argument("--flags", default=str(HERE / "kf3_flags.sample.env"))
    ap.add_argument("--ovmf-vars", default=OVMF_VARS)
    ap.add_argument("--hold-until-file")
    ap.add_argument("--superposition-run", action="store_true")
    ap.add_argument("--fb-mb", type=int)
    a = ap.parse_args(argv)
    out = Path(a.out)
    out.mkdir(parents=True, exist_ok=True)
    rows = []

    def record(step, status, detail="", evidence="NONE", classification="harness"):
        rows.append({"step": step, "verdict": status, "detail": str(detail)[:1000], "evidence": evidence, "class": classification})
        print(step, status, evidence, str(detail)[:200], flush=True)

    try:
        reg = R.load_registry()
        swtpm = os.environ.get("SWTPM", "swtpm")
        version = run([swtpm, "--version"], timeout=30)
        if version.returncode:
            raise ValueError("Prerequisite: swtpm >= 0.10 required")
        require_swtpm(version.stdout + version.stderr)
        record("prerequisite", "PASS", "swtpm >= 0.10")
        disk, work = Path(a.disk).resolve(), Path(a.work).resolve()
        if not disk.is_file():
            raise ValueError("disk is not a file")
        (work / "run").mkdir(parents=True)
        (work / "firmware").mkdir()
        (work / "tpm").mkdir(mode=0o700)
        shutil.copyfile(a.ovmf_vars, work / "firmware/OVMF_VARS.fd")
        info = run(["qemu-img", "info", "--output=json", str(disk)])
        if info.returncode:
            raise ValueError("qemu-img cannot read the disk")
        fmt = json.loads(info.stdout).get("format", "")
        if not re.fullmatch(r"qcow2|raw|vpc|vmdk|vhdx", fmt):
            raise ValueError("unsupported disk format")
        command, clone, state = launch(a, work)
        made = run(["qemu-img", "create", "-q", "-f", "qcow2", "-b", str(disk), "-F", fmt, str(clone)])
        if made.returncode:
            raise ValueError("overlay creation failed: " + made.stderr[-200:])
        record("overlay", "PASS", "throwaway qcow2 overlay of the given disk; the disk is not booted")
        record("clock", "PASS", "real host UTC (-rtc base=utc); network %s; identity NOT pinned" % ("ON" if a.net == "on" else "off"))
        env = dict(os.environ)
        for line in Path(a.flags).read_text().splitlines():
            line = line.strip()
            if not line or line.startswith("#"):
                continue
            key, sep, value = line.partition("=")
            if not sep or not re.fullmatch(r"KF3_[A-Z0-9_]+", key) or not value.isdigit():
                raise ValueError("Refused KF3 flag line")
            env[key] = value
        if a.fb_mb is not None:
            state["qemu"]["fb_mb"] = a.fb_mb
            command, clone, _ = launch(a, work, seed=state["seed"])
        with ExitStack() as stack:
            r = work / "run"
            sw = subprocess.Popen([swtpm, "socket", "--tpm2", "--tpmstate", "dir=%s/tpm,mode=0600" % work, "--ctrl",
                                   "type=unixio,path=%s/swtpm.sock,mode=0600" % work, "--terminate"],
                                  stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
            stack.callback(stop_process, sw)
            for _ in range(50):
                if (work / "swtpm.sock").exists() or sw.poll() is not None:
                    break
                time.sleep(0.2)
            if not (work / "swtpm.sock").exists():
                raise ValueError("swtpm did not start")
            (r / "last_cmd.txt").write_text(shlex.join(command) + "\n")
            log = stack.enter_context(open(r / "qemu.log", "w"))
            q = subprocess.Popen(command, env=env, stdout=log, stderr=subprocess.STDOUT, stdin=subprocess.DEVNULL)
            stack.callback(stop_process, q)
            booted = time.monotonic()
            record("boot", "PASS", "plain mode, restrict=%s, disposable overlay" % ("off" if a.net == "on" else "on"))
            g = Guest(state, a.ssh_key, state["ids"]["net"]["ssh_port"])
            try:
                while time.monotonic() - booted < 600 and q.poll() is None:
                    if g.ssh(["echo", "up"], timeout=40).returncode == 0:
                        break
                    time.sleep(5)
                else:
                    raise ValueError("Guest SSH unavailable")
                record("ssh", "PASS", "loopback guest transport ready")
                time.sleep(60)
                power = power_fix(g)
                (out / "power.json").write_text(json.dumps(power, indent=2))
                record("power", "OK" if power["ok"] else "NOT OK", "values=" + json.dumps(power["values"], separators=(",", ":")), "FILE")
                if not power["ok"]:
                    raise ValueError("Power prerequisites NOT OK; app checks not run")
                probed = run_probe(g)
                ids, unknown = R.discover(reg, probed)
                (out / "probe.json").write_text(json.dumps({"installed": probed, "supported": ids, "unsupported": unknown}, indent=2))
                record("probe", "PASS", "%d programs, %d supported, %d with no driver" % (len(probed), len(ids), len(unknown)))
                names = ids if a.apps == "probe" else [R.canonical(reg, n) or n for n in a.apps.split(",") if n]
                if a.hold_until_file:
                    record("hold", hold_for_release(a.hold_until_file, 3 * 3600), "guest held; ssh port %s" % g.port)
                for name in names:
                    if name not in reg or (reg[name]["driver"] is None and name not in CHECKS):
                        record("app:" + name, "NOT_RUN", "NO_DRIVER")
                        continue
                    app = run_app(reg[name], out, g, superposition_run=a.superposition_run)
                    status, classification = app_classification(app)
                    record("app:" + name, status, app.get("error") or app.get("signal", ""), app["evidence"], classification)
            finally:
                try:
                    g.ps("Stop-Computer -Force", timeout=30)
                except (OSError, subprocess.TimeoutExpired):
                    pass
                try:
                    q.wait(timeout=180)
                except subprocess.TimeoutExpired:
                    stop_process(q)
                record("stop", "PASS" if q.poll() is not None else "FAIL", "own QEMU stopped")
    except (PI.PropError, ValueError, TypeError, OSError, KeyError, subprocess.SubprocessError) as exc:
        record("harness", "HARNESS", exc)
    (out / "result.json").write_text(json.dumps(rows, indent=2))
    (out / "result.md").write_text(result_markdown(rows) + "\n")
    return 0 if rows and all(r["verdict"] in ("PASS", "OK") for r in rows) else 1


def run_app(definition, out, guest, **options):
    """One application through its definition's driver: propimage_check.run_check for the core table, else `module:function`."""
    import importlib.util
    target = R.driver_for(definition)
    if target is None:
        return run_check(definition["id"], {}, out, guest, **options)
    path, func = target
    spec = importlib.util.spec_from_file_location("kf_driver_" + path.stem, path)
    mod = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(mod)
    return getattr(mod, func)(definition["id"], out, guest)


def run_probe(guest):
    done = guest.ps(PROBE_PS, timeout=120)
    return parse_probe(done.stdout) if done.returncode == 0 else []


if __name__ == "__main__":
    sys.exit(main())
