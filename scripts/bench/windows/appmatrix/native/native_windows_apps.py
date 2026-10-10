#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later
"""native_windows_apps.py -- the Windows app matrix on a NON-kayfabe, already-running Windows (the native-NVIDIA baseline).

The baseline box is a Windows 11 VM on a vast.ai host with a real NVIDIA GPU passed through (the owner's
windows-on-vast template): the only door is public-key SSH (user `vast`, an administrator, default shell Windows
PowerShell). This adapter reuses the lane unchanged -- winapps.GuestSession, verdict.py, apps.json, the guest/*.ps1
scripts and the result-file protocol (docs/design/V3_WINDOWS_APP_MATRIX.md §2.2) -- and replaces only the two
transports the kayfabe lane has:

  * qga.Qga (QEMU guest agent over a unix socket)  ->  SshQga (one ssh per call, multiplexed with ControlMaster)
  * BrokerVm (windows_broker_prod2.sh, QMP)        ->  NativeVm ("a fresh guest" = reboot the box; the app disk is the
    very same kfapps.iso, mounted with Mount-DiskImage so the guest scripts find volume KFAPPS exactly as on kayfabe;
    screenshots are taken inside the interactive session instead of QMP screendump)

winapps.py and the guest scripts are not modified. Run `native_windows_apps.py --help`.

Subcommands
  prep      one-time box preparation (throwaway local password + autologon of the account, no sleep/lock/UAC prompts,
            Defender exclusions); the password is read from $KF_GUEST_PW, never logged, never written to the run dir
  push-iso  stream kfapps.iso from stdin (`cat kfapps.iso | ... push-iso`) to the box and verify its sha256
  run       the matrix (same options as winapps.py: --apps/--tier/--category/--exclude/--per-guest/--resume/...)
  shot      take one screenshot of the interactive desktop

Connection: $KF_SSH_HOST $KF_SSH_PORT $KF_SSH_USER (default vast) $KF_SSH_KEY $KF_SSH_KNOWN_HOSTS ; the host/port never go into git.
"""
import argparse
import base64
import hashlib
import json
import os
import re
import shlex
import subprocess
import sys
import time

HERE = os.path.dirname(os.path.abspath(__file__))
APPM = os.path.abspath(os.path.join(HERE, ".."))
sys.path.insert(0, APPM)
import qga as qgamod  # noqa: E402
import winapps  # noqa: E402

PS_EXE = r"C:\Windows\System32\WindowsPowerShell\v1.0\powershell.exe"


def psq(s):
    """single-quoted PowerShell string literal"""
    return "'" + str(s).replace("'", "''") + "'"


