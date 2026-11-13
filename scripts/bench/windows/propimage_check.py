#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later
"""propimage_check.py: one unattended run of the proprietary Windows image, OFFLINE, on a Vast box (or any KVM host with kf3).

STATUS: DESIGN + CODE, 2026-11-13 (first hardware run: see the evidence directory of the run that added this file).
The logic lives here; shell is only glue (the box's ssh, a presigned URL file). Steps, in order, each one a row of the result table:

  unpack   image.zip -> <work>/img (propimage.unpack: sha256 of every member, seed check), swtpm and OVMF files restored
  clock    a COPY of state.json with clock.last_time_at_stop_utc raised when --stop-utc is given (never before the stored value)
  overlay  <work>/run/clone.qcow2, a throwaway qcow2 overlay of the unpacked base (the base is never opened for writing)
  boot     swtpm, the repo's display broker (TEST backend: no window, the guest still gets a monitor) and kf3 qemu with the
           KF3_* flags of kf3_flags.sample.env exported, slirp restrict=on, RTC = last stop + gap (propimage.boot_clock)
  ssh      wait for the guest's sshd on the loopback forward
  verify   propimage.compare: machine ids, clock window, TPM, GPU UUID (blocking mismatches fail the run)
  gpu      nvidia-smi in the guest
  apps     one check per app (APPS below): launch in the INTERACTIVE session through a scheduled task (Register-ScheduledTask
           -Principal kfimage Interactive), wait, screenshot with CopyFromScreen, copy the PNG off the guest
  stop     clean guest shutdown, qemu/broker/swtpm gone

Output: <out>/result.json, <out>/result.md (the table), <out>/shots/*.png, <out>/*.txt. Exit 0 only if every row is PASS or an
explicit, named SKIP; a FAIL row is never retried here (the caller decides).
Rules (owner, binding): the image is ALWAYS offline (restrict=on is in propimage.build_argv for every mode but setup/install,
and this script refuses those modes), ALWAYS on an overlay, and never booted with internet.
Needs: python3, qemu-img, swtpm, OVMF (secboot), ssh/scp, the kf3 qemu, the broker built (broker/make) or --display off.
"""
import argparse
import importlib.util
import json
import os
import shlex
import shutil
import signal
import subprocess
import sys
import time
from pathlib import Path

HERE = Path(__file__).resolve().parent


def _load_propimage():
    spec = importlib.util.spec_from_file_location("propimage", HERE / "propimage.py")
    mod = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(mod)
    return mod


PI = _load_propimage()
ROWS = []  # (step, verdict, detail)


def row(step, verdict, detail=""):
    ROWS.append((step, verdict, str(detail)[:400]))
    print("%-5s %-14s %s" % (verdict, step, str(detail)[:200]), flush=True)
    return verdict


def run(cmd, **kw):
    kw.setdefault("capture_output", True)
    kw.setdefault("text", True)
    return subprocess.run(cmd, **kw)


