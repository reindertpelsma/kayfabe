"""V8 rental regression: provider readiness is an acknowledged report, not elapsed time."""

import json
import os
from pathlib import Path
import shutil
import threading
import tempfile
import unittest
from unittest.mock import patch

import test_vast_box as box
import vast_setup as setup

setUpModule = box.setUpModule
tearDownModule = box.tearDownModule
HERE = Path(__file__).resolve().parent


class ReadinessSignal(unittest.TestCase):
    def signal(self, body):
        read_fd, write_fd = os.pipe()
        try:
            os.write(write_fd, body)
            os.close(write_fd)
            write_fd = None
            with patch.dict(os.environ, {"KF_BROKER_READY_FD": str(read_fd)}):
                setup.wait_for_ready()
        finally:
            for fd in [read_fd, write_fd]:
                if fd is not None:
                    try:
                        os.close(fd)
                    except OSError:
                        pass

    def test_acknowledged_progress_releases_cache_fetch(self):
        self.signal(b"READY\n")

    def test_refused_report_cannot_release_cache_fetch(self):
        with self.assertRaises(setup.CacheRefused):
            self.signal(b"REFUSED\n")

    def test_arbitrary_bytes_cannot_release_cache_fetch(self):
        with self.assertRaises(setup.CacheRefused):
            self.signal(b"garbage\n")

    def test_closed_channel_cannot_release_cache_fetch(self):
        with self.assertRaises(setup.CacheRefused):
            self.signal(b"")

    def test_missing_channel_fails_closed(self):
        with patch.dict(os.environ, {}, clear=True):
            with self.assertRaises(setup.CacheRefused):
                setup.wait_for_ready()

    def test_standard_input_cannot_be_used_as_the_channel(self):
        with patch.dict(os.environ, {"KF_BROKER_READY_FD": "0"}):
            with self.assertRaises(setup.CacheRefused):
                setup.wait_for_ready()

    def test_channel_blocks_until_the_ack_event(self):
        read_fd, write_fd = os.pipe()
        finished, errors = threading.Event(), []

        def consume():
            try:
                with patch.dict(os.environ, {"KF_BROKER_READY_FD": str(read_fd)}):
                    setup.wait_for_ready()
            except Exception as error:
                errors.append(type(error).__name__)
            finally:
                finished.set()

        thread = threading.Thread(target=consume)
        thread.start()
        try:
            self.assertFalse(finished.wait(0.05))
            os.write(write_fd, b"READY\n")
            self.assertTrue(finished.wait(2))
            self.assertEqual(errors, [])
        finally:
            os.close(write_fd)
            thread.join(2)
            try:
                os.close(read_fd)
            except OSError:
                pass


class ReadinessIntegration(unittest.TestCase):
    def invoke(self, faults, accepted, invalid_token=False, permanent_status=None):
        case = box.Onstart("test_pass_run_posts_result_and_exits_zero")
        case.setUp()
        try:
            broker = case.broker()
            broker.faults["report"] = faults
            if invalid_token:
                broker.token = "different-run-token"
            original = broker.handle

            def handle(path, headers, body, method="POST"):
                reply = original(path, headers, body, method)
                if permanent_status is not None and reply[0] == 400:
                    reply = permanent_status, reply[1], reply[2]
                if reply[0] == 200 and json.loads(body).get("kind") == "progress":
                    (case.tmp / "dump.ack").write_text("acknowledged")
                return reply

            broker.handle = handle
            wait = (
                "python3 -c 'import runpy; "
                's=runpy.run_path("scripts/ci/vast_setup.py",run_name="readiness_import"); '
                's["wait_for_ready"]()' + "' || exit 2\n"
                'test -f "$KF_TEST_DUMP.ack" || exit 3\n'
            )
            root = case.tree(wait + case.OK_PAYLOAD)
            shutil.copyfile(HERE / "vast_setup.py", root / "scripts/ci/vast_setup.py")
            process = case.onstart(
                root,
                broker,
                env_extra={"KF_SOFT_DEADLINE": str(int(__import__("time").time()) + 10)},
            )
            if accepted:
                self.assertEqual(process.returncode, 0, process.stdout + process.stderr)
                self.assertTrue((case.tmp / "dump").exists())
                self.assertEqual(broker.results()[0]["verdict"], "PASS")
            else:
                self.assertEqual(process.returncode, 2, process.stdout + process.stderr)
                self.assertFalse((case.tmp / "dump").exists())
        finally:
            case.tearDown()

    def test_not_ready_progress_must_be_acknowledged_before_cache_fetch(self):
        self.invoke(["not_ready", "not_ready"], True)

    def test_lost_progress_ack_cannot_release_cache_fetch(self):
        self.invoke(["500_after_apply"], True)

    def test_unauthorised_progress_never_starts_cache_fetch(self):
        self.invoke(["401"], False)

    def test_rejected_run_token_never_releases_cache_fetch(self):
        self.invoke([], False, invalid_token=True)

    def test_rejected_progress_request_refuses_cache_without_waiting_for_deadline(self):
        self.invoke(["400"], False)

    def test_oversized_progress_request_refuses_cache_without_waiting_for_deadline(self):
        self.invoke(["400"], False, permanent_status=413)


class RestoreReadiness(unittest.TestCase):
    def invoke(self, signal):
        with tempfile.TemporaryDirectory() as d:
            root = Path(d)
            (root / "scripts/ci").mkdir(parents=True)
            (root / "scripts/ci/setup-cache.lock.json").write_text(
                '{"assets":[{"name":"debs.tar.gz"}],"tag":"cache"}'
            )
            env = {}
            fd = writer = None
            if signal is not None:
                fd, writer = os.pipe()
                os.write(writer, signal)
                os.close(writer)
                env["KF_BROKER_READY_FD"] = str(fd)
            try:
                with patch.dict(os.environ, env, clear=True), patch.object(
                    setup, "validate_lock"
                ), patch.object(setup, "policy_digest", return_value="policy"), patch.object(
                    setup.subprocess,
                    "check_output",
                    side_effect=AssertionError("locator called before acknowledgement"),
                ) as mint:
                    with self.assertRaises(setup.CacheRefused):
                        setup.restore(root, root / "cache")
                    mint.assert_not_called()
            finally:
                if fd is not None:
                    try:
                        os.close(fd)
                    except OSError:
                        pass

    def test_restore_refuses_before_mint_when_report_was_refused(self):
        self.invoke(b"REFUSED\n")

    def test_restore_refuses_before_mint_without_the_readiness_channel(self):
        self.invoke(None)
