#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later
"""winapps.py -- the Windows app matrix driver (the part of run_windows_apps.sh that talks to a guest).

For each guest ("session"): boot it through a Vm controller, wait for the guest agent, sign in (keystrokes only as
the bootstrap), hot-plug the read-only app disk, push the guest scripts by QGA, run kf_guest_setup.ps1, then run
the apps one after another. Every app is a DETACHED supervisor task in the guest (kf_launch.ps1 -> kf_run_app.ps1)
that the driver polls, so a hung or TDR-stalled app can never hold the agent. After each app the driver takes the
verdict (verdict.py), checks guest health and decides whether the guest may go on: a bugcheck/reboot, a wedged
agent, an unhealthy display adapter, or more than --max-tdr TDR events in this guest end the session and the
remaining apps continue in a FRESH guest. Like apps_matrix.sh: phase 1 batches --per-guest apps per guest, phase 2
re-runs every non-PASS app alone in its own fresh guest.

Result files (run dir): win.res / win_isolated.res (`APPRES ...` + `APPDIG ...`, the Linux format plus guest_tdr=,
wer=, gsp_cycles=, proof=), apps/<id>.{json,log,verdict.json}, shots/*.png, guests/<tag>/*, session.log, matrix.json.
"""
import argparse
import base64
import datetime
import json
import os
import re
import shlex
import shutil
import subprocess
import sys
import time

HERE = os.path.dirname(os.path.abspath(__file__))
sys.path.insert(0, HERE)
import appdisk  # noqa: E402
import qga as qgamod  # noqa: E402
import verdict as V  # noqa: E402

REPO = os.path.abspath(os.path.join(HERE, "..", "..", "..", ".."))
PS_PUSH = {  # guest path -> repo path (relative to HERE unless absolute)
    r"C:\kf\kf_common.ps1": "guest/kf_common.ps1", r"C:\kf\kf_apphelpers.ps1": "guest/kf_apphelpers.ps1",
    r"C:\kf\kf_guest_setup.ps1": "guest/kf_guest_setup.ps1", r"C:\kf\kf_stage.ps1": "guest/kf_stage.ps1",
    r"C:\kf\kf_run_app.ps1": "guest/kf_run_app.ps1", r"C:\kf\kf_launch.ps1": "guest/kf_launch.ps1",
    r"C:\kf\kf_health.ps1": "guest/kf_health.ps1", r"C:\kf\kf_cleanup.ps1": "guest/kf_cleanup.ps1", r"C:\kf\kf_edge.ps1": "guest/kf_edge.ps1",
    r"C:\kf\d3d12_signal_probe.ps1": "../d3d12_signal_probe.ps1",
}
WEB = ["webgl", "webgpu", "video"]
PY_OWN = ["win_cuda_equiv.py", "torch_burn.py", "ort_dml.py", "cupy_nvrtc.py", "kf_runpy.py"]
PY_LINUX = ["torch_correct.py", "ai_bench.py", "cupy_check.py", "blender_render.py"]     # the unmodified Linux-row scripts
PS = r"C:\Windows\System32\WindowsPowerShell\v1.0\powershell.exe"


def utc():
    return datetime.datetime.now(datetime.timezone.utc).strftime("%Y-%m-%dT%H:%M:%SZ")


class Log:
    def __init__(self, path, echo=True):
        self.path, self.echo = path, echo
        os.makedirs(os.path.dirname(path), exist_ok=True)

    def __call__(self, *a):
        line = f"{utc()} " + " ".join(str(x) for x in a)
        with open(self.path, "a") as f:
            f.write(line + "\n")
        if self.echo:
            print(line, flush=True)


# ------------------------------------------------------------------------------------------ pure helpers (unit-tested)
def render_app_script(prelude, app, cd):
    body = prelude + "\n" + app["ps"]
    body = body.replace("@@ID@@", app["id"]).replace("@@CD@@", cd)
    body.encode("ascii")                       # PS 5.1 reads a BOM-less file as ANSI: keep scripts ASCII
    return body.replace("\r\n", "\n").replace("\n", "\r\n")


def app_spec(app, script_path, token, sample_s=2):
    return dict(id=app["id"], script=script_path, timeout_s=app["timeout_s"], sample_s=sample_s, smi=bool(app.get("smi", True)),
                kill_names=app.get("kill_names", []), token=token)


def closure(pkgs, table_deps):
    out, seen = [], set()

    def visit(p):
        if p in seen:
            return
        seen.add(p)
        for d in table_deps.get(p, []):
            visit(d)
        out.append(p)
    for p in pkgs:
        visit(p)
    return out


