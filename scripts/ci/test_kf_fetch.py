"""Loopback-only behavioural tests for the isolated stdlib fetcher."""
import hashlib
import http.server
import importlib.util
import json
import os
from pathlib import Path
import re
import socket
import subprocess
import sys
import tempfile
import threading
import time
import unittest

HERE = Path(__file__).resolve().parent
SOURCE = Path(os.environ.get("KF_FETCH_SOURCE", HERE / "kf_fetch.py"))
DATA = bytes(range(251)) * 10000
PIN = hashlib.sha256(DATA).hexdigest()
CHUNK = 1024 * 1024


class Fixture:
    def __init__(self, **options):
        self.data = DATA
        self.etag = '"v1"'
        self.options = options
        self.requests = []
        self.failures = {}
        self.mints = 0
        self.active = self.peak = 0
        self.lock = threading.Lock()
        fixture = self

        class Handler(http.server.BaseHTTPRequestHandler):
            def log_message(self, *args):
                pass

            def do_HEAD(self):
                self.serve(head=True)

            def do_GET(self):
                self.serve()

            def serve(self, head=False):
                path = self.path.split("?", 1)[0]
                span = self.headers.get("Range")
                with fixture.lock:
                    fixture.requests.append((self.path, span, dict(self.headers)))
                    if path in ("/worker/object.bin", "/chain") or path.startswith("/bench-data/"):
                        fixture.mints += 1
                        mint = fixture.mints
                if path in ("/worker/object.bin", "/chain") or path.startswith("/bench-data/"):
                    if fixture.options.get("origin_reset") and mint == 1:
                        self.connection.shutdown(socket.SHUT_RDWR)
                        return
                    if fixture.options.get("origin_500") and mint == 1:
                        self.send_response(500)
                        self.end_headers()
                        return
                    if fixture.options.get("origin_auth"):
                        self.send_response(403)
                        self.end_headers()
                        return
                    self.send_response(302)
                    location = "/again" if path == "/chain" else f"/object.bin?signature=private-query&v={mint}"
                    self.send_header("Location", fixture.url.replace("http:", "https:") + location if fixture.options.get("https_location") else location)
                    self.end_headers()
                    return
                if path == "/again":
                    self.send_response(302)
                    self.send_header("Location", "/object.bin?signature=second-query")
                    self.end_headers()
                    return
                opts = fixture.options
                probe = span == "bytes=0-0" or head
                if opts.get("auth") or (opts.get("expire") and not probe and
                                         (opts.get("forever") or "v=1" in self.path)):
                    self.send_response(403)
                    self.end_headers()
                    self.wfile.write(b"<Message>Request has expired</Message>" if not opts.get("auth") else b"AccessDenied")
                    return
                with fixture.lock:
                    count = fixture.failures.get(span, 0)
                    fixture.failures[span] = count + 1
                if not probe and opts.get("flaky") and count == 0:
                    self.send_response(500)
                    self.end_headers()
                    return
                data = fixture.data
                total = len(data)
                ranges = not opts.get("no_ranges")
                if ranges and span:
                    start, end = map(int, re.fullmatch(r"bytes=(\d+)-(\d+)", span).groups())
                    if start >= total:
                        self.send_response(416)
                        self.send_header("Content-Range", f"bytes */{total}")
                        self.end_headers()
                        return
                    end = min(end, total - 1)
                    body = data[start:end + 1]
                    self.send_response(206)
                    offset = 1 if opts.get("wrong_range") and not probe else 0
                    self.send_header("Content-Range", f"bytes {start + offset}-{end}/{total}")
                else:
                    start = 0
                    body = data
                    self.send_response(200)
                if opts.get("corrupt") and not probe:
                    body = bytes([body[0] ^ 1]) + body[1:]
                if opts.get("encoding"):
                    self.send_header("Content-Encoding", "gzip")
                self.send_header("Content-Length", str(len(body) + (1 if opts.get("short") and not probe else 0)))
                self.send_header("ETag", fixture.etag)
                self.end_headers()
                if head:
                    return
                with fixture.lock:
                    fixture.active += 1
                    fixture.peak = max(fixture.peak, fixture.active)
                try:
                    if opts.get("slow") and not probe:
                        # Ensure the first completed chunk can be checkpointed before others finish.
                        time.sleep(0.03 if start == 0 else 0.8)
                    if opts.get("reset") and not probe and count == int(bool(opts.get("flaky"))):
                        self.wfile.write(body[:100])
                        self.wfile.flush()
                        self.connection.shutdown(socket.SHUT_RDWR)
                        return
                    self.wfile.write(body)
                except (BrokenPipeError, ConnectionResetError, OSError):
                    pass
                finally:
                    with fixture.lock:
                        fixture.active -= 1

        self.server = http.server.ThreadingHTTPServer(("127.0.0.1", 0), Handler)
        self.server.daemon_threads = True
        self.thread = threading.Thread(target=self.server.serve_forever, kwargs={"poll_interval": 0.02})

    def __enter__(self):
        self.thread.start()
        self.url = f"http://127.0.0.1:{self.server.server_port}"
        return self

    def __exit__(self, *args):
        self.server.shutdown()
        self.server.server_close()
        self.thread.join()

    def chunks(self):
        return [r for r in self.requests if r[1] != "bytes=0-0" and r[0].startswith("/object.bin")]


class FetchTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        self.out = Path(self.temp.name) / "out.bin"
        self.state = Path(self.temp.name) / "state.json"

    def command(self, url, *extra, pin=PIN, streams=3):
        return [sys.executable, "-I", str(SOURCE), url, str(self.out), "--sha256", pin,
                "--streams", str(streams), "--chunk-mb", "1", "--state", str(self.state), *extra]

    def run_fetch(self, server, *extra, route="/object.bin?signature=private-query", pin=PIN, streams=3):
        p = subprocess.run(self.command(server.url + route, *extra, pin=pin, streams=streams),
                           capture_output=True, text=True, timeout=20)
        self.assertNotIn("private-query", p.stdout + p.stderr)
        self.assertNotIn(server.url, p.stdout + p.stderr)
        return p

    def assert_good(self, p, data=DATA):
        self.assertEqual(p.returncode, 0, p.stdout + p.stderr)
        self.assertEqual(self.out.read_bytes(), data)
        self.assertIn("FETCH_DONE secs=", p.stdout)
        self.assertIn("FETCH_PROGRESS bytes=", p.stdout)

    def seed(self, server, *, size=None, etag=None, completed=None, truncate=None):
        self.out.write_bytes(DATA if truncate is None else DATA[:truncate])
        record = {"version": 1, "size": len(DATA) if size is None else size,
                  "etag": server.etag if etag is None else etag, "chunk_size": CHUNK,
                  "sha256": PIN, "source": hashlib.sha256((server.url + "/object.bin").encode()).hexdigest(),
                  "completed": {"0": hashlib.sha256(DATA[:CHUNK]).hexdigest()} if completed is None else completed}
        self.state.write_text(json.dumps(record))

    def test_parallel_ranges_and_final_pin(self):
        with Fixture(slow=True) as server:
            self.assert_good(self.run_fetch(server))
            self.assertEqual(len(server.chunks()), 3)
            self.assertGreaterEqual(server.peak, 2)
            self.assertIn("streams=3", self.run_fetch(server).stdout)

    def test_single_stream_ranges(self):
        with Fixture() as server:
            self.assert_good(self.run_fetch(server, streams=1))
            self.assertEqual(len(server.chunks()), 3)

    def test_no_range_falls_back_to_one_stream(self):
        with Fixture(no_ranges=True) as server:
            p = self.run_fetch(server)
            self.assert_good(p)
            self.assertIn("streams=1", p.stdout)
            self.assertEqual(len(server.chunks()), 1)

    def test_no_range_never_requests_chunk_offsets(self):
        with Fixture(no_ranges=True) as server:
            p = self.run_fetch(server, streams=8)
            self.assert_good(p)
            self.assertIn("streams=1", p.stdout)
            self.assertTrue(all(span is None for _, span, _ in server.chunks()))

    def test_flaky_500_then_reset_retries_each_chunk(self):
        with Fixture(flaky=True, reset=True) as server:
            self.assert_good(self.run_fetch(server))
            self.assertEqual(len(server.chunks()), 9)

    def test_reset_retries_without_duplicating_output(self):
        with Fixture(reset=True) as server:
            self.assert_good(self.run_fetch(server, streams=1))
            self.assertEqual(len(server.chunks()), 6)

    def test_redirect_is_resolved_once(self):
        with Fixture() as server:
            self.assert_good(self.run_fetch(server, "--follow-redirect", route="/worker/object.bin"))
            self.assertEqual(server.mints, 1)
            self.assertEqual(len(server.chunks()), 3)

    def test_redirect_origin_500_does_not_spend_more_mints(self):
        with Fixture(origin_500=True) as server:
            p = self.run_fetch(server, "--follow-redirect", route="/worker/object.bin")
            self.assertNotEqual(p.returncode, 0)
            self.assertIn("FETCH_FAIL class=NETWORK", p.stdout)
            self.assertEqual(server.mints, 1)

    def test_redirect_origin_reset_does_not_spend_more_mints(self):
        with Fixture(origin_reset=True) as server:
            p = self.run_fetch(server, "--follow-redirect", route="/worker/object.bin")
            self.assertNotEqual(p.returncode, 0)
            self.assertIn("FETCH_FAIL class=NETWORK", p.stdout)
            self.assertEqual(server.mints, 1)

    def test_redirect_requires_opt_in(self):
        with Fixture() as server:
            p = self.run_fetch(server, route="/worker/object.bin")
            self.assertNotEqual(p.returncode, 0)
            self.assertIn("FETCH_FAIL class=NETWORK", p.stdout)
            self.assertFalse(server.chunks())

    def test_redirect_chain_refused(self):
        with Fixture() as server:
            p = self.run_fetch(server, "--follow-redirect", route="/chain")
            self.assertNotEqual(p.returncode, 0)
            self.assertFalse(server.chunks())

    def test_redirect_chain_logs_no_location(self):
        with Fixture() as server:
            p = self.run_fetch(server, "--follow-redirect", route="/chain")
            self.assertNotIn("second-query", p.stdout + p.stderr)
            self.assertIn("FETCH_FAIL class=NETWORK", p.stdout)

    def test_expiring_url_refresh_is_shared_by_workers(self):
        with Fixture(expire=True) as server:
            self.assert_good(self.run_fetch(server, "--follow-redirect", route="/worker/object.bin"))
            self.assertEqual(server.mints, 2)

    def test_expiring_url_refresh_single_stream(self):
        with Fixture(expire=True) as server:
            self.assert_good(self.run_fetch(server, "--follow-redirect", route="/worker/object.bin", streams=1))
            self.assertEqual(server.mints, 2)

    def test_refresh_budget_is_two(self):
        with Fixture(expire=True, forever=True) as server:
            p = self.run_fetch(server, "--follow-redirect", route="/worker/object.bin")
            self.assertNotEqual(p.returncode, 0)
            self.assertIn("FETCH_FAIL class=AUTH", p.stdout)
            self.assertEqual(server.mints, 3)

    def test_refresh_budget_single_stream(self):
        with Fixture(expire=True, forever=True) as server:
            p = self.run_fetch(server, "--follow-redirect", route="/worker/object.bin", streams=1)
            self.assertNotEqual(p.returncode, 0)
            self.assertEqual(server.mints, 3)

    def test_auth_failure_does_not_refresh(self):
        with Fixture(auth=True) as server:
            p = self.run_fetch(server, "--follow-redirect", route="/worker/object.bin")
            self.assertNotEqual(p.returncode, 0)
            self.assertIn("FETCH_FAIL class=AUTH", p.stdout)
            self.assertEqual(server.mints, 1)

    def test_corrupt_chunk_is_bad_hash_and_retained(self):
        with Fixture(corrupt=True) as server:
            p = self.run_fetch(server)
            self.assertNotEqual(p.returncode, 0)
            self.assertIn("FETCH_BAD_HASH", p.stdout)
            self.assertIn("FETCH_FAIL class=HASH", p.stdout)
            self.assertTrue(self.out.exists())
            self.assertTrue(self.state.exists())

    def test_wrong_pin_is_bad_hash_and_retained(self):
        with Fixture() as server:
            p = self.run_fetch(server, pin="0" * 64)
            self.assertNotEqual(p.returncode, 0)
            self.assertIn("FETCH_BAD_HASH", p.stdout)
            self.assertEqual(self.out.read_bytes(), DATA)

    def test_bad_hash_checkpoint_is_invalidated_for_retry(self):
        with Fixture(corrupt=True) as server:
            self.assertNotEqual(self.run_fetch(server).returncode, 0)
            server.options.clear()
            server.requests.clear()
            self.assert_good(self.run_fetch(server))
            self.assertEqual(len(server.chunks()), 3)

    def test_resume_skips_only_verified_completed_chunks(self):
        with Fixture() as server:
            self.seed(server)
            self.assert_good(self.run_fetch(server))
            self.assertEqual(len(server.chunks()), 2)
            self.assertNotIn("bytes=0-1048575", [r[1] for r in server.chunks()])

    def test_resume_truncated_file_rechecks_bounds(self):
        with Fixture() as server:
            self.seed(server, truncate=20)
            self.assert_good(self.run_fetch(server))
            self.assertEqual(len(server.chunks()), 3)

    def test_resume_partly_truncated_chunk_is_refetched(self):
        with Fixture() as server:
            self.seed(server, completed={"1": hashlib.sha256(DATA[CHUNK:2*CHUNK]).hexdigest()}, truncate=CHUNK + 10)
            self.assert_good(self.run_fetch(server))
            self.assertEqual(len(server.chunks()), 3)

    def test_resume_local_corruption_is_refetched(self):
        with Fixture() as server:
            self.seed(server)
            with self.out.open("r+b") as f:
                f.write(b"wrong")
            self.assert_good(self.run_fetch(server))
            self.assertEqual(len(server.chunks()), 3)

    def test_etag_change_restarts(self):
        with Fixture() as server:
            self.seed(server, etag='"old"')
            self.assert_good(self.run_fetch(server))
            self.assertEqual(len(server.chunks()), 3)

    def test_size_change_restarts(self):
        with Fixture() as server:
            self.seed(server, size=len(DATA) + 10)
            self.assert_good(self.run_fetch(server))
            self.assertEqual(len(server.chunks()), 3)

    def test_invalid_chunk_indices_do_not_count_as_completed(self):
        with Fixture() as server:
            self.seed(server, completed={"-1": PIN, "999999": PIN})
            self.assert_good(self.run_fetch(server))
            self.assertEqual(len(server.chunks()), 3)

    def test_broken_json_restarts(self):
        with Fixture() as server:
            self.state.write_text("{")
            self.assert_good(self.run_fetch(server))

    def test_larger_pinned_size_is_refused_before_allocation(self):
        with Fixture() as server:
            self.out.write_bytes(b"keep existing output")
            p = self.run_fetch(server, "--size", str(len(DATA) + 1))
            self.assertNotEqual(p.returncode, 0)
            self.assertIn("FETCH_FAIL class=SIZE", p.stdout + p.stderr)
            self.assertEqual(self.out.read_bytes(), b"keep existing output")
            self.assertFalse(server.chunks())

    def test_smaller_pinned_size_is_refused_before_allocation(self):
        with Fixture() as server:
            self.out.write_bytes(b"keep existing output")
            p = self.run_fetch(server, "--size", str(len(DATA) - 1))
            self.assertNotEqual(p.returncode, 0)
            self.assertIn("FETCH_FAIL class=SIZE", p.stdout + p.stderr)
            self.assertEqual(self.out.read_bytes(), b"keep existing output")
            self.assertFalse(server.chunks())

    def test_wrong_content_range_is_size_failure(self):
        with Fixture(wrong_range=True) as server:
            p = self.run_fetch(server)
            self.assertNotEqual(p.returncode, 0)
            self.assertIn("FETCH_FAIL class=SIZE", p.stdout)
            self.assertNotIn("FETCH_DONE", p.stdout)

    def test_wrong_content_range_single_stream(self):
        with Fixture(wrong_range=True) as server:
            p = self.run_fetch(server, streams=1)
            self.assertNotEqual(p.returncode, 0)
            self.assertIn("FETCH_FAIL class=SIZE", p.stdout)

    def test_short_body_never_completes(self):
        with Fixture(short=True) as server:
            p = self.run_fetch(server)
            self.assertNotEqual(p.returncode, 0)
            self.assertNotIn("FETCH_DONE", p.stdout)

    def test_encoded_body_refused(self):
        with Fixture(encoding=True) as server:
            p = self.run_fetch(server)
            self.assertNotEqual(p.returncode, 0)
            self.assertIn("FETCH_FAIL class=SIZE", p.stdout)

    def test_empty_object(self):
        with Fixture() as server:
            server.data = b""
            self.assert_good(self.run_fetch(server, pin=hashlib.sha256(b"").hexdigest()), b"")

    def test_resume_after_sigkill(self):
        with Fixture(slow=True) as server:
            p = subprocess.Popen(self.command(server.url + "/object.bin?signature=private-query", streams=1),
                                 stdout=subprocess.PIPE, stderr=subprocess.PIPE, text=True)
            try:
                until = time.monotonic() + 10
                while time.monotonic() < until:
                    try:
                        if json.loads(self.state.read_text())["completed"]:
                            break
                    except (FileNotFoundError, json.JSONDecodeError):
                        pass
                    time.sleep(0.01)
                else:
                    self.fail("no checkpoint before timeout")
                p.kill()
                p.communicate(timeout=5)
            finally:
                if p.poll() is None:
                    p.kill()
                    p.communicate()
            server.options.clear()
            server.requests.clear()
            self.assert_good(self.run_fetch(server, streams=1))
            self.assertNotIn("bytes=0-1048575", [r[1] for r in server.chunks()])

    def test_stdin_url_and_origin_headers_never_forwarded(self):
        with Fixture() as server:
            env = {**os.environ, "KF_RUN_TOKEN": "fixture-token", "KF_ACCESS_ID": "fixture-id", "KF_ACCESS_SECRET": "fixture-secret"}
            p = subprocess.run(self.command("-", "--follow-redirect", "--worker-auth"), env=env,
                               input=server.url + "/worker/object.bin\n", capture_output=True, text=True, timeout=20)
            self.assert_good(p)
            origin = server.requests[0][2]
            self.assertEqual(origin.get("Authorization"), "Bearer fixture-token")
            for _, _, headers in server.requests[1:]:
                self.assertNotIn("Authorization", headers)
                self.assertNotIn("Cf-Access-Client-Secret", headers)
            self.assertNotIn("fixture-token", p.stdout + p.stderr + self.state.read_text())
            self.assertNotIn("private-query", self.state.read_text())

    def test_worker_credentials_stay_on_origin_during_refresh(self):
        with Fixture(expire=True) as server:
            env = {**os.environ, "KF_RUN_TOKEN": "fixture-token", "KF_ACCESS_ID": "fixture-id", "KF_ACCESS_SECRET": "fixture-secret"}
            p = subprocess.run(self.command("-", "--follow-redirect", "--worker-auth"), env=env,
                               input=server.url + "/worker/object.bin\n", capture_output=True, text=True, timeout=20)
            self.assert_good(p)
            for path, _, headers in server.requests:
                if path.startswith("/worker/"):
                    self.assertEqual(headers.get("Authorization"), "Bearer fixture-token")
                    self.assertEqual(headers.get("Cf-Access-Client-Id"), "fixture-id")
                    self.assertEqual(headers.get("Cf-Access-Client-Secret"), "fixture-secret")
                else:
                    self.assertNotIn("Authorization", headers)
                    self.assertNotIn("Cf-Access-Client-Id", headers)
                    self.assertNotIn("Cf-Access-Client-Secret", headers)

    def test_etag_disappearing_restarts(self):
        with Fixture() as server:
            self.seed(server, etag=None)
            record = json.loads(self.state.read_text())
            record["etag"] = None
            self.state.write_text(json.dumps(record))
            self.assert_good(self.run_fetch(server))
            self.assertEqual(len(server.chunks()), 3)

    def test_size_change_from_smaller_object_restarts(self):
        with Fixture() as server:
            self.seed(server, size=len(DATA) - 100)
            self.assert_good(self.run_fetch(server))
            self.assertEqual(len(server.chunks()), 3)

    def test_resume_second_chunk_corruption_is_refetched(self):
        with Fixture() as server:
            self.seed(server, completed={"1": hashlib.sha256(DATA[CHUNK:2*CHUNK]).hexdigest()})
            with self.out.open("r+b") as f:
                f.seek(CHUNK + 9)
                f.write(b"wrong")
            self.assert_good(self.run_fetch(server))
            self.assertEqual(len(server.chunks()), 3)

    def test_rotated_signed_query_can_resume_same_object(self):
        with Fixture() as server:
            self.seed(server)
            self.assert_good(self.run_fetch(server, route="/object.bin?signature=another-query"))
            self.assertEqual(len(server.chunks()), 2)

    def test_json_list_is_ignored_as_invalid_state(self):
        with Fixture() as server:
            self.state.write_text("[]")
            self.assert_good(self.run_fetch(server))

    def test_default_state_path(self):
        with Fixture() as server:
            command = self.command(server.url + "/object.bin")
            index = command.index("--state")
            del command[index:index+2]
            p = subprocess.run(command, capture_output=True, text=True, timeout=20)
            self.assert_good(p)
            record = json.loads(Path(str(self.out) + ".fetch-state.json").read_text())
            self.assertEqual(len(record["completed"]), 3)

    def test_partial_pwrite_advances_offset(self):
        from unittest.mock import patch
        spec = importlib.util.spec_from_file_location("fetch_unit", SOURCE)
        module = importlib.util.module_from_spec(spec)
        spec.loader.exec_module(module)
        actual = os.pwrite
        with self.out.open("w+b") as output, patch.object(module.os, "pwrite", side_effect=lambda fd, data, offset: actual(fd, data[:3], offset)):
            module.write_at(output.fileno(), b"abcdefghij", 7)
        self.assertEqual(self.out.read_bytes(), b"\0" * 7 + b"abcdefghij")

    def test_one_byte_pwrite_keeps_all_bytes(self):
        from unittest.mock import patch
        spec = importlib.util.spec_from_file_location("fetch_unit", SOURCE)
        module = importlib.util.module_from_spec(spec)
        spec.loader.exec_module(module)
        actual = os.pwrite
        with self.out.open("w+b") as output, patch.object(module.os, "pwrite", side_effect=lambda fd, data, offset: actual(fd, data[:1], offset)):
            module.write_at(output.fileno(), b"abcde", 2)
        self.assertEqual(self.out.read_bytes(), b"\0\0abcde")

    def test_cli_errors_do_not_echo_secrets(self):
        p = subprocess.run(self.command("https://example.invalid/object?signature=private-query", "--typo=private-query"),
                           capture_output=True, text=True)
        self.assertNotEqual(p.returncode, 0)
        self.assertNotIn("private-query", p.stdout + p.stderr)


if __name__ == "__main__":
    unittest.main()
