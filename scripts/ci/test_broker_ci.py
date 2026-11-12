#!/usr/bin/env python3
"""The display broker in the CI box run (broker_ci.py and its place in vast_payload.py), GPU-free.

FALSIFIER (written first): the broker is on for the small tier or off for middle/large without KF_BROKER; a broker
failure (no start, no build, death, error line, no attach, no frame, late attach, leftover, a missing summary, a blank
first frame, no resize attach, no input) grades as anything but a FAIL named broker_*; a broker failure is turned into a
retry/operator/provider class or a skip, or is hidden behind a guest error or a ProviderFault excuse; a ProviderFault
loses its provider verdict; the broker outlives its stage; a write to its stdin can block.
NOTHING here proves the broker works on a box (docs/design/V3_BROKER_IN_CI.md).
"""

import importlib.util
import io
import os
import re
import stat
import subprocess
import sys
import tempfile
import textwrap
import unittest
from pathlib import Path
from unittest.mock import patch

import test_vast_payload_v7 as v7

HERE = Path(__file__).resolve().parent
import broker_ci as B  # noqa: E402
import broker_explicit_guest as G  # noqa: E402

GOOD = dict(started=1, built=1, accepted=1, attaches=3, commits=9, first_attach_s=40, died=0, errors=0, stopped=1)
LOG_OK = (
    "nvkvm-broker: accepted uid 0 pid 12\n"
    "nvkvm-broker: TEST attach: id=125 1920x1080 stride=7680 offset=0 XR24 mod=0x0 size=8294400\n"
    "nvkvm-broker: TEST commit: id=125 1920x1080\n"
)


def payload():
    spec = importlib.util.spec_from_file_location("payload_broker", HERE / "vast_payload.py")
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


def fake_broker(directory, mode):
    """A python stand-in for nvkvm-display-broker taking the same arguments."""
    path = Path(directory) / "fake-broker"
    path.write_text(
        textwrap.dedent(
            """\
            #!/usr/bin/env python3
            import os, socket, sys, time
            a = sys.argv
            sock = a[a.index("--socket") + 1]
            mode = %r
            if mode == "nosock":
                time.sleep(30)
            if mode == "exit0":
                sys.exit(0)
            s = socket.socket(socket.AF_UNIX); s.bind(sock)
            print("nvkvm-broker: listening", flush=True)
            if mode in ("ok", "die", "noattach"):
                if mode != "noattach":
                    print("nvkvm-broker: accepted uid 0 pid 1", flush=True)
                    print("nvkvm-broker: TEST attach: id=1 1920x1080 stride=7680 offset=0 XR24 mod=0x0 size=1", flush=True)
                    print("nvkvm-broker: TEST commit: id=1 1920x1080", flush=True)
            if mode == "die":
                time.sleep(0.4); sys.exit(3)
            if mode == "noread":
                time.sleep(60)
            for line in sys.stdin:
                print("nvkvm-broker: TEST " + line.strip(), flush=True)
            """
        )
        % mode
    )
    path.chmod(path.stat().st_mode | stat.S_IXUSR)
    return path


class Wanted(unittest.TestCase):
    def test_default_follows_the_tier(self):
        for tier, want in (("small", False), ("adhoc", False), ("unknown", False), (None, False), ("middle", True), ("large", True)):
            with self.subTest(tier):
                self.assertEqual(B.broker_wanted({}, tier), (want, "tier"))

    def test_override_wins_and_is_popped(self):
        env = {"KF_BROKER": "1"}
        self.assertEqual(B.broker_wanted(env, "small"), (True, "env"))
        self.assertNotIn("KF_BROKER", env)
        self.assertEqual(B.broker_wanted({"KF_BROKER": "0"}, "large"), (False, "env"))

    def test_malformed_override_is_refused(self):
        for bad in ("", "2", "yes", "true"):
            with self.subTest(bad):
                with self.assertRaises(ValueError):
                    B.broker_wanted({"KF_BROKER": bad}, "middle")

    def test_payload_refuses_a_malformed_override_as_preflight(self):
        m = payload()
        with self.assertRaises(m.EvidenceError) as c:
            m.broker_requested({"KF_BROKER": "x"}, "middle")
        self.assertEqual(c.exception.name, "preflight")


class Environment(unittest.TestCase):
    def test_the_uid_is_part_of_the_device_properties(self):
        self.assertIn(",display-broker-uid=0", B.kf3_props("/s"))
        self.assertIn(",display-broker-uid=1000", B.kf3_props("/s", 1000))

    def test_kf3_props_attach_the_device(self):
        self.assertEqual(B.kf3_props("/run/kf-broker/a.sock"), "display=on,display-broker=/run/kf-broker/a.sock,display-broker-uid=0")

    def test_child_environment_appends_and_sets(self):
        env = B.child_environment({"KF3_DEV_EXTRA": "guest-driver=1", "X": "1"}, "/s")
        self.assertTrue(env["KF3_DEV_EXTRA"].startswith("guest-driver=1,display=on,display-broker=/s"))
        self.assertEqual(env["WINVM_BROKER_SOCK"], "/s")
        self.assertEqual(env["KF_BROKER_SOCK"], "/s")
        self.assertEqual(env["X"], "1")
        self.assertTrue(B.child_environment({}, "/s")["KF3_DEV_EXTRA"].startswith("display=on"))