# ---------------------------------------------------------------- guest access
class Guest:
    def __init__(self, state, key, port):
        self.key, self.port, self.user = key, port, state["guest"]["user"]

    def ssh(self, argv, stdin=None, timeout=120):
        base = ["ssh", "-i", self.key, "-p", str(self.port), "-o", "StrictHostKeyChecking=no", "-o", "UserKnownHostsFile=/dev/null",
                "-o", "LogLevel=ERROR", "-o", "ConnectTimeout=20", "-o", "BatchMode=yes", "%s@127.0.0.1" % self.user]
        return run(base + argv, input=stdin, timeout=timeout)

    def ps(self, script, timeout=120):
        """PowerShell. A one-liner goes over stdin; a multi-line script is copied to C:\\kfhw and run with -File (stdin parsing of
        multi-line blocks silently printed nothing on the first hardware run)."""
        if "\n" not in script.strip():
            return self.ssh(["powershell", "-NoProfile", "-ExecutionPolicy", "Bypass", "-Command", "-"], stdin=script, timeout=timeout)
        tmp = Path("/tmp") / ("kfhw_ps_%d.ps1" % os.getpid())
        tmp.write_text(script)
        self.ssh(["powershell", "-NoProfile", "-Command", "New-Item -Force -ItemType Directory C:\\kfhw | Out-Null"], timeout=60)
        self.scp(str(tmp), "C:/kfhw/ps_run.ps1", True)
        return self.ssh(["powershell", "-NoProfile", "-ExecutionPolicy", "Bypass", "-File", "C:\\kfhw\\ps_run.ps1"], timeout=timeout)

    def scp(self, src, dst, to_guest):
        base = ["scp", "-q", "-i", self.key, "-P", str(self.port), "-o", "StrictHostKeyChecking=no", "-o", "UserKnownHostsFile=/dev/null",
                "-o", "LogLevel=ERROR"]
        remote = "%s@127.0.0.1:%s" % (self.user, dst if to_guest else src)
        return run(base + ([src, remote] if to_guest else [remote, dst]), timeout=300)

    def interactive(self, name, body, wait_s=0):
        """Run PowerShell `body` in the logged-in desktop session (a scheduled task whose principal is the interactive user)."""
        tmp = Path("/tmp") / ("kfhw_%s.ps1" % name)
        tmp.write_text(body)
        self.ps("New-Item -Force -ItemType Directory C:\\kfhw | Out-Null")
        self.scp(str(tmp), "C:/kfhw/%s.ps1" % name, True)
        reg = ("$n='%s'\nUnregister-ScheduledTask -TaskName $n -Confirm:$false -ErrorAction SilentlyContinue\n"
               "$a=New-ScheduledTaskAction -Execute 'powershell.exe' -Argument \"-NoProfile -ExecutionPolicy Bypass -File C:\\kfhw\\$n.ps1\"\n"
               "$p=New-ScheduledTaskPrincipal -UserId '%s' -LogonType Interactive -RunLevel Highest\n"
               "Register-ScheduledTask -TaskName $n -Action $a -Principal $p | Out-Null\nStart-ScheduledTask -TaskName $n\n'ok'\n") % (name, self.user)
        r = self.ps(reg)
        if wait_s:
            time.sleep(wait_s)
        return r

    def shot(self, name, outdir):
        body = ("Add-Type -AssemblyName System.Windows.Forms,System.Drawing\n$b=[System.Windows.Forms.SystemInformation]::VirtualScreen\n"
                "$bmp=New-Object System.Drawing.Bitmap $b.Width,$b.Height\n$g=[System.Drawing.Graphics]::FromImage($bmp)\n"
                "$g.CopyFromScreen($b.Left,$b.Top,0,0,$bmp.Size)\n$bmp.Save('C:\\kfhw\\shot_%s.png',[System.Drawing.Imaging.ImageFormat]::Png)\n" % name)
        self.interactive("shot_" + name, body, wait_s=6)
        dst = Path(outdir) / "shots" / (name + ".png")
        dst.parent.mkdir(parents=True, exist_ok=True)
        r = self.scp("C:/kfhw/shot_%s.png" % name, str(dst), False)
        return dst if r.returncode == 0 and dst.exists() and dst.stat().st_size > 2000 else None


# ---------------------------------------------------------------- app checks
# Launch-level checks (verdict LAUNCH_ONLY, never PASS: a window existing is not work done; owner rule 2026-11-13) measured on the first hardware run (docs: the evidence README of that run): each app is started in the
# interactive session, given `wait` seconds, photographed, and judged by (a) the named process being alive and (b) a main window
# title. Deeper checks (Time Spy run + SAVE, a SolidWorks part rotation, a Resolve render, a Superposition FPS run) need UI
# clicks and are NOT in this file yet (see the evidence README for what was tried by hand).
APPS = {
    # name: (exe, args, wait_s, process regex, window-title regex)
    "3dmark": (r"C:\Program Files\UL\3DMark\3DMark.exe", "", 150, "3DMark", "3DMark"),
    "solidworks": (r"C:\Program Files\SOLIDWORKS Corp\SOLIDWORKS\SLDWORKS.exe", "", 150, "SLDWORKS", "SOLIDWORKS"),
    "resolve": (r"C:\Program Files\Blackmagic Design\DaVinci Resolve\Resolve.exe", "", 120, "Resolve", "DaVinci|Resolve"),
    "superposition": (r"C:\Program Files\Unigine\Superposition Benchmark\bin\launcher.exe", "", 60, "launcher|superposition", "Superposition|UNIGINE"),
    # Steam is signed out and stays so: the launch only proves the tool's own window (or Steam's dialog for it) comes up
    "blackmyth": (r"C:\Users\kfimage\Desktop\Black Myth Wukong Benchmark Tool.url", "", 90, "steam|b1", "Black Myth|Steam"),
}


