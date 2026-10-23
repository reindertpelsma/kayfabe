#!/usr/bin/env python3
"""GPU-free, network-free falsifiers for the repo-side Vast box scripts.

A fake broker (TLS on 127.0.0.1, self-signed) records every request. The scripts under test
are vast_onstart.sh, vast_report.py, kf-box-kill and kf-box-blob-url.
"""

import hashlib
import http.server
import importlib.util
import json
import os
import shutil
import socket
import ssl
import stat
import subprocess
import sys
import tempfile
import threading
import time
import unittest
from pathlib import Path

SCRIPTS = Path(__file__).resolve().parent
FILES = ["vast_onstart.sh", "vast_report.py", "kf-box-kill"]
RUN = "4242-7"
# A run token is a compact HS256 JWT (box protocol 7); the box never verifies it.
TOKEN = ".".join(
    [
        "eyJhbGciOiJIUzI1NiJ9",
        "eyJydW4iOiI0MjQyIiwiYXR0ZW1wdCI6IjcifQ",
        ("Zk9_-Qw" * 7)[:43],
    ]
)
NONCE = "ab" * 16
SHA = "1234abcd" * 5
RNONCE = "cd" * 32
ACCESS = {"KF_ACCESS_ID": "0123abcd" * 4 + ".access", "KF_ACCESS_SECRET": "9f8e7d6c" * 8}
PREFIX = "/pfx-9f3a7c"
HARDWARE = {
    "gpu_name": "NVIDIA GeForce RTX 3060",
    "compute_cap": 860,
    "driver_version": "580.159.04",
}
ROWS = [{"name": "smoke", "pass": 3, "fail": 0, "skip": 0}]


def fast(budget="6"):
    args = ["--poll", "0.05", "--flush", "0.2", "--min-gap", "0.05", "--retry-base", "0.1"]
    return args + ["--retry-cap", "0.4", "--rate-backoff", "0.2", "--final-budget", budget]


FAST = fast()
TOOLS = ["bash", "sh", "env", "python3", "git", "timeout", "install", "mkdir", "chmod", "cat"]
TOOLS += ["sleep", "date", "rm", "mv", "dirname", "tr", "setsid", "nohup", "kill", "ls", "head"]
TOOLS += ["id", "basename", "touch", "tee", "wc", "grep", "sort", "seq", "cp", "sed", "ln"]
TOOLS += ["readlink", "mktemp", "true", "false", "cut", "stat", "uname", "printf", "echo"]
CERT = {}


def setUpModule():
    d = tempfile.mkdtemp(prefix="kf-box-cert-")
    cert, key = Path(d, "c.pem"), Path(d, "k.pem")
    subprocess.run(
        ["openssl", "req", "-x509", "-newkey", "ec", "-pkeyopt", "ec_paramgen_curve:prime256v1"]
        + ["-nodes", "-keyout", str(key), "-out", str(cert), "-days", "2", "-subj", "/CN=localhost"]
        + ["-addext", "subjectAltName=DNS:localhost,IP:127.0.0.1"],
        check=True,
        capture_output=True,
    )
    CERT.update(dir=d, cert=str(cert), key=str(key))


def tearDownModule():
    shutil.rmtree(CERT["dir"], ignore_errors=True)


def load():
    spec = importlib.util.spec_from_file_location("vast_report", SCRIPTS / "vast_report.py")
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


class Broker:
    """Worker-like fake: auth, numbered idempotent reports, kill; fault injection."""

    def __init__(self, token=TOKEN, run=RUN, prefix=PREFIX):
        self.token, self.run, self.prefix = token, run, prefix
        self.requests, self.accepted, self.faults, self.delay = [], [], {}, {}
        self.last_seq, self.last_digest, self.lock = 0, None, threading.Lock()
        self.redirect_to = None
        broker = self

        class Handler(http.server.BaseHTTPRequestHandler):
            def log_message(self, *args):
                pass

            def do_GET(self):
                self.do_POST()

            def do_POST(self):
                length = int(self.headers.get("Content-Length") or 0)
                body = self.rfile.read(length)
                status, doc, headers = broker.handle(self.path, self.headers, body, self.command)
                raw = json.dumps(doc).encode()
                try:
                    self.send_response(status)
                    for key, value in headers.items():
                        self.send_header(key, value)
                    self.send_header("Content-Type", "application/json")
                    self.send_header("Content-Length", str(len(raw)))
                    self.end_headers()
                    self.wfile.write(raw)
                except OSError:
                    pass

        self.server = http.server.ThreadingHTTPServer(("127.0.0.1", 0), Handler)
        self.server.daemon_threads = True
        context = ssl.SSLContext(ssl.PROTOCOL_TLS_SERVER)
        context.load_cert_chain(CERT["cert"], CERT["key"])
        self.server.socket = context.wrap_socket(self.server.socket, server_side=True)
        self.url = "https://localhost:%d%s" % (self.server.server_address[1], prefix)
        threading.Thread(target=self.server.serve_forever, daemon=True).start()

    def stop(self):
        self.server.shutdown()
        self.server.server_close()

    def handle(self, path, headers, body, method="POST"):
        auth = headers.get("Authorization")
        record = {"path": path, "auth": auth, "body": body, "method": method, "t": time.time()}
        record["access"] = (
            headers.get("CF-Access-Client-Id"),
            headers.get("CF-Access-Client-Secret"),
        )
        with self.lock:
            self.requests.append(record)
        parts = path.split("/")
        base = self.prefix.split("/")
        ok = parts[: len(base)] == base and parts[len(base) + 1 :] == [self.run]
        kind = parts[len(base)] if len(parts) > len(base) else ""
        if not ok or kind not in ("report", "kill"):
            return 404, {"state": "NOT_FOUND"}, {}
        if auth != "Bearer " + self.token:
            return 401, {"state": "UNAUTHORIZED"}, {}
        time.sleep(self.delay.get(kind, 0))
        fault = None
        with self.lock:
            queue = self.faults.get(kind, [])
            if queue:
                fault = queue.pop(0)
        if fault == "redirect":
            return 302, {}, {"Location": self.redirect_to + "/stolen"}
        if fault in ("401", "429", "400", "500"):
            return int(fault), {"state": "FAULT"}, {}
        if kind != "report":
            return 200, {"state": "OK"}, {}
        if fault == "not_ready":
            return 409, {"state": "NOT_READY"}, {}
        status, doc = self.apply(body)
        if fault == "500_after_apply":
            return 500, {"state": "LOST"}, {}
        return status, doc, {}

    def apply(self, body):
        try:
            payload = json.loads(body)
        except ValueError:
            return 400, {"state": "BAD_REPORT"}
        digest = hashlib.sha256(json.dumps(payload).encode()).hexdigest()
        with self.lock:
            if payload["seq"] <= self.last_seq:
                if payload["seq"] == self.last_seq and digest == self.last_digest:
                    return 200, {"state": "ACCEPTED"}
                return 409, {"state": "REPORT_SEQUENCE"}
            self.last_seq, self.last_digest = payload["seq"], digest
            self.accepted.append(payload)
        extra = {"grace_remaining_seconds": 300} if payload["kind"] == "result" else {}
        return 200, dict(state="ACCEPTED", **extra)

    def reports(self):
        return [r for r in self.requests if "/report/" in r["path"]]

    def hits(self, kind):
        return [r for r in self.requests if "/%s/" % kind in r["path"]]

    def texts(self):
        return "".join(p.get("text", "") for p in self.accepted)

    def results(self):
        return [p["result"] for p in self.accepted if p["kind"] == "result"]


