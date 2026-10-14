#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later
"""mock_guest.py -- a mock Windows guest behind a real QGA unix socket, for the GPU-free tests and for
`run_windows_apps.sh --dry-run`. It speaks the guest-agent protocol (guest-sync/ping/exec/exec-status/file-*)
and EMULATES what the guest scripts of guest/ print and write (KFSETUP, KFSTAGE, KFLAUNCH, KFHEALTH lines,
C:\\kf\\results\\ID.json facts, logs), so winapps.py's polling, verdicts, TDR accounting, reboot detection,
wedge recovery and fresh-guest restarts are exercised end to end without Windows or a GPU. It does NOT run
any PowerShell: the PowerShell itself is only parse-checked (tests/test_guest_scripts.py) and is untested on a
real guest.

Scenario (JSON, per app id; default "pass"):
  "pass" | "fail" | "timeout" | "nogpu" (passes its string, no GPU proof) | "warp" (engine activity on a non-NVIDIA adapter)
  {"kind": "tdr", "n": 2}         passes, with n nvlddmkm 153 events during the app
  {"kind": "reboot"}              the guest bugchecks and reboots mid-app (agent drops, new boot time, NVIDIA in Code 43)
  {"kind": "wedge"}               the agent stops answering for good
  {"kind": "hang"}                the supervisor never writes a result but the guest stays healthy
  {"kind": "qemu_exit"}           the VM process dies
  world keys: "_stage_fail": [pkg,...] (kf_stage fails for those), "_boot_fail": N (the first N guests have no NVIDIA adapter)
Any of these may carry "once": true (only the first guest misbehaves; fresh guests pass).
"""
import base64
import json
import os
import re
import socket
import threading
import time


def sample_from_regex(rx):
    """A string that matches the (simple) regex `rx`; used to fabricate a success log. Not a general inverter."""
    s = rx
    if "|" in s and "(" not in s and "[" not in s.replace("[0-9]", ""):
        s = s.split("|")[0]
    s = re.sub(r"\(([^()|]*)\|[^()]*\)", r"\1", s)
    for a, b in (("\\d+", "1"), ("[0-9]+", "1"), ("\\s*", ""), ("\\s+", " "), (" *", " "), (".*", "x"), (".+", "x")):
        s = s.replace(a, b)
    s = re.sub(r"\\(.)", r"\1", s)
    return s.lstrip("^").rstrip("$")


class State:
    def __init__(self, world, boot_no):
        self.world, self.boot_no = world, boot_no
        self.fs = {}
        self.handles = {}
        self.next_handle = 1
        self.execs = {}
        self.next_pid = 100
        self.boot_utc = f"2026-10-10T10:{boot_no:02d}:00.0000000Z"
        self.pending = {}
        self.signed_in = False
        self.typed = []
        self.pw_set = None
        self.tdr = 0
        self.display_problem = "CM_PROB_NONE"
        self.wedged = False
        self.down_accepts = 0
        self.adapters_ok = True
        self.cd = "E:"
        self.staged = set()
        self.launch_log = []
        self.qemu_dead = False


class MockWorld:
    def __init__(self, apps_doc, scenario, tmpdir):
        self.apps = {a["id"]: a for a in apps_doc["apps"]}
        self.scenario = scenario
        self.tmp = tmpdir
        os.makedirs(tmpdir, exist_ok=True)
        self.boots = 0
        self.cycles = 0
        self.consumed_once = set()
        self.guests = []

    def new_vm(self):
        return MockVm(self)

    def behaviour(self, app_id):
        b = self.scenario.get(app_id, "pass")
        if isinstance(b, str):
            b = {"kind": b}
        b = dict(b)
        if b.get("once"):
            if app_id in self.consumed_once:
                return {"kind": "pass"}
            self.consumed_once.add(app_id)
        return b