class LogAndSummary(unittest.TestCase):
    def test_parse_log_counts_frames_attaches_and_sizes(self):
        d = B.parse_log(LOG_OK + "nvkvm-broker: TEST attach: id=2 1280x720 stride=1 offset=0 XR24 mod=0x0 size=1\n")
        self.assertEqual((d["accepted"], d["attaches"], d["commits"]), (1, 2, 1))
        self.assertEqual(d["sizes"], ["1920x1080", "1280x720"])
        self.assertEqual(B.count_attach(LOG_OK, "1920x1080"), 1)

    def test_a_refused_attach_is_not_an_attach(self):
        self.assertEqual(B.parse_log("TEST attach: REFUSED XR24 mod=0x1, as DRI3 refuses an x")["attaches"], 0)

    def test_error_markers(self):
        self.assertEqual(B.parse_log("x framing violation y\nSegmentation fault\n")["errors"], 2)
        self.assertEqual(B.parse_log(LOG_OK)["errors"], 0)

    def test_summary_round_trip(self):
        line = B.format_summary("cuda_guest", GOOD)
        self.assertEqual(B.parse_summary(line + "\n", "cuda_guest"), GOOD)
        self.assertIsNone(B.parse_summary(line, "apps"))
        self.assertIsNone(B.parse_summary(line + "\n" + line, "cuda_guest"))  # two summaries cannot be trusted
        self.assertIsNone(B.parse_summary("", "cuda_guest"))

    def test_every_failure_class_has_its_condition(self):
        cases = {
            "broker_start": dict(GOOD, started=0),
            "broker_build": dict(GOOD, built=0),
            "broker_died": dict(GOOD, died=1),
            "broker_error": dict(GOOD, errors=1),
            "broker_no_attach": dict(GOOD, attaches=0),
            "broker_no_frames": dict(GOOD, commits=0),
            "broker_late_attach": dict(GOOD, first_attach_s=B.ATTACH_LIMIT_S + 1),
            "broker_leftover": dict(GOOD, stopped=0),
        }
        for want, d in cases.items():
            with self.subTest(want):
                self.assertEqual(B.failure_class(d), want)
        self.assertEqual(B.failure_class(dict(GOOD, accepted=0)), "broker_no_attach")
        self.assertEqual(B.failure_class(None), "broker_start")
        self.assertIsNone(B.failure_class(GOOD))
        self.assertIsNone(B.failure_class(dict(GOOD, first_attach_s=B.ATTACH_LIMIT_S)))

    def test_no_class_is_a_retry_operator_provider_or_skip_name(self):
        row_name = re.compile(r"[a-z][a-z0-9_]{0,47}")  # vast_report.SUITE_NAME
        for name in B.CLASSES:
            self.assertTrue(name.startswith("broker_"), name)
            self.assertIsNotNone(row_name.fullmatch(name), name)
            for word in ("retry", "operator", "provider", "unattributed", "skip", "not_run", "notrun", "media"):
                self.assertNotIn(word, name)

    def test_stage_rows_fail_for_every_class_never_skip(self):
        for d in (None, dict(GOOD, died=1), dict(GOOD, attaches=0), dict(GOOD, commits=0)):
            text = "" if d is None else B.format_summary("apps", d)
            with self.assertRaises(B.BrokerFailure) as c:
                B.stage_rows(text, "apps")
            self.assertIn(c.exception.name, B.CLASSES)
        self.assertEqual(B.stage_rows(B.format_summary("apps", GOOD), "apps"), [B.row("broker_apps", 1)])


class Lifecycle(unittest.TestCase):
    def session(self, mode, d=None):
        tmp = tempfile.TemporaryDirectory()
        self.addCleanup(tmp.cleanup)
        work = Path(tmp.name)
        return B.BrokerSession(fake_broker(work, mode), work / "run", "s", sleep=lambda s: __import__("time").sleep(min(s, 0.02))), work

    def test_start_serves_a_socket_and_stop_leaves_nothing(self):
        s, work = self.session("ok")
        self.assertTrue(s.start(wait=10))
        pid = s.proc.pid
        self.assertTrue(s.sock.exists())
        self.assertEqual(oct(os.stat(s.dir).st_mode & 0o777), "0o700")
        s.mark_guest_started()
        for _ in range(100):
            s.poll()
            if s.first_attach is not None:
                break
            __import__("time").sleep(0.05)
        self.assertIsNotNone(s.first_attach)
        s.stop()
        self.assertEqual(s.stopped, 1)
        with self.assertRaises(ProcessLookupError):
            os.killpg(pid, 0)
        self.assertFalse(s.sock.exists())
        self.assertFalse(s.fifo.exists())
        d = s.summary()
        self.assertIsNone(B.failure_class(d), d)

    def test_a_broker_without_a_socket_does_not_start(self):
        s, _ = self.session("nosock")
        self.assertFalse(s.start(wait=0.5))
        s.stop()
        self.assertEqual(B.failure_class(s.summary()), "broker_start")
        self.assertEqual(s.stopped, 1)

    def test_a_missing_binary_does_not_start(self):
        with tempfile.TemporaryDirectory() as t:
            s = B.BrokerSession(Path(t) / "nope", Path(t) / "r", "s")
            self.assertFalse(s.start(wait=0.2))
            s.stop()

    def test_death_while_the_guest_runs_is_noted(self):
        s, _ = self.session("die")
        self.assertTrue(s.start(wait=10))
        s.mark_guest_started()
        for _ in range(100):
            s.poll()
            if s.died:
                break
            __import__("time").sleep(0.05)
        s.stop()
        self.assertEqual(s.died, 1)
        self.assertEqual(B.failure_class(s.summary()), "broker_died")

    def test_stdin_lines_reach_the_broker_and_a_full_fifo_never_blocks(self):
        s, _ = self.session("ok")
        self.assertTrue(s.start(wait=10))
        s.send("a 960 540")
        for _ in range(100):
            if "TEST a 960 540" in s.log_text():
                break
            __import__("time").sleep(0.05)
        self.assertIn("TEST a 960 540", s.log_text())
        s.stop()
        # a FIFO nobody reads: writes fail with BlockingIOError instead of blocking
        r, w = os.pipe()
        os.set_blocking(w, False)
        s.fifo_fd = w
        with self.assertRaises(BlockingIOError):
            for _ in range(200000):
                s.send("x" * 100)
        os.close(r)
        os.close(w)

    def test_no_attach_summary_when_nobody_connects(self):
        s, _ = self.session("noattach")
        self.assertTrue(s.start(wait=10))
        s.stop()
        self.assertEqual(B.failure_class(s.summary()), "broker_no_attach")


