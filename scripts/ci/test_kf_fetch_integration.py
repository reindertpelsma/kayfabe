"""Local transfers and process stubs for the merged pinned download entry points."""
import importlib.util
import json
import os
from pathlib import Path
import shutil
import subprocess
import sys
import tempfile
import unittest
from unittest.mock import patch

from test_kf_fetch import DATA, PIN, Fixture

HERE = Path(__file__).resolve().parent
ROOT = HERE.parents[1]
SETUP_SOURCE = Path(os.environ.get("KF_FETCH_SETUP_SOURCE", HERE / "vast_setup.py"))
BENCH_SOURCE = Path(os.environ.get("KF_FETCH_BENCH_SOURCE", ROOT / "scripts/bench/box/fetch_bench_data.sh"))
FETCH_SOURCE = Path(os.environ.get("KF_FETCH_SOURCE", HERE / "kf_fetch.py"))


class SetupIntegration(unittest.TestCase):
    def setUp(self):
        spec = importlib.util.spec_from_file_location("setup_fetch_integration", SETUP_SOURCE)
        self.setup = importlib.util.module_from_spec(spec)
        spec.loader.exec_module(self.setup)
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        self.out = Path(self.temp.name) / "cache.tar.gz"
        self.asset = {"name": "cache.tar.gz", "size": len(DATA), "sha256": PIN}
        self.locator = {"url": "https://release-assets.githubusercontent.com/object.bin?signature=fixture-query",
                        "name": "cache.tar.gz", "size": len(DATA)}
        self.calls = []

    def download(self, server, *, mode=8):
        actual_run = subprocess.run

        def local_run(command, **kwargs):
            self.calls.append((command, kwargs))
            self.assertIn("-I", command)
            self.assertEqual(command[3], "-")
            self.assertNotIn(self.locator["url"], command)
            self.assertEqual(kwargs["input"].decode(), self.locator["url"] + "\n")
            self.assertNotIn("--follow-redirect", command)
            self.assertNotIn("--worker-auth", command)
            self.assertIn("--size", command)
            self.assertEqual(command[command.index("--size") + 1], str(self.asset["size"]))
            command = list(command)
            command[2] = str(FETCH_SOURCE)
            kwargs["input"] = (server.url + "/object.bin?signature=fixture-query\n").encode()
            return actual_run(command, **kwargs)

        with patch.dict(os.environ, {} if mode is None else {"KF_FETCH_STREAMS": str(mode)}):
            if mode is None:
                os.environ.pop("KF_FETCH_STREAMS", None)
            # A baseline that bypasses the subprocess may NOT touch a public endpoint.
            with patch.object(self.setup.urllib.request, "build_opener", side_effect=AssertionError("unexpected networking")), patch.object(self.setup.subprocess, "run", side_effect=local_run):
                self.setup.download_asset(self.locator, self.asset, self.out)

    def test_opt_in_uses_eight_streams_and_pinned_bytes(self):
        with Fixture() as server:
            self.download(server, mode=8)
            self.assertEqual(self.out.read_bytes(), DATA)
            args = self.calls[0][0]
            self.assertEqual(args[args.index("--streams") + 1], "8")
            self.assertEqual(args[args.index("--sha256") + 1], PIN)
            self.assertFalse(self.out.with_suffix(".partial").exists())

    def test_stream_override_is_honoured(self):
        with Fixture() as server:
            self.download(server, mode=4)
            self.assertEqual(self.out.read_bytes(), DATA)
            args = self.calls[0][0]
            self.assertEqual(args[args.index("--streams") + 1], "4")

    def test_parallel_wrong_hash_keeps_failure_verdict(self):
        with Fixture(corrupt=True) as server:
            with self.assertRaises(self.setup.CacheRefused):
                self.download(server)
            self.assertFalse(self.out.exists())
            self.assertFalse(self.out.with_suffix(".partial").exists())
            self.assertEqual(len(self.calls), 1)

    def test_parallel_wrong_size_keeps_failure_verdict(self):
        with Fixture() as server:
            self.asset["size"] += 1
            self.locator["size"] += 1
            with self.assertRaises(self.setup.CacheRefused):
                self.download(server)
            self.assertFalse(self.out.exists())
            self.assertEqual(len(self.calls), 1)

    def test_parallel_smaller_pinned_size_keeps_failure_verdict(self):
        with Fixture() as server:
            self.asset["size"] -= 1
            self.locator["size"] -= 1
            with self.assertRaises(self.setup.CacheRefused):
                self.download(server)
            self.assertFalse(self.out.exists())
            self.assertEqual(len(self.calls), 1)

    def bad_fetcher(self, body):
        def fake_success(command, **kwargs):
            Path(command[4]).write_bytes(body)
            return subprocess.CompletedProcess(command, 0)
        with patch.dict(os.environ, {"KF_FETCH_STREAMS": "8"}), patch.object(self.setup.subprocess, "run", side_effect=fake_success):
            with self.assertRaises(self.setup.CacheRefused):
                self.setup.download_asset(self.locator, self.asset, self.out)
        self.assertFalse(self.out.exists())
        self.assertFalse(self.out.with_suffix(".partial").exists())

    def test_successful_fetch_process_with_wrong_size_is_refused(self):
        self.bad_fetcher(DATA[:-1])

    def test_successful_fetch_process_with_wrong_hash_is_refused(self):
        self.bad_fetcher(b"X" + DATA[1:])

    def test_bad_locator_is_rejected_before_fetcher(self):
        with Fixture() as server:
            self.locator["url"] = server.url + "/object.bin"
            with self.assertRaises(self.setup.CacheRefused):
                self.download(server)
            self.assertFalse(self.calls)
            self.assertFalse(server.requests)

    def legacy_download(self, mode):
        from test_vast_setup_cache import Opener
        opener = Opener(DATA)
        with patch.dict(os.environ, {}):
            if mode is None:
                os.environ.pop("KF_FETCH_STREAMS", None)
            else:
                os.environ["KF_FETCH_STREAMS"] = str(mode)
            with patch.object(self.setup.urllib.request, "build_opener", return_value=opener) as build, patch.object(self.setup.subprocess, "run", side_effect=AssertionError("no fetch subprocess")):
                self.setup.download_asset(self.locator, self.asset, self.out)
            self.assertIsInstance(build.call_args.args[0], self.setup.NoRedirect)
            self.assertEqual(len(opener.requests), 1)
        self.assertEqual(self.out.read_bytes(), DATA)
        request, timeout = opener.requests[0]
        call = (request.get_method(), request.full_url, request.headers, timeout)
        self.assertEqual(call, ("GET", self.locator["url"], {}, 30))
        return call

    def test_default_uses_established_opener_path(self):
        self.legacy_download(None)

    def test_default_matches_explicit_one_request(self):
        self.assertEqual(self.legacy_download(None), self.legacy_download(1))

    def test_single_stream_uses_established_opener_path(self):
        self.legacy_download(1)

    def test_failed_fetcher_does_not_leak_locator_in_exception(self):
        with Fixture(short=True) as server:
            with self.assertRaises(self.setup.CacheRefused) as caught:
                self.download(server)
            self.assertNotIn("fixture-query", str(caught.exception))
            self.assertEqual(len(self.calls), 1)