class Conn:
    """ssh to the box. One persistent control connection; every call is a fresh remote PowerShell."""

    def __init__(self, host=None, port=None, user=None, key=None, known_hosts=None, ctl_dir=None):
        self.host = host or os.environ["KF_SSH_HOST"]
        self.port = str(port or os.environ.get("KF_SSH_PORT", "22"))
        self.user = user or os.environ.get("KF_SSH_USER", "vast")
        self.key = key or os.environ.get("KF_SSH_KEY", os.path.expanduser("~/.ssh/id_ed25519"))
        self.kh = known_hosts or os.environ.get("KF_SSH_KNOWN_HOSTS", "")
        self.ctl = os.path.join(ctl_dir or os.environ.get("KF_SSH_CTL_DIR", "/tmp"), f"kfnat-{os.getpid()}-%C")

    def argv(self):
        a = ["ssh", "-i", self.key, "-o", "IdentitiesOnly=yes", "-o", "BatchMode=yes", "-o", "LogLevel=ERROR",
             "-o", "ConnectTimeout=15", "-o", "ServerAliveInterval=15", "-o", "ServerAliveCountMax=4",
             "-o", "ControlMaster=auto", "-o", f"ControlPath={self.ctl}", "-o", "ControlPersist=120",
             "-p", self.port]
        if self.kh:
            a += ["-o", "StrictHostKeyChecking=yes", "-o", f"UserKnownHostsFile={self.kh}"]
        else:
            a += ["-o", "StrictHostKeyChecking=no", "-o", "UserKnownHostsFile=/dev/null"]
        return a + [f"{self.user}@{self.host}"]

    def scp_argv(self):
        a = ["scp", "-q", "-i", self.key, "-o", "IdentitiesOnly=yes", "-o", "BatchMode=yes", "-o", "LogLevel=ERROR", "-o", "ConnectTimeout=15",
             "-o", "ControlMaster=auto", "-o", f"ControlPath={self.ctl}", "-o", "ControlPersist=120", "-P", self.port]
        if self.kh:
            a += ["-o", "StrictHostKeyChecking=yes", "-o", f"UserKnownHostsFile={self.kh}"]
        else:
            a += ["-o", "StrictHostKeyChecking=no", "-o", "UserKnownHostsFile=/dev/null"]
        return a

    @staticmethod
    def rpath(gpath):
        """C:\\kf\\x -> /C:/kf/x (the Windows OpenSSH sftp-server form)"""
        return "/" + gpath.replace("\\", "/")

    def put(self, local, gpath, timeout=600):
        p = subprocess.run(self.scp_argv() + [local, f"{self.user}@{self.host}:{self.rpath(gpath)}"], capture_output=True, timeout=timeout)
        if p.returncode != 0:
            raise qgamod.QgaCommandError(f"scp put {gpath} rc={p.returncode} {p.stderr.decode('utf-8', 'replace')[-200:]}")

    def get(self, gpath, local, timeout=600):
        p = subprocess.run(self.scp_argv() + [f"{self.user}@{self.host}:{self.rpath(gpath)}", local], capture_output=True, timeout=timeout)
        if p.returncode != 0:
            raise qgamod.QgaCommandError(f"scp get {gpath} rc={p.returncode} {p.stderr.decode('utf-8', 'replace')[-200:]}")

    def run(self, ps, stdin=None, timeout=60.0):
        """Run PowerShell text on the box. Returns (rc, stdout bytes, stderr bytes) or raises on a transport fault.
        rc None = timed out (the local ssh was killed)."""
        wrapped = ("$ProgressPreference='SilentlyContinue';$ErrorActionPreference='Continue';"
                   "[Console]::OutputEncoding=New-Object Text.UTF8Encoding $false;" + ps)
        b64 = base64.b64encode(wrapped.encode("utf-8")).decode()
        cmd = ("& ([scriptblock]::Create([Text.Encoding]::UTF8.GetString([Convert]::FromBase64String('" + b64 + "'))))")
        try:
            p = subprocess.run(self.argv() + [cmd], input=stdin, capture_output=True, timeout=timeout)
        except subprocess.TimeoutExpired:
            return None, b"", b""
        if p.returncode == 255:
            raise qgamod.QgaError("ssh transport: " + p.stderr.decode("utf-8", "replace").strip()[-200:])
        return p.returncode, p.stdout, p.stderr


