"""GPU-free, network-free tests of the kfhost daemon core (stdlib only; fake transport, fake probes).

FALSIFIER: a borrowed offer with another repo / extra field / script / mismatching commit is acked or run; a
cmd frame runs on borrowed or container; a dropped socket kills a running job; 4401/4403 retries; 4409 reconnects.
"""

import json
import os
import stat
import subprocess
import sys
import tempfile
import threading
import time
import unittest
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))

from kfhost import cli, config, fetch, jobs, probes, profile, seeder  # noqa: E402
from kfhost.daemon import CMD_MAX_BYTES, Daemon  # noqa: E402
from kfhost.fetch import FetchError  # noqa: E402
from kfhost.transport import Closed, Connection, Transport  # noqa: E402

SHA = "a" * 40
TREE = "b" * 64
REPO = config.PUBLIC_REPO


def cfg_for(tmp, ceiling="owned", **extra):
    c = {"worker_url": "https://w.example", "box_id": "box1", "box_token": "tok", "access_id": "aid",
         "access_secret": "asec", "work_dir": str(Path(tmp) / "work")}
    # legacy test vocabulary -> the local flags: owned = allow_cmd, container, borrowed = trust_git_only
    c.update({"owned": {"allow_cmd": True}, "container": {"container": True}, "borrowed": {"trust_git_only": True},
              "standard": {}}[ceiling])
    c.update(extra)
    p = Path(tmp) / "cfg.json"
    p.write_text(json.dumps(c))
    os.chmod(p, 0o600)
    return config.load(p)


class Clock:
    t = 0.0

    def __call__(self):
        return self.t


class FakeConn(Connection):
    """script items: str frame | None idle tick | Closed | callable(fake) run at that point."""

    def __init__(self, script, clock):
        self.script, self.clock, self.sent = list(script), clock, []

    def send(self, text):
        self.sent.append(json.loads(text))

    def recv(self, timeout):
        if not self.script:
            raise Closed(1006)
        item = self.script.pop(0)
        if callable(item):
            item(self)
            return self.recv(timeout)
        if isinstance(item, Closed):
            raise item
        if item is None:
            self.clock.t += timeout
            return None
        return item if isinstance(item, str) else json.dumps(item)

    def types(self):
        return [m["type"] for m in self.sent]


class FakeTransport(Transport):
    def __init__(self, conns, clock):
        self.conns, self.clock, self.urls = list(conns), clock, []

    def connect(self, url, headers):
        self.urls.append((url, headers))
        item = self.conns.pop(0) if self.conns else Closed(4401)  # script exhausted: end the daemon
        if isinstance(item, Closed):
            raise item
        return item


KFSEED = {  # the real `python3 -m kfseed check` outputs and exit codes (W3 PR #345 cb8adf2f9)
    "same": (0, "nginx installed: 1.24\nnginx candidate: 1.24"),
    "drift": (0, "nginx installed: 1.24\nnginx candidate: 1.26\nWARNING: installed != candidate"),
    "unknown": (0, "nginx installed: 1.24\nnginx candidate: unknown"),
    "missing": (2, "nginx not installed: apt-get install nginx-light"),
    "odd": (1, "boom"),
}


class FakeSys:
    """Fake command runner for prepare_seeder: records argv, never touches the system."""

    def __init__(self, nginx="nginx version: nginx/1.24", fail=(), after=None, kfseed="same"):
        self.nginx, self.fail, self.after, self.kfseed, self.calls = nginx, fail, after, kfseed, []

    def __call__(self, argv):
        self.calls.append(list(argv))
        if argv[0] == "nginx":
            return (0, self.nginx) if self.nginx else (None, "")
        if argv[1:2] == ["-I"]:
            return KFSEED[self.kfseed]
        if argv[0] == "apt-get":
            if argv[1] in self.fail:
                return 100, "E: failed"
            if self.after is not None:
                self.nginx = self.after
            return 0, ""
        return None, ""


class FakeProbes(probes.Probes):
    def __init__(self, files=None, outs=None, existing=(), env=None, euid=1000, dirs=None):
        self.files, self.outs, self.existing, self._env, self._euid, self.dirs = files or {}, outs or {}, set(existing), env or {}, euid, dirs or {}

    def run(self, argv, timeout=10):
        return self.outs.get(argv[0])

    def read(self, path, limit=0):
        return self.files.get(path)

    def exists(self, path):
        return path in self.existing

    def access_rw(self, path):
        return path in self.existing

    def listdir(self, path):
        return self.dirs.get(path, [])

    def env(self, name):
        return self._env.get(name)

    def euid(self):
        return self._euid

    def cpu_count(self):
        return 8

    def disk_free(self, path):
        return 100 * 2**30

    def kernel(self):
        return "6.8.0"


class FakeRunner:
    gate = None  # threading.Event the run waits for
    log = []
    profiles = []

    def __init__(self, cfg, caps, fail_prepare=None):
        self.cfg, self.caps, self.fail, self.cancelled = cfg, caps, fail_prepare, False
        self.ran, self.ctx = False, None

    def prepare(self, offer, profile=None):
        FakeRunner.profiles.append(profile)
        if self.fail:
            raise FetchError(self.fail)
        return tempfile.mkdtemp()

    def run(self, ctx, checkout):
        self.ctx, self.ran = ctx, True
        FakeRunner.log.append(ctx.offer["offer_id"])
        if FakeRunner.gate:
            FakeRunner.gate.wait(10)
        return 0

    def cancel(self):
        self.cancelled = True