class BenchIntegration(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name)
        self.bin = self.root / "bin"
        self.bin.mkdir()
        self.out = self.root / "result.bin"
        actual_curl = shutil.which("curl")
        self.assertIsNotNone(actual_curl)
        # Only the loopback fixture's transport is rewritten; production still requires
        # HTTPS. Record argv, never stdin, so the test can verify capability hygiene.
        for name, body in {
            "curl": f'''#!{sys.executable}
import json,os,subprocess,sys
with open(os.environ["CURL_ARGV_LOG"], "a") as log:
    log.write(json.dumps(sys.argv[1:]) + "\\n")
args = [x.replace("=https", "=http") for x in sys.argv[1:]]
data = sys.stdin.buffer.read().replace(b"https://127.0.0.1", b"http://127.0.0.1")
raise SystemExit(subprocess.run([{actual_curl!r}, *args], input=data).returncode)
''',
            "python3": f'''#!{sys.executable}
import json,os,subprocess,sys
with open(os.environ["FETCH_ARGV_LOG"], "a") as log:
    log.write(json.dumps(sys.argv[1:]) + "\\n")
args = sys.argv[1:]
args[1] = os.environ["FETCH_TEST_SOURCE"]
data = sys.stdin.buffer.read().replace(b"https://127.0.0.1", b"http://127.0.0.1")
raise SystemExit(subprocess.run([sys.executable, *args], input=data).returncode)
''',
        }.items():
            script = self.bin / name
            script.write_text(body)
            script.chmod(0o700)

    def run_bench(self, server, *, pin=PIN, mode=8, attempts=3):
        env = {**os.environ, "PATH": str(self.bin) + ":" + os.environ["PATH"],
               "KF_BROKER_URL": server.url.replace("http:", "https:"),
               "KF_RUN_ID": "123-1", "KF_RUN_TOKEN": "fixture-token", "KF_ACCESS_ID": "", "KF_ACCESS_SECRET": "",
               "KF_CI_ENV_FILE": "/nonexistent", "KF_BENCH_RETRY_SLEEP": "0",
               "FETCH_ARGV_LOG": str(self.root / "argv.jsonl"), "CURL_ARGV_LOG": str(self.root / "curl.jsonl"), "FETCH_TEST_SOURCE": str(FETCH_SOURCE)}
        if mode is None:
            env.pop("KF_FETCH_STREAMS", None)
        else:
            env["KF_FETCH_STREAMS"] = str(mode)
        result = subprocess.run(["bash", str(BENCH_SOURCE), "object.bin", "--sha256", pin,
                                 "--out", str(self.out), "--attempts", str(attempts)], env=env, capture_output=True, text=True, timeout=30)
        self.assertNotIn("private-query", result.stdout + result.stderr)
        self.assertNotIn("fixture-token", result.stdout + result.stderr)
        self.assertNotIn(server.url, result.stdout + result.stderr)
        return result

    def test_file_mode_parallel_opt_in_verifies_and_renames(self):
        with Fixture(https_location=True) as server:
            p = self.run_bench(server, mode=8)
            self.assertEqual(p.returncode, 0, p.stdout + p.stderr)
            self.assertEqual(self.out.read_bytes(), DATA)
            calls = (self.root / "argv.jsonl").read_text()
            self.assertNotIn("private-query", calls)
            self.assertNotIn("fixture-token", calls)
            self.assertIn('"--streams", "8"', calls)
            self.assertEqual(server.mints, 1)
            self.assertFalse(Path(str(self.out) + ".part").exists())

    def legacy_curl(self, server, mode):
        p = self.run_bench(server, mode=mode)
        self.assertEqual(p.returncode, 0, p.stdout + p.stderr)
        self.assertEqual(self.out.read_bytes(), DATA)
        self.assertFalse((self.root / "argv.jsonl").exists(), "legacy mode must not invoke kf_fetch.py")
        calls = [json.loads(line) for line in (self.root / "curl.jsonl").read_text().splitlines()]
        downloads = [call for call in calls if "--retry" in call]
        # Exact single-stream command, including retry, resume, timeouts, protocol,
        # no-proxy, stdin URL configuration and J2's numeric timing output.
        expected = ["--config", "-", "--fail", "--silent", "--show-error",
                    "--proto", "=https", "--noproxy", "*", "--max-redirs", "0",
                    "--user-agent", "kayfabe-bench/1.0 (+https://github.com/reindertpelsma/kayfabe)",
                    "-C", "-", "--retry", "5", "--retry-delay", "5", "--connect-timeout", "30",
                    "--speed-limit", "1024", "--speed-time", "120", "--write-out",
                    r"%{stderr}\nKF_DOWNLOAD_METRIC seconds=%{time_total} bytes=%{size_download}\n",
                    "--output", str(self.out) + ".part"]
        self.assertEqual(downloads, [expected])
        return calls

    def test_file_mode_default_uses_exact_legacy_curl_command(self):
        with Fixture(https_location=True) as server:
            self.legacy_curl(server, None)

    def test_file_mode_one_uses_exact_legacy_curl_command(self):
        with Fixture(https_location=True) as server:
            self.legacy_curl(server, 1)

    def test_file_mode_default_matches_explicit_one_command(self):
        with Fixture(https_location=True) as server:
            default = self.legacy_curl(server, None)
            self.out.unlink()
            (self.root / "curl.jsonl").write_text("")
            self.assertEqual(default, self.legacy_curl(server, 1))

    def test_file_mode_hash_failure_is_exit_three_without_retry(self):
        with Fixture(https_location=True) as server:
            p = self.run_bench(server, pin="0" * 64)
            self.assertEqual(p.returncode, 3, p.stdout + p.stderr)
            self.assertFalse(self.out.exists())
            self.assertFalse(Path(str(self.out) + ".part").exists())
            self.assertEqual(server.mints, 1)

    def test_file_mode_corrupt_chunk_is_exit_three_without_retry(self):
        with Fixture(https_location=True, corrupt=True) as server:
            p = self.run_bench(server)
            self.assertEqual(p.returncode, 3, p.stdout + p.stderr)
            self.assertFalse(self.out.exists())
            self.assertEqual(server.mints, 1)

    def test_file_mode_stream_override_reaches_fetcher(self):
        with Fixture(https_location=True) as server:
            p = self.run_bench(server, mode=4)
            self.assertEqual(p.returncode, 0, p.stdout + p.stderr)
            self.assertIn('"--streams", "4"', (self.root / "argv.jsonl").read_text())

    def test_file_mode_flaky_endpoint_retries_without_reminting(self):
        with Fixture(https_location=True, flaky=True, reset=True) as server:
            p = self.run_bench(server)
            self.assertEqual(p.returncode, 0, p.stdout + p.stderr)
            self.assertEqual(self.out.read_bytes(), DATA)
            self.assertEqual(server.mints, 1)

    def test_invalid_zero_streams_is_refused_before_mint(self):
        with Fixture(https_location=True) as server:
            p = self.run_bench(server, mode=0, attempts=1)
            self.assertEqual(p.returncode, 2, p.stdout + p.stderr)
            self.assertEqual(server.mints, 0)
            self.assertFalse(server.requests)

    def test_excess_streams_is_refused_before_mint(self):
        with Fixture(https_location=True) as server:
            p = self.run_bench(server, mode=257, attempts=1)
            self.assertEqual(p.returncode, 2, p.stdout + p.stderr)
            self.assertEqual(server.mints, 0)
            self.assertFalse(server.requests)

    def test_file_mode_auth_failure_still_exits_four(self):
        with Fixture(https_location=True, auth=True) as server:
            # Refuse the origin too: exercise the established Worker refusal mapping.
            server.options["origin_auth"] = True
            p = self.run_bench(server)
            self.assertEqual(p.returncode, 4, p.stdout + p.stderr)
            self.assertFalse(self.out.exists())


if __name__ == "__main__":
    unittest.main()