class RunStage(unittest.TestCase):
    def test_runs_the_command_with_the_environment_and_prints_the_summary(self):
        with tempfile.TemporaryDirectory() as t:
            t = Path(t)
            fb = fake_broker(t, "ok")
            marker = t / "env.txt"
            out = io.StringIO()
            rc = B.run_stage(
                "cuda_guest",
                [sys.executable, "-c", "import os,sys;open(sys.argv[1],'w').write(os.environ['KF3_DEV_EXTRA']+'|'+os.environ['WINVM_BROKER_SOCK']);import time;time.sleep(0.5)", str(marker)],
                t, directory=t / "run", environ={"PATH": os.environ["PATH"]}, binary_finder=lambda root: (fb, 1), out=out, tick=0.1,
            )
            self.assertEqual(rc, 0)
            self.assertIn("display-broker=" + str(t / "run/cuda_guest.sock"), marker.read_text())
            d = B.parse_summary(out.getvalue(), "cuda_guest")
            self.assertIsNone(B.failure_class(d), d)

    def test_the_commands_exit_code_is_relayed(self):
        with tempfile.TemporaryDirectory() as t:
            t = Path(t)
            fb = fake_broker(t, "ok")
            rc = B.run_stage("apps", [sys.executable, "-c", "raise SystemExit(7)"], t, directory=t / "r",
                             environ={}, binary_finder=lambda root: (fb, 1), out=io.StringIO(), tick=0.05)
            self.assertEqual(rc, 7)

    def test_a_broker_that_does_not_start_blocks_the_command(self):
        with tempfile.TemporaryDirectory() as t:
            t = Path(t)
            fb = fake_broker(t, "nosock")
            marker = t / "ran"
            out = io.StringIO()
            with patch.object(B, "START_WAIT_S", 0.3):
                rc = B.run_stage("apps", [sys.executable, "-c", "open(%r,'w')" % str(marker)], t, directory=t / "r",
                                 environ={}, binary_finder=lambda root: (fb, 1), out=out, tick=0.05,
                                 session_factory=lambda b, d, n: B.BrokerSession(b, d, n))
            self.assertEqual(rc, 1)
            self.assertFalse(marker.exists())
            self.assertEqual(B.failure_class(B.parse_summary(out.getvalue(), "apps")), "broker_start")

    def test_a_missing_build_is_broker_build(self):
        out = io.StringIO()
        rc = B.run_stage("apps", ["true"], "/x", directory="/tmp/none", environ={}, binary_finder=lambda r: (Path("/x/b"), 0), out=out)
        self.assertEqual(rc, 1)
        self.assertEqual(B.failure_class(B.parse_summary(out.getvalue(), "apps")), "broker_build")

    def test_the_broker_is_gone_after_the_stage(self):
        with tempfile.TemporaryDirectory() as t:
            t = Path(t)
            fb = fake_broker(t, "ok")
            made = []

            def factory(b, d, n):
                made.append(B.BrokerSession(b, d, n))
                return made[0]

            B.run_stage("apps", ["true"], t, directory=t / "r", environ={}, binary_finder=lambda r: (fb, 1),
                        out=io.StringIO(), tick=0.05, session_factory=factory)
            self.assertIsNotNone(made[0].proc.poll())
            self.assertEqual(made[0].stopped, 1)


