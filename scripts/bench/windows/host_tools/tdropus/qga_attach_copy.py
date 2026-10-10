#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later
"""qga.py -- QEMU guest-agent and QMP clients for the Windows app matrix driver (stdlib only).

Qga: guest-sync handshake (flushes stale replies after a reconnect), guest-ping, guest-exec with
polled guest-exec-status, guest-file-{open,write,read,seek,close}. Nothing here blocks forever: every
call has a socket timeout and every exec a deadline. The agent cannot kill a guest process, so the driver
never runs anything long through exec: long work is launched detached (guest/kf_launch.ps1) and polled.

Qmp: just what the lane needs (hot-plug of the read-only app disk, screendump, send-key, query-status).

CLI (for manual use):  qga.py SOCK ping | exec PROG [ARG...] | read GUESTPATH | write GUESTPATH LOCALFILE
"""
import base64
import json
import os
import random
import socket
import sys
import time


class QgaError(Exception):
    pass


class QgaTimeout(QgaError):
    pass


class QgaCommandError(QgaError):
    pass


class ExecResult:
    def __init__(self, exitcode, out, err, timed_out=False, pid=None):
        self.exitcode, self.out, self.err, self.timed_out, self.pid = exitcode, out, err, timed_out, pid

    def text(self):
        return self.out.decode("utf-8", "replace") if isinstance(self.out, (bytes, bytearray)) else str(self.out)

    def __repr__(self):
        return f"ExecResult(rc={self.exitcode}, out={len(self.out)}B, err={len(self.err)}B, timed_out={self.timed_out})"