class SshQga:
    """Duck-types qga.Qga for winapps.GuestSession (ping/exec/powershell/powershell_command/file_*)."""

    def __init__(self, conn, io_timeout=20.0):
        self.c, self.io_timeout = conn, io_timeout

    def connect(self):
        return None

    def close(self):
        return None

    def ping(self, timeout=5.0):
        try:
            rc, out, _ = self.c.run("'KFPONG'", timeout=max(timeout, 12.0))
        except qgamod.QgaError:
            return False
        return rc == 0 and b"KFPONG" in out

    def exec(self, path, args=(), timeout=60.0, poll=0.5, input_data=None):
        ps = "& " + psq(path) + " " + " ".join(psq(a) for a in args) + "\nexit $LASTEXITCODE"
        rc, out, err = self.c.run(ps, stdin=input_data, timeout=timeout)
        if rc is None:
            return qgamod.ExecResult(None, b"", b"", True, None)
        return qgamod.ExecResult(rc, out, err, False, None)

    def powershell(self, script_path, args=(), timeout=60.0):
        return self.exec(PS_EXE, ["-NoProfile", "-NonInteractive", "-ExecutionPolicy", "Bypass", "-File", script_path, *args], timeout=timeout)

    def powershell_command(self, command, timeout=60.0):
        rc, out, err = self.c.run(command, timeout=timeout)
        if rc is None:
            return qgamod.ExecResult(None, b"", b"", True, None)
        return qgamod.ExecResult(rc, out, err, False, None)

    def file_write(self, gpath, data, chunk=0):
        # stdin through Windows OpenSSH with a PowerShell default shell hangs after ~100 KB: files go by scp (sftp-server)
        import tempfile
        with tempfile.TemporaryDirectory(prefix="kfput") as t:
            lp = os.path.join(t, "f")
            with open(lp, "wb") as f:
                f.write(data)
            try:
                self.c.put(lp, gpath)
            except qgamod.QgaCommandError:                 # the directory may not exist yet
                self.c.run(f"New-Item -ItemType Directory -Force -Path {psq(gpath.rsplit(chr(92), 1)[0])}|Out-Null")
                self.c.put(lp, gpath)

    def file_read(self, gpath, head=1 << 20, tail=3 << 20):
        import tempfile
        with tempfile.TemporaryDirectory(prefix="kfget") as t:
            lp = os.path.join(t, "f")
            try:
                self.c.get(gpath, lp)
            except qgamod.QgaCommandError as e:
                raise qgamod.QgaCommandError(f"file_read {gpath}: {e}")
            data = open(lp, "rb").read()
        if len(data) <= head + tail:
            return data
        return data[:head] + b"\n=== [kf: middle of the log elided] ===\n" + data[-tail:]

    def file_exists(self, gpath):
        rc, _, _ = self.c.run(f"if(Test-Path -LiteralPath {psq(gpath)} -PathType Leaf){{exit 0}}else{{exit 7}}", timeout=30)
        if rc is None:
            raise qgamod.QgaTimeout("file_exists timed out")
        return rc == 0


class NativeVm(winapps.Vm):
    """The 'guest' is the box itself. launch(): the first one just waits for ssh; later ones reboot first (a fresh guest)."""

    def __init__(self, conn, log, iso_guest_path=r"C:\kfmedia\kfapps.iso", reboot_fresh=True, shot_dir=None):
        self.c, self.log, self.iso, self.reboot_fresh = conn, log, iso_guest_path, reboot_fresh
        self.count = 0
        self.shot_dir = shot_dir

    def qga_path(self):
        return "ssh"

    def alive(self):
        return True

    def run_dir(self):
        return None

    def _wait_ssh(self, timeout=900):
        t0 = time.time()
        q = SshQga(self.c)
        while time.time() - t0 < timeout:
            if q.ping(8):
                return True
            time.sleep(5)
        return False

    def launch(self, tag):
        if self.count > 0 and self.reboot_fresh:
            self.log(f"[{tag}] fresh guest = reboot of the box")
            try:
                self.c.run("shutdown.exe /r /t 3 /f /c 'kf native baseline: fresh guest'", timeout=30)
            except qgamod.QgaError:
                pass
            time.sleep(40)                      # let sshd go down before waiting for it to come back
        self.count += 1
        if not self._wait_ssh():
            raise RuntimeError("box did not answer ssh after reboot")

    def stop(self):
        return "noop (native box stays up)"

    def send_key(self, qcode, shift=False):
        return None

    def attach_disk(self, iso):
        ps = (f"$i={psq(self.iso)};if(-not(Test-Path $i)){{'KFMOUNT missing';exit 5}};"
              "$d=Get-DiskImage -ImagePath $i;if(-not $d.Attached){Mount-DiskImage -ImagePath $i -StorageType ISO -Access ReadOnly|Out-Null;Start-Sleep 3};"
              "$v=Get-Volume|Where-Object{$_.FileSystemLabel -eq 'KFAPPS'}|Select-Object -First 1;"
              "if($v){'KFMOUNT '+$v.DriveLetter+':'}else{'KFMOUNT novolume';exit 6}")
        rc, out, _ = self.c.run(ps, timeout=120)
        txt = out.decode("utf-8", "replace").strip()
        if rc != 0:
            raise RuntimeError(f"app disk mount failed: {txt}")
        return txt

    def detach_disk(self):
        return None

    def screendump(self, png_path):
        """capture the interactive desktop (task in the signed-in user's session), pull the PNG, check its magic"""
        try:
            r = SshQga(self.c).powershell(r"C:\kf\kf_native_shot.ps1", timeout=90)
            if r.exitcode != 0:
                return False
            data = SshQga(self.c).file_read(r"C:\kf\shot_native.png", head=8 << 20, tail=8 << 20)
        except qgamod.QgaError:
            return False
        if not data.startswith(b"\x89PNG\r\n\x1a\n"):
            return False
        os.makedirs(os.path.dirname(png_path), exist_ok=True)
        with open(png_path, "wb") as f:
            f.write(data)
        return True