class Guarded(unittest.TestCase):
    """vast_payload.broker_guarded: the grading rules of the owner ruling."""

    def setUp(self):
        self.m = payload()
        self.tmp = tempfile.TemporaryDirectory()
        self.addCleanup(self.tmp.cleanup)
        self.out = Path(self.tmp.name) / "stage.run"

    def grade(self, summary, guest, rc=0, name="apps"):
        if summary is not None:
            self.out.write_text(B.format_summary(name, summary) + "\n")
        elif self.out.exists():
            self.out.unlink()
        return self.m.broker_guarded(guest, name, self.out)(rc)

    @staticmethod
    def ok(rc):
        return [{"name": "apps", "pass": 3, "fail": 0, "skip": 0}]

    @staticmethod
    def raising(error_name, provider=False):
        def grade(rc):
            m = Guarded.m_ref
            raise (m.ProviderFault if provider else m.EvidenceError)(error_name)
        return grade

    def test_a_good_broker_adds_its_row(self):
        rows = self.grade(GOOD, self.ok)
        self.assertEqual(rows[-1], B.row("broker_apps", 1))
        self.assertEqual(len(rows), 2)

    def test_every_broker_failure_is_a_plain_fail_even_when_the_guest_passed(self):
        for cls, d in {
            "broker_died": dict(GOOD, died=1), "broker_no_frames": dict(GOOD, commits=0),
            "broker_no_attach": dict(GOOD, attaches=0), "broker_late_attach": dict(GOOD, first_attach_s=99999),
            "broker_start": dict(GOOD, started=0), "broker_error": dict(GOOD, errors=2),
        }.items():
            with self.subTest(cls):
                with self.assertRaises(self.m.EvidenceError) as c:
                    self.grade(d, self.ok)
                self.assertEqual(c.exception.name, cls)
                self.assertNotIsInstance(c.exception, self.m.ProviderFault)
                self.assertEqual(c.exception.rows, self.ok(0))  # the guest's earned rows stay

    def test_a_missing_summary_or_output_is_broker_start_not_infrastructure(self):
        with self.assertRaises(self.m.EvidenceError) as c:
            self.grade(None, self.ok)
        self.assertEqual(c.exception.name, "broker_start")

    def test_a_broker_failure_wins_over_a_guest_retry_and_keeps_the_guest_row(self):
        Guarded.m_ref = self.m
        for guest in ("apps_retry_operator", "apps_retry_hostref", "apps", "windows_apps_retry_provider"):
            with self.subTest(guest):
                with self.assertRaises(self.m.EvidenceError) as c:
                    self.grade(dict(GOOD, died=1), self.raising(guest))
                self.assertEqual(c.exception.name, "broker_died")
                self.assertIn({"name": guest, "pass": 0, "fail": 1, "skip": 0}, c.exception.rows)

    def test_a_provider_fault_keeps_its_provider_verdict(self):
        Guarded.m_ref = self.m
        with self.assertRaises(self.m.ProviderFault) as c:
            self.grade(dict(GOOD, died=1), self.raising("provider_gpu_unavailable", provider=True))
        self.assertEqual(c.exception.name, "provider_gpu_unavailable")

    def test_a_pre_boot_refusal_keeps_its_class_only_with_a_healthy_broker_that_saw_nothing(self):
        Guarded.m_ref = self.m
        idle = dict(GOOD, accepted=0, attaches=0, commits=0, first_attach_s=None)
        with self.assertRaises(self.m.EvidenceError) as c:
            self.grade(idle, self.raising("apps_retry_media"))
        self.assertEqual(c.exception.name, "apps_retry_media")
        for sick in (dict(idle, died=1), dict(idle, errors=1), dict(idle, started=0)):
            with self.subTest(sick):
                with self.assertRaises(self.m.EvidenceError) as c:
                    self.grade(sick, self.raising("apps_retry_media"))
                self.assertTrue(c.exception.name.startswith("broker_"))
        # an operator retry is NOT a pre-boot refusal: the broker's silence wins
        with self.assertRaises(self.m.EvidenceError) as c:
            self.grade(idle, self.raising("apps_retry_operator"))
        self.assertEqual(c.exception.name, "broker_no_attach")

    def test_a_timeout_stays_a_timeout_and_a_passing_timeout_grade_is_untouched(self):
        Guarded.m_ref = self.m
        with self.assertRaises(self.m.EvidenceError) as c:
            self.grade(dict(GOOD, died=1), self.raising("apps"), rc=124)
        self.assertEqual(c.exception.name, "apps")

    def test_execute_stages_grades_a_broker_failure_as_fail_not_provider_fault(self):
        for d in (dict(GOOD, died=1), dict(GOOD, commits=0)):
            with self.subTest(d):
                self.out.write_text(B.format_summary("apps", d) + "\n")
                result = {"verdict": "FAIL", "suites": [], "hardware": {}, "host_reference": "MISSING"}
                stage = dict(name="apps", command=["x"], output=self.out, progress=[], detail=[],
                             grade=self.m.broker_guarded(self.ok, "apps", self.out))
                dest = Path(self.tmp.name) / "result.json"
                with patch.object(self.m, "refresh_claims"), patch.object(self.m, "print_final_summary"), \
                        patch.object(self.m, "first_failing_line", return_value=None), \
                        patch.object(self.m, "failed_tests", return_value=([], 0)), \
                        patch.object(self.m, "first_panic", return_value=None):
                    rc = self.m.execute_stages([stage], 10**15, result, dest, runner=lambda *a: 0)
                self.assertEqual(rc, 1)
                self.assertEqual(result["verdict"], "FAIL")
                failing = [r["name"] for r in result["suites"] if r["fail"]]
                self.assertEqual(len(failing), 1)
                self.assertTrue(failing[0].startswith("broker_"), failing)
                self.assertTrue(all(not r["skip"] for r in result["suites"]))


class WiredIntoMain(unittest.TestCase):
    setUp = v7.V7Tests.setUp
    _main_fixture = v7.V7Tests._main_fixture

    def commands_for(self, tier, broker=None, windows="1"):
        seen = {}
        original = self.m.watched

        def spy(command, root, name):
            seen[name.split("_", 1)[1]] = list(command)
            return original(command, root, name)

        env = {"KF_TIER": tier}
        if broker is not None:
            env["KF_BROKER"] = broker
        with patch.object(self.m, "watched", side_effect=spy):
            rc, result, calls = self._main_fixture(windows=windows, soft_ms=300 * 60000, extra_env=env, broker=True)
        return seen, result

    def test_small_has_no_broker_and_middle_large_wrap_the_guest_stages(self):
        seen, _ = self.commands_for("small")
        self.assertNotIn("broker_ci.py", " ".join(seen["cuda_guest"]))
        self.assertNotIn("broker", seen)
        for tier in ("middle", "large"):
            with self.subTest(tier):
                seen, result = self.commands_for(tier)
                cmd = seen["cuda_guest"]
                self.assertEqual(cmd[:2], ["python3", "-B"])
                self.assertIn("broker_ci.py", cmd[2])
                self.assertEqual(cmd[3:7], ["run", "--name", "cuda_guest", "--"])
                self.assertIn("scripts/ci/broker_ci.py", " ".join(seen["windows_gate"]))
                # without a BROKER_SUMMARY from the (fake) stage the stage FAILS as broker_start, never passes
                failing = [r["name"] for r in result["suites"] if r["fail"]]
                self.assertEqual(failing, ["broker_start"])
                # host and merge-bar stages are never wrapped
                self.assertNotIn("broker_ci.py", " ".join(seen["merge_bar"]))
                self.assertNotIn("broker_ci.py", " ".join(seen["cuda_host"]))

    def test_the_override_turns_it_off_for_middle_and_on_for_small(self):
        seen, result = self.commands_for("middle", broker="0")
        self.assertNotIn("broker_ci.py", " ".join(seen["cuda_guest"]))
        self.assertEqual([r["name"] for r in result["suites"] if r["fail"]], [])
        seen, _ = self.commands_for("small", broker="1")
        self.assertIn("broker_ci.py", " ".join(seen["cuda_guest"]))

    def test_the_explicit_stage_follows_the_windows_gate_and_needs_windows(self):
        m = payload()
        self.assertEqual(m.BROKER_GUEST_STAGES[:1], ("cuda_guest",))
        self.assertNotIn("merge_bar", m.BROKER_GUEST_STAGES)
        self.assertNotIn("cuda_host", m.BROKER_GUEST_STAGES)