class Qga:
    def __init__(self, path, io_timeout=20.0):
        self.path = path
        self.io_timeout = io_timeout
        self.sock = None
        self.f = None
        self.buf = b""

    # ---- connection
    def connect(self):
        self.close()
        s = socket.socket(socket.AF_UNIX, socket.SOCK_STREAM)
        s.settimeout(self.io_timeout)
        try:
            s.connect(self.path)
        except OSError as e:
            s.close()
            raise QgaError(f"cannot connect {self.path}: {e}")
        self.sock, self.f, self.buf = s, True, b""
        self._sync()

    def close(self):
        try:
            if self.sock:
                self.sock.close()
        except OSError:
            pass
        self.sock = self.f = None
        self.buf = b""

    def _send(self, obj):
        self.sock.sendall(json.dumps(obj).encode() + b"\n")          # sendall: an unbuffered makefile().write may send only part

    def _recv(self):
        while b"\n" not in self.buf:
            try:
                chunk = self.sock.recv(1 << 16)
            except (socket.timeout, TimeoutError):
                raise QgaTimeout(f"no reply from {self.path} within {self.io_timeout}s")
            except OSError as e:
                raise QgaError(f"read failed: {e}")
            if not chunk:
                raise QgaError("agent socket closed")
            self.buf += chunk
        line, self.buf = self.buf.split(b"\n", 1)
        try:
            return json.loads(line)
        except ValueError:
            return {"_garbage": line[:80]}

    def _sync(self):
        """The documented handshake: a 0xFF byte makes the agent drop a half-parsed request, then guest-sync echoes our id."""
        n = random.randint(1, 2 ** 31 - 1)
        self.sock.sendall(b"\xff")
        self._send({"execute": "guest-sync", "arguments": {"id": n}})
        for _ in range(64):
            r = self._recv()
            if r.get("return") == n:
                return
        raise QgaError("guest-sync did not echo the id")

    # ---- raw call
    def call(self, cmd, args=None, retry=True):
        attempts = 2 if retry else 1
        for attempt in range(attempts):
            try:
                if self.f is None:
                    self.connect()
                self._send({"execute": cmd, **({"arguments": args} if args else {})})
                while True:
                    r = self._recv()
                    if "return" in r:
                        return r["return"]
                    if "error" in r:
                        raise QgaCommandError(f"{cmd}: {r['error'].get('desc', r['error'])}")
            except QgaCommandError:
                raise                              # the agent understood and refused: not a transport fault
            except QgaTimeout:
                self.close()
                raise
            except (QgaError, OSError) as e:       # transport fault: reconnect once
                self.close()
                if attempt + 1 >= attempts:
                    raise e if isinstance(e, QgaError) else QgaError(str(e))
        raise QgaError("unreachable")

    # ---- convenience
    def ping(self, timeout=5.0):
        old = self.io_timeout
        self.io_timeout = timeout
        try:
            if self.sock:
                self.sock.settimeout(timeout)
            self.call("guest-ping", retry=False)
            return True
        except QgaError:
            self.close()
            return False
        finally:
            self.io_timeout = old
            if self.sock:
                self.sock.settimeout(old)

    def exec_start(self, path, args=(), input_data=None, capture=True):
        a = {"path": path, "arg": list(args), "capture-output": capture}
        if input_data is not None:
            a["input-data"] = base64.b64encode(input_data).decode()
        return self.call("guest-exec", a)["pid"]

    def exec_status(self, pid):
        return self.call("guest-exec-status", {"pid": pid})

    def exec(self, path, args=(), timeout=60.0, poll=0.5, input_data=None):
        """Run a guest program to completion (bounded by `timeout`). On timeout the program keeps running in the
        guest (the agent has no kill) and the result has timed_out=True."""
        pid = self.exec_start(path, args, input_data)
        deadline = time.time() + timeout
        while True:
            try:
                st = self.exec_status(pid)
            except QgaTimeout:
                return ExecResult(None, b"", b"", True, pid)
            if st.get("exited"):
                out = base64.b64decode(st.get("out-data", "") or "")
                err = base64.b64decode(st.get("err-data", "") or "")
                return ExecResult(st.get("exitcode"), out, err, False, pid)
            if time.time() > deadline:
                return ExecResult(None, b"", b"", True, pid)
            time.sleep(poll)

    def powershell(self, script_path, args=(), timeout=60.0):
        """Run a script already present in the guest with the inbox Windows PowerShell."""
        return self.exec(r"C:\Windows\System32\WindowsPowerShell\v1.0\powershell.exe",
                         ["-NoProfile", "-NonInteractive", "-ExecutionPolicy", "Bypass", "-File", script_path, *args], timeout=timeout)

    def powershell_command(self, command, timeout=60.0):
        return self.exec(r"C:\Windows\System32\WindowsPowerShell\v1.0\powershell.exe",
                         ["-NoProfile", "-NonInteractive", "-ExecutionPolicy", "Bypass", "-Command", command], timeout=timeout)

    # ---- files
    def file_write(self, gpath, data, chunk=96 * 1024):
        h = self.call("guest-file-open", {"path": gpath, "mode": "wb"})
        try:
            for i in range(0, max(len(data), 1), chunk):
                self.call("guest-file-write", {"handle": h, "buf-b64": base64.b64encode(data[i:i + chunk]).decode()})
        finally:
            self.call("guest-file-close", {"handle": h})

    def file_read(self, gpath, head=1 << 20, tail=3 << 20):
        """Whole file if it is at most head+tail bytes, else its first `head` and last `tail` bytes joined by a marker."""
        h = self.call("guest-file-open", {"path": gpath, "mode": "rb"})
        marker = b"\n=== [kf: middle of the log elided] ===\n"
        try:
            buf = bytearray()
            eof = False
            while len(buf) <= head + tail and not eof:
                r = self.call("guest-file-read", {"handle": h, "count": 1 << 20})
                buf += base64.b64decode(r.get("buf-b64", "") or "")
                eof = bool(r.get("eof") or not r.get("count"))
            if eof:
                data = bytes(buf)
                return data if len(data) <= head + tail else data[:head] + marker + data[-tail:]
            first = bytes(buf[:head])
            try:
                self.call("guest-file-seek", {"handle": h, "offset": -tail, "whence": 2})
                t = bytearray()
                while True:
                    r = self.call("guest-file-read", {"handle": h, "count": 1 << 20})
                    t += base64.b64decode(r.get("buf-b64", "") or "")
                    if r.get("eof") or not r.get("count"):
                        break
                return first + marker + bytes(t)
            except QgaError:
                return first + b"\n=== [kf: log truncated, tail unreadable] ===\n"
        finally:
            self.call("guest-file-close", {"handle": h})

    def file_exists(self, gpath):
        try:
            h = self.call("guest-file-open", {"path": gpath, "mode": "rb"})
        except QgaCommandError:
            return False                       # the agent answered: no such file (transport faults propagate)
        self.call("guest-file-close", {"handle": h})
        return True