def check_app(g, out, name):
    exe, args, wait_s, proc_re, title_re = APPS[name]
    body = (
        "Add-Type -AssemblyName System.Windows.Forms,System.Drawing\n"
        "$o='C:\\kfhw\\r_%(n)s.txt'; Remove-Item $o -ErrorAction SilentlyContinue\n"
        "Start-Process -FilePath '%(exe)s'%(args)s\nStart-Sleep %(w)d\n"
        "$b=[System.Windows.Forms.SystemInformation]::VirtualScreen\n"
        "$bmp=New-Object System.Drawing.Bitmap $b.Width,$b.Height\n$g=[System.Drawing.Graphics]::FromImage($bmp)\n"
        "$g.CopyFromScreen($b.Left,$b.Top,0,0,$bmp.Size)\n$bmp.Save('C:\\kfhw\\app_%(n)s.png',[System.Drawing.Imaging.ImageFormat]::Png)\n"
        "$ps=Get-Process | ? { $_.ProcessName -match '%(pre)s' }\n"
        "\"procs=$(($ps|%%{$_.ProcessName}) -join ',')\" | Add-Content $o\n"
        "\"titles=$(($ps|? {$_.MainWindowTitle}|%%{$_.MainWindowTitle}) -join '|')\" | Add-Content $o\n"
        "$ps | Stop-Process -Force -ErrorAction SilentlyContinue\n'done' | Add-Content $o\n"
    ) % dict(n=name, exe=exe, args=(" -ArgumentList '%s'" % args if args else ""), w=wait_s, pre=proc_re)
    g.interactive("app_" + name, body)
    t0 = time.time()
    txt = ""
    while time.time() - t0 < wait_s + 90:
        time.sleep(10)
        r = g.ps("Get-Content C:\\kfhw\\r_%s.txt -ErrorAction SilentlyContinue" % name)
        txt = r.stdout
        if "done" in txt:
            break
    dst = Path(out) / "shots" / ("app_%s.png" % name)
    dst.parent.mkdir(parents=True, exist_ok=True)
    g.scp("C:/kfhw/app_%s.png" % name, str(dst), False)
    (Path(out) / ("app_%s.txt" % name)).write_text(txt)
    import re
    alive = re.search("procs=.*(%s)" % proc_re, txt) is not None
    titled = re.search("titles=.*(%s)" % title_re, txt) is not None
    ok = alive and titled and dst.exists() and dst.stat().st_size > 2000
    return ("LAUNCH_ONLY" if ok else "FAIL"), "evidence=SCREEN(launch only; the app did no work) alive=%s window=%s %s" % (alive, titled, txt.replace("\n", " ")[:160])


# ---------------------------------------------------------------- GPU class key (die-bound images)
def gpu_fix(g):
    """The image's driver was set up on one die; a host with another die shows kf3's guest a NEW display device instance, whose class key
    lacks EnableGpuFirmware (MEASURED on a 4090 host with an image captured on a 4070: Code 43 until the key is set on the new instance).
    Set the key on every NVIDIA display class instance that lacks it (inside the overlay only) and restart the device."""
    r = g.ps(r"""
$base='HKLM:\SYSTEM\CurrentControlSet\Control\Class\{4d36e968-e325-11ce-bfc1-08002be10318}'
$done=@()
Get-ChildItem $base -ErrorAction SilentlyContinue | ? { $_.PSChildName -match '^\d{4}$' } | % {
  $p=$_.PSPath; $ip=Get-ItemProperty $p -ErrorAction SilentlyContinue
  if ($ip.DriverDesc -match 'NVIDIA' -and $ip.EnableGpuFirmware -ne 1) { New-ItemProperty -Path $p -Name EnableGpuFirmware -PropertyType DWord -Value 1 -Force | Out-Null; $done += $ip.DriverDesc }
}
if ($done.Count) { Get-PnpDevice -Class Display | ? { $_.FriendlyName -match 'NVIDIA' -and $_.Status -ne 'OK' } | % { pnputil /restart-device $_.InstanceId | Out-Null }; Start-Sleep 15 }
"fixed=$($done -join ',')"
""")
    return r.stdout.strip()