class Explicit(unittest.TestCase):
    class FakeGuest:
        def __init__(self, sd=0.25, resize=True, logger=None, session=None):
            self.sd, self.resize, self.logger, self.session = sd, resize, logger, session
            self.started = False

        def screendump_stats(self):
            return 0.5, self.sd

        def set_resolution(self, w, h):
            if self.resize or (w, h) == (1920, 1080) and False:
                self.session.add("nvkvm-broker: TEST attach: id=9 %dx%d stride=1 offset=0 XR24 mod=0x0 size=1\n" % (w, h))

        def logger_start(self):
            self.started = True

        def logger_text(self):
            return self.logger if self.logger is not None else KFIN_OK

    class FakeSession:
        def __init__(self, text):
            self.text, self.sent = text, []

        def add(self, s):
            self.text += s

        def log_text(self):
            return self.text

        def send(self, line):
            self.sent.append(line)

    def run_it(self, text=LOG_OK, **kw):
        s = self.FakeSession(text)
        g = self.FakeGuest(session=s, **kw)
        tick = [0.0]

        def clock():
            tick[0] += 0.5
            return tick[0]

        return B.run_explicit(s, g, clock=clock, sleep=lambda x: None, frame_wait=5, resize_wait=5, input_wait=5), s

    def test_all_three_pass(self):
        rows, s = self.run_it()
        self.assertEqual([r["name"] for r in rows], ["broker_frames", "broker_resize", "broker_input"])
        self.assertEqual(s.sent, B.SCRIPT_LINES)

    def test_a_blank_first_frame_fails_frames(self):
        with self.assertRaises(B.BrokerFailure) as c:
            self.run_it(sd=0.0)
        self.assertEqual((c.exception.name, c.exception.rows), ("broker_frames", []))

    def test_no_commit_fails_frames(self):
        with self.assertRaises(B.BrokerFailure) as c:
            self.run_it(text="nvkvm-broker: accepted uid 0 pid 1\n")
        self.assertEqual(c.exception.name, "broker_frames")

    def test_no_new_attach_fails_resize(self):
        with self.assertRaises(B.BrokerFailure) as c:
            self.run_it(resize=False)
        self.assertEqual(c.exception.name, "broker_resize")
        self.assertEqual([r["name"] for r in c.exception.rows], ["broker_frames"])

    def test_missing_input_fails_input(self):
        for logger in ("KFIN START screen=1x1\n", KFIN_OK.replace("up inj", "dn inj"),
                       KFIN_OK.replace("x=959", "x=900"), KFIN_OK.replace("x=1799", "x=1000")):
            with self.subTest(logger[:60]):
                with self.assertRaises(B.BrokerFailure) as c:
                    self.run_it(logger=logger)
                self.assertEqual(c.exception.name, "broker_input")

    def test_the_tolerance_is_two_pixels(self):
        self.assertTrue(B.input_arrived(KFIN_OK))
        self.assertFalse(B.input_arrived(KFIN_OK.replace("x=959 y=539", "x=956 y=539")))

    def test_ppm_stats(self):
        flat = b"P6\n4 4 255\n" + bytes([128]) * 48
        mean, sd = G.ppm_stats(flat)
        self.assertAlmostEqual(mean, 128 / 255, 3)
        self.assertLess(sd, B.BLANK_SD)
        busy = b"P6\n4 4 255\n" + bytes([0, 255] * 24)
        self.assertGreater(G.ppm_stats(busy)[1], 0.4)
        with self.assertRaises(ValueError):
            G.ppm_stats(b"P5\n")

    def test_explicit_line_grading(self):
        ok = "BROKER_EXPLICIT frames=PASS resize=PASS input=PASS"
        self.assertEqual([r["name"] for r in B.explicit_rows(ok + "\n")], ["broker_frames", "broker_resize", "broker_input"])
        for line, want in (
            ("BROKER_EXPLICIT frames=FAIL resize=NOT_RUN input=NOT_RUN", "broker_frames"),
            ("BROKER_EXPLICIT frames=PASS resize=FAIL input=NOT_RUN", "broker_resize"),
            ("BROKER_EXPLICIT frames=PASS resize=PASS input=FAIL", "broker_input"),
            ("BROKER_EXPLICIT frames=FAIL resize=NOT_RUN input=NOT_RUN class=broker_start", "broker_start"),
            ("BROKER_EXPLICIT frames=PASS resize=PASS input=NOT_RUN", "broker_input"),
            ("", "broker_start"),
            (ok + "\n" + ok, "broker_start"),
        ):
            with self.subTest(line):
                with self.assertRaises(B.BrokerFailure) as c:
                    B.explicit_rows(line + "\n")
                self.assertEqual(c.exception.name, want)

    def test_format_explicit_names_the_failure(self):
        f = B.BrokerFailure("broker_resize", [B.row("broker_frames", 1)])
        self.assertEqual(B.format_explicit(f.rows, f), "BROKER_EXPLICIT frames=PASS resize=FAIL input=NOT_RUN")
        s = B.BrokerFailure("broker_start")
        self.assertEqual(B.format_explicit([], s), "BROKER_EXPLICIT frames=FAIL resize=NOT_RUN input=NOT_RUN class=broker_start")

    def test_payload_maps_explicit_failure_to_a_plain_fail(self):
        m = payload()
        with self.assertRaises(m.EvidenceError) as c:
            m.explicit_broker_rows("BROKER_EXPLICIT frames=PASS resize=PASS input=FAIL\n")
        self.assertEqual(c.exception.name, "broker_input")
        self.assertNotIsInstance(c.exception, m.ProviderFault)
        self.assertEqual([r["name"] for r in c.exception.rows], ["broker_frames", "broker_resize"])