def select_apps(apps, names, tier, categories, exclude):
    sel = []
    for a in apps:
        if names and names != ["all"] and a["id"] not in names:
            continue
        if a["tier"] > tier and not (names and names != ["all"]):
            continue
        if categories and a["category"] not in categories:
            continue
        if a["id"] in exclude:
            continue
        sel.append(a)
    if names and names != ["all"]:
        missing = [n for n in names if n not in {a["id"] for a in apps}]
        if missing:
            raise SystemExit(f"unknown apps: {missing}")
        sel.sort(key=lambda a: names.index(a["id"]))        # an explicit list runs in the order given
    return sel


QCODE = {**{c: c for c in "abcdefghijklmnopqrstuvwxyz0123456789"}, " ": "spc", "\n": "ret", "-": "minus", ".": "dot", "/": "slash", "=": "equal", ",": "comma", ";": "semicolon"}


def keys_for(text):
    out = []
    for ch in text:
        if ch.isupper():
            out.append(("shift", ch.lower()))
        elif ch in QCODE:
            out.append((None, QCODE[ch]))
        else:
            raise ValueError(f"cannot type {ch!r} (extend QCODE)")
    return out


def parse_marker(text, marker):
    for line in text.splitlines():
        if line.startswith(marker + " "):
            try:
                return json.loads(line[len(marker) + 1:])
            except ValueError:
                return None
    return None


# ------------------------------------------------------------------------------------------ VM control
class Vm:
    """What the driver needs from a VM launcher. BrokerVm drives windows_broker*.sh; tests use MockVm."""
    def launch(self, tag): raise NotImplementedError
    def qga_path(self): raise NotImplementedError
    def alive(self): raise NotImplementedError
    def stop(self): raise NotImplementedError
    def send_key(self, qcode, shift=False): raise NotImplementedError
    def screendump(self, png_path): raise NotImplementedError
    def attach_disk(self, iso): raise NotImplementedError
    def detach_disk(self): raise NotImplementedError
    def gsp_cycles(self): return None
    def run_dir(self): return None


class BrokerVm(Vm):
    """windows_broker_prod2.sh / windows_broker.sh `run N` (the launcher of the Windows production runs)."""
    def __init__(self, broker, w_dir, kf3_rev, run_base, log, env_extra=None, pre_cmd="", post_cmd=""):
        self.broker, self.w, self.rev, self.base, self.log = broker, w_dir, kf3_rev, run_base, log
        self.pre_cmd, self.post_cmd = pre_cmd, post_cmd
        self.n = None
        self.env_extra = env_extra or {}
        self.qpid = None
        self.count = 0

    def run_dir(self):
        return os.path.join(self.w, f"boundary-kayfabe-{self.n}")

    def launch(self, tag):
        self.n = self.base + self.count
        self.count += 1
        if self.pre_cmd:                       # host-specific, e.g. the 1.20 host's IOMMU group -> identity (tdr-run.sh does this per run)
            r = subprocess.run(["bash", "-c", self.pre_cmd], capture_output=True, text=True, timeout=300)
            self.log(f"pre-guest hook rc={r.returncode} {r.stdout.strip()[-120:]}")
            if r.returncode != 0:
                raise RuntimeError(f"pre-guest hook failed rc={r.returncode}")
        env = dict(os.environ, KF3_REV=self.rev, WIN_REUSE="0", **self.env_extra)
        r = subprocess.run(["bash", self.broker, "run", str(self.n)], env=env, capture_output=True, text=True, timeout=300)
        self.log(f"broker run {self.n} rc={r.returncode} {r.stdout.strip().splitlines()[-1:] or ''}")
        if r.returncode != 0:
            raise RuntimeError(f"broker run failed rc={r.returncode}: {r.stderr[-300:] or r.stdout[-300:]}")
        st = os.path.join(self.w, f"windows-broker-run{self.n}.state")
        for _ in range(20):
            if os.path.exists(st):
                break
            time.sleep(0.5)
        m = re.search(r"^QPID=(\d+)", open(st).read(), re.M) if os.path.exists(st) else None
        self.qpid = int(m.group(1)) if m else None

    def qga_path(self):
        return os.path.join(self.run_dir(), "qga.sock")

    def _qmp(self):
        return qgamod.Qmp(os.path.join(self.run_dir(), "qmp.sock"))

    def alive(self):
        if self.qpid is None:
            return False
        try:
            os.kill(self.qpid, 0)
            return True
        except OSError:
            return False

    def stop(self):
        how = "none"
        if self.n is None:
            return how
        r = subprocess.run(["bash", self.broker, "stop", str(self.n)], capture_output=True, text=True, timeout=600)
        how = "clean" if r.returncode == 0 else f"broker-stop-rc{r.returncode}"
        if self.alive():
            try:
                self._qmp().cmd("quit")
                how += "+qmp-quit"
            except qgamod.QgaError:
                pass
            for _ in range(30):
                if not self.alive():
                    break
                time.sleep(1)
        if self.post_cmd:                      # always runs, also after a failed stop (restores what pre_cmd changed)
            r = subprocess.run(["bash", "-c", self.post_cmd], capture_output=True, text=True, timeout=300)
            self.log(f"post-guest hook rc={r.returncode} {r.stdout.strip()[-120:]}")
        return how

    def send_key(self, qcode, shift=False):
        keys = ([{"type": "qcode", "data": "shift"}] if shift else []) + [{"type": "qcode", "data": qcode}]
        self._qmp().cmd("send-key", {"keys": keys})

    def screendump(self, png_path):
        ppm = png_path + ".ppm"
        try:
            self._qmp().screendump(ppm)
            qgamod.ppm_to_png(ppm, png_path)
            return True
        except Exception:
            return False
        finally:
            try:
                os.unlink(ppm)
            except OSError:
                pass

    def attach_disk(self, iso):
        return self._qmp().attach_cdrom(iso)

    def detach_disk(self):
        self._qmp().detach_cdrom()

    def gsp_cycles(self):
        p = os.path.join(self.run_dir(), "qemu.log")
        if not os.path.exists(p):
            return None
        r = subprocess.run(["grep", "-a", "-c", "GSP phase Running -> Suspending", p], capture_output=True, text=True)
        try:
            return int(r.stdout.strip() or 0)
        except ValueError:
            return None