class MockVm:
    def __init__(self, world):
        self.world = world
        self.state = None
        self.server = None
        self.sock = None
        self._alive = False
        self.attached = None
        self.shots = []

    def launch(self, tag):
        w = self.world
        w.boots += 1
        self.state = State(w, w.boots)
        if w.boots <= int(w.scenario.get("_boot_fail", 0)):
            self.state.adapters_ok = False                  # the first N guests come up with the NVIDIA device dead (Code 43)
        w.guests.append(self.state)
        self.sock = os.path.join(w.tmp, f"qga{w.boots}.sock")
        if os.path.exists(self.sock):
            os.unlink(self.sock)
        self.server = Server(self.sock, self.state)
        self.server.start()
        self._alive = True

    def qga_path(self):
        return self.sock

    def alive(self):
        return self._alive and self.server is not None and not self.state.qemu_dead

    def stop(self):
        if self.server:
            self.server.shutdown()
        self._alive = False
        return "mock-clean"

    def send_key(self, qcode, shift=False):
        st = self.state
        if qcode == "ret":
            st.signed_in = (st.pw_set is not None and "".join(st.typed) == st.pw_set)
            st.typed = []
        elif qcode != "spc":
            st.typed.append(qcode.upper() if shift else qcode)

    def screendump(self, png_path):
        os.makedirs(os.path.dirname(png_path), exist_ok=True)
        with open(png_path, "wb") as f:
            f.write(b"\x89PNG mock")
        self.shots.append(png_path)
        return True

    def attach_disk(self, iso):
        self.attached = iso
        return "mock-usb-cd"

    def detach_disk(self):
        self.attached = None

    def gsp_cycles(self):
        return self.world.cycles

    def run_dir(self):
        return self.world.tmp

    def kill_qemu(self):
        self._alive = False


