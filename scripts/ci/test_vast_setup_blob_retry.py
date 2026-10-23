"""Owner-approved blob GET retries: the provider can still report created during onstart."""
import json
import os
from pathlib import Path
import signal
import subprocess
import tempfile
import time
import unittest

import test_vast_box as box

setUpModule = box.setUpModule
tearDownModule = box.tearDownModule
HERE = Path(__file__).resolve().parent


class BlobRetry(unittest.TestCase):
    def run_helper(self, root, url, path=None, timeout=8):
        envfile = root / "env"
        envfile.write_text(
            'export KF_BROKER_URL=' + json.dumps(url) + '\n'
            'export KF_RUN_ID=' + json.dumps(box.RUN) + '\n'
            'export KF_RUN_TOKEN=' + json.dumps(box.TOKEN) + '\n'
        )
        env = box.base_env({
            "KF_CI_ENV_FILE": str(envfile), "KF_TIMEOUT_ARGS": str(root / "timeout-args"),
        })
        if path:
            env["PATH"] = str(path) + ":" + env["PATH"]
        started = time.monotonic()
        process = subprocess.Popen(
            ["sh", str(HERE / "kf-box-blob-url"), "setup-cache-test", "debs.tar.gz"],
            env=env, stdout=subprocess.PIPE, stderr=subprocess.PIPE, start_new_session=True,
        )
        try:
            out, err = process.communicate(timeout=timeout)
        except subprocess.TimeoutExpired:
            os.killpg(process.pid, signal.SIGKILL)
            process.communicate()
            raise
        return process.returncode, out, err, time.monotonic() - started

    def server_case(self, statuses):
        with tempfile.TemporaryDirectory() as d:
            root = Path(d)
            broker = box.Broker()
            calls = []
            todo = list(statuses)
            result = {"name": "debs.tar.gz", "size": 4,
                      "url": "https://release-assets.githubusercontent.com/dummy"}

            def handle(path, headers, body, method="POST"):
                self.assertEqual(method, "GET")
                self.assertEqual(headers.get("Authorization"), "Bearer " + box.TOKEN)
                self.assertTrue(path.endswith("/blob-url/" + box.RUN + "/setup-cache-test/debs.tar.gz"))
                calls.append(time.monotonic())
                code = todo.pop(0) if todo else 200
                return code, result if code == 200 else {"state": "UNAUTHORIZED"}, {}

            broker.handle = handle
            try:
                rc, out, err, _ = self.run_helper(root, broker.url)
                self.assertEqual(rc, 0, err.decode())
                self.assertEqual(json.loads(out), result)
                self.assertEqual(len(calls), len(statuses))
                return calls
            finally:
                broker.stop()

    def test_created_401_is_retried_until_running(self):
        calls = self.server_case([401, 200])
        self.assertGreaterEqual(calls[1] - calls[0], 0.8)

    def test_multiple_created_401_responses_use_backoff(self):
        calls = self.server_case([401, 401, 200])
        self.assertGreaterEqual(calls[2] - calls[1], 1.8)

    def test_provider_503_responses_use_backoff(self):
        calls = self.server_case([503, 503, 200])
        self.assertGreaterEqual(calls[2] - calls[1], 1.8)

    def short_outer_timer(self, root):
        path = root / "timeout"
        path.write_text(
            '#!/bin/sh\nprintf "%s\\n" "$@" > "$KF_TIMEOUT_ARGS"\n'
            'signal=$1; shift\nrequested=$1; shift\n'
            'exec /usr/bin/timeout "$signal" 0.15s "$@"\n'
        )
        path.chmod(0o700)

    def assert_outer_budget(self, root):
        args = (root / "timeout-args").read_text().splitlines()
        self.assertEqual(args[0], "--signal=KILL")
        self.assertGreater(float(args[1].removesuffix("s")), 0)
        self.assertLessEqual(float(args[1].removesuffix("s")), 300)

    def test_outer_deadline_kills_a_stuck_transport(self):
        with tempfile.TemporaryDirectory() as d:
            root = Path(d)
            self.short_outer_timer(root)
            curl = root / "curl"
            curl.write_text('#!/bin/sh\ntrap "" TERM\nsleep 2\nprintf "eventually"\n')
            curl.chmod(0o700)
            rc, _, _, seconds = self.run_helper(root, "https://broker.example/prefix", path=root)
            self.assertNotEqual(rc, 0)
            self.assertLess(seconds, 1.5)
            self.assert_outer_budget(root)

    def test_outer_deadline_bounds_a_permanent_created_401(self):
        with tempfile.TemporaryDirectory() as d:
            root = Path(d)
            self.short_outer_timer(root)
            broker = box.Broker()
            broker.handle = lambda *args: (401, {"state": "UNAUTHORIZED"}, {})
            try:
                rc, _, _, seconds = self.run_helper(root, broker.url, path=root)
                self.assertNotEqual(rc, 0)
                self.assertLess(seconds, 1.5)
                self.assert_outer_budget(root)
            finally:
                broker.stop()