# ------------------------------------------------------------------------------------------ the guest session
class Cfg:
    def __init__(self, **kw):
        self.__dict__.update(dict(
            poll_s=3.0, slack_s=90, boot_timeout=900, signin_delay=30, session_timeout=300, max_tdr=3, per_guest=8, recover_wait=180,
            stage_timeout=1800, user="vast", password=None, iso=None, image_identity=None, tier=1, screenshots=True, sample_s=2,
            max_guests=200, max_boot_failures=3, io_timeout=10.0, ping_s=5.0, time_scale=1.0), **kw)


class GuestSession:
    def __init__(self, cfg, vm, apps_doc, manifest, run_dir, log, tag, sleep=time.sleep, now=time.time):
        self.cfg, self.vm, self.doc, self.man, self.run_dir, self.log, self.tag = cfg, vm, apps_doc, manifest, run_dir, log, tag
        self.sleep, self.now = sleep, now
        self.q = None
        self.cd = None
        self.boot_utc = None
        self.start_utc = None
        self.tdr_total = 0
        self.staged = set()
        self.dir = os.path.join(run_dir, "guests", tag)
        os.makedirs(self.dir, exist_ok=True)
        self.deps = {p["id"]: p.get("needs", []) for p in manifest["packages"]}
        self.stage_json = appdisk.stage_table(manifest)
        self.pkg_by_id = {p["id"]: p for p in manifest["packages"]}

    # ---- boot / sign-in / setup
    def start(self):
        c = self.cfg
        t0 = self.now()
        self.vm.launch(self.tag)
        self.log(f"[{self.tag}] launched; waiting for the guest agent (<= {c.boot_timeout}s)")
        self.q = qgamod.Qga(self.vm.qga_path(), io_timeout=c.io_timeout)
        ta = None
        while self.now() - t0 < c.boot_timeout:
            if not self.vm.alive():
                raise GuestFailure("qemu exited before the guest agent answered")
            if self.q.ping(self.cfg.ping_s):
                ta = self.now()
                break
            self.sleep(2)
        if ta is None:
            raise GuestFailure(f"no guest agent within {c.boot_timeout}s")
        self.log(f"[{self.tag}] agent answered after {int(ta - t0)}s")
        if not self._signin(ta):
            self._wait_session()
        self._push()
        how = self.vm.attach_disk(c.iso)
        self.log(f"[{self.tag}] app disk attached ({how})")
        self._setup()
        h = self.health()
        self.boot_utc = h.get("boot_utc")
        self.start_utc = h.get("utc")
        self.log(f"[{self.tag}] READY boot={self.boot_utc} user={h.get('user')} smi_ok={h.get('smi_ok')}")

    def _signin(self, ta):
        c = self.cfg
        if not c.password:
            self.log(f"[{self.tag}] no KF_GUEST_PW: assuming autologon")
            return False
        r = self.q.exec(r"C:\Windows\System32\net.exe", ["user", c.user, c.password], timeout=60)
        self.log(f"[{self.tag}] password set for {c.user} rc={r.exitcode}")
        while self.now() - ta < c.signin_delay:
            self.sleep(1)
        for attempt in (1, 2, 3):
            self.vm.screendump(os.path.join(self.dir, f"pre-signin-{attempt}.png"))
            self.log(f"[{self.tag}] sign-in keystrokes (attempt {attempt}) as {c.user}")
            self.vm.send_key("spc")
            self.sleep(3)
            for shift, qc in keys_for(c.password):
                self.vm.send_key(qc, shift=bool(shift))
                self.sleep(0.25)
            self.vm.send_key("ret")
            if self._wait_session(soft=True, timeout=min(120, c.session_timeout)):
                return True
        return False                                   # start() calls _wait_session(), which raises with the screenshot taken

    def _wait_session(self, soft=False, timeout=None):
        t0 = self.now()
        timeout = timeout or self.cfg.session_timeout
        while self.now() - t0 < timeout:
            try:
                r = self.q.exec(PS, ["-NoProfile", "-Command", "(Get-CimInstance Win32_ComputerSystem).UserName; [bool](Get-Process explorer -ErrorAction SilentlyContinue)"], timeout=60)
                out = r.text().split()
                if len(out) >= 2 and out[-1] == "True" and "\\" in out[0]:
                    self.log(f"[{self.tag}] interactive session: {out[0]}")
                    return True
            except qgamod.QgaError:
                pass
            self.sleep(3)
        if soft:
            return False
        self.vm.screendump(os.path.join(self.dir, "no-session.png"))
        raise GuestFailure("no interactive session after sign-in")

    def _push(self):
        q = self.q
        q.exec(PS, ["-NoProfile", "-Command", "New-Item -ItemType Directory -Force -Path C:\\kf\\py,C:\\kf\\web,C:\\kf\\apps,C:\\kf\\results,C:\\kf\\logs | Out-Null"], timeout=60)
        n = 0
        for g, rel in PS_PUSH.items():
            q.file_write(g, open(os.path.join(HERE, rel), "rb").read()); n += 1
        for w in WEB:
            q.file_write(rf"C:\kf\web\{w}.html", open(os.path.join(HERE, "guest", "web", f"{w}.html"), "rb").read()); n += 1
        for p in PY_OWN:
            q.file_write(rf"C:\kf\py\{p}", open(os.path.join(HERE, "guest", "py", p), "rb").read()); n += 1
        for p in PY_LINUX:
            q.file_write(rf"C:\kf\py\{p}", open(os.path.join(REPO, "scripts", "apps", "src", p), "rb").read()); n += 1
        q.file_write(r"C:\kf\stage.json", json.dumps(self.stage_json).encode()); n += 1
        self.log(f"[{self.tag}] pushed {n} guest files")

    def _setup(self):
        for attempt in (1, 2, 3):
            r = self.q.powershell(r"C:\kf\kf_guest_setup.ps1", ["-ExpectedIdentity", self.cfg.image_identity or ""], timeout=300)
            info = parse_marker(r.text(), "KFSETUP") or {}
            with open(os.path.join(self.dir, "setup.json"), "w") as f:
                json.dump(info, f, indent=1)
            if info.get("ok"):
                self.cd = info["cd"]
                if info.get("notes"):
                    self.log(f"[{self.tag}] setup notes: {info['notes']}")
                self.log(f"[{self.tag}] setup ok: cd={self.cd} nvidia_adapters={info.get('nvidia_adapters')} {info.get('smi', '')}")
                self._stage(["vc_redist"])
                return
            self.log(f"[{self.tag}] setup attempt {attempt} not ok: cd={info.get('cd')} nvidia_adapters={info.get('nvidia_adapters')} notes={info.get('notes')}")
            self.sleep(20)
        raise GuestFailure("guest setup failed (app disk missing or no NVIDIA adapter)")

    # ---- packages
    def _stage(self, pkgs):
        for p in closure(pkgs, self.deps):
            if p in self.staged:
                continue
            r = self.q.powershell(r"C:\kf\kf_stage.ps1", ["-Pkg", p], timeout=self.cfg.stage_timeout)
            info = parse_marker(r.text(), "KFSTAGE") or {}
            if r.timed_out or not info.get("ok"):
                raise StageFailure(p, info.get("detail") or ("stage timeout" if r.timed_out else "no KFSTAGE line"))
            self.staged.add(p)
            self.log(f"[{self.tag}] staged {p} ({info.get('mode')}, {info.get('secs')}s)")

    # ---- health
    def health(self, since=None):
        args = ["-Since", since] if since else []
        try:
            r = self.q.powershell(r"C:\kf\kf_health.ps1", args, timeout=90)
        except qgamod.QgaError:
            return {"unreachable": True}
        if r.timed_out:
            return {"unreachable": True, "timed_out": True}
        return parse_marker(r.text(), "KFHEALTH") or {"unreachable": True, "unparsed": True}

    @staticmethod
    def health_ok(h):
        if h.get("unreachable"):
            return False, "agent-unreachable"
        for d in h.get("display") or []:
            if d.get("problem") not in (None, "", "CM_PROB_NONE", "0"):
                return False, f"display-problem:{d.get('problem')}"
        if not h.get("display"):
            return False, "no-nvidia-display-device"
        if not h.get("smi_ok"):
            return False, "nvidia-smi-failed"
        return True, ""

    # ---- one app
    def run_app(self, app, tag=None):
        c = self.cfg
        aid = app["id"]
        token = f"{self.tag}-{aid}-{int(self.now())}"
        adir = os.path.join(self.run_dir, "apps")
        os.makedirs(adir, exist_ok=True)
        facts, log_text, need_restart = None, "", ""
        g0 = self.vm.gsp_cycles()
        t0 = self.now()
        try:
            self._stage(list(app.get("pkgs") or []))
        except StageFailure as e:
            d = V.decide(app, {"notrun": f"stage-failed:{e.pkg}:{e.detail}"[:120]}, "")
            return self._finish(app, d, "", {}, g0, need_restart="")
        script = render_app_script(self.doc["prelude"], app, self.cd)
        gscript = rf"C:\kf\apps\{aid}.ps1"
        self.q.file_write(gscript, script.encode("ascii"))
        self.q.file_write(rf"C:\kf\apps\{aid}.json", json.dumps(app_spec(app, gscript, token, c.sample_s)).encode())
        lr = self.q.powershell(r"C:\kf\kf_launch.ps1", ["-Id", aid, "-Mode", app["session"]], timeout=120)
        lt = lr.text()
        if "KFLAUNCH ok" not in lt:
            why = (re.search(r"KFLAUNCH fail (.*)", lt) or [None, lt.strip()[:80] or "launch failed"])[1]
            d = V.decide(app, {"notrun": f"launch:{why}"[:100]}, "")
            return self._finish(app, d, "", {}, g0, need_restart="")
        start_wall = self.now()
        deadline = start_wall + app["timeout_s"] * c.time_scale + c.slack_s
        shot_at = (start_wall + app["screenshot_s"]) if (app.get("screenshot_s") and c.screenshots) else None
        shots = []
        miss = 0
        miss_since = None
        had_gap = False
        while True:
            now = self.now()
            if shot_at and now >= shot_at:
                p = os.path.join(self.run_dir, "shots", f"{aid}-mid.png")
                os.makedirs(os.path.dirname(p), exist_ok=True)
                if self.vm.screendump(p):
                    shots.append(p)
                shot_at = None
            if not self.vm.alive():
                need_restart = "qemu-exited"
                break
            try:
                if self.q.file_exists(rf"C:\kf\results\{aid}.json"):
                    raw = self.q.file_read(rf"C:\kf\results\{aid}.json")
                    f = json.loads(raw.decode("utf-8-sig"))
                    if f.get("token") == token or f.get("token") is None:
                        facts = f
                        break
                if miss:
                    had_gap = True
                    h = self.health()                       # the agent was away and is back: did the guest reboot meanwhile?
                    if h.get("boot_utc") and self.boot_utc and h["boot_utc"] != self.boot_utc:
                        need_restart = "guest-rebooted"
                        break
                miss = 0
                miss_since = None
            except (qgamod.QgaError, ValueError):
                miss += 1
                miss_since = miss_since or now
                if now - miss_since > c.recover_wait:
                    need_restart = "agent-lost"
                    break
            if now > deadline:
                need_restart = "supervisor-deadline"
                break
            self.sleep(c.poll_s)
        # ---- collect
        if facts is not None:
            try:
                log_text = self.q.file_read(rf"C:\kf\logs\{aid}.log").decode("utf-8-sig", "replace")
            except qgamod.QgaError:
                log_text = "\n".join(facts.get("log_tail") or [])
            if facts.get("boot_utc_end") and self.boot_utc and facts["boot_utc_end"] != self.boot_utc:
                facts["rebooted"] = True
                need_restart = need_restart or "guest-rebooted"
        else:
            facts = self._dead_facts(app, need_restart, t0)
            if need_restart != "qemu-exited":
                need_restart = need_restart or "supervisor-lost"
            self._shot(f"{aid}-end.png")
        d = V.decide(app, facts, log_text)
        if d["verdict"] != "PASS" and facts.get("token") is not None and need_restart != "qemu-exited":
            self._shot(f"{aid}-end.png")
        self.tdr_total += int(d.get("tdr", 0))
        if not need_restart:
            need_restart = self._after_app(aid)
        return self._finish(app, d, log_text, facts, g0, need_restart=need_restart)

    def _shot(self, name):
        if not self.cfg.screenshots:
            return
        p = os.path.join(self.run_dir, "shots", name)
        os.makedirs(os.path.dirname(p), exist_ok=True)
        self.vm.screendump(p)

    def _dead_facts(self, app, why, t0):
        """No result file: the supervisor died with the guest, the agent is gone, or the app outlived its deadline."""
        h = self.health()
        crash = f"{why}"
        if not h.get("unreachable") and h.get("boot_utc") and self.boot_utc and h["boot_utc"] != self.boot_utc:
            crash = "guest-rebooted (bugcheck?)"
        elif h.get("unreachable"):
            crash = f"{why}: guest unresponsive"
        facts = dict(crash=crash, rc=None, secs=int(self.now() - t0), rebooted="reboot" in crash)
        if why == "supervisor-deadline" and not h.get("unreachable") and not facts["rebooted"]:
            facts = dict(timed_out=True, rc=124, secs=int(self.now() - t0))
        return facts

    def _after_app(self, aid):
        try:
            self.q.powershell(r"C:\kf\kf_cleanup.ps1", ["-Id", aid], timeout=90)
        except qgamod.QgaError:
            pass
        h = self.health(since=self.start_utc)
        ok, why = self.health_ok(h)
        ev = h.get("events") or {}
        guest_tdr = int(ev.get("nvlddmkm_153", 0)) + int(ev.get("display_4101", 0))
        self.tdr_total = max(self.tdr_total, guest_tdr)
        with open(os.path.join(self.dir, f"health-after-{aid}.json"), "w") as f:
            json.dump(h, f)
        if not ok:
            if why == "agent-unreachable":
                t0 = self.now()
                while self.now() - t0 < self.cfg.recover_wait:
                    if self.q.ping(self.cfg.ping_s):
                        h2 = self.health()
                        ok2, why2 = self.health_ok(h2)
                        if ok2 and h2.get("boot_utc") == self.boot_utc:
                            return ""
                        break
                    self.sleep(3)
            return f"unhealthy:{why}"
        if h.get("boot_utc") and self.boot_utc and h["boot_utc"] != self.boot_utc:
            return "guest-rebooted"
        if self.tdr_total >= self.cfg.max_tdr:
            return f"tdr-budget:{self.tdr_total}>={self.cfg.max_tdr}"
        return ""

    def _finish(self, app, d, log_text, facts, g0, need_restart):
        aid = app["id"]
        g1 = self.vm.gsp_cycles()
        cyc = (g1 - g0) if (g0 is not None and g1 is not None) else "-"
        adir = os.path.join(self.run_dir, "apps")
        with open(os.path.join(adir, f"{aid}.log"), "w") as f:
            f.write(log_text)
        with open(os.path.join(adir, f"{aid}.json"), "w") as f:
            json.dump(facts, f, indent=1)
        d["gsp_cycles"] = cyc
        with open(os.path.join(adir, f"{aid}.verdict.json"), "w") as f:
            json.dump(dict(d, restart=need_restart, guest=self.tag), f, indent=1)
        line = V.appres_line("win", aid, d, boot=self.tag, gsp_cycles=cyc, extra=f"tier={app['tier']} cat={app['category']}")
        return dict(app=aid, verdict=d["verdict"], line=line, digests=V.digest_lines(log_text), restart=need_restart, d=d)

    # ---- end
    def stop(self):
        try:
            self.vm.detach_disk()
        except Exception:
            pass
        how = self.vm.stop()
        self.log(f"[{self.tag}] stopped ({how})")
        return how


