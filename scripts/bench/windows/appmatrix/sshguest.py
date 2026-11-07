# SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later
"""sshguest.py -- the guest transport of the Windows app matrix on a win_vm.sh guest (a rented box).

STATUS: DESIGN + CODE, 2026-11-07 (docs/design/V3_WINDOWS_APPS_GATE.md). GPU-free tests only
(tests/test_sshguest.py); the first hardware use is recorded in traces/winapps_20261107/.

`win_vm.sh` guests have no usable QEMU guest agent in the gate image (README of scripts/bench/windows),
but they have a key-only OpenSSH server with PowerShell as the default shell, forwarded on
127.0.0.1:<port>. `SshGuest` offers the subset of the `qga.Qga` interface that `winapps.GuestSession`
uses (ping, exec, powershell, powershell_command, file_write, file_read, file_exists, close), so the
driver, the guest scripts, the verdict rules and the result protocol are the same on both transports.

Every call is bounded. A transport fault (ssh cannot connect, the connection drops, a copy fails) raises
`qga.QgaError`, exactly as the QGA transport does, and the driver's existing recovery paths apply; a
guest program that outlives its timeout keeps running in the guest (as with QGA, nothing here kills it:
the guest-side supervisor `kf_run_app.ps1` owns that). Nothing here prints or stores a secret: the
password of the guest account is never passed through this module (autologon is set by `desktop-prep`).
"""
import base64
import os
import shutil
import subprocess
import tempfile

import qga as qgamod

PS_EXE = r"C:\Windows\System32\WindowsPowerShell\v1.0\powershell.exe"


def ps_quote(s):
    """A PowerShell single-quoted literal. An empty argument becomes '""': Windows PowerShell 5.1 drops an empty
    string handed to a native program, but passes a literal pair of double quotes, which the program's argument
    parser reads as an empty argument."""
    s = str(s)
    if s == "":
        return "'\"\"'"
    return "'" + s.replace("'", "''") + "'"


def encoded_command(script):
    """The remote command line for a PowerShell script: no quoting layer between ssh, the default shell and
    powershell.exe. PowerShell wants UTF-16LE in base64."""
    return "powershell -NoProfile -NonInteractive -EncodedCommand " + base64.b64encode(script.encode("utf-16-le")).decode()