class MoreCoverage(unittest.TestCase):
    """Second killers for the conditions the first tests pin only once (see broker_ci_mutants.py)."""

    def session(self, mode):
        tmp = tempfile.TemporaryDirectory()
        self.addCleanup(tmp.cleanup)
        work = Path(tmp.name)
        return B.BrokerSession(fake_broker(work, mode), work / "run", "s"), work

    # --- the harness side of the FIFO never blocks
    def test_the_harness_descriptor_is_non_blocking(self):
        s, _ = self.session("ok")
        self.assertTrue(s.start(wait=10))
        self.addCleanup(s.stop)
        self.assertFalse(os.get_blocking(s.fifo_fd))

    def test_a_broker_that_never_reads_cannot_block_the_harness(self):
        s, _ = self.session("noread")
        self.assertTrue(s.start(wait=10))
        self.addCleanup(s.stop)
        with self.assertRaises(BlockingIOError):
            for _ in range(2000000):
                s.send("x" * 200)

    def test_the_socket_directory_is_made_private_even_if_it_existed(self):
        s, work = self.session("ok")
        (work / "run").mkdir(mode=0o755)
        os.chmod(work / "run", 0o755)
        self.assertTrue(s.start(wait=10))
        self.addCleanup(s.stop)
        self.assertEqual(os.stat(work / "run").st_mode & 0o777, 0o700)

    def test_the_fifo_is_private(self):
        s, _ = self.session("ok")
        self.assertTrue(s.start(wait=10))
        self.addCleanup(s.stop)
        self.assertEqual(os.stat(s.fifo).st_mode & 0o777, 0o600)

    # --- run_stage
    def stage(self, mode, command, environ=None):
        with tempfile.TemporaryDirectory() as t:
            t = Path(t)
            fb = fake_broker(t, mode)
            out = io.StringIO()
            with patch.object(B, "START_WAIT_S", 1.0):
                rc = B.run_stage("apps", command, t, directory=t / "r", environ=environ or {"PATH": os.environ["PATH"]},
                                 binary_finder=lambda r: (fb, 1), out=out, tick=0.05)
        return rc, out.getvalue()

    def test_a_broker_that_exits_at_once_blocks_the_command_too(self):
        marker = tempfile.mktemp()
        rc, text = self.stage("exit0", [sys.executable, "-c", "open(%r,'w')" % marker])
        self.assertEqual(rc, 1)
        self.assertFalse(os.path.exists(marker))
        self.assertEqual(B.failure_class(B.parse_summary(text, "apps")), "broker_start")

    def test_an_exit_code_of_five_is_relayed_with_the_summary(self):
        rc, text = self.stage("ok", [sys.executable, "-c", "import time;time.sleep(0.3);raise SystemExit(5)"])
        self.assertEqual(rc, 5)
        self.assertIsNotNone(B.parse_summary(text, "apps"))

    def test_an_existing_kf3_extra_survives_into_the_command(self):
        marker = tempfile.mktemp()
        self.stage("ok", [sys.executable, "-c", "import os;open(%r,'w').write(os.environ['KF3_DEV_EXTRA'])" % marker],
                   {"PATH": os.environ["PATH"], "KF3_DEV_EXTRA": "guest-driver=580.159.04"})
        text = Path(marker).read_text()
        self.assertTrue(text.startswith("guest-driver=580.159.04,display=on,"), text)
        os.unlink(marker)

    # --- failure classes alone
    def test_a_leftover_broker_alone_fails(self):
        with self.assertRaises(B.BrokerFailure) as c:
            B.stage_rows(B.format_summary("apps", dict(GOOD, stopped=0)), "apps")
        self.assertEqual(c.exception.name, "broker_leftover")

    def test_a_late_attach_alone_fails(self):
        with self.assertRaises(B.BrokerFailure) as c:
            B.stage_rows(B.format_summary("apps", dict(GOOD, first_attach_s=B.ATTACH_LIMIT_S + 5)), "apps")
        self.assertEqual(c.exception.name, "broker_late_attach")

    def test_the_summary_of_another_stage_is_not_ours(self):
        with self.assertRaises(B.BrokerFailure) as c:
            B.stage_rows(B.format_summary("cuda_guest", GOOD), "apps")
        self.assertEqual(c.exception.name, "broker_start")

    def test_two_summaries_fail_as_broker_start(self):
        line = B.format_summary("apps", GOOD)
        with self.assertRaises(B.BrokerFailure) as c:
            B.stage_rows(line + "\n" + line + "\n", "apps")
        self.assertEqual(c.exception.name, "broker_start")

    def test_the_override_is_popped_before_any_stage_runs(self):
        seen = []
        t = WiredIntoMain("run")
        t.setUp()
        t._main_fixture(windows="0", soft_ms=300 * 60000, extra_env={"KF_TIER": "small", "KF_BROKER": "0"}, broker=True,
                        probe=lambda env: seen.append("KF_BROKER" in env))
        self.assertTrue(seen)
        self.assertFalse(any(seen))

    # --- explicit stage, the conditions one by one
    class Guest(Explicit.FakeGuest):
        def __init__(self, restore=True, **kw):
            super().__init__(**kw)
            self.restore = restore

        def set_resolution(self, w, h):
            if self.restore or (w, h) != (1920, 1080):
                self.session.add("nvkvm-broker: TEST attach: id=9 %dx%d stride=1 offset=0 XR24 mod=0x0 size=1\n" % (w, h))

    def explicit(self, text=LOG_OK, **kw):
        s = Explicit.FakeSession(text)
        g = self.Guest(session=s, **kw)
        tick = [0.0]

        def clock():
            tick[0] += 0.5
            return tick[0]

        return B.run_explicit(s, g, clock=clock, sleep=lambda x: None, frame_wait=5, resize_wait=5, input_wait=5)

    def test_a_resize_that_does_not_come_back_fails_resize(self):
        with self.assertRaises(B.BrokerFailure) as c:
            self.explicit(restore=False)
        self.assertEqual(c.exception.name, "broker_resize")

    def test_a_failed_restore_keeps_the_frames_row_and_names_resize(self):
        with self.assertRaises(B.BrokerFailure) as c:
            self.explicit(restore=False)
        self.assertEqual([r["name"] for r in c.exception.rows], ["broker_frames"])
        self.assertEqual(c.exception.name, "broker_resize")

    def test_an_attach_without_any_commit_fails_frames(self):
        with self.assertRaises(B.BrokerFailure) as c:
            self.explicit(text="accepted uid 0\nTEST attach: id=1 1920x1080 stride=1 offset=0 XR24 mod=0x0 size=1\n")
        self.assertEqual(c.exception.name, "broker_frames")

    def test_pixels_blank_decides_on_the_deviation(self):
        self.assertTrue(B.pixels_blank(0.5, 0.0))
        self.assertTrue(B.pixels_blank(0.5, B.BLANK_SD / 2))
        self.assertFalse(B.pixels_blank(0.5, 0.25))
        self.assertFalse(B.pixels_blank(0.0, 0.05))

    def test_moves_without_keys_fail_input(self):
        moves_only = "\n".join(l for l in KFIN_OK.splitlines() if " M " in l or "START" in l)
        with self.assertRaises(B.BrokerFailure) as c:
            self.explicit(logger=moves_only)
        self.assertEqual(c.exception.name, "broker_input")

    def test_keys_without_moves_fail_input(self):
        keys_only = "\n".join(l for l in KFIN_OK.splitlines() if " K " in l or "START" in l)
        with self.assertRaises(B.BrokerFailure) as c:
            self.explicit(logger=keys_only)
        self.assertEqual(c.exception.name, "broker_input")

    def test_a_key_down_without_its_up_fails_input(self):
        self.assertFalse(B.input_arrived("\n".join(l for l in KFIN_OK.splitlines() if "up inj" not in l)))

    def test_a_key_up_before_any_down_fails_input(self):
        lines = KFIN_OK.splitlines()
        swapped = [l for l in lines if "up inj" in l] + [l for l in lines if "down inj" in l and "vk=0x41" in l]
        self.assertFalse(B.input_arrived("\n".join(l for l in lines if " M " in l) + "\n" + "\n".join(swapped)))

    def test_the_wrong_key_does_not_count(self):
        self.assertFalse(B.input_arrived(KFIN_OK.replace("vk=0x41", "vk=0x42")))

    def test_a_missing_explicit_line_is_broker_start(self):
        with self.assertRaises(B.BrokerFailure) as c:
            B.explicit_rows("some other output\n")
        self.assertEqual(c.exception.name, "broker_start")

    def test_a_duplicated_explicit_line_is_broker_start(self):
        ok = "BROKER_EXPLICIT frames=PASS resize=PASS input=PASS\n"
        with self.assertRaises(B.BrokerFailure) as c:
            B.explicit_rows(ok + ok)
        self.assertEqual(c.exception.name, "broker_start")

    def test_a_carried_class_names_the_failure(self):
        for cls in ("broker_start", "broker_build"):
            with self.assertRaises(B.BrokerFailure) as c:
                B.explicit_rows("BROKER_EXPLICIT frames=FAIL resize=NOT_RUN input=NOT_RUN class=%s\n" % cls)
            self.assertEqual(c.exception.name, cls)

    def test_a_not_run_step_is_a_failure_not_a_pass(self):
        for line, want in (("frames=PASS resize=NOT_RUN input=NOT_RUN", "broker_resize"),
                           ("frames=PASS resize=PASS input=NOT_RUN", "broker_input")):
            with self.assertRaises(B.BrokerFailure) as c:
                B.explicit_rows("BROKER_EXPLICIT " + line + "\n")
            self.assertEqual(c.exception.name, want)

    def test_payload_explicit_failures_are_plain_fails_for_each_step(self):
        m = payload()
        for line, want in (("frames=FAIL resize=NOT_RUN input=NOT_RUN", "broker_frames"),
                           ("frames=PASS resize=FAIL input=NOT_RUN", "broker_resize"),
                           ("frames=PASS resize=PASS input=FAIL", "broker_input")):
            with self.assertRaises(m.EvidenceError) as c:
                m.explicit_broker_rows("BROKER_EXPLICIT " + line + "\n")
            self.assertEqual(c.exception.name, want)
            self.assertNotIsInstance(c.exception, m.ProviderFault)

    def test_a_missing_explicit_output_is_broker_start_in_the_payload(self):
        m = payload()
        with self.assertRaises(m.EvidenceError) as c:
            m.explicit_broker_rows(m.broker_text(Path("/nonexistent/broker.run")))
        self.assertEqual(c.exception.name, "broker_start")