def good_env(url):
    return {
        "KF_PROTOCOL": "7",
        "KF_BROKER_URL": url,
        "KF_RUN_ID": RUN,
        "KF_RUN_TOKEN": TOKEN,
        "KF_NONCE": NONCE,
        "KF_SHA": SHA,
        "KF_REPO": "reindertpelsma/kayfabe",
        "KF_RUNNER_NONCE": RNONCE,
        "KF_BOOT_DEADLINE": str(int(time.time()) + 600),
        "KF_HARD_DEADLINE": str(int(time.time()) + 3600),
        "KF_SOFT_DEADLINE": str(int(time.time()) + 3000),
        "KF_HEARTBEAT_SECONDS": "30",
        "KF_GRACE_PASS": "300",
        "KF_GRACE_FAIL": "600",
    }


def base_env(extra):
    env = {"PATH": os.environ["PATH"], "HOME": os.environ.get("HOME", "/tmp"), "LC_ALL": "C.UTF-8"}
    env.update(SSL_CERT_FILE=CERT["cert"], CURL_CA_BUNDLE=CERT["cert"])
    env.update(extra)
    return env


def wait_until(predicate, seconds=10.0):
    end = time.time() + seconds
    while time.time() < end:
        if predicate():
            return True
        time.sleep(0.02)
    return predicate()


def write_done(path, rc=0, timeout=0, reason=None):
    text = "rc=%d\ntimeout=%d\n" % (rc, timeout) + ("reason=%s\n" % reason if reason else "")
    Path(str(path) + ".tmp").write_text(text)
    os.replace(str(path) + ".tmp", path)


def result_file(path, verdict="PASS", rows=None, hardware=HARDWARE):
    doc = {"verdict": verdict, "suites": ROWS if rows is None else rows}
    if hardware is not None:
        doc["hardware"] = hardware
    Path(path).write_text(json.dumps(doc))


class Tmp(unittest.TestCase):
    def setUp(self):
        self.tmp = Path(tempfile.mkdtemp(prefix="kf-box-test-"))
        self.addCleanup(shutil.rmtree, self.tmp, True)
        self.brokers = []

    def broker(self, **kw):
        b = Broker(**kw)
        self.brokers.append(b)
        self.addCleanup(b.stop)
        return b


class EnvValidation(Tmp):
    def test_good_env_has_no_problems(self):
        self.assertEqual(load().validate_env(good_env("https://h.example/pfx")), [])

    def test_each_required_variable_missing_is_named(self):
        m = load()
        required = ["KF_PROTOCOL", "KF_BROKER_URL", "KF_RUN_ID", "KF_RUN_TOKEN", "KF_NONCE"]
        required += ["KF_SHA", "KF_REPO", "KF_RUNNER_NONCE", "KF_SOFT_DEADLINE"]
        required += ["KF_HEARTBEAT_SECONDS", "KF_GRACE_FAIL"]
        for name in required:
            with self.subTest(name=name):
                env = good_env("https://h.example/pfx")
                del env[name]
                problems = m.validate_env(env)
                self.assertEqual(len(problems), 1, problems)
                self.assertIn(name, problems[0])

    def test_malformed_values_are_rejected(self):
        m = load()
        bad = {
            "KF_PROTOCOL": ["3", "", "6", "7.0", "07"],
            "KF_BROKER_URL": ["http://h.example/p", "https://h.example/p/", "https://h/p?x=1"]
            + ["https://u:p@h.example/p", "ftp://h/p", "https://h/p#f", "https://h/ p", "h/p"],
            "KF_RUN_ID": ["0-1", "12", "1-2-3", "a-b"],
            "KF_RUN_TOKEN": [TOKEN[:-1], TOKEN + "x", TOKEN + ".x", "a.b", "x", "a b.c.d"],
            "KF_NONCE": ["ab" * 15, "AB" * 16, "zz" * 16],
            "KF_SHA": ["a" * 39, "A" * 40, "g" * 40],
            "KF_REPO": ["owner", "o w/n", "a/b/c", "/b"],
            "KF_RUNNER_NONCE": ["cd" * 31, "CD" * 32],
            "KF_SOFT_DEADLINE": ["0", "-5", "soon", "1.5", "99999999999999"],
            "KF_HEARTBEAT_SECONDS": ["0", "-1", "x", "100000"],
            "KF_GRACE_FAIL": ["0", "x"],
            "KF_HARD_DEADLINE": ["x", "1"],
        }
        for name, values in bad.items():
            for value in values:
                with self.subTest(name=name, value=value):
                    env = good_env("https://h.example/pfx")
                    env[name] = value
                    problems = m.validate_env(env)
                    self.assertTrue(any(name in p for p in problems), problems)

    def test_access_pair_is_optional_and_checked_when_present(self):
        env = good_env("https://h.example/pfx")
        self.assertEqual(load().validate_env(dict(env, **ACCESS)), [])
        for name in ACCESS:
            bad = dict(env, **ACCESS)
            bad[name] = "has space" + ACCESS[name]
            problems = load().validate_env(bad)
            self.assertEqual(problems, [name + ": malformed"])
            self.assertNotIn(ACCESS[name], "".join(problems))

    def test_problems_never_contain_values(self):
        env = good_env("https://h.example/pfx")
        env["KF_RUN_TOKEN"] = "v1.1.2.3.secret-token-value-xyz"
        env["KF_NONCE"] = "NONCE-secret-value"
        text = "\n".join(load().validate_env(env))
        self.assertNotIn("secret-token-value-xyz", text)
        self.assertNotIn("NONCE-secret-value", text)

    def check_env(self, env, *args):
        return subprocess.run(
            [sys.executable, str(SCRIPTS / "vast_report.py"), "check-env", *args],
            env=base_env(env),
            capture_output=True,
            text=True,
            timeout=60,
        )

    def test_cli_failure_posts_structured_fail_and_hides_token(self):
        b = self.broker()
        env = good_env(b.url)
        env["KF_HEARTBEAT_SECONDS"] = "0"
        proc = self.check_env(env, "--retry-base", "0.1")
        self.assertNotEqual(proc.returncode, 0)
        self.assertNotIn(TOKEN, proc.stdout + proc.stderr)
        self.assertIn("KF_HEARTBEAT_SECONDS", b.texts())
        self.assertNotIn(TOKEN, b.texts())
        result = b.results()[0]
        self.assertEqual(result["verdict"], "FAIL")
        self.assertEqual(
            result["suites"], [{"name": "env_contract", "pass": 0, "fail": 1, "skip": 0}]
        )
        self.assertEqual((result["nonce"], result["sha"]), (NONCE, SHA))
        self.assertEqual([r["auth"] for r in b.reports()], ["Bearer " + TOKEN] * 2)

    def test_cli_failure_with_unbindable_result_posts_only_a_log(self):
        b = self.broker()
        env = good_env(b.url)
        del env["KF_NONCE"]
        proc = self.check_env(env, "--retry-base", "0.1")
        self.assertNotEqual(proc.returncode, 0)
        self.assertEqual(b.results(), [])
        self.assertIn("KF_NONCE", b.texts())

    def test_cli_never_sends_the_token_over_plain_http(self):
        listener = socket.socket()
        listener.bind(("127.0.0.1", 0))
        listener.listen(4)
        listener.setblocking(False)
        self.addCleanup(listener.close)
        env = good_env("http://127.0.0.1:%d/pfx" % listener.getsockname()[1])
        proc = self.check_env(env, "--retry-base", "0.1")
        self.assertNotEqual(proc.returncode, 0)
        with self.assertRaises(BlockingIOError):
            listener.accept()

    def test_cli_write_makes_a_private_env_file_of_contract_names_only(self):
        b = self.broker()
        env = good_env(b.url)
        env["AWS_SECRET_ACCESS_KEY"] = "unrelated-secret"
        target = self.tmp / "run" / "env"
        proc = self.check_env(env, "--write", str(target))
        self.assertEqual(proc.returncode, 0, proc.stderr)
        self.assertEqual(stat.S_IMODE(target.stat().st_mode), 0o600)
        self.assertEqual(stat.S_IMODE(target.parent.stat().st_mode), 0o700)
        text = target.read_text()
        self.assertNotIn("unrelated-secret", text)
        out = subprocess.run(
            ["sh", "-c", '. "$1"; printf "%s|%s" "$KF_RUN_TOKEN" "$KF_BROKER_URL"', "sh", target],
            capture_output=True,
            text=True,
        )
        self.assertEqual(out.stdout, TOKEN + "|" + b.url)
        self.assertEqual(b.requests, [])


