#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later
"""GPU-free regression tests for the Windows gate's QMP event-logger attach (fix 4300fc5d8).

Failure it pins: GA106 gate run dca106a (2026-11-04) lost its boot-1 event logger because ONE attach attempt
timed out while QEMU's main loop was busy in kf3's realize; no events file existed, the run loop read the
guest's reboot as `reason=no-events-file` and started no second QEMU. The fix is two halves:

  1. `qmp.py events` retries the attach (KF_QMP_EVENTS_ATTACH_S) and closes a half-open client;
  2. `windows_gate.sh` classifies a run loop that ended with `no-events-file` as OPERATOR (RETRY), not FAIL.

Deterministic: the QMP socket and the clock are stubbed, nothing sleeps.
"""
import importlib.util
import io
import json
import os
import re
import subprocess
import tempfile
import unittest
from pathlib import Path
from unittest.mock import Mock, patch

ROOT = Path(__file__).resolve().parents[3]
QMP = Path(os.environ.get("KF_QMP_PY") or ROOT / "scripts/bench/windows/qmp.py")
GATE = Path(os.environ.get("KF_WINDOWS_GATE") or ROOT / "scripts/bench/box/windows_gate.sh")

spec = importlib.util.spec_from_file_location("windows_qmp_attach", QMP)
qmp = importlib.util.module_from_spec(spec)
spec.loader.exec_module(qmp)


class Clock:
    """A fake clock: sleep() advances it, so a retry loop terminates without waiting."""

    def __init__(self):
        self.now = 1000.0

    def time(self):
        return self.now

    def sleep(self, s):
        self.now += s


def run_events(opens, attach_s, out):
    """qmp.events() with qmp_open scripted by `opens` (an exception, or 'ok'), and a one-event stream."""
    clock = Clock()
    calls = []

    def fake_open(path, timeout=10.0):
        calls.append(timeout)
        step = opens[min(len(calls) - 1, len(opens) - 1)]
        if step != "ok":
            raise step
        return Mock(), Mock()

    stream = iter([{"event": "SHUTDOWN", "data": {"reason": "guest-reset"}}, EOFError()])

    def fake_recv(f):
        r = next(stream)
        if isinstance(r, Exception):
            raise r
        return r

    with patch.object(qmp, "qmp_open", fake_open), patch.object(qmp, "recv", fake_recv), \
            patch.object(qmp.time, "time", clock.time), patch.object(qmp.time, "sleep", clock.sleep), \
            patch.object(qmp.sys, "stderr", new_callable=io.StringIO) as err:
        rc = qmp.events("fixture.sock", out, attach_s)
    return rc, calls, err.getvalue()


class EventsAttach(unittest.TestCase):
    def setUp(self):
        self.d = tempfile.TemporaryDirectory()
        self.addCleanup(self.d.cleanup)
        self.out = os.path.join(self.d.name, "events.jsonl")

    def lines(self):
        return [json.loads(x) for x in Path(self.out).read_text().splitlines()]

    def test_a_late_greeting_is_retried_until_it_attaches(self):
        busy = TimeoutError("greeting timed out while QEMU realized kf3")
        rc, calls, _ = run_events([busy, busy, busy, "ok"], 120.0, self.out)
        self.assertEqual(rc, 0)
        self.assertEqual(len(calls), 4, "three failed attaches did not end the logger")
        ev = [r["event"] for r in self.lines()]
        self.assertEqual(ev, ["KF_LOGGER_ATTACHED", "SHUTDOWN"])

    def test_every_attach_failure_kind_the_fix_names_is_retried(self):
        for exc in (OSError("refused"), EOFError(), ValueError("bad json"), RuntimeError("not a QMP greeting")):
            with self.subTest(exc=type(exc).__name__):
                rc, calls, _ = run_events([exc, "ok"], 120.0, self.out)
                self.assertEqual((rc, len(calls)), (0, 2))
                os.unlink(self.out)

    def test_it_gives_up_at_the_deadline_and_never_creates_the_file(self):
        # "no-events-file" must keep meaning "never attached": nothing may touch the file before the attach.
        rc, calls, err = run_events([OSError("refused")], 3.0, self.out)
        self.assertEqual(rc, 1)
        self.assertGreater(len(calls), 1)
        self.assertLess(len(calls), 20, "bounded by the deadline, not endless")
        self.assertFalse(os.path.exists(self.out))
        self.assertIn("not attached after", err)

    def test_the_attach_window_comes_from_the_environment_with_a_safe_default(self):
        for env, want in (({"KF_QMP_EVENTS_ATTACH_S": "7"}, 7.0), ({"KF_QMP_EVENTS_ATTACH_S": "junk"}, 120.0), ({}, 120.0)):
            clock = Clock()
            opens = []

            def fake_open(path, timeout=10.0):
                opens.append(clock.now)
                raise OSError("down")

            with patch.dict(os.environ, env, clear=False), patch.object(qmp, "qmp_open", fake_open), \
                    patch.object(qmp.time, "time", clock.time), patch.object(qmp.time, "sleep", clock.sleep), \
                    patch.object(qmp.sys, "stderr", new_callable=io.StringIO):
                if "KF_QMP_EVENTS_ATTACH_S" not in env:
                    os.environ.pop("KF_QMP_EVENTS_ATTACH_S", None)
                qmp.events("fixture.sock", self.out)
            span = opens[-1] - opens[0]
            self.assertGreaterEqual(span, want - 1.0, env)
            self.assertLessEqual(span, want + 1.0, env)

    def test_a_failed_handshake_closes_the_half_open_client(self):
        # a QMP server socket serves one client at a time: a leaked half-open client blocks every retry
        for greeting in ({"return": {}}, ):
            s, f = Mock(), Mock()
            with patch.object(qmp, "connect", return_value=(s, f)), patch.object(qmp, "recv", return_value=greeting):
                with self.assertRaises(RuntimeError):
                    qmp.qmp_open("fixture.sock")
            s.close.assert_called_once()
        s, f = Mock(), Mock()
        with patch.object(qmp, "connect", return_value=(s, f)), patch.object(qmp, "recv", side_effect=EOFError):
            with self.assertRaises(EOFError):
                qmp.qmp_open("fixture.sock")
        s.close.assert_called_once()