class MoreGuarded(Guarded):
    """The ruling's edges, each pinned by two tests."""

    def test_provider_fault_wins_even_in_execute_stages(self):
        Guarded.m_ref = self.m
        self.out.write_text(B.format_summary("apps", dict(GOOD, died=1)) + "\n")
        result = {"verdict": "FAIL", "suites": [], "hardware": {}, "host_reference": "MISSING"}
        stage = dict(name="apps", command=["x"], output=self.out, progress=[], detail=[],
                     grade=self.m.broker_guarded(self.raising("provider_gpu_unavailable", provider=True), "apps", self.out))
        with patch.object(self.m, "refresh_claims"), patch.object(self.m, "print_final_summary"), \
                patch.object(self.m, "first_failing_line", return_value=None), \
                patch.object(self.m, "failed_tests", return_value=([], 0)), patch.object(self.m, "first_panic", return_value=None):
            self.m.execute_stages([stage], 10**15, result, Path(self.tmp.name) / "r.json", runner=lambda *a: 0)
        self.assertEqual(result["verdict"], "PROVIDER_FAULT")

    def test_an_operator_retry_is_never_a_pre_boot_refusal(self):
        Guarded.m_ref = self.m
        idle = dict(GOOD, accepted=0, attaches=0, commits=0, first_attach_s=None)
        for guest in ("apps_retry_operator", "windows_apps_retry_hostref", "apps_retry_provider"):
            with self.subTest(guest):
                with self.assertRaises(self.m.EvidenceError) as c:
                    self.grade(idle, self.raising(guest))
                self.assertEqual(c.exception.name, "broker_no_attach")

    def test_a_pre_boot_refusal_never_excuses_a_dead_or_frameless_broker(self):
        Guarded.m_ref = self.m
        for d, want in ((dict(GOOD, died=1), "broker_died"), (dict(GOOD, commits=0), "broker_no_frames"),
                        (dict(GOOD, errors=1), "broker_error"), (dict(GOOD, stopped=0), "broker_leftover")):
            with self.subTest(want):
                with self.assertRaises(self.m.EvidenceError) as c:
                    self.grade(d, self.raising("apps_retry_media"))
                self.assertEqual(c.exception.name, want)
                self.assertNotIsInstance(c.exception, self.m.ProviderFault)

    def test_the_guest_path_is_a_plain_evidence_error(self):
        Guarded.m_ref = self.m
        with self.assertRaises(self.m.EvidenceError) as c:
            self.grade(dict(GOOD, died=1), self.raising("apps_retry_operator"))
        self.assertNotIsInstance(c.exception, self.m.ProviderFault)
        self.assertNotIn("retry", c.exception.name)

    def test_a_timeout_with_no_summary_stays_the_guests_error(self):
        Guarded.m_ref = self.m
        with self.assertRaises(self.m.EvidenceError) as c:
            self.grade(None, self.raising("timeout_guest"), rc=124)
        self.assertEqual(c.exception.name, "timeout_guest")

    def test_two_summaries_make_a_passing_guest_fail(self):
        line = B.format_summary("apps", GOOD) + "\n"
        self.out.write_text(line + line)
        with self.assertRaises(self.m.EvidenceError) as c:
            self.m.broker_guarded(self.ok, "apps", self.out)(0)
        self.assertEqual(c.exception.name, "broker_start")