class Daemon(Tmp):
    def start(self, broker, extra_args=(), env_extra=None, log_text=""):
        self.log, self.done, self.result = (
            self.tmp / n for n in ("out.log", "done", "result.json")
        )
        self.log.write_text(log_text)
        env = good_env(broker.url)
        env.update(env_extra or {})
        args = ["--log", str(self.log), "--done", str(self.done), "--result", str(self.result)]
        self.proc = subprocess.Popen(
            [sys.executable, str(SCRIPTS / "vast_report.py"), "daemon", *args, *FAST, *extra_args],
            env=base_env(env),
            stdout=subprocess.PIPE,
            stderr=subprocess.STDOUT,
            text=True,
        )
        self.addCleanup(self.proc.kill)
        return self.proc

    def append(self, text):
        with open(self.log, "a") as handle:
            handle.write(text)

    def finish(self, rc=0, timeout=0, reason=None, wait=30):
        write_done(self.done, rc, timeout, reason)
        out = self.proc.communicate(timeout=wait)[0]
        return self.proc.returncode, out


class Reporter(Daemon):
    def test_progress_logs_result_are_numbered_and_authorised(self):
        b = self.broker()
        self.start(b, log_text="first line\n")
        result_file(self.result)
        self.append("second line\n")
        rc, out = self.finish()
        self.assertEqual(rc, 0, out)
        kinds = [p["kind"] for p in b.accepted]
        self.assertEqual(kinds[0], "progress")
        self.assertEqual(kinds[-1], "result")
        self.assertEqual([p["seq"] for p in b.accepted], list(range(1, len(kinds) + 1)))
        self.assertIn("first line\nsecond line\n", b.texts())
        self.assertEqual({r["auth"] for r in b.reports()}, {"Bearer " + TOKEN})
        self.assertEqual({r["path"] for r in b.requests}, {PREFIX + "/report/" + RUN})
        for payload in b.accepted:
            self.assertLessEqual(set(payload), {"seq", "kind", "text", "result"})

    def test_reports_carry_the_access_pair_only_when_set(self):
        for access, expected in ((ACCESS, tuple(ACCESS.values())), ({}, (None, None))):
            b = self.broker()
            self.start(b, env_extra=access, log_text="line\n")
            result_file(self.result)
            rc, out = self.finish()
            self.assertEqual(rc, 0, out)
            self.assertEqual({r["access"] for r in b.reports()}, {expected})
            for value in ACCESS.values():
                self.assertNotIn(value, out + b.texts())

    def test_result_has_exactly_the_worker_schema_and_bindings(self):
        b = self.broker()
        self.start(b)
        result_file(self.result)
        self.assertEqual(self.finish()[0], 0)
        result = b.results()[0]
        keys = ["hardware", "nonce", "runner_nonce", "sha", "suites", "verdict"]
        self.assertEqual(sorted(result), keys)
        self.assertEqual(result["verdict"], "PASS")
        self.assertEqual(result["suites"], ROWS)
        self.assertEqual(result["hardware"], HARDWARE)
        self.assertEqual((result["nonce"], result["sha"]), (NONCE, SHA))
        self.assertEqual(result["runner_nonce"], RNONCE)

    def test_payload_cannot_choose_the_binding_fields(self):
        b = self.broker()
        self.start(b)
        doc = {"verdict": "PASS", "suites": ROWS, "hardware": HARDWARE, "nonce": "f" * 32}
        doc.update(sha="e" * 40, runner_nonce="d" * 64)
        self.result.write_text(json.dumps(doc))
        self.assertEqual(self.finish()[0], 1)
        result = b.results()[0]
        self.assertEqual(result["verdict"], "FAIL")
        self.assertEqual(result["suites"][0]["name"], "result_invalid")
        self.assertEqual((result["nonce"], result["sha"]), (NONCE, SHA))
        self.assertEqual(result["runner_nonce"], RNONCE)

    def test_payload_may_echo_the_correct_binding_fields(self):
        b = self.broker()
        self.start(b)
        doc = {"verdict": "PASS", "suites": ROWS, "hardware": HARDWARE, "nonce": NONCE}
        doc.update(sha=SHA, runner_nonce=RNONCE)
        self.result.write_text(json.dumps(doc))
        self.assertEqual(self.finish()[0], 0)
        self.assertEqual(b.results()[0]["verdict"], "PASS")

    def test_lost_response_retry_reuses_payload_and_sequence(self):
        b = self.broker()
        b.faults["report"] = [None, "500_after_apply", "500_after_apply"]
        self.start(b, log_text="only line\n")
        result_file(self.result)
        self.assertTrue(wait_until(lambda: len(b.reports()) >= 4))
        self.assertEqual(self.finish()[0], 0)
        by_seq = {}
        for r in b.reports():
            by_seq.setdefault(json.loads(r["body"])["seq"], []).append(r["body"])
        retried = [bodies for bodies in by_seq.values() if len(bodies) > 1]
        self.assertTrue(retried, "no retry observed")
        for bodies in retried:
            self.assertEqual(len(set(bodies)), 1, "retry changed the payload")
        self.assertEqual(b.texts().count("only line"), 1)
        self.assertEqual([p["seq"] for p in b.accepted], sorted(by_seq))
        self.assertEqual(list(by_seq), list(range(1, max(by_seq) + 1)), "sequence gap")

    def test_failed_request_is_retried_with_same_sequence(self):
        b = self.broker()
        b.faults["report"] = ["500", "500", "429"]
        self.start(b)
        result_file(self.result)
        self.assertEqual(self.finish()[0], 0)
        first = [json.loads(r["body"])["seq"] for r in b.reports()[:4]]
        self.assertEqual(first, [1, 1, 1, 1])
        self.assertEqual(b.accepted[0]["seq"], 1)
        self.assertEqual(b.results()[0]["verdict"], "PASS")

    def test_not_ready_is_retried_not_dropped(self):
        b = self.broker()
        b.faults["report"] = ["not_ready", "not_ready"]
        self.start(b, log_text="boot line\n")
        result_file(self.result)
        self.assertEqual(self.finish()[0], 0)
        self.assertEqual(b.accepted[0]["kind"], "progress")
        self.assertIn("boot line", b.texts())

    def test_unauthorised_stops_sending_but_never_hangs(self):
        b = self.broker()
        b.faults["report"] = ["401"]
        self.start(b, log_text="line\n")
        result_file(self.result)
        rc, out = self.finish(wait=20)
        self.assertNotEqual(rc, 0)
        self.assertEqual(len(b.reports()), 1)
        self.assertEqual(self.log.read_text(), "line\n")

    def test_rejected_result_falls_back_to_a_conservative_fail(self):
        b = self.broker()
        b.faults["report"] = [None, "400"]
        self.start(b)
        self.assertTrue(wait_until(lambda: len(b.accepted) == 1))
        result_file(self.result)
        self.assertEqual(self.finish()[0], 1)
        result = b.results()[0]
        self.assertEqual(result["verdict"], "FAIL")
        self.assertEqual(result["suites"][0]["name"], "result_rejected")

    def test_redirects_are_never_followed_and_token_does_not_travel(self):
        b, evil = self.broker(), self.broker()
        b.redirect_to = evil.url
        b.faults["report"] = ["redirect", "redirect"]
        self.start(b)
        result_file(self.result)
        self.assertEqual(self.finish()[0], 0)
        self.assertEqual(evil.requests, [])
        self.assertTrue(b.results())

    def test_heartbeat_is_sent_with_zero_new_lines(self):
        b = self.broker()
        self.start(b, env_extra={"KF_HEARTBEAT_SECONDS": "1"})
        time.sleep(3.2)
        result_file(self.result)
        self.assertEqual(self.finish()[0], 0)
        beats = [p for p in b.accepted if p["kind"] == "log" and p["text"] == ""]
        self.assertGreaterEqual(len(beats), 2)
        self.assertLessEqual(len(beats), 5)
        self.assertEqual([p["seq"] for p in b.accepted], list(range(1, len(b.accepted) + 1)))

    def test_heartbeat_waits_for_the_interval(self):
        b = self.broker()
        self.start(b, env_extra={"KF_HEARTBEAT_SECONDS": "30"})
        time.sleep(1.5)
        result_file(self.result)
        self.assertEqual(self.finish()[0], 0)
        self.assertEqual([p for p in b.accepted if p.get("text") == ""], [])

    def test_post_count_cap_omits_excess_and_still_sends_result(self):
        b = self.broker()
        self.start(b, ["--max-posts", "3"], log_text="".join("x" * 99 + "\n" for _ in range(400)))
        result_file(self.result)
        self.assertEqual(self.finish()[0], 0)
        logs = [p for p in b.accepted if p["kind"] == "log" and p["text"]]
        self.assertLessEqual(len(logs), 3)
        self.assertGreaterEqual(len(logs), 1)
        self.assertRegex(b.results() and b.accepted[-1]["text"], r"omitted [1-9][0-9]* raw bytes")

    def test_byte_cap_bounds_the_total_log_body_bytes(self):
        b = self.broker()
        self.start(
            b, ["--max-bytes", "10000"], log_text="".join("y" * 99 + "\n" for _ in range(500))
        )
        result_file(self.result)
        self.assertEqual(self.finish()[0], 0)
        sent = sum(len(r["body"]) for r in b.reports() if b"yyyy" in r["body"])
        self.assertGreater(sent, 0)
        self.assertLessEqual(sent, 10000)
        self.assertRegex(b.accepted[-1]["text"], r"omitted [1-9][0-9]* raw bytes")

    def test_chunks_are_bounded(self):
        b = self.broker()
        self.start(b, log_text="".join("z" * 99 + "\n" for _ in range(300)))
        result_file(self.result)
        self.assertEqual(self.finish()[0], 0)
        for payload in b.accepted:
            self.assertLessEqual(len(payload.get("text", "").encode()), 8192)
        self.assertEqual(b.texts().count("z" * 99), 300)

    def test_long_lines_are_capped_and_the_rest_survives(self):
        b = self.broker()
        self.start(b, ["--line-cap", "200"], log_text="a" * 50000 + "\nshort line\n")
        result_file(self.result)
        self.assertEqual(self.finish()[0], 0)
        lines = b.texts().split("\n")
        self.assertIn("short line", lines)
        self.assertLess(max(len(x) for x in lines), 300)
        self.assertIn("truncated", b.texts())
        self.assertEqual(self.log.read_text().count("a"), 50000)

    def test_multibyte_text_survives_chunk_boundaries(self):
        b = self.broker()
        text = ("é€\U0001f600 " * 4000) + "\n"
        self.start(b, ["--line-cap", "100000"], log_text=text)
        result_file(self.result)
        self.assertEqual(self.finish()[0], 0)
        self.assertEqual(b.texts().replace("�", "?").count("?"), 0)
        self.assertEqual(b.texts().count("\U0001f600"), 4000)

    def test_missing_result_file_is_a_structured_fail(self):
        b = self.broker()
        self.start(b)
        self.assertEqual(self.finish()[0], 1)
        result = b.results()[0]
        self.assertEqual(result["verdict"], "FAIL")
        self.assertEqual(
            result["suites"], [{"name": "result_missing", "pass": 0, "fail": 1, "skip": 0}]
        )

    def test_payload_missing_reason_is_a_structured_fail(self):
        b = self.broker()
        self.start(b, log_text="scripts/ci/vast_payload.sh is absent\n")
        result_file(self.result)
        self.assertEqual(self.finish(rc=127, reason="payload_missing")[0], 1)
        result = b.results()[0]
        self.assertEqual(result["verdict"], "FAIL")
        self.assertEqual(result["suites"][0]["name"], "payload_missing")
        self.assertIn("vast_payload.sh is absent", b.texts())

    def test_done_without_the_log_content_loss(self):
        b = self.broker()
        self.start(b)
        self.append("late line\n")
        result_file(self.result)
        self.assertEqual(self.finish()[0], 0)
        self.assertIn("late line\n", b.texts())

    def test_unreachable_broker_gives_up_within_the_budget(self):
        listener = socket.socket()
        listener.bind(("127.0.0.1", 0))
        port = listener.getsockname()[1]
        listener.close()
        b = self.broker()
        b.url = "https://localhost:%d%s" % (port, PREFIX)
        self.start(b, ["--final-budget", "1"], log_text="offline line\n")
        started = time.time()
        rc, out = self.finish(wait=30)
        self.assertNotEqual(rc, 0)
        self.assertLess(time.time() - started, 15)
        self.assertEqual(self.log.read_text(), "offline line\n")