class HalfOpenClientOnARealSocket(unittest.TestCase):
    def test_a_handshake_that_times_out_leaves_no_client_behind(self):
        """A server that never greets (QEMU busy in realize): the failed attach must hang up, so the
        single-client QMP socket is free for the retry."""
        import socket

        with tempfile.TemporaryDirectory() as d:
            path = os.path.join(d, "qmp.sock")
            srv = socket.socket(socket.AF_UNIX, socket.SOCK_STREAM)
            srv.bind(path)
            srv.listen(1)
            self.addCleanup(srv.close)
            made = []
            real_connect = qmp.connect

            def spy(p, timeout):
                s, f = real_connect(p, timeout)
                made.append(s)  # keeps the socket alive: only an explicit close() can end it
                return s, f

            with patch.object(qmp, "connect", spy):
                with self.assertRaises((OSError, EOFError, ValueError)):
                    qmp.qmp_open(path, timeout=0.2)
            self.assertEqual(len(made), 1)
            self.assertEqual(made[0].fileno(), -1, "the failed attach closed its socket")


def reboot_wait_loop():
    """The gate's `until grep ... QEMU_START boot=2` loop, cut out of windows_gate.sh."""
    text = GATE.read_text()
    m = re.search(r'(until grep -q "QEMU_START tag=\$TAG boot=2".*?\ndone)\n', text, re.S)
    assert m, "the reboot wait loop moved"
    return m[1]


class GateClassification(unittest.TestCase):
    def run_loop(self, log):
        with tempfile.TemporaryDirectory() as d:
            Path(d, "logs").mkdir()
            Path(d, "logs", "T_run.log").write_text(log)
            script = (
                "TAG=T; VM=%s\nfail_op() { echo \"FAIL_OP $*\"; exit 0; }\n"
                "fail_guest() { echo \"FAIL_GUEST $*\"; exit 0; }\n"
                "cap() { echo 10; }\nsleep() { :; }\n%s\necho REBOOTED\n" % (d, reboot_wait_loop())
            )
            return subprocess.run(["bash", "-c", script], capture_output=True, text=True, timeout=30).stdout

    def test_a_run_loop_that_never_saw_the_reboot_is_the_harnesss_fault(self):
        out = self.run_loop("QEMU_START tag=T boot=1\nWINVM_RUN_END tag=T boots=1 reason=no-events-file\n")
        self.assertIn("FAIL_OP", out)
        self.assertNotIn("FAIL_GUEST", out)

    def test_a_guest_that_really_did_not_restart_is_still_the_guests(self):
        out = self.run_loop("QEMU_START tag=T boot=1\n")
        self.assertIn("FAIL_GUEST", out)
        self.assertNotIn("FAIL_OP", out)

    def test_the_no_events_file_branch_is_an_operator_fail_in_the_script(self):
        text = GATE.read_text()
        m = re.search(r'reason=no-events-file" "[^\n]*; then\n\s*(\w+) ', text)
        self.assertIsNotNone(m, "the no-events-file branch moved")
        self.assertEqual(m[1], "fail_op")

    def test_another_tags_run_end_line_is_not_ours(self):
        out = self.run_loop("QEMU_START tag=T boot=1\nWINVM_RUN_END tag=OTHER boots=1 reason=no-events-file\n")
        self.assertIn("FAIL_GUEST", out)

    def test_a_restart_passes_the_wait(self):
        out = self.run_loop("QEMU_START tag=T boot=1\nQEMU_START tag=T boot=2\n")
        self.assertIn("REBOOTED", out)


if __name__ == "__main__":
    unittest.main()