class ExplicitStageInMain(unittest.TestCase):
    setUp = v7.V7Tests.setUp
    _main_fixture = v7.V7Tests._main_fixture

    def run_it(self, tier="middle", windows="1", broker=None):
        env = {"KF_TIER": tier}
        if broker is not None:
            env["KF_BROKER"] = broker
        # the wrapped stages' own broker grading is covered above; here only the stage list is looked at
        with patch.object(self.m, "broker_guarded", side_effect=lambda grade, name, output: grade):
            return self._main_fixture(windows=windows, soft_ms=300 * 60000, extra_env=env, broker=True)

    def test_it_runs_right_after_the_windows_gate(self):
        rc, result, calls = self.run_it()
        self.assertEqual(calls[-2:], ["windows_gate", "broker"])

    def test_without_a_broker_verdict_line_it_fails_as_broker_start(self):
        rc, result, calls = self.run_it()
        self.assertEqual(rc, 1)
        self.assertEqual([r["name"] for r in result["suites"] if r["fail"]], ["broker_start"])
        self.assertEqual(result["verdict"], "FAIL")

    def test_not_for_small_not_without_windows_not_when_overridden_off(self):
        for kw in (dict(tier="small"), dict(windows="0"), dict(broker="0")):
            with self.subTest(kw):
                rc, result, calls = self.run_it(**kw)
                self.assertNotIn("broker", calls)


KFIN_OK = """KFIN START screen=1536x864
KFIN 7797 M msg=0x200 x=959 y=539 data=0
KFIN 8234 M msg=0x200 x=1799 y=999 data=0
KFIN 8484 K vk=0x41 sc=0x1e down inj=0
KFIN 8734 K vk=0x41 sc=0x1e up inj=0
"""


if __name__ == "__main__":
    unittest.main()