class FinalResult(Tmp):
    def build(self, doc, rc=0, timed_out=False, reason=None, raw=None):
        path = self.tmp / "r.json"
        if raw is not None:
            path.write_bytes(raw)
        elif doc is not None:
            path.write_text(json.dumps(doc))
        return load().final_result(good_env("https://h/p"), rc, timed_out, reason, str(path))

    def good(self, **kw):
        doc = {"verdict": "PASS", "suites": [dict(r) for r in ROWS], "hardware": dict(HARDWARE)}
        doc.update(kw)
        return doc

    def names(self, result):
        return [r["name"] for r in result["suites"]]

    def test_valid_pass_is_kept(self):
        result = self.build(self.good())
        self.assertEqual(result["verdict"], "PASS")
        self.assertEqual(result["suites"], ROWS)

    def test_exit_code_demotes_pass(self):
        result = self.build(self.good(), rc=3)
        self.assertEqual(result["verdict"], "FAIL")
        self.assertIn("app_exit", self.names(result))

    def test_skips_or_failures_or_zero_pass_demote_pass(self):
        for row in (
            {"name": "a", "pass": 1, "fail": 0, "skip": 1},
            {"name": "a", "pass": 0, "fail": 0, "skip": 0},
        ):
            with self.subTest(row=row):
                result = self.build(self.good(suites=[row]))
                self.assertEqual(result["verdict"], "FAIL")
                self.assertTrue(any(r["fail"] > 0 for r in result["suites"]))

    def test_unknown_or_old_hardware_demotes_pass(self):
        for hw in (None, dict(HARDWARE, compute_cap=700), dict(HARDWARE, driver_version="unknown")):
            with self.subTest(hw=hw):
                doc = self.good()
                if hw is None:
                    del doc["hardware"]
                else:
                    doc["hardware"] = hw
                self.assertEqual(self.build(doc)["verdict"], "FAIL")

    def test_missing_hardware_reports_unknown_not_invented(self):
        doc = self.good()
        del doc["hardware"]
        hw = self.build(doc)["hardware"]
        self.assertEqual(hw, {"gpu_name": "unknown", "compute_cap": 0, "driver_version": "unknown"})

    def test_payload_fail_without_failing_row_gets_one(self):
        result = self.build(self.good(verdict="FAIL"))
        self.assertEqual(result["verdict"], "FAIL")
        self.assertTrue(any(r["fail"] > 0 for r in result["suites"]))

    def test_unavailable_is_preserved_with_a_failing_row(self):
        result = self.build(self.good(verdict="UNAVAILABLE"))
        self.assertEqual(result["verdict"], "UNAVAILABLE")
        self.assertTrue(any(r["fail"] > 0 for r in result["suites"]))

    def test_timeout_forces_fail_and_keeps_the_finished_suites(self):
        rows = [{"name": "done_suite", "pass": 5, "fail": 0, "skip": 0}]
        result = self.build(self.good(suites=rows), rc=124, timed_out=True)
        self.assertEqual(result["verdict"], "FAIL")
        self.assertEqual(self.names(result), ["done_suite", "timeout"])
        self.assertEqual(result["suites"][0], rows[0])
        self.assertEqual(result["suites"][1], {"name": "timeout", "pass": 0, "fail": 1, "skip": 0})

    def test_timeout_without_result_file(self):
        result = self.build(None, rc=124, timed_out=True)
        self.assertEqual(result["verdict"], "FAIL")
        self.assertEqual(self.names(result), ["timeout"])

    def test_timeout_cannot_be_hidden_by_a_payload_pass(self):
        result = self.build(self.good(verdict="PASS"), rc=0, timed_out=True)
        self.assertEqual(result["verdict"], "FAIL")

    def test_bad_result_files_fail_closed(self):
        cases = {
            "not json": b"{nope",
            "not object": b"[1]",
            "dup key": b'{"verdict":"FAIL","verdict":"PASS",'
            b'"suites":[{"name":"a","pass":1,"fail":0,"skip":0}],'
            b'"hardware":{"gpu_name":"NVIDIA GeForce RTX 3060","compute_cap":860,'
            b'"driver_version":"580.159.04"}}',
            "extra key": json.dumps(dict(self.good(), extra=1)).encode(),
            "bad verdict": json.dumps(self.good(verdict="pass")).encode(),
            "no suites": json.dumps(self.good(suites=[])).encode(),
            "bad name": json.dumps(
                self.good(suites=[{"name": "A-b", "pass": 1, "fail": 0, "skip": 0}])
            ).encode(),
            "dup names": json.dumps(self.good(suites=ROWS + ROWS)).encode(),
            "float count": json.dumps(
                self.good(suites=[{"name": "a", "pass": 1.0, "fail": 0, "skip": 0}])
            ).encode(),
            "bool count": json.dumps(
                self.good(suites=[{"name": "a", "pass": True, "fail": 0, "skip": 0}])
            ).encode(),
            "huge count": json.dumps(
                self.good(suites=[{"name": "a", "pass": 10**9, "fail": 0, "skip": 0}])
            ).encode(),
            "too many rows": json.dumps(
                self.good(
                    suites=[{"name": "s%d" % i, "pass": 1, "fail": 0, "skip": 0} for i in range(20)]
                )
            ).encode(),
            "oversize": b" " * 70000 + json.dumps(self.good()).encode(),
            "bad utf8": b'{"verdict":"\xff"}',
            "wrong nonce": json.dumps(self.good(nonce="f" * 32)).encode(),
            "wrong sha": json.dumps(self.good(sha="e" * 40)).encode(),
            "wrong runner nonce": json.dumps(self.good(runner_nonce="d" * 64)).encode(),
            "bad hardware": json.dumps(
                self.good(
                    hardware={"gpu_name": "x;rm", "compute_cap": 860, "driver_version": "580.1"}
                )
            ).encode(),
        }
        for label, raw in cases.items():
            with self.subTest(label):
                result = self.build(None, raw=raw)
                self.assertEqual(result["verdict"], "FAIL")
                self.assertEqual(self.names(result), ["result_invalid"])

    def test_payload_missing_reason_overrides_any_file(self):
        result = self.build(self.good(), rc=127, reason="payload_missing")
        self.assertEqual(result["verdict"], "FAIL")
        self.assertEqual(self.names(result), ["payload_missing"])

    def test_result_passes_the_worker_schema_rules(self):
        for result in (self.build(self.good()), self.build(None), self.build(self.good(), rc=2)):
            self.assertEqual(
                sorted(result), ["hardware", "nonce", "runner_nonce", "sha", "suites", "verdict"]
            )
            self.assertTrue(1 <= len(result["suites"]) <= 20)
            if result["verdict"] != "PASS":
                self.assertTrue(any(r["fail"] > 0 for r in result["suites"]))