# ---------------------------------------------------------------- the run
def main(argv=None):
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("--zip", help="image.zip (skipped when --dir already holds the unpacked image)")
    ap.add_argument("--dir", required=True, help="unpacked image directory (image.qcow2, state.json)")
    ap.add_argument("--work", required=True, help="work directory (tpm/, firmware/, run/)")
    ap.add_argument("--out", required=True, help="evidence directory")
    ap.add_argument("--qemu", required=True, help="the kf3 qemu binary")
    ap.add_argument("--stop-utc", help="override clock.last_time_at_stop_utc (e.g. 2026-11-12T17:17:00Z) in a COPY of the state")
    ap.add_argument("--flags", default=str(HERE / "kf3_flags.sample.env"))
    ap.add_argument("--broker", default=str(HERE.parents[2] / "broker" / "nvkvm-display-broker"))
    ap.add_argument("--display", choices=("broker", "off"), default="broker")
    ap.add_argument("--apps", default=",".join(APPS) or "all")
    ap.add_argument("--keep", action="store_true", help="leave the guest running at the end (debugging)")
    a = ap.parse_args(argv)
    out = Path(a.out)
    out.mkdir(parents=True, exist_ok=True)
    work = Path(a.work)
    (work / "run").mkdir(parents=True, exist_ok=True)
    procs = []

    def finish(rc):
        (out / "result.json").write_text(json.dumps([dict(step=s, verdict=v, detail=d) for s, v, d in ROWS], indent=1))
        md = ["| step | verdict | detail |", "|---|---|---|"] + ["| %s | %s | %s |" % (s, v, d.replace("|", "/")) for s, v, d in ROWS]
        (out / "result.md").write_text("\n".join(md) + "\n")
        bad = [r for r in ROWS if r[1] not in ("PASS", "SKIP")]
        print("RESULT %s (%d rows, %d not PASS)" % ("PASS" if not bad else "FAIL", len(ROWS), len(bad)))
        return 0 if not bad and rc == 0 else 1

    # unpack
    try:
        if a.zip:
            state = PI.unpack(a.zip, a.dir)
            row("unpack", "PASS", "members and seed verified")
        else:
            state = json.loads((Path(a.dir) / "state.json").read_text())
            PI.validate_state(state, require_password=True)
            row("unpack", "PASS" if PI.seed_matches(state) else "FAIL", "unpacked directory used; seed check")
        if a.stop_utc:
            old = PI.utc(state["clock"]["last_time_at_stop_utc"])
            new = PI.utc(a.stop_utc)
            if new < old:
                row("clock", "FAIL", "refusing to move the stop time backwards")
                return finish(1)
            state["clock"]["last_time_at_stop_utc"] = PI.fmt(new)
        stp = work / "state.json"
        stp.write_text(json.dumps(state))
        PI.restore_files(state, str(work))
        base = PI.boot_clock(state, "ci")
        row("clock", "PASS", "rtc base %s (stop + gap, never earlier)" % base.isoformat())
    except (PI.PropError, OSError, KeyError, json.JSONDecodeError) as e:
        row("unpack", "FAIL", e)
        return finish(1)

    r = work / "run"
    clone = r / "clone.qcow2"
    if clone.exists():
        clone.unlink()
    p = run(["qemu-img", "create", "-q", "-f", "qcow2", "-b", str(Path(a.dir) / "image.qcow2"), "-F", "qcow2", str(clone)])
    if p.returncode:
        row("overlay", "FAIL", p.stderr)
        return finish(1)
    row("overlay", "PASS", "throwaway overlay of the unpacked base")

    # boot
    for f in ("swtpm.sock", "run/swtpm.pid", "run/broker.sock"):
        try:
            (work / f).unlink()
        except FileNotFoundError:
            pass
    sw = subprocess.Popen([os.environ.get("SWTPM", "swtpm"), "socket", "--tpm2", "--tpmstate", "dir=%s/tpm,mode=0600" % work,
                           "--ctrl", "type=unixio,path=%s/swtpm.sock,mode=0600" % work, "--log", "file=%s/swtpm.log,level=20" % r,
                           "--pid", "file=%s/swtpm.pid" % r, "--terminate"], stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
    procs.append(sw)
    for _ in range(50):
        if (work / "swtpm.sock").exists():
            break
        time.sleep(0.2)
    broker_sock = str(r / "broker.sock")
    if a.display == "broker":
        fifo = r / "broker.fifo"
        if fifo.exists():
            fifo.unlink()
        os.mkfifo(fifo)
        fd = os.open(fifo, os.O_RDWR)  # keeps the broker's stdin open (the test backend reads its clock there)
        bp = subprocess.Popen([a.broker, "--socket", broker_sock, "--backend", "test", "--persist", "--verbose", "--size", "1920x1080"],
                              stdin=fd, stdout=open(r / "broker.log", "w"), stderr=subprocess.STDOUT)
        procs.append(bp)
        for _ in range(50):
            if os.path.exists(broker_sock):
                break
            time.sleep(0.2)
    kw = dict(broker_sock=broker_sock, gop_efi=str(work / "firmware" / "kf-gop.signed.efi"), uid=0) if a.display == "broker" else {}
    argv_q = PI.build_argv(state, "ci", str(clone), str(work), qemu=a.qemu, display="broker" if a.display == "broker" else "off", **kw)
    env = dict(os.environ)
    for line in Path(a.flags).read_text().splitlines():
        line = line.strip()
        if line and not line.startswith("#"):
            k, _, v = line.partition("=")
            if not (k.startswith("KF3_") and v.isdigit()):
                row("boot", "FAIL", "refused env line %r" % line)
                return finish(1)
            env[k] = v
    rows_cpu = [x for x in PI.cpu_preflight(state, Path("/proc/cpuinfo").read_text()) if x[1] != "MATCH"]
    if rows_cpu:
        row("boot", "FAIL", "CPU_PROFILE_UNSUPPORTED %s" % rows_cpu)
        return finish(1)
    (r / "last_cmd.txt").write_text(shlex.join(argv_q) + "\n")
    q = subprocess.Popen(argv_q, env=env, stdout=open(r / "qemu.log", "w"), stderr=subprocess.STDOUT, stdin=subprocess.DEVNULL)
    procs.append(q)
    row("boot", "PASS", "qemu pid %d, offline, display=%s, flags=%s" % (q.pid, a.display, Path(a.flags).name))

    g = Guest(state, str(work / "id_kf"), state["ids"]["net"]["ssh_port"])
    keyf = work / "id_kf"
    keyf.write_text(state["guest"]["ssh_private_key"])
    keyf.chmod(0o600)

    def teardown():
        if a.keep:
            return
        g.ps("Stop-Computer -Force", timeout=30)
        for _ in range(90):
            if q.poll() is not None:
                break
            time.sleep(2)
        for pr in reversed(procs):
            if pr.poll() is None:
                pr.terminate()
        time.sleep(1)
        for pr in procs:
            if pr.poll() is None:
                pr.kill()

    try:
        t0 = time.time()
        up = False
        while time.time() - t0 < 420 and q.poll() is None:
            if g.ssh(["echo", "up"], timeout=40).returncode == 0:
                up = True
                break
            time.sleep(5)
        if not up:
            row("ssh", "FAIL", "no ssh in 420 s (qemu rc=%s)" % q.poll())
            return finish(1)
        row("ssh", "PASS", "up after %d s" % (time.time() - t0))
        time.sleep(60)  # the desktop session (autologon) needs a moment after sshd
        fixed = gpu_fix(g)
        (out / "gpu_fix.txt").write_text(fixed + "\n")
        # verify (identity)
        try:
            guest_facts = PI.run_guest_probe(state, str(keyf))
            vrows = PI.compare(state, guest_facts)
            bad = [x for x in vrows if x[1] == "MISMATCH"]
            (out / "verify.txt").write_text("\n".join("%s %s %s" % x for x in vrows) + "\n")
            row("verify", "PASS" if not bad else "FAIL", "MATCH" if not bad else "; ".join(x[0] for x in bad))
        except Exception as e:  # noqa: BLE001
            row("verify", "FAIL", e)
        r2 = g.ps("& nvidia-smi --query-gpu=name,uuid,driver_version --format=csv,noheader; (Get-PnpDevice -Class Display | "
                  "Select-Object Status,FriendlyName | Format-List | Out-String)")
        (out / "gpu.txt").write_text(r2.stdout + r2.stderr)
        row("gpu", "PASS" if r2.returncode == 0 and "GPU-" in r2.stdout else "FAIL", r2.stdout.strip().splitlines()[:1])
        want = list(APPS) if a.apps == "all" else [x for x in a.apps.split(",") if x]
        for name in want:
            if name not in APPS:
                row("app:" + name, "SKIP", "no check defined")
                continue
            try:
                verdict, detail = check_app(g, out, name)
            except Exception as e:  # noqa: BLE001
                verdict, detail = "FAIL", "exception %r" % (e,)
            row("app:" + name, verdict, detail)
    finally:
        teardown()
        row("stop", "PASS" if q.poll() is not None else "FAIL", "qemu rc=%s" % q.poll())
        if a.display == "broker":
            try:
                os.close(fd)
            except OSError:
                pass
    return finish(0)


if __name__ == "__main__":
    sys.exit(main())