class SshGuest:
    def __init__(self, port, key, user="kf", host="127.0.0.1", io_timeout=20.0, ssh="ssh", scp="scp", workdir=None):
        self.port, self.key, self.user, self.host = int(port), key, user, host
        self.io_timeout = io_timeout
        self.ssh_bin, self.scp_bin = ssh, scp
        self._own_dir = workdir is None
        self.dir = workdir or tempfile.mkdtemp(prefix="kfwa-ssh.")
        self.cm = os.path.join(self.dir, "cm")
        self._master_up = False

    # ---- plumbing
    def _opts(self, connect_timeout):
        return ["-i", self.key, "-o", "BatchMode=yes", "-o", "StrictHostKeyChecking=no", "-o", "UserKnownHostsFile=/dev/null",
                "-o", "LogLevel=ERROR", "-o", f"ConnectTimeout={max(1, int(connect_timeout))}", "-o", "ServerAliveInterval=15",
                "-o", "ServerAliveCountMax=4", "-o", "ControlMaster=auto", "-o", f"ControlPath={self.cm}", "-o", "ControlPersist=60"]

    def _run(self, argv, timeout, input_data=None):
        try:
            p = subprocess.run(argv, input=input_data, capture_output=True, timeout=timeout)
        except subprocess.TimeoutExpired:
            return None
        except OSError as e:
            raise qgamod.QgaError(f"cannot run {argv[0]}: {e}")
        return p

    def close(self):
        if self._master_up or os.path.exists(self.cm):
            subprocess.run([self.ssh_bin, "-o", f"ControlPath={self.cm}", "-O", "exit", f"{self.user}@{self.host}", "-p", str(self.port)],
                           capture_output=True, timeout=10)
        self._master_up = False

    def cleanup(self):
        self.close()
        if self._own_dir:
            shutil.rmtree(self.dir, ignore_errors=True)

    def _ssh(self, command, timeout, input_data=None):
        """-> (rc, out, err, timed_out). rc 255 is ssh's own failure (no connection)."""
        argv = [self.ssh_bin, "-p", str(self.port), *self._opts(min(timeout, max(self.io_timeout, 5))), f"{self.user}@{self.host}", command]
        p = self._run(argv, timeout + 5, input_data)
        if p is None:
            return None, b"", b"", True
        self._master_up = True
        return p.returncode, p.stdout, p.stderr, False

    # ---- the Qga-compatible surface
    def ping(self, timeout=5.0):
        try:
            rc, out, _, to = self._ssh("echo KF_SSH_UP", timeout)
        except qgamod.QgaError:
            return False
        return (not to) and rc == 0 and b"KF_SSH_UP" in out

    def exec(self, path, args=(), timeout=60.0, poll=0.5, input_data=None):
        script = "& " + ps_quote(path) + "".join(" " + ps_quote(a) for a in args) + "\nexit $LASTEXITCODE"
        if path.lower().endswith("powershell.exe") and "-Command" in list(args):
            # `powershell -Command X` is X in the default shell: one process less, same result, no quoting layers
            i = list(args).index("-Command")
            script = "$ErrorActionPreference = 'Continue'\n" + str(args[i + 1]) + "\nif ($?) { exit 0 } else { exit 1 }"
        rc, out, err, to = self._ssh(encoded_command(script), timeout, input_data)
        if to:
            return qgamod.ExecResult(None, b"", b"", True, None)
        if rc == 255:
            raise qgamod.QgaError("ssh: no connection to the guest: " + err.decode("utf-8", "replace").strip()[-120:])
        return qgamod.ExecResult(rc, out, err, False, None)

    def powershell(self, script_path, args=(), timeout=60.0):
        return self.exec(PS_EXE, ["-NoProfile", "-NonInteractive", "-ExecutionPolicy", "Bypass", "-File", script_path, *args], timeout=timeout)

    def powershell_command(self, command, timeout=60.0):
        return self.exec(PS_EXE, ["-NoProfile", "-NonInteractive", "-ExecutionPolicy", "Bypass", "-Command", command], timeout=timeout)

    def _scp(self, src, dst, timeout):
        argv = [self.scp_bin, "-q", "-P", str(self.port), *self._opts(min(timeout, max(self.io_timeout, 5))), src, dst]
        p = self._run(argv, timeout)
        if p is None:
            raise qgamod.QgaTimeout(f"scp timed out after {timeout}s")
        self._master_up = True
        return p

    @staticmethod
    def _gpath(path):
        return path.replace("\\", "/")

    def file_write(self, gpath, data, chunk=0):
        fd, tmp = tempfile.mkstemp(dir=self.dir, prefix="w.")
        try:
            with os.fdopen(fd, "wb") as f:
                f.write(data)
            p = self._scp(tmp, f"{self.user}@{self.host}:{self._gpath(gpath)}", max(60.0, len(data) / 2e6 + 30))
            if p.returncode != 0:
                raise qgamod.QgaError(f"scp to {gpath} failed rc={p.returncode}: {p.stderr.decode('utf-8', 'replace').strip()[-120:]}")
        finally:
            try:
                os.unlink(tmp)
            except OSError:
                pass

    def file_read(self, gpath, head=1 << 20, tail=3 << 20):
        """Whole file if it is at most head+tail bytes, else its first `head` and last `tail` bytes joined by a marker."""
        fd, tmp = tempfile.mkstemp(dir=self.dir, prefix="r.")
        os.close(fd)
        try:
            p = self._scp(f"{self.user}@{self.host}:{self._gpath(gpath)}", tmp, 120.0)
            if p.returncode != 0:
                err = p.stderr.decode("utf-8", "replace")
                if p.returncode == 1 and ("No such file" in err or "not a regular file" in err):
                    raise qgamod.QgaCommandError(f"no such file: {gpath}")
                raise qgamod.QgaError(f"scp from {gpath} failed rc={p.returncode}: {err.strip()[-120:]}")
            data = open(tmp, "rb").read()
        finally:
            try:
                os.unlink(tmp)
            except OSError:
                pass
        if len(data) <= head + tail:
            return data
        return data[:head] + b"\n=== [kf: middle of the log elided] ===\n" + data[-tail:]

    def file_exists(self, gpath):
        r = self.exec(PS_EXE, ["-NoProfile", "-NonInteractive", "-Command", f"Test-Path -LiteralPath {ps_quote(gpath)}"], timeout=30.0)
        if r.timed_out:
            raise qgamod.QgaTimeout("Test-Path timed out")
        return r.exitcode == 0 and r.text().strip().lower() == "true"