def make_bin(dest, names, extra=None):
    dest.mkdir()
    for name in names:
        found = shutil.which(name)
        if found:
            (dest / name).symlink_to(found)
    for name, body in (extra or {}).items():
        (dest / name).write_text(body)
        (dest / name).chmod(0o755)
    return dest


class Kill(Tmp):
    def env_file(self, url, access=None):
        path = self.tmp / "env"
        lines = ["export KF_BROKER_URL='%s'" % url, "export KF_RUN_ID='%s'" % RUN]
        lines += ["export %s='%s'" % item for item in (access or {}).items()]
        path.write_text("\n".join(lines + ["export KF_RUN_TOKEN='%s'" % TOKEN]) + "\n")
        path.chmod(0o600)
        return path

    def run_tool(self, name, url, *args, env=None, timeout=40, access=None):
        env_file = self.env_file(url, access)
        return subprocess.run(
            ["sh", str(SCRIPTS / name), *args],
            env=base_env(dict({"KF_CI_ENV_FILE": str(env_file)}, **(env or {}))),
            capture_output=True,
            text=True,
            timeout=timeout,
        )

    def test_kill_request_shape(self):
        b = self.broker()
        proc = self.run_tool("kf-box-kill", b.url, "because tests")
        self.assertEqual(proc.returncode, 0, proc.stderr)
        self.assertEqual(len(b.requests), 1)
        request = b.requests[0]
        self.assertEqual(request["method"], "POST")
        self.assertEqual(request["path"], PREFIX + "/kill/" + RUN)
        self.assertEqual(request["auth"], "Bearer " + TOKEN)
        self.assertEqual(request["body"], b"because tests")

    def test_kill_default_reason_and_bounded_sanitised_reason(self):
        b = self.broker()
        self.assertEqual(self.run_tool("kf-box-kill", b.url).returncode, 0)
        self.assertEqual(b.requests[-1]["body"], b"box-request")
        self.assertEqual(self.run_tool("kf-box-kill", b.url, 'x"; rm\n' + "y" * 500).returncode, 0)
        body = b.requests[-1]["body"]
        self.assertLessEqual(len(body), 200)
        self.assertNotIn(b'"', body)
        self.assertNotIn(b"\n", body)

    def test_kill_does_not_follow_redirects_and_fails(self):
        b, evil = self.broker(), self.broker()
        b.redirect_to = evil.url
        b.faults["kill"] = ["redirect"]
        proc = self.run_tool("kf-box-kill", b.url)
        self.assertNotEqual(proc.returncode, 0)
        self.assertEqual(evil.requests, [])
        self.assertEqual(len(b.requests), 1)

    def test_kill_failure_status_is_an_error(self):
        b = self.broker(token="other")
        self.assertNotEqual(self.run_tool("kf-box-kill", b.url).returncode, 0)

    def test_kill_refuses_plain_http(self):
        listener = socket.socket()
        listener.bind(("127.0.0.1", 0))
        listener.listen(4)
        listener.setblocking(False)
        self.addCleanup(listener.close)
        url = "http://127.0.0.1:%d%s" % (listener.getsockname()[1], PREFIX)
        self.assertNotEqual(self.run_tool("kf-box-kill", url).returncode, 0)
        with self.assertRaises(BlockingIOError):
            listener.accept()

    def test_kill_keeps_the_token_off_the_command_line(self):
        record = self.tmp / "rec"
        stub = 'printf "%s\\n" "$@" > "$KF_REC.argv"; cat > "$KF_REC.stdin"; printf 200\n'
        bindir = make_bin(
            self.tmp / "bin", ["sh", "cat", "printf", "tr", "head"], {"curl": "#!/bin/sh\n" + stub}
        )
        proc = self.run_tool(
            "kf-box-kill",
            "https://h.example/p",
            "why",
            env={"PATH": str(bindir), "KF_REC": str(record)},
        )
        self.assertEqual(proc.returncode, 0, proc.stderr)
        argv = Path(str(record) + ".argv").read_text()
        self.assertNotIn(TOKEN, argv)
        self.assertIn(TOKEN, Path(str(record) + ".stdin").read_text())
        for flag in ("--proto", "=https", "--max-redirs", "--max-time", "--noproxy"):
            self.assertIn(flag, argv)
        self.assertNotIn("--location", argv)
        self.assertNotIn("-L", argv.split())

    # Owner ruling 2026-10-23 21:45: the box sends the Worker-injected Access pair, when set.
    def test_kill_sends_the_access_pair_only_when_set(self):
        b = self.broker()
        self.assertEqual(self.run_tool("kf-box-kill", b.url, access=ACCESS).returncode, 0)
        self.assertEqual(self.run_tool("kf-box-kill", b.url).returncode, 0)
        pair = (ACCESS["KF_ACCESS_ID"], ACCESS["KF_ACCESS_SECRET"])
        self.assertEqual([r["access"] for r in b.requests], [pair, (None, None)])

    def test_kill_and_blob_keep_the_access_pair_off_the_command_line(self):
        record = self.tmp / "rec"
        stub = 'printf "%s\\n" "$@" > "$KF_REC.argv"; cat > "$KF_REC.stdin"; printf 200\n'
        bindir = make_bin(
            self.tmp / "bin", ["sh", "cat", "printf", "tr", "head"], {"curl": "#!/bin/sh\n" + stub}
        )
        for tool, args in (("kf-box-kill", ["why"]), ("kf-box-blob-url", ["tag", "name"])):
            for access in (ACCESS, None):
                proc = self.run_tool(
                    tool,
                    "https://h.example/p",
                    *args,
                    env={"PATH": str(bindir), "KF_REC": str(record)},
                    access=access,
                )
                self.assertEqual(proc.returncode, 0, tool + proc.stderr)
                argv = Path(str(record) + ".argv").read_text()
                stdin = Path(str(record) + ".stdin").read_text()
                for value in ACCESS.values():
                    self.assertNotIn(value, argv + proc.stdout + proc.stderr)
                    self.assertEqual(value in stdin, access is not None, tool)
                self.assertEqual("CF-Access-Client-Secret:" in stdin, access is not None)

    def test_tools_refuse_a_malformed_access_pair(self):
        b = self.broker()
        for tool, args in (("kf-box-kill", []), ("kf-box-blob-url", ["tag", "name"])):
            bad = dict(ACCESS, KF_ACCESS_SECRET='x" url = "https://evil.example')
            self.assertNotEqual(self.run_tool(tool, b.url, *args, access=bad).returncode, 0)
        self.assertEqual(b.requests, [])