class Qmp:
    def __init__(self, path, timeout=15.0):
        self.path = path
        self.timeout = timeout

    def _open(self):
        s = socket.socket(socket.AF_UNIX, socket.SOCK_STREAM)
        s.settimeout(self.timeout)
        s.connect(self.path)
        f = s.makefile("rwb", buffering=0)
        json.loads(f.readline())                      # greeting
        s.sendall(b'{"execute":"qmp_capabilities"}\n')
        while True:
            r = json.loads(f.readline())
            if "return" in r:
                return s, f
            if "error" in r:
                raise QgaError(f"qmp_capabilities: {r['error']}")

    def cmd(self, name, args=None):
        s, f = self._open()
        try:
            s.sendall(json.dumps({"execute": name, **({"arguments": args} if args else {})}).encode() + b"\n")
            while True:
                line = f.readline()
                if not line:
                    raise QgaError("qmp socket closed")
                r = json.loads(line)
                if "event" in r:
                    continue
                if "error" in r:
                    raise QgaError(f"qmp {name}: {r['error'].get('desc', r['error'])}")
                return r.get("return")
        finally:
            s.close()

    def send_key(self, qcode):
        self.cmd("send-key", {"keys": [{"type": "qcode", "data": qcode}]})

    def screendump(self, filename, device="kf0"):
        self.cmd("screendump", {"filename": filename, "device": device})

    def attach_cdrom(self, iso, node="kfapps_node", bus_id="kfappsbot", dev_id="kfappscd", xhci_bus="xhci.0"):
        """Hot-plug the read-only app disk as a USB-attached SCSI CD-ROM (inbox Windows drivers; no virtio driver needed)."""
        self.cmd("blockdev-add", {"driver": "raw", "node-name": node, "read-only": True,
                                  "file": {"driver": "file", "filename": iso, "read-only": True}})
        try:
            self.cmd("device_add", {"driver": "usb-bot", "id": bus_id, "bus": xhci_bus})
            self.cmd("device_add", {"driver": "scsi-cd", "id": dev_id, "bus": bus_id + ".0", "drive": node})
            return "usb-bot+scsi-cd"
        except QgaError:
            try:
                self.cmd("device_del", {"id": bus_id})
            except QgaError:
                pass
            self.cmd("device_add", {"driver": "usb-storage", "id": bus_id, "bus": xhci_bus, "drive": node, "removable": True})
            return "usb-storage"

    def detach_cdrom(self, node="kfapps_node", bus_id="kfappsbot", dev_id="kfappscd"):
        for name, a in (("device_del", {"id": dev_id}), ("device_del", {"id": bus_id})):
            try:
                self.cmd(name, a)
            except QgaError:
                pass
        time.sleep(1)
        try:
            self.cmd("blockdev-del", {"node-name": node})
        except QgaError:
            pass


# ---- PPM -> PNG without ImageMagick (the screendump format is binary PPM, P6)
def ppm_to_png(ppm_path, png_path):
    import struct
    import zlib
    with open(ppm_path, "rb") as f:
        data = f.read()
    parts = data.split(None, 4)
    if parts[0] != b"P6":
        raise ValueError("not a P6 PPM")
    w, h, mx = int(parts[1]), int(parts[2]), int(parts[3])
    pix = data[len(data) - w * h * 3:]
    raw = b"".join(b"\x00" + pix[y * w * 3:(y + 1) * w * 3] for y in range(h))

    def chunk(t, d):
        c = struct.pack(">I", len(d)) + t + d
        return c + struct.pack(">I", zlib.crc32(t + d) & 0xffffffff)
    png = b"\x89PNG\r\n\x1a\n" + chunk(b"IHDR", struct.pack(">IIBBBBB", w, h, 8, 2, 0, 0, 0)) + chunk(b"IDAT", zlib.compress(raw, 6)) + chunk(b"IEND", b"")
    with open(png_path, "wb") as f:
        f.write(png)


def main(argv):
    if len(argv) < 3:
        print(__doc__)
        return 2
    q = Qga(argv[1])
    op = argv[2]
    if op == "ping":
        ok = q.ping()
        print("pong" if ok else "no answer")
        return 0 if ok else 1
    if op == "exec":
        r = q.exec(argv[3], argv[4:], timeout=float(os.environ.get("QGA_TIMEOUT", "60")))
        sys.stdout.write(r.text())
        sys.stderr.write(r.err.decode("utf-8", "replace"))
        return 124 if r.timed_out else (r.exitcode or 0)
    if op == "read":
        sys.stdout.buffer.write(q.file_read(argv[3]))
        return 0
    if op == "write":
        q.file_write(argv[3], open(argv[4], "rb").read())
        return 0
    print(__doc__)
    return 2


if __name__ == "__main__":
    sys.exit(main(sys.argv))