class Server(threading.Thread):
    def __init__(self, path, state):
        super().__init__(daemon=True)
        self.path, self.st = path, state
        self.stop_flag = threading.Event()
        self.srv = socket.socket(socket.AF_UNIX, socket.SOCK_STREAM)
        self.srv.bind(path)
        self.srv.listen(4)
        self.srv.settimeout(0.1)

    def shutdown(self):
        self.stop_flag.set()
        try:
            self.srv.close()
        except OSError:
            pass
        self.join(timeout=2)

    def run(self):
        while not self.stop_flag.is_set():
            try:
                c, _ = self.srv.accept()
            except (socket.timeout, OSError):
                continue
            if self.st.down_accepts > 0:
                self.st.down_accepts -= 1
                c.close()
                continue
            try:
                self.serve(c)
            except (OSError, ValueError):
                pass
            finally:
                try:
                    c.close()
                except OSError:
                    pass

    def serve(self, c):
        c.settimeout(0.2)
        buf = b""
        while not self.stop_flag.is_set():
            try:
                chunk = c.recv(1 << 16)
            except (socket.timeout, TimeoutError):
                continue
            if not chunk:
                return
            buf += chunk
            while b"\n" in buf:
                line, buf = buf.split(b"\n", 1)
                line = line.lstrip(b"\xff").strip()
                if not line:
                    continue
                req = json.loads(line)
                if self.st.wedged and req.get("execute") != "guest-sync":
                    continue                                    # a wedged guest reads and never answers
                try:
                    ret = self.handle(req["execute"], req.get("arguments") or {})
                    reply = {"return": ret}
                except Err as e:
                    reply = {"error": {"class": "GenericError", "desc": str(e)}}
                if self.st.down_accepts > 0 and req["execute"] != "guest-sync":
                    return                                      # the guest just rebooted: drop the connection
                c.sendall(json.dumps(reply).encode() + b"\n")

    # ---------------------------------------------------------------- commands
    def handle(self, cmd, a):
        st = self.st
        if cmd == "guest-sync":
            return a["id"]
        if cmd == "guest-ping":
            return {}
        if cmd == "guest-exec":
            pid = st.next_pid
            st.next_pid += 1
            out = self.run_exec(a["path"], a.get("arg") or [])
            st.execs[pid] = out
            return {"pid": pid}
        if cmd == "guest-exec-status":
            out = st.execs.get(a["pid"])
            if out is None:
                raise Err("unknown pid")
            return {"exited": True, "exitcode": out[0], "out-data": base64.b64encode(out[1].encode()).decode(), "err-data": ""}
        if cmd == "guest-file-open":
            path, mode = a["path"], a["mode"]
            if "r" in mode:
                self.tick(path)
                if path not in st.fs:
                    raise Err(f"No such file: {path}")
            else:
                st.fs[path] = b""
            h = st.next_handle
            st.next_handle += 1
            st.handles[h] = [path, 0]
            return h
        if cmd == "guest-file-write":
            p = st.handles[a["handle"]][0]
            st.fs[p] += base64.b64decode(a["buf-b64"])
            return {"count": len(a["buf-b64"]), "eof": False}
        if cmd == "guest-file-read":
            p, off = st.handles[a["handle"]]
            data = st.fs[p][off:off + a["count"]]
            st.handles[a["handle"]][1] = off + len(data)
            return {"count": len(data), "buf-b64": base64.b64encode(data).decode(), "eof": off + len(data) >= len(st.fs[p])}
        if cmd == "guest-file-seek":
            h = st.handles[a["handle"]]
            n = len(st.fs[h[0]])
            h[1] = max(0, n + a["offset"]) if a["whence"] == 2 else a["offset"]
            return {"position": h[1], "eof": False}
        if cmd == "guest-file-close":
            st.handles.pop(a["handle"], None)
            return {}
        raise Err(f"unsupported command {cmd}")

    def run_exec(self, path, args):
        st = self.st
        base = path.replace("\\", "/").split("/")[-1].lower()
        if base == "net.exe" and args[:1] == ["user"]:
            st.pw_set = args[2]
            return (0, "The command completed successfully.\n")
        if base != "powershell.exe":
            return (0, "")
        if "-File" in args:
            script = args[args.index("-File") + 1].replace("\\", "/").split("/")[-1].lower()
            rest = args[args.index("-File") + 2:]
            fn = getattr(self, "ps_" + script.replace(".ps1", ""), None)
            if fn is None:
                return (0, "")
            return fn(rest)
        cmdline = " ".join(args)
        if "Win32_ComputerSystem" in cmdline:
            return (0, ("MOCK\\vast\nTrue\n" if st.signed_in else "\nFalse\n"))
        return (0, "")

    @staticmethod
    def opt(rest, name, default=""):
        return rest[rest.index(name) + 1] if name in rest else default

    # ---------------------------------------------------------------- the guest scripts, emulated
    def ps_kf_guest_setup(self, rest):
        st = self.st
        ok = st.adapters_ok and st.display_problem == "CM_PROB_NONE"
        info = dict(ok=ok, cd=st.cd, image_identity=self.opt(rest, "-ExpectedIdentity"), notes=[], nvidia_adapters=1 if ok else 0,
                    smi="NVIDIA GeForce RTX 4070, 595.91, 12282 MiB", os="mock", boot_utc=st.boot_utc, user="MOCK\\vast", explorer=True)
        return (0 if ok else 3, "KFSETUP " + json.dumps(info) + "\n")

    def ps_kf_stage(self, rest):
        pkg = self.opt(rest, "-Pkg")
        if pkg in (self.st.world.scenario.get("_stage_fail") or []):
            return (4, "KFSTAGE " + json.dumps(dict(pkg=pkg, ok=False, detail="mock stage failure")) + "\n")
        self.st.staged.add(pkg)
        return (0, "KFSTAGE " + json.dumps(dict(pkg=pkg, ok=True, mode="mock", secs=0, detail="mock")) + "\n")

    def ps_kf_health(self, rest):
        st = self.st
        h = dict(utc="2026-10-10T10:59:00.0000000Z", boot_utc=st.boot_utc, uptime_s=100,
                 display=[dict(name="NVIDIA GeForce RTX 4070", status="OK" if st.display_problem == "CM_PROB_NONE" else "Error", problem=st.display_problem)],
                 smi_ok=st.display_problem == "CM_PROB_NONE", smi="NVIDIA GeForce RTX 4070, 0, 400", user="MOCK\\vast" if st.signed_in else "", explorer=st.signed_in)
        if "-Since" in rest:
            h["events"] = dict(nvlddmkm_153=st.tdr, display_4101=0, wer_1001=0, kernel_power_41=0)
        return (0, "KFHEALTH " + json.dumps(h) + "\n")

    def ps_kf_cleanup(self, rest):
        return (0, "KFCLEAN killed=0\n")

    def ps_kf_launch(self, rest):
        st = self.st
        aid, mode = self.opt(rest, "-Id"), self.opt(rest, "-Mode", "service")
        if mode == "interactive" and not st.signed_in:
            return (3, "KFLAUNCH fail no-interactive-user\n")
        spec = json.loads(st.fs[rf"C:\kf\apps\{aid}.json"].decode())
        assert rf"C:\kf\apps\{aid}.ps1" in st.fs, "app script was not pushed before launch"
        beh = st.world.behaviour(aid)
        st.pending[aid] = dict(beh=beh, polls=beh.get("polls", 2), token=spec["token"], spec=spec)
        st.fs[rf"C:\kf\results\{aid}.start"] = json.dumps(dict(app=aid, boot_utc=st.boot_utc)).encode()
        st.launch_log.append((aid, mode))
        return (0, f"KFLAUNCH ok mode={mode} user=MOCK\\vast\n")

    # ---------------------------------------------------------------- app results appear after a few polls
    def tick(self, path):
        m = re.match(r"C:\\kf\\results\\(.+)\.json$", path)
        if not m:
            return
        aid = m.group(1)
        p = self.st.pending.get(aid)
        if not p:
            return
        kind = p["beh"].get("kind", "pass")
        if kind == "qemu_exit":
            self.st.qemu_dead = True
            raise OSError("qemu died")
        if kind == "wedge":
            self.st.wedged = True
            return
        if kind == "hang":
            return
        if kind == "reboot":
            st = self.st
            st.boot_no += 100
            st.boot_utc = f"2026-10-10T11:{st.boot_no % 60:02d}:00.0000000Z"
            st.pending.clear()
            st.down_accepts = 2
            st.display_problem = "CM_PROB_FAILED_START"
            st.signed_in = False
            raise OSError("reboot")
        p["polls"] -= 1
        if p["polls"] > 0:
            return
        self.materialize(aid, p)
        del self.st.pending[aid]

    def materialize(self, aid, p):
        st = self.st
        app = st.world.apps[aid]
        kind = p["beh"].get("kind", "pass")
        rc, timed_out, tdr = 0, False, 0
        lines = [f"=== {aid} start=mock"]
        gpu_log = kind not in ("nogpu",)
        if kind in ("pass", "tdr", "nogpu", "warp"):
            lines.append(sample_from_regex(app["rx"]))
        elif kind == "fail":
            rc = 1
            lines.append("ERROR: mock failure")
        elif kind == "timeout":
            rc, timed_out = 124, True
            lines.append("still running")
        engines = {}
        if gpu_log and kind != "fail":
            for pr in app.get("proof") or []:
                k, _, arg = pr.partition(":")
                if k == "out":
                    lines.append(sample_from_regex(arg))
                elif k == "pdh":
                    engines[arg.split("|")[0]] = 17.0
        if kind == "tdr":
            tdr = int(p["beh"].get("n", 1))
            st.tdr += tdr
            st.world.cycles += tdr
        lines.append(f"=== end rc={rc} secs=1 mock")
        facts = dict(app=aid, token=p["token"], rc=rc, secs=1, quiet=0, timed_out=timed_out, boot_utc_start=st.boot_utc, boot_utc_end=st.boot_utc,
                     pdh=dict(nv=engines, other=({"3D": 30.0} if kind == "warp" else {}), samples=3, pids=[1], nvidia_luids=["0_1"]),
                     smi=dict(util_max=20 if gpu_log else 0, mem_used_max_mb=900 if gpu_log else 400, mem_used_base_mb=400, samples=2),
                     events=dict(nvlddmkm_153=tdr, display_4101=0, wer_1001=0, kernel_power_41=0, lines=[]), log_tail=lines[-5:], digests=[])
        st.fs[rf"C:\kf\logs\{aid}.log"] = ("\n".join(lines) + "\n").encode()
        st.fs[rf"C:\kf\results\{aid}.json"] = json.dumps(facts).encode()


class Err(Exception):
    pass