PAYLOAD_HEADER = "#!/bin/bash\n"


class Onstart(Tmp):
    def tree(self, payload=None):
        root = self.tmp / "tree"
        ci = root / "scripts" / "ci"
        ci.mkdir(parents=True, exist_ok=True)
        for name in FILES:
            shutil.copy(SCRIPTS / name, ci / name)
        if payload is not None:
            (ci / "vast_payload.sh").write_text(PAYLOAD_HEADER + payload)
        self.ws, self.run_dir = self.tmp / "ws", self.tmp / "run"
        self.bin, self.profile = self.tmp / "bin", self.tmp / "profile.d"
        self.profile.mkdir(exist_ok=True)
        self.bin.mkdir(exist_ok=True)
        return root

    def onstart(self, root, broker, env_extra=None, drop=(), path=None, timeout=90, **overrides):
        env = good_env(broker.url if broker else "https://localhost:1/p")
        for name in drop:
            env.pop(name, None)
        env.update(
            KF_CI_WORKSPACE=str(self.ws),
            KF_CI_RUN_DIR=str(self.run_dir),
            KF_CI_BIN_DIR=str(self.bin),
            KF_CI_PROFILE_DIR=str(self.profile),
            KF_CI_REPORTER_ARGS=" ".join(FAST),
            KF_TEST_DUMP=str(self.tmp / "dump"),
        )
        env.update(env_extra or {})
        full = base_env(env)
        if path is not None:
            full["PATH"] = path
        started = time.time()
        proc = subprocess.run(
            ["bash", str(root / "scripts" / "ci" / "vast_onstart.sh")],
            env=full,
            cwd=str(self.tmp),
            capture_output=True,
            text=True,
            timeout=timeout,
        )
        self.took = time.time() - started
        return proc

    OK_PAYLOAD = (
        'echo "payload args: $1 $2"\n'
        'env | grep "^KF_" | sort > "$KF_TEST_DUMP"\n'
        'echo "$@" > "$KF_TEST_DUMP.args"\n'
        'cat > "$KF_CI_WORKSPACE/kf-result.json" <<EOF\n'
        '{"verdict":"PASS","suites":[{"name":"smoke","pass":3,"fail":0,"skip":0}],\n'
        ' "hardware":{"gpu_name":"NVIDIA GeForce RTX 3060","compute_cap":860,'
        '"driver_version":"580.159.04"}}\n'
        "EOF\n"
    )

    def test_pass_run_posts_result_and_exits_zero(self):
        b = self.broker()
        root = self.tree(self.OK_PAYLOAD)
        proc = self.onstart(root, b)
        self.assertEqual(proc.returncode, 0, proc.stdout + proc.stderr)
        self.assertEqual(b.results()[0]["verdict"], "PASS")
        self.assertIn("payload args: %s %s" % (SHA, NONCE), b.texts())
        self.assertEqual((self.tmp / "dump.args").read_text().split(), [SHA, NONCE])
        self.assertIn("payload args", (self.ws / "kf-ci-output.log").read_text())
        self.assertRegex(proc.stdout, r"kf-box-kill.*Worker reports 5 minutes remaining")

    def test_failing_payload_exit_code_demotes_and_is_propagated(self):
        b = self.broker()
        root = self.tree(self.OK_PAYLOAD + "exit 3\n")
        proc = self.onstart(root, b)
        self.assertEqual(proc.returncode, 3)
        result = b.results()[0]
        self.assertEqual(result["verdict"], "FAIL")
        self.assertIn("app_exit", [r["name"] for r in result["suites"]])

    def payload_env(self):
        b = self.broker()
        root = self.tree(self.OK_PAYLOAD)
        self.assertEqual(self.onstart(root, b).returncode, 0)
        return (self.tmp / "dump").read_text()

    def test_payload_never_sees_the_run_token(self):
        dump = self.payload_env()
        self.assertNotIn("KF_RUN_TOKEN=", dump)
        self.assertNotIn(TOKEN, dump)

    def test_payload_never_sees_the_broker_endpoint_or_run_id(self):
        dump = self.payload_env()
        for name in ("KF_BROKER_URL", "KF_RUN_ID", "KF_RUNNER_NONCE"):
            self.assertNotIn(name + "=", dump)
        self.assertNotIn(PREFIX, dump)

    def test_payload_never_sees_the_access_pair_but_the_helpers_do(self):
        b = self.broker()
        root = self.tree(self.OK_PAYLOAD)
        self.assertEqual(self.onstart(root, b, env_extra=ACCESS).returncode, 0)
        dump = (self.tmp / "dump").read_text()
        for name, value in ACCESS.items():
            self.assertNotIn(name + "=", dump)
            self.assertNotIn(value, dump)
        self.assertEqual({r["access"] for r in b.reports()}, {tuple(ACCESS.values())})
        out = subprocess.run(
            ["sh", str(self.bin / "kf-box-kill"), "after run"],
            env=base_env({"KF_CI_ENV_FILE": str(self.run_dir / "env")}),
            capture_output=True,
            text=True,
        )
        self.assertEqual(out.returncode, 0, out.stderr)
        self.assertEqual(b.hits("kill")[0]["access"], tuple(ACCESS.values()))

    def test_payload_still_gets_its_own_inputs(self):
        dump = self.payload_env()
        self.assertIn("KF_SHA=" + SHA, dump)
        self.assertIn("KF_NONCE=" + NONCE, dump)
        self.assertRegex(dump, r"KF_SOFT_DEADLINE_MS=[0-9]{13}")

    def test_env_file_is_root_only_and_helpers_are_installed(self):
        b = self.broker()
        root = self.tree(self.OK_PAYLOAD)
        self.assertEqual(self.onstart(root, b).returncode, 0)
        env_file = self.run_dir / "env"
        self.assertEqual(stat.S_IMODE(env_file.stat().st_mode), 0o600)
        self.assertEqual(stat.S_IMODE(self.run_dir.stat().st_mode), 0o700)
        for name in ("kf-box-kill",):
            mode = stat.S_IMODE((self.bin / name).stat().st_mode)
            self.assertTrue(mode & 0o100 and not mode & 0o077, oct(mode))
            self.assertNotIn(TOKEN, (self.bin / name).read_text())
        self.assertFalse((self.bin / "kf-box-hold").exists())
        self.assertFalse((self.profile / "kf-ci-hold.sh").exists())
        out = subprocess.run(
            ["sh", str(self.bin / "kf-box-kill"), "after run"],
            env=base_env({"KF_CI_ENV_FILE": str(env_file)}),
            capture_output=True,
            text=True,
        )
        self.assertEqual(out.returncode, 0, out.stderr)
        self.assertEqual(b.hits("kill")[0]["body"], b"after run")

    def test_payload_absent_is_a_structured_fail(self):
        b = self.broker()
        root = self.tree(None)
        proc = self.onstart(root, b)
        self.assertNotEqual(proc.returncode, 0)
        result = b.results()[0]
        self.assertEqual(result["verdict"], "FAIL")
        self.assertEqual([r["name"] for r in result["suites"]], ["payload_missing"])
        self.assertIn("scripts/ci/vast_payload.sh is absent", b.texts())

    def test_missing_variable_fails_before_the_payload_runs(self):
        b = self.broker()
        root = self.tree('touch "$KF_CI_WORKSPACE/payload-ran"\n' + self.OK_PAYLOAD)
        proc = self.onstart(root, b, drop=("KF_NONCE",))
        self.assertNotEqual(proc.returncode, 0)
        self.assertFalse((self.ws / "payload-ran").exists())
        self.assertIn("KF_NONCE", b.texts())
        self.assertEqual(b.results(), [])
        self.assertNotIn(TOKEN, proc.stdout + proc.stderr)

    def test_malformed_variable_posts_a_fail_result_and_skips_the_payload(self):
        b = self.broker()
        root = self.tree('touch "$KF_CI_WORKSPACE/payload-ran"\n')
        proc = self.onstart(root, b, env_extra={"KF_GRACE_FAIL": "soon"})
        self.assertNotEqual(proc.returncode, 0)
        self.assertFalse((self.ws / "payload-ran").exists())
        self.assertEqual(b.results()[0]["suites"][0]["name"], "env_contract")

    def test_stale_result_file_is_not_trusted(self):
        b = self.broker()
        root = self.tree("true\n")
        self.ws.mkdir(exist_ok=True)
        result_file(self.ws / "kf-result.json")
        proc = self.onstart(root, b)
        self.assertNotEqual(b.results()[0]["verdict"], "PASS")
        self.assertEqual(b.results()[0]["suites"][0]["name"], "result_missing")
        self.assertEqual(proc.returncode, 1)

    def test_soft_deadline_kills_the_payload_and_reports_the_finished_prefix(self):
        b = self.broker()
        script = 'echo "suite one done"\nsleep 120\necho never-printed\n'
        root = self.tree(script)
        deadline = str(int(time.time()) + 4)
        proc = self.onstart(root, b, env_extra={"KF_SOFT_DEADLINE": deadline})
        self.assertNotEqual(proc.returncode, 0)
        self.assertLess(self.took, 40)
        result = b.results()[0]
        self.assertEqual(result["verdict"], "FAIL")
        self.assertIn("timeout", [r["name"] for r in result["suites"]])
        self.assertIn("suite one done", b.texts())
        self.assertNotIn("never-printed", b.texts())

    def test_soft_deadline_keeps_partial_suites_in_the_result(self):
        b = self.broker()
        script = self.OK_PAYLOAD + "sleep 120\n"
        root = self.tree(script)
        deadline = str(int(time.time()) + 4)
        self.onstart(root, b, env_extra={"KF_SOFT_DEADLINE": deadline})
        result = b.results()[0]
        self.assertEqual(result["verdict"], "FAIL")
        self.assertEqual([r["name"] for r in result["suites"]], ["smoke", "timeout"])

    def test_soft_deadline_already_passed_does_not_run_the_payload(self):
        b = self.broker()
        root = self.tree('touch "$KF_CI_WORKSPACE/payload-ran"\n')
        proc = self.onstart(root, b, env_extra={"KF_SOFT_DEADLINE": str(int(time.time()) - 5)})
        self.assertNotEqual(proc.returncode, 0)
        self.assertFalse((self.ws / "payload-ran").exists())
        self.assertIn("timeout", [r["name"] for r in b.results()[0]["suites"]])

    def test_backup_log_is_complete_when_the_broker_is_unreachable(self):
        root = self.tree("seq 1 2000\nexit 4\n")
        proc = self.onstart(root, None, env_extra={"KF_CI_REPORTER_ARGS": " ".join(fast("1"))})
        self.assertEqual(proc.returncode, 4)
        lines = (self.ws / "kf-ci-output.log").read_text().split()
        self.assertEqual(lines[:2000], [str(i) for i in range(1, 2001)])

    def test_missing_tools_are_installed_and_present_tools_are_not(self):
        calls = self.tmp / "apt-calls"
        curl = shutil.which("curl")
        stub = '#!/bin/sh\necho "$@" >> "%s"\nln -s "%s" "$KF_STUB_BIN/curl"\n' % (calls, curl)
        b = self.broker()
        root = self.tree(self.OK_PAYLOAD)
        bindir = make_bin(self.tmp / "tools", TOOLS, {"apt-get": stub})
        proc = self.onstart(root, b, path=str(bindir), env_extra={"KF_STUB_BIN": str(bindir)})
        self.assertEqual(proc.returncode, 0, proc.stdout + proc.stderr)
        text = calls.read_text()
        self.assertIn("install", text)
        for package in ("curl", "python3", "git", "ca-certificates"):
            self.assertIn(package, text)
        calls.unlink()
        b2 = self.broker()
        proc = self.onstart(
            self.tree(self.OK_PAYLOAD), b2, path=str(bindir), env_extra={"KF_STUB_BIN": str(bindir)}
        )
        self.assertEqual(proc.returncode, 0, proc.stdout + proc.stderr)
        self.assertFalse(calls.exists(), "apt-get ran although every tool was present")

    def test_tool_install_failure_is_a_clear_error(self):
        bindir = make_bin(self.tmp / "tools", TOOLS, {"apt-get": "#!/bin/sh\nexit 0\n"})
        root = self.tree(self.OK_PAYLOAD)
        proc = self.onstart(root, self.broker(), path=str(bindir))
        self.assertNotEqual(proc.returncode, 0)
        self.assertIn("curl", proc.stdout + proc.stderr)


if __name__ == "__main__":
    unittest.main()