class GuestFailure(Exception):
    pass


class StageFailure(Exception):
    def __init__(self, pkg, detail):
        super().__init__(f"{pkg}: {detail}")
        self.pkg, self.detail = pkg, detail


# ------------------------------------------------------------------------------------------ the matrix
def append(path, lines):
    with open(path, "a") as f:
        for l in lines:
            f.write(l + "\n")


def read_done(path):
    done = {}
    if os.path.exists(path):
        for line in open(path, errors="replace"):
            m = re.match(r"APPRES side=\S+ app=(\S+) verdict=(\S+)", line)
            if m:
                done[m.group(1)] = m.group(2)
    return done


def run_matrix(cfg, vm_factory, apps_doc, manifest, run_dir, apps, log, sleep=time.sleep, now=time.time, isolate=True, resume=False):
    res, res_iso = os.path.join(run_dir, "win.res"), os.path.join(run_dir, "win_isolated.res")
    done = read_done(res) if resume else {}
    todo = [a for a in apps if a["id"] not in done]
    guest_no, boot_failures = 0, 0
    log(f"MATRIX_START apps={len(todo)} (resume skips {len(done)}) per_guest={cfg.per_guest} max_tdr={cfg.max_tdr} tier<={cfg.tier}")

    def one_guest(batch, resfile, tagname):
        nonlocal guest_no, boot_failures
        guest_no += 1
        tag = f"{tagname}{guest_no}"
        vm = vm_factory()
        g = GuestSession(cfg, vm, apps_doc, manifest, run_dir, log, tag, sleep=sleep, now=now)
        finished = []
        try:
            g.start()
            boot_failures = 0
        except (GuestFailure, qgamod.QgaError, RuntimeError, OSError) as e:
            boot_failures += 1
            log(f"[{tag}] BOOT_FAIL: {e}")
            first = batch[0]
            d = dict(verdict="BOOT_FAIL", rc="-", secs=0, quiet="-", note=f"guest-did-not-start:{str(e)[:80]}", tdr=0, wer=0, proof="-")
            append(resfile, [V.appres_line("win", first["id"], d, boot=tag, gsp_cycles="-", extra=f"tier={first['tier']} cat={first['category']}")])
            finished.append(first["id"])
            try:
                g.stop()
            except Exception:
                pass
            return finished
        why = ""
        try:
            for i, app in enumerate(batch):
                r = g.run_app(app)
                append(resfile, [r["line"]] + [f"APPDIG side=win app={app['id']} {x}" for x in r["digests"]])
                finished.append(app["id"])
                log(f"[{tag}] {app['id']}: {r['verdict']}" + (f" (restart guest: {r['restart']})" if r["restart"] else ""))
                if r["restart"]:
                    why = r["restart"]
                    break
        finally:
            g.stop()
        if why:
            log(f"[{tag}] session ended early: {why}; {len(batch) - len(finished)} apps move to a fresh guest")
        return finished

    # ---- phase 1: batched
    while todo:
        if guest_no >= cfg.max_guests:
            log("MATRIX_ABORT max guests reached")
            return 4
        batch = todo[:cfg.per_guest]
        fin = one_guest(batch, res, "g")
        todo = [a for a in todo if a["id"] not in fin]
        if boot_failures >= cfg.max_boot_failures:
            log(f"MATRIX_ABORT {boot_failures} consecutive guest boot failures")
            return 3
    # ---- phase 2: every non-PASS app alone in a fresh guest
    if isolate and cfg.per_guest > 1:
        first = read_done(res)
        redo = [a for a in apps if first.get(a["id"]) not in (None, "PASS", "NOTRUN")]
        log(f"PHASE2 re-running {len(redo)} non-PASS apps alone")
        for a in redo:
            fin = one_guest([a], res_iso, "i")
            if boot_failures >= cfg.max_boot_failures:
                log("MATRIX_ABORT repeated boot failures in phase 2")
                return 3
    p1 = read_done(res)
    log(f"MATRIX_DONE batched: {sum(1 for v in p1.values() if v == 'PASS')} pass / {len(p1)}")
    return 0