def install_override(conn, override_dir):
    """Overlay locally built test programs (build_tools.sh output, never committed) over the image's tools after each guest's
    setup: lets the baseline run a fixed probe without rebuilding the 6.8 GB image."""
    orig = winapps.GuestSession._setup

    def _setup(self):
        orig(self)
        q = SshQga(conn)
        for f in sorted(os.listdir(override_dir)):
            if f.endswith(".exe"):
                conn.run(rf"Remove-Item -Force -ErrorAction SilentlyContinue C:\kfapps\tools\{f}")      # the copy from the ISO is read-only
                q.file_write(rf"C:\kfapps\tools\{f}", open(os.path.join(override_dir, f), "rb").read())
                self.log(f"[{self.tag}] override tool {f}")
    winapps.GuestSession._setup = _setup


def install_transport(conn):
    qgamod.Qga = lambda path, io_timeout=20.0: SshQga(conn, io_timeout)


def push_helpers(conn):
    q = SshQga(conn)
    q.c.run(r"New-Item -ItemType Directory -Force -Path C:\kf,C:\kfmedia | Out-Null")
    q.file_write(r"C:\kf\kf_native_shot.ps1", open(os.path.join(HERE, "kf_native_shot.ps1"), "rb").read())


def cmd_prep(a, conn):
    pw = os.environ.get("KF_GUEST_PW")
    if not pw:
        raise SystemExit("KF_GUEST_PW (a throwaway password) is required for prep; it is not printed")
    q = SshQga(conn)
    push_helpers(conn)
    q.file_write(r"C:\kf\kf_native_prep.ps1", open(os.path.join(HERE, "kf_native_prep.ps1"), "rb").read())
    r = q.powershell(r"C:\kf\kf_native_prep.ps1", ["-User", a.user, "-Password", pw], timeout=300)
    rc, txt = r.exitcode, r.text() + r.err.decode("utf-8", "replace")
    print(txt.replace(pw, "<pw>"))
    return rc or 0


def cmd_push_iso(a, conn):
    """stdin -> C:\\kfmedia\\kfapps.iso on the box, sha256 verified against --sha256 (no secrets pass through)"""
    push_helpers(conn)
    dst = r"C:\kfmedia\kfapps.iso"
    ps = (f"$p={psq(dst + '.part')};$fs=[IO.File]::Open($p,'Create','Write','Read');try{{[Console]::OpenStandardInput().CopyTo($fs,8MB)}}finally{{$fs.Close()}};"
          f"$h=(Get-FileHash -Algorithm SHA256 $p).Hash.ToLower();'KFISO '+$h+' '+(Get-Item $p).Length;"
          f"if($h -eq {psq(a.sha256)}){{Move-Item -Force $p {psq(dst)};'KFISO verified'}}else{{'KFISO MISMATCH';exit 9}}")
    p = subprocess.run(conn.argv() + ["& ([scriptblock]::Create([Text.Encoding]::UTF8.GetString([Convert]::FromBase64String('" +
                                      base64.b64encode(ps.encode()).decode() + "'))))"], stdin=sys.stdin.buffer, capture_output=True)
    print(p.stdout.decode("utf-8", "replace"), p.stderr.decode("utf-8", "replace")[-300:])
    return p.returncode