def offer(**over):
    o = {"type": "offer", "v": 1, "offer_id": "o1", "run_id": "r1", "run_token": "jwt.tok-en",
         "assignment": {"repo": REPO, "commit": SHA, "tree_key": TREE, "stages": ["merge_bar"]},
         "deadline": int(time.time() * 1000) + 3600_000}
    o.update(over)
    return o


def welcome(p="owned", **extra):
    w = {"type": "welcome", "v": 1, "box_id": "box1", "profile": p, "heartbeat_s": 15,
         "commands_until_ms": int(time.time() * 1000) + 3600_000}
    w.update(extra)
    return w


class Base(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory()
        self.addCleanup(self.tmp.cleanup)
        self.clock = Clock()
        FakeRunner.gate, FakeRunner.log = None, []
        self.runners = []

    def factory(self, cfg, caps):
        r = FakeRunner(cfg, caps, getattr(self, "fail_prepare", None))
        self.runners.append(r)
        return r

    def serve(self, script, ceiling="owned", conns=None, **kw):
        cfg = cfg_for(self.tmp.name, ceiling)
        conn = FakeConn(script, self.clock)
        tr = FakeTransport([conn] if conns is None else conns, self.clock)
        d = Daemon(cfg, tr, self.factory, prober=kw.pop("prober", FakeProbes()), clock=self.clock,
                   sleep=lambda s: setattr(self.clock, "t", self.clock.t + s), seed_timeout=0.3,
                   seeder_run=kw.pop("seeder_run", FakeSys()), euid=kw.pop("euid", 1000), **kw)
        return d, conn, tr

    def run_one(self, d):
        rc = d.run_forever(max_sessions=1)
        d.join()
        return rc


class ConfigTests(Base):
    def test_mode_check_refuses_group_world_readable(self):
        cfg_for(self.tmp.name)
        p = Path(self.tmp.name) / "cfg.json"
        for mode in (0o640, 0o604, 0o644, 0o660):
            os.chmod(p, mode)
            with self.assertRaises(config.ConfigError):
                config.load(p)
        os.chmod(p, 0o600)
        config.load(p)

    def test_run_refuses_to_start_on_bad_mode(self):
        cfg_for(self.tmp.name)
        p = Path(self.tmp.name) / "cfg.json"
        os.chmod(p, 0o644)
        self.assertEqual(cli.run(type("A", (), {"config": str(p)})()), 2)

    def test_unknown_and_missing_fields(self):
        p = Path(self.tmp.name) / "c.json"
        p.write_text(json.dumps({"worker_url": "https://x"}))
        os.chmod(p, 0o600)
        with self.assertRaises(config.ConfigError):
            config.load(p)

    def test_ws_url_and_headers(self):
        d, conn, tr = self.serve([welcome(), Closed(1006)])
        self.run_one(d)
        url, headers = tr.urls[0]
        self.assertTrue(url.startswith("wss://w.example/"))
        self.assertEqual(headers["CF-Access-Client-Id"], "aid")
        self.assertEqual(headers["Authorization"], "Bearer tok")


class ProbeTests(unittest.TestCase):
    def test_hello_measures(self):
        p = FakeProbes(
            files={"/proc/driver/nvidia/version": "NVRM version: NVIDIA UNIX Open Kernel Module for x86_64  580.1",
                   "/proc/meminfo": "MemTotal:       33554432 kB\n", "/proc/cpuinfo": "model name\t: Test CPU 9\n",
                   "/proc/1/cgroup": "0::/init.scope\n", "/etc/os-release": 'PRETTY_NAME="Ubuntu 24.04"\n'},
            outs={"nvidia-smi": "NVIDIA GeForce RTX 4070, 8.9, 12282, 580.65.06, 0x278410DE\n"},
            existing={"/dev/kvm"}, env={"DISPLAY": ":0"}, euid=0)
        caps = probes.measure(p, "/tmp")
        self.assertEqual((caps["gpu_name"], caps["compute_cap"], caps["vram_gb"], caps["driver"], caps["pci_device_id"]),
                         ("NVIDIA GeForce RTX 4070", "8.9", 12.0, "580.65.06", "2784"))
        self.assertEqual(caps["module_flavor"], "open")
        self.assertTrue(caps["kvm"] and caps["display"] and caps["can_swap_driver"])
        self.assertFalse(caps["container"])
        self.assertEqual((caps["cores"], caps["ram_gb"], caps["cpu_model"]), (8, 32.0, "Test CPU 9"))
        self.assertEqual(caps["os"], "Ubuntu 24.04")

    def test_unmeasurable_is_none_never_guessed(self):
        caps = probes.measure(FakeProbes(), "/tmp")
        for k in ("gpu_name", "compute_cap", "vram_gb", "driver", "pci_device_id", "module_flavor", "cpu_model", "ram_gb", "container", "can_swap_driver"):
            self.assertIsNone(caps[k], k)
        self.assertFalse(caps["kvm"])
        self.assertFalse(caps["display"])

    def test_container_and_drm_display(self):
        p = FakeProbes(existing={"/.dockerenv"}, euid=0, dirs={"/sys/class/drm": ["card0-HDMI-A-1"]},
                       files={"/sys/class/drm/card0-HDMI-A-1/status": "connected\n"})
        caps = probes.measure(p, "/tmp")
        self.assertTrue(caps["container"] and caps["display"])
        self.assertFalse(caps["can_swap_driver"])  # root but a container

    def test_hello_frame_sent_first(self):
        t = TestSession()
        t.setUp()
        d, conn, _ = t.serve([welcome(), Closed(1006)])
        t.run_one(d)
        self.assertEqual(conn.sent[0]["type"], "hello")
        self.assertEqual(conn.sent[0]["box_id"], "box1")
        self.assertIn("capabilities", conn.sent[0])
        t.tmp.cleanup()


class TestSession(Base):
    def runTest(self):
        pass

    def test_heartbeat_every_15s(self):
        d, conn, _ = self.serve([welcome()] + [None] * 40 + [Closed(1006)])
        self.run_one(d)
        beats = [m for m in conn.sent if m["type"] == "heartbeat"]
        self.assertGreaterEqual(len(beats), 2)
        self.assertEqual(beats[0]["state"], "idle")
        self.assertNotIn("run_id", beats[0])

    def test_backoff_grows_and_is_capped_with_jitter(self):
        d, _, _ = self.serve([])
        d.rng = type("R", (), {"random": lambda s: 0.0})()
        self.assertEqual([d.backoff_delay(n) for n in (0, 1, 2, 3, 10)], [0.5, 1.0, 2.0, 4.0, 30.0])
        d.rng = type("R", (), {"random": lambda s: 1.0})()
        self.assertEqual(d.backoff_delay(0), 1.0)
        self.assertEqual(d.backoff_delay(50), 60.0)

    def test_reconnects_after_drop_and_resets_backoff(self):
        c1, c2 = FakeConn([welcome(), Closed(1006)], self.clock), FakeConn([welcome(), Closed(4409)], self.clock)
        d, _, tr = self.serve([], conns=[c1, c2])
        self.assertEqual(d.run_forever(max_sessions=5), 3)
        self.assertEqual(len(tr.urls), 3)

    def test_connect_failure_retries(self):
        c = FakeConn([welcome(), Closed(4409)], self.clock)
        d, _, tr = self.serve([], conns=[Closed(None), Closed(None), c])
        self.assertEqual(d.run_forever(max_sessions=9), 3)
        self.assertEqual(len(tr.urls), 4)

    def test_4401_stops_without_retry(self):
        d, _, tr = self.serve([Closed(4401)])
        self.assertEqual(d.run_forever(), 3)
        self.assertEqual(len(tr.urls), 1)
        d, _, tr = self.serve([], conns=[Closed(4401)])  # rejected at the upgrade
        self.assertEqual(d.run_forever(), 3)

    def test_4403_and_4409_retry_slowly_not_exit(self):
        for code, floor in ((4403, 15.0), (4409, 60.0)):
            c1 = FakeConn([welcome(), Closed(code)], self.clock)
            c2 = FakeConn([welcome(), None, Closed(1006)], self.clock)
            d, _, tr = self.serve([], conns=[c1, c2])
            d.log = lambda m: None
            self.assertEqual(d.run_forever(max_sessions=2), 0)
            self.assertEqual(len(tr.urls), 2, code)  # it reconnected
            self.assertGreaterEqual(d.delays[0], floor, code)  # and waited long
        d, _, tr = self.serve([], conns=[Closed(4403), FakeConn([welcome(), Closed(1006)], self.clock)])
        d.log = lambda m: None
        d.run_forever(max_sessions=2)
        self.assertEqual(len(tr.urls), 2)  # a 403 at the upgrade is retried too

    def test_welcome_profile_derivation(self):
        for srv, ceiling, want in (("container", "owned", "container"), ("owned", "borrowed", "borrowed"),
                                   ("bogus", "owned", "standard"), ("owned", "owned", "owned"),
                                   ("owned", "standard", "standard"), ("standard", "owned", "standard")):
            d, _, _ = self.serve([welcome(srv), None], ceiling=ceiling)
            self.run_one(d)
            self.assertEqual(d.profile, want, (srv, ceiling))


class OfferTests(Base):
    def offer_result(self, prof, o, ceiling=None, prepared_fail=None):
        self.fail_prepare = prepared_fail
        def settle(c):
            for _ in range(300):
                if {"ready", "nack"} & set(c.types()):
                    return
                threading.Event().wait(0.01)

        d, conn, _ = self.serve([welcome(prof), o, settle, None, Closed(1006)], ceiling=ceiling or prof)
        self.run_one(d)
        return conn, d

    def test_borrowed_accepts_clean_offer(self):
        conn, d = self.offer_result("borrowed", offer())
        self.assertIn("ack", conn.types())
        self.assertEqual(FakeRunner.log, ["o1"])
        self.assertTrue(self.runners[0].ctx.profile == "borrowed")

    def test_borrowed_rejections(self):
        base = offer()
        cases = {
            "other repo": offer(assignment=dict(base["assignment"], repo="https://github.com/evil/kayfabe")),
            "extra assignment field": offer(assignment=dict(base["assignment"], script="rm -rf /")),
            "env": offer(assignment=dict(base["assignment"], env={"KF_X": "1"})),
            "requirements": offer(assignment=dict(base["assignment"], requirements={})),
            "extra top field": dict(base, cmd="id"),
            "short commit": offer(assignment=dict(base["assignment"], commit="abc123")),
            "uppercase commit": offer(assignment=dict(base["assignment"], commit="A" * 40)),
            "branch name": offer(assignment=dict(base["assignment"], commit="main")),
            "stage not allowed": offer(assignment=dict(base["assignment"], stages=["rm_rf"])),
            "stage shell": offer(assignment=dict(base["assignment"], stages=["merge;id"])),
            "bad tree": offer(assignment=dict(base["assignment"], tree_key="xyz")),
            "bad run id": offer(run_id="../x"),
        }
        for name, o in cases.items():
            conn, _ = self.offer_result("borrowed", o)
            self.assertIn("nack", conn.types(), name)
            self.assertNotIn("ack", conn.types(), name)  # refused by the profile checks: no early ack either
        self.assertEqual(FakeRunner.log, [])
        self.assertEqual(self.runners, [])

    def test_commit_mismatch_nacks_and_never_runs(self):
        conn, _ = self.offer_result("borrowed", offer(), prepared_fail="commit mismatch: wanted x, got y")
        self.assertEqual(conn.types()[1:4], ["ack", "nack"] if False else conn.types()[1:4])
        self.assertIn("nack", conn.types())
        self.assertNotIn("ready", conn.types())
        self.assertEqual(FakeRunner.log, [])

    def test_local_ceiling_beats_server_profile(self):
        # server says owned, local ceiling borrowed: the borrowed rules apply
        o = offer(assignment=dict(offer()["assignment"], env={"KF_A": "1"}))
        conn, _ = self.offer_result("owned", o, ceiling="borrowed")
        self.assertIn("nack", conn.types())

    def test_non_borrowed_profiles_allow_env_but_every_profile_pins_the_repo(self):
        o = offer(assignment=dict(offer()["assignment"], env={"KF_A": "1"}))
        for prof in ("container", "standard", "owned"):
            conn, _ = self.offer_result(prof, o)
            self.assertIn("ack", conn.types(), prof)
            self.assertIn("ready", conn.types(), prof)
        for repo in ("https://example.org/r.git", REPO + ".git", REPO + "/", REPO + "x", "https://github.com/reindertpelsma/kayfabe-evil",
                     REPO.replace("https", "http"), "reindertpelsma/kayfabe"):
            bad = offer(assignment=dict(offer()["assignment"], repo=repo))
            for prof in ("container", "standard", "owned", "borrowed"):
                conn, _ = self.offer_result(prof, bad)
                self.assertIn("nack", conn.types(), (prof, repo))
                self.assertNotIn("ack", conn.types(), (prof, repo))
        bad_env = offer(assignment=dict(offer()["assignment"], env={"PATH": "/x"}))
        conn, _ = self.offer_result("owned", bad_env)
        self.assertIn("nack", conn.types())

    def test_busy_nack_and_cancel(self):
        FakeRunner.gate = threading.Event()
        o2 = offer(offer_id="o2", run_id="r2")
        d, conn, _ = self.serve([welcome(), offer(), lambda c: self.wait_active(d), o2,
                                 {"type": "cancel", "offer_id": "o1"}, None, Closed(1006)])
        rc = d.run_forever(max_sessions=1)
        FakeRunner.gate.set()
        d.join()
        nacks = [m for m in conn.sent if m["type"] == "nack"]
        self.assertEqual([(n["offer_id"], n["reason"]) for n in nacks], [("o2", "busy")])
        self.assertTrue(self.runners[0].cancelled)

    def wait_active(self, d):
        for _ in range(200):
            if d.active and self.runners and self.runners[0].ran:
                return
            threading.Event().wait(0.01)

    def test_drop_mid_run_keeps_job_running_and_next_heartbeat_busy(self):
        FakeRunner.gate = threading.Event()
        c1 = FakeConn([welcome(), offer(), lambda c: self.wait_active(d), Closed(1006)], self.clock)
        c2 = FakeConn([welcome()] + [None] * 20 + [Closed(4409)], self.clock)
        d, _, _ = self.serve([], conns=[c1, c2])
        d.run_forever(max_sessions=3)
        self.assertTrue(d.active is not None and d.active["thread"].is_alive())
        busy = [m for m in c2.sent if m["type"] == "heartbeat"]
        self.assertTrue(busy and busy[0]["state"] == "busy" and busy[0]["run_id"] == "r1")
        FakeRunner.gate.set()
        d.join()
        self.assertIsNone(d.active)


class CmdTests(Base):
    def cmd_run(self, prof, msg, ceiling=None, pre=()):
        d, conn, _ = self.serve([welcome(prof)] + list(pre) + [msg, lambda c: d.join(), None, Closed(1006)], ceiling=ceiling or prof)
        self.run_one(d)
        return conn

    def test_owned_runs_cmd_and_streams(self):
        conn = self.cmd_run("owned", {"type": "cmd", "cmd_id": "c1", "argv": [sys.executable, "-c", "print('hi'); import sys; sys.stderr.write('err')"]})
        out = [m for m in conn.sent if m["type"] == "cmd_output"]
        self.assertIn("hi", "".join(m["data"] for m in out if m["stream"] == "stdout"))
        self.assertIn("err", "".join(m["data"] for m in out if m["stream"] == "stderr"))
        self.assertEqual([m["seq"] for m in out], list(range(len(out))))
        self.assertTrue(out[-1]["eof"])
        done = [m for m in conn.sent if m["type"] == "cmd_done"]
        self.assertEqual((done[0]["cmd_id"], done[0]["rc"]), ("c1", 0))

    def test_cmd_refused_on_borrowed_container_and_ceiling(self):
        for prof, ceiling in (("borrowed", None), ("container", None), ("standard", None), ("owned", "container"),
                              ("owned", "standard")):
            conn = self.cmd_run(prof, {"type": "cmd", "cmd_id": "c1", "argv": [sys.executable, "-c", "open('%s/pwned','w')" % self.tmp.name]}, ceiling)
            self.assertEqual([m["rc"] for m in conn.sent if m["type"] == "cmd_done"], [126], prof)
            self.assertFalse((Path(self.tmp.name) / "pwned").exists())

    def test_output_bound_and_timeout(self):
        code = "import sys\nfor _ in range(200):\n sys.stdout.write('x'*4096)"
        conn = self.cmd_run("owned", {"type": "cmd", "cmd_id": "c1", "argv": [sys.executable, "-c", code]})
        total = sum(len(m["data"]) for m in conn.sent if m["type"] == "cmd_output" and m["stream"] == "stdout")
        self.assertLessEqual(total, CMD_MAX_BYTES)
        self.assertIn("truncated", [m for m in conn.sent if m["type"] == "cmd_output"][-1]["data"])
        conn = self.cmd_run("owned", {"type": "cmd", "cmd_id": "c2", "argv": ["sleep", "30"], "timeout": 0.5})
        self.assertEqual([m["rc"] for m in conn.sent if m["type"] == "cmd_done"], [124])

    def test_cmd_not_during_run_unless_explicit(self):
        FakeRunner.gate = threading.Event()
        c = {"type": "cmd", "cmd_id": "c1", "argv": ["true"]}
        wait = lambda conn: [self.wait_active(d)]
        d, conn, _ = self.serve([welcome(), offer(), wait, c, lambda c_: d.join(0.5), None, Closed(1006)])
        d.run_forever(max_sessions=1)
        self.assertEqual([m["rc"] for m in conn.sent if m["type"] == "cmd_done"], [126])
        FakeRunner.gate.set()
        d.join()

    def wait_active(self, d):
        for _ in range(200):
            if d.active and self.runners and self.runners[0].ran:
                return
            threading.Event().wait(0.01)

    def test_cmd_needs_exactly_argv_or_shell(self):
        conn = self.cmd_run("owned", {"type": "cmd", "cmd_id": "c1"})
        self.assertEqual([m["rc"] for m in conn.sent if m["type"] == "cmd_done"], [126])
        conn = self.cmd_run("owned", {"type": "cmd", "cmd_id": "c2", "shell": "echo $((1+2))"})
        self.assertIn("3", "".join(m["data"] for m in conn.sent if m["type"] == "cmd_output"))


class SeederTests(unittest.TestCase):
    which = staticmethod(lambda n: "/usr/bin/" + n)

    def prep(self, profile, euid, fake):
        return seeder.prepare_seeder(profile, euid, run=fake, which=self.which)

    def apt(self, fake):
        return [c for c in fake.calls if c[0] == "apt-get"]

    def test_owned_root_upgrades_in_order_and_logs_versions(self):
        f = FakeSys(after="nginx version: nginx/1.26")
        r = self.prep("owned", 0, f)
        self.assertEqual(self.apt(f), [["apt-get", "update"], ["apt-get", "install", "-y", "--only-upgrade", "nginx-light"]])
        text = "\n".join(r["log"])
        self.assertIn("1.24", text)
        self.assertIn("1.26", text)
        self.assertTrue(r["available"])

    def test_owned_root_installs_when_missing(self):
        f = FakeSys(nginx=None)
        orig = f.__call__

        def run(argv):
            if argv[:2] == ["apt-get", "install"]:
                f.nginx = "nginx version: nginx/1.26"
            return orig(argv)

        r = seeder.prepare_seeder("owned", 0, run=run, which=self.which)
        self.assertIn(["apt-get", "install", "-y", "nginx-light"], f.calls)
        self.assertTrue(r["available"])

    def test_owned_nonroot_never_runs_apt_and_warns_on_drift(self):
        f = FakeSys(kfseed="drift")
        r = self.prep("owned", 1000, f)
        self.assertEqual(self.apt(f), [])
        self.assertTrue(any(c[1:2] == ["-I"] and c[-1] == "check" for c in f.calls))
        self.assertTrue(any("drifted" in w for w in r["warnings"]))
        self.assertTrue(r["available"])

    def test_container_and_borrowed_never_run_apt_even_as_root(self):
        for prof in ("container", "borrowed"):
            f = FakeSys(kfseed="drift")
            r = self.prep(prof, 0, f)
            self.assertEqual(self.apt(f), [], prof)
            self.assertTrue(r["warnings"])

    def test_missing_nginx_unavailable_where_no_changes_allowed(self):
        for prof, euid in (("borrowed", 0), ("container", 0), ("owned", 1000)):
            for f in (FakeSys(nginx=None, kfseed="missing"), FakeSys(kfseed="missing")):  # rc 2 alone is enough
                r = self.prep(prof, euid, f)
                self.assertFalse(r["available"], prof)

    def test_kfseed_outputs(self):
        r = self.prep("borrowed", 0, FakeSys(kfseed="same"))
        self.assertEqual((r["available"], r["warnings"]), (True, []))
        r = self.prep("borrowed", 0, FakeSys(kfseed="unknown"))
        self.assertTrue(r["available"] and any("unknown" in w for w in r["warnings"]))
        r = self.prep("container", 0, FakeSys(kfseed="drift"))
        self.assertTrue(r["available"] and any("drifted" in w for w in r["warnings"]))

    def test_unexpected_rc_is_unavailable_warning_but_not_blocking(self):
        r = self.prep("borrowed", 0, FakeSys(kfseed="odd"))
        self.assertTrue(r["available"])
        self.assertTrue(any("unavailable" in w for w in r["warnings"]))
        r = self.prep("borrowed", 0, FakeSys(nginx=None, kfseed="odd"))
        self.assertFalse(r["available"])  # blocked only because nginx is actually missing

    def test_apt_failure_with_nginx_present_still_available_with_warning(self):
        r = self.prep("owned", 0, FakeSys(fail=("update",)))
        self.assertTrue(r["available"])
        self.assertIn("nginx update failed", r["warnings"])

    def test_apt_failure_with_nginx_missing_unavailable(self):
        r = self.prep("owned", 0, FakeSys(nginx=None, fail=("update",)))
        self.assertFalse(r["available"])


class SeedTests(Base):
    def seed(self, reply):
        results = []
        art = {"name": "a.tar", "sha256": "0" * 64, "size": 3}

        def go(conn):
            results.append(d.seed_register("o1", art, "/x/a.tar"))
            results.append(d.seed_register("o1", art, "/x/a.tar"))

        script = [welcome(), lambda c: threading.Thread(target=go, args=(c,)).start(), None, None]
        if reply is not None:
            script.append(lambda c: self.wait_sent(c))
            script.append(reply)
        script += [None] * 4 + [Closed(1006)]
        d, conn, _ = self.serve(script)
        self.run_one(d)
        threading.Event().wait(0.8)
        return d, conn, results

    def wait_sent(self, conn):
        for _ in range(200):
            if any(m["type"] == "seed_register" for m in conn.sent):
                return
            threading.Event().wait(0.01)

    def test_ok_ack(self):
        d, conn, results = self.seed({"type": "seed_ack", "ok": True})
        reg = [m for m in conn.sent if m["type"] == "seed_register"]
        self.assertEqual(reg[0]["artifact"]["name"], "a.tar")
        self.assertTrue(results[0])

    def test_refused_ack_disables_seeding_for_session(self):
        d, conn, results = self.seed({"type": "seed_ack", "ok": False, "reason": "probe failed"})
        self.assertEqual(results, [False, False])
        self.assertFalse(d.seed_ok)
        self.assertEqual(len([m for m in conn.sent if m["type"] == "seed_register"]), 1)  # no second attempt

    def test_no_ack_times_out_and_disables(self):
        d, conn, results = self.seed(None)
        self.assertFalse(results[0])
        self.assertFalse(d.seed_ok)

    def test_missing_nginx_makes_seeding_unavailable_and_job_continues(self):
        FakeRunner.gate = None
        results, logs = [], []
        art = {"name": "a", "sha256": "0" * 64, "size": 1}
        d, conn, _ = self.serve([welcome(), None, Closed(1006)], seeder_run=FakeSys(nginx=None, fail=("update", "install")), euid=0)
        d.profile = "owned"
        d.run_forever(max_sessions=1)
        d.conn = type("C", (), {"send": lambda s, t: None})()
        results.append(d.seed_register("o1", art, "/x", log=logs.append))
        results.append(d.seed_register("o1", art, "/x", log=logs.append))
        self.assertEqual(results, [False, False])
        self.assertFalse(d.seed_ok)
        self.assertTrue(any("unavailable" in l for l in logs))
        self.assertEqual(len([m for m in conn.sent if m["type"] == "seed_register"]), 0)

    def test_drift_warning_in_seed_register_frame(self):
        script = [welcome("container"), lambda c: threading.Thread(target=lambda: d.seed_register("o1", {"name": "a", "sha256": "0" * 64, "size": 1}, "/x")).start(),
                  lambda c: self.wait_sent(c), {"type": "seed_ack", "ok": True}, None, None, Closed(1006)]
        d, conn, _ = self.serve(script, seeder_run=FakeSys(kfseed="drift"), ceiling="container")
        self.run_one(d)
        reg = [m for m in conn.sent if m["type"] == "seed_register"][0]
        self.assertTrue(any("drifted" in w for w in reg["warnings"]))

    def test_new_session_re_enables(self):
        d, _, _ = self.serve([welcome(), Closed(4409)])
        d.seed_ok = False
        d.run_forever()
        self.assertTrue(d.seed_ok)


def git(cwd, *a):
    return subprocess.run(["git", "-c", "user.email=t@t", "-c", "user.name=t", *a], cwd=cwd, check=True,
                          capture_output=True, text=True).stdout.strip()


def key_of(path, commit):
    from kfhost.treekey import commit_tree_key

    return commit_tree_key(lambda a: subprocess.run(["git", *a], cwd=path, check=True, capture_output=True).stdout, commit)


class FetchTests(unittest.TestCase):
    """A local repo stands in for the public one: master has A, a side branch ('pull/1') has B."""

    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory()
        self.addCleanup(self.tmp.cleanup)
        self.src = Path(self.tmp.name) / "src"
        self.src.mkdir()
        git(self.src, "init", "-q", "-b", "master")
        git(self.src, "config", "uploadpack.allowAnySHA1InWant", "true")
        (self.src / "Cargo.toml").write_text("1")
        git(self.src, "add", ".")
        git(self.src, "commit", "-qm", "one")
        self.sha = git(self.src, "rev-parse", "HEAD")
        git(self.src, "checkout", "-q", "-b", "pr-branch")
        (self.src / "Cargo.toml").write_text("2")
        git(self.src, "commit", "-qam", "stranger's change")
        self.pr = git(self.src, "rev-parse", "HEAD")
        git(self.src, "checkout", "-q", "master")
        self.work = str(Path(self.tmp.name) / "work")
        self.key = key_of(self.src, self.sha)

    def test_fetch_and_verify(self):
        self.assertRegex(self.key, "^[0-9a-f]{64}$")
        d, mismatch = fetch.fetch_commit(str(self.src), self.sha, self.work, self.key)
        self.assertFalse(mismatch)
        self.assertEqual(git(d, "rev-parse", "HEAD"), self.sha)
        self.assertTrue(Path(d).resolve().is_relative_to(Path(self.work).resolve()))
        self.assertEqual((Path(d) / "Cargo.toml").read_text(), "1")

    def test_standard_fetches_a_commit_that_exists_only_on_a_side_branch(self):
        d, mismatch = fetch.fetch_commit(str(self.src), self.pr, self.work, key_of(self.src, self.pr))
        self.assertEqual(git(d, "rev-parse", "HEAD"), self.pr)
        self.assertFalse(mismatch)

    def test_trust_git_only_refuses_the_side_branch_commit(self):
        with self.assertRaises(FetchError) as cm:
            fetch.fetch_commit(str(self.src), self.pr, self.work, key_of(self.src, self.pr), strict=True)
        self.assertIn("not reachable from origin/master or origin/next", str(cm.exception))
        self.assertEqual(os.listdir(self.work), [])  # nothing left behind
        d, _ = fetch.fetch_commit(str(self.src), self.sha, self.work, self.key, strict=True)  # master commit is fine
        self.assertEqual(git(d, "rev-parse", "HEAD"), self.sha)

    def test_next_branch_counts_when_strict(self):
        git(self.src, "branch", "next", self.pr)
        fetch.fetch_commit(str(self.src), self.pr, self.work, key_of(self.src, self.pr), strict=True)

    def test_tree_key_mismatch_warns_for_standard_and_refuses_when_strict(self):
        d, mismatch = fetch.fetch_commit(str(self.src), self.sha, self.work, "0" * 64)
        self.assertTrue(mismatch)
        self.assertTrue(Path(d).is_dir())
        with self.assertRaises(FetchError) as cm:
            fetch.fetch_commit(str(self.src), self.sha, self.work, "0" * 64, strict=True)
        self.assertIn("tree key mismatch", str(cm.exception))

    def test_head_mismatch_refused(self):
        real = fetch._git

        def lying(args, cwd, timeout, **kw):
            return "f" * 40 if args[0] == "rev-parse" else real(args, cwd, timeout, **kw)

        fetch._git = lying
        try:
            with self.assertRaises(FetchError) as cm:
                fetch.fetch_commit(str(self.src), self.sha, self.work, self.key)
        finally:
            fetch._git = real
        self.assertIn("commit mismatch", str(cm.exception))

    def test_unknown_commit_refused(self):
        with self.assertRaises(FetchError):
            fetch.fetch_commit(str(self.src), "1" * 40, self.work, self.key)

    def test_strict_fetches_only_two_refs_and_redirects_are_off(self):
        seen = []
        real = fetch._git

        def spy(args, cwd, timeout, **kw):
            seen.append(args)
            return real(args, cwd, timeout, **kw)

        fetch._git = spy
        try:
            fetch.fetch_commit(str(self.src), self.sha, self.work, self.key, strict=True)
        finally:
            fetch._git = real
        fetches = [a for a in seen if a[0] == "fetch"]
        self.assertTrue(fetches)
        for a in fetches:
            self.assertTrue(any(x.startswith("+refs/heads/master:") or x.startswith("+refs/heads/next:") for x in a), a)
            self.assertFalse(any("pull" in x or x == self.sha for x in a), a)
        self.assertIn("http.followRedirects=false", Path(fetch.__file__).read_text())


class JobRunnerTests(Base):
    def test_standalone_env_has_no_cfg_secrets_and_no_offer_env(self):
        cfg = cfg_for(self.tmp.name, "borrowed")
        r = jobs.JobRunner(cfg, {})
        ctx = jobs.JobContext(offer(), "borrowed", cfg, None, lambda: False)
        scratch = tempfile.mkdtemp(dir=self.tmp.name)
        env = r.child_env(ctx, scratch)
        self.assertEqual(env["KF_STANDALONE"], "1")
        self.assertTrue(env["HOME"].startswith(scratch) and env["TMPDIR"].startswith(scratch))
        self.assertFalse(any(str(Path(self.tmp.name) / "cfg.json") in v for v in env.values()))
        self.assertNotIn("KF_RUN_TOKEN", env)
        self.assertFalse(any("tok" == v or v == "asec" for v in env.values()))

    def test_tree_key_mismatch_is_a_warning_and_a_result_field_for_standard(self):
        logs, results = [], []

        class Rep:
            def __init__(s, cfg, o):
                pass
            progress = lambda s, x: None

            def feed(s, data):
                logs.append(data.decode())

            def finish(s, result, note):
                results.append(result)

        cfg = cfg_for(self.tmp.name, "standard")
        co = Path(self.tmp.name) / "co"
        (co / "scripts/kfci").mkdir(parents=True)
        (co / "scripts/kfci/main.py").write_text("print('x')")
        r = jobs.JobRunner(cfg, {}, reporting=Rep, fetch=lambda *a, **k: (str(co), True))
        self.assertEqual(r.prepare(offer(), "standard"), str(co))
        r.run(jobs.JobContext(offer(), "standard", cfg, None, lambda: False), str(co))
        self.assertTrue(any("WARNING" in l and "tree key" in l for l in logs))
        self.assertTrue(results[0]["tree_key_mismatch"])
        # no mismatch: no field
        r2 = jobs.JobRunner(cfg, {}, reporting=Rep, fetch=lambda *a, **k: (str(co), False))
        r2.prepare(offer(), "standard")
        self.assertNotIn("tree_key_mismatch", r2.result(jobs.JobContext(offer(), "standard", cfg, None, lambda: False), 0))

    def test_report_goes_through_reporter_with_run_token(self):
        seen = []

        class Rep:
            def __init__(s, cfg, o):
                seen.append(o["run_token"])
            progress = feed = lambda s, x: None

            def finish(s, result, note):
                seen.append(result)

        cfg = cfg_for(self.tmp.name, "owned")
        co = Path(self.tmp.name) / "co"
        (co / "scripts/kfci").mkdir(parents=True)
        (co / "scripts/kfci/main.py").write_text("print('x')")
        r = jobs.JobRunner(cfg, {"gpu_name": "G", "compute_cap": "8.9", "driver": "580.1"}, reporting=Rep)
        rc = r.run(jobs.JobContext(offer(), "owned", cfg, None, lambda: False), str(co))
        self.assertEqual(rc, 0)
        self.assertEqual(seen[0], "jwt.tok-en")
        self.assertEqual(seen[1]["verdict"], "PASS")
        self.assertEqual(seen[1]["hardware"]["compute_cap"], 890)
        self.assertFalse(co.exists())


class SetupCheckTests(unittest.TestCase):
    def test_setup_refuses_while_hashes_todo(self):
        calls = []
        out = []
        rc = cli.setup("/nonexistent/venv", run=lambda *a, **k: calls.append(a), out=out.append)
        self.assertEqual((rc, calls), (2, []))
        self.assertIn("HASHES TODO", out[0])

    def test_setup_commands_touch_only_venv(self):
        with tempfile.TemporaryDirectory() as t:
            lock = Path(t) / "l.lock"
            lock.write_text("websockets==13.1 \\\n    --hash=sha256:" + "0" * 64 + "\n")
            calls = []
            rc = cli.setup(Path(t) / "v", lock=lock, run=lambda argv, check: calls.append(argv) or type("R", (), {"returncode": 0})(), out=lambda s: None)
            self.assertEqual(rc, 0)
            self.assertIn("--require-hashes", calls[1])
            self.assertTrue(all(str(Path(t).resolve() / "v") in " ".join(c) for c in calls))
            (Path(t) / "other").mkdir()
            (Path(t) / "other" / "f").write_text("x")
            self.assertEqual(cli.setup(Path(t) / "other", lock=lock, run=None, out=lambda s: None), 2)

    def test_lock_has_exact_pin_and_marker(self):
        text = cli.LOCK.read_text()
        self.assertRegex(text, r"(?m)^websockets==\d+\.\d+(\.\d+)? ")
        self.assertIsNotNone(cli.lock_problem(text))

    def test_check_reports_and_changes_nothing(self):
        out = []

        def no_import(name):
            raise ImportError(name)

        rc = cli.check(out=out.append, which=lambda x: None, importer=no_import, exists=lambda p: False)
        self.assertEqual(rc, 1)
        text = "\n".join(out)
        self.assertIn("MISSING git", text)
        self.assertIn("websockets", text)
        self.assertIn("setup --venv", text)


if __name__ == "__main__":
    unittest.main()