def main(argv=None):
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("--run-dir", required=True)
    ap.add_argument("--apps", default="all", help="comma list of app ids, or all")
    ap.add_argument("--tier", type=int, default=1)
    ap.add_argument("--category", default="")
    ap.add_argument("--exclude", default="")
    ap.add_argument("--manifest", default=os.path.join(HERE, "manifest.json"))
    ap.add_argument("--apps-json", default=os.path.join(HERE, "apps.json"))
    ap.add_argument("--iso", help="the app disk image (kfapps.iso) to hot-plug")
    ap.add_argument("--image-manifest", help="manifest.json written next to the image by build_appdisk.sh (gives the image identity)")
    ap.add_argument("--vm", choices=("broker", "mock"), default="broker")
    ap.add_argument("--broker", default="")
    ap.add_argument("--w-dir", default=os.environ.get("WIN_DIR", "/var/lib/kf-windows-20261005"))
    ap.add_argument("--kf3-rev", default=os.environ.get("KF3_REV", ""))
    ap.add_argument("--run-base", type=int, default=int(os.environ.get("WIN_RUN_BASE", "0")), help="first boundary-kayfabe-N run number; each guest takes the next")
    ap.add_argument("--pre-guest-cmd", default=os.environ.get("WIN_PRE_GUEST_CMD", ""), help="shell command run before each guest boots (host-specific)")
    ap.add_argument("--post-guest-cmd", default=os.environ.get("WIN_POST_GUEST_CMD", ""), help="shell command run after each guest stopped")
    ap.add_argument("--per-guest", type=int, default=8)
    ap.add_argument("--max-tdr", type=int, default=3)
    ap.add_argument("--no-isolate", action="store_true")
    ap.add_argument("--resume", action="store_true")
    ap.add_argument("--list", action="store_true")
    ap.add_argument("--mock-scenario", default="", help="(mock vm) JSON file of per-app behaviours")
    ap.add_argument("--fast", action="store_true", help="(mock vm) poll and sleep in milliseconds")
    a = ap.parse_args(argv)
    doc = json.load(open(a.apps_json))
    man = json.load(open(a.manifest))
    names = [x for x in a.apps.split(",") if x]
    apps = select_apps(doc["apps"], names, a.tier, [x for x in a.category.split(",") if x], set(x for x in a.exclude.split(",") if x))
    if a.list:
        for x in apps:
            print(f"{x['id']:28s} {x['category']:11s} tier{x['tier']} {x['session']:11s} {x['timeout_s']:5d}s {x['difficulty']}")
        return 0
    os.makedirs(a.run_dir, exist_ok=True)
    log = Log(os.path.join(a.run_dir, "session.log"))
    ident = None
    if a.image_manifest and os.path.exists(a.image_manifest):
        ident = json.load(open(a.image_manifest)).get("image", {}).get("identity")
    cfg = Cfg(iso=a.iso, image_identity=ident, per_guest=a.per_guest, max_tdr=a.max_tdr, tier=a.tier, password=os.environ.get("KF_GUEST_PW"),
              user=os.environ.get("KF_GUEST_USER", "vast"))
    sleep, now = time.sleep, time.time
    if a.vm == "mock":
        import mock_guest
        scen = json.load(open(a.mock_scenario)) if a.mock_scenario else {}
        if a.fast:
            cfg.poll_s, cfg.signin_delay, cfg.recover_wait, cfg.boot_timeout, cfg.slack_s, cfg.session_timeout = 0.01, 0, 1.0, 20, 2, 20
            cfg.io_timeout, cfg.ping_s, cfg.time_scale = 0.4, 0.4, 0.01
        mock = mock_guest.MockWorld(doc, scen, os.path.join(a.run_dir, "mock"))
        vm_factory = mock.new_vm
        cfg.password = cfg.password or "mockpw"
        sleep = lambda s: time.sleep(min(s, 0.02) if a.fast else s)
    else:
        broker = a.broker or next((p for p in (os.path.join(HERE, "..", "windows_broker_prod2.sh"), os.path.join(HERE, "..", "windows_broker.sh"),
                                               os.path.join(a.w_dir, "kayfabe-win-6fafcc6e", "scripts", "bench", "windows", "windows_broker_prod2.sh")) if os.path.exists(p)), None)
        if not broker or not a.kf3_rev or not a.iso:
            raise SystemExit("broker vm needs --broker (or a windows_broker*.sh found), --kf3-rev and --iso")
        # one BrokerVm per matrix so run numbers keep increasing
        shared = BrokerVm(broker, a.w_dir, a.kf3_rev, a.run_base, log, pre_cmd=a.pre_guest_cmd, post_cmd=a.post_guest_cmd)
        vm_factory = lambda: shared
    meta = dict(started_utc=utc(), args=vars(a), image_identity=ident, apps=[x["id"] for x in apps], host=os.uname().nodename)
    with open(os.path.join(a.run_dir, "matrix.json"), "w") as f:
        json.dump(meta, f, indent=1)
    log(f"START run_dir={a.run_dir} vm={a.vm} kf3_rev={a.kf3_rev or '-'} apps={len(apps)}")
    rc = run_matrix(cfg, vm_factory, doc, man, a.run_dir, apps, log, sleep=sleep, now=now, isolate=not a.no_isolate, resume=a.resume)
    log(f"EXIT rc={rc}")
    return rc


if __name__ == "__main__":
    sys.exit(main())