def cmd_shot(a, conn):
    push_helpers(conn)
    vm = NativeVm(conn, print)
    ok = vm.screendump(a.out)
    print("shot", "ok" if ok else "FAILED", a.out)
    return 0 if ok else 1


def cmd_run(a, conn):
    doc = json.load(open(a.apps_json))
    man = json.load(open(a.manifest))
    names = [x for x in a.apps.split(",") if x]
    apps = winapps.select_apps(doc["apps"], names, a.tier, [x for x in a.category.split(",") if x], set(x for x in a.exclude.split(",") if x))
    if a.list:
        for x in apps:
            print(f"{x['id']:28s} {x['category']:11s} tier{x['tier']} {x['session']:11s} {x['timeout_s']:5d}s {x['difficulty']}")
        return 0
    os.makedirs(a.run_dir, exist_ok=True)
    log = winapps.Log(os.path.join(a.run_dir, "session.log"))
    ident = None
    if a.image_manifest and os.path.exists(a.image_manifest):
        ident = json.load(open(a.image_manifest)).get("image", {}).get("identity")
    cfg = winapps.Cfg(iso=None, image_identity=ident, per_guest=a.per_guest, max_tdr=a.max_tdr, tier=a.tier, password=None,
                      user=conn.user, screenshots=True, boot_timeout=a.boot_timeout, session_timeout=420, recover_wait=300, io_timeout=30.0, ping_s=10.0)
    install_transport(conn)
    if a.override_dir:
        install_override(conn, a.override_dir)
    push_helpers(conn)
    vm = NativeVm(conn, log, reboot_fresh=not a.no_reboot)
    meta = dict(started_utc=winapps.utc(), kind="native-nvidia-baseline", args={k: v for k, v in vars(a).items() if k not in ("func",)},
                image_identity=ident, apps=[x["id"] for x in apps])
    with open(os.path.join(a.run_dir, "matrix.json"), "w") as f:
        json.dump(meta, f, indent=1)
    log(f"START run_dir={a.run_dir} vm=native apps={len(apps)}")
    rc = winapps.run_matrix(cfg, lambda: vm, doc, man, a.run_dir, apps, log, isolate=not a.no_isolate, resume=a.resume)
    log(f"EXIT rc={rc}")
    return rc


def main(argv=None):
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    sub = ap.add_subparsers(dest="cmd", required=True)
    p = sub.add_parser("prep")
    p.add_argument("--user", default="vast")
    p.set_defaults(func=cmd_prep)
    p = sub.add_parser("push-iso")
    p.add_argument("--sha256", required=True)
    p.set_defaults(func=cmd_push_iso)
    p = sub.add_parser("shot")
    p.add_argument("--out", required=True)
    p.set_defaults(func=cmd_shot)
    p = sub.add_parser("run")
    p.add_argument("--run-dir", required=True)
    p.add_argument("--apps", default="all")
    p.add_argument("--tier", type=int, default=1)
    p.add_argument("--category", default="")
    p.add_argument("--exclude", default="")
    p.add_argument("--manifest", default=os.path.join(APPM, "manifest.json"))
    p.add_argument("--apps-json", default=os.path.join(APPM, "apps.json"))
    p.add_argument("--image-manifest", default=os.path.join(APPM, "manifest.json"))
    p.add_argument("--per-guest", type=int, default=1000, help="apps per boot of the box (native: no contamination bound needed; default = never reboot between apps)")
    p.add_argument("--max-tdr", type=int, default=1000)
    p.add_argument("--boot-timeout", type=int, default=900)
    p.add_argument("--no-isolate", action="store_true")
    p.add_argument("--override-dir", default="", help="directory of locally built *.exe copied over C:\\kfapps\\tools after each guest's setup")
    p.add_argument("--no-reboot", action="store_true", help="a 'fresh guest' does not reboot the box")
    p.add_argument("--resume", action="store_true")
    p.add_argument("--list", action="store_true")
    p.set_defaults(func=cmd_run)
    a = ap.parse_args(argv)
    conn = Conn()
    return a.func(a, conn)


if __name__ == "__main__":
    sys.exit(main())
