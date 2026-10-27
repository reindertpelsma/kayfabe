#!/usr/bin/env python3
"""Box-side reporter for the Vast CI broker (stdlib only).

Run by scripts/ci/vast_onstart.sh on the rented box. Subcommands:

  check-env [--write PATH]   validate the KF_* environment contract; on failure post a
                             structured failure to the broker (when the URL, run id and token
                             are themselves valid) and exit 2; otherwise write the root-only
                             env file.
  daemon --log L --done D    tail the payload log, post numbered reports, send heartbeats and,
                             once D appears, post the structured result.

Contract (docs: scripts/ci/README.md, ci/vast-broker/docs/BOX_PROTOCOL.md): every request is
POST <KF_BROKER_URL>/report/<run> with the run token as a Bearer credential, JSON
{seq, kind, text?, result?}. A message keeps its sequence number and its exact bytes until the
broker acknowledges it, so retries are idempotent. Nothing is stripped from the log text (the
Worker and the runner sanitise); line length, chunk size, request count and total bytes are
capped. The token is sent only to KF_BROKER_URL, over HTTPS, and redirects are never followed.
"""

import argparse
import codecs
import hashlib
import json
import math
import os
import re
import shlex
import ssl
import sys
import time
import urllib.error
import urllib.request

# The Worker reads this exact line at the tested commit before renting (V6); bump it, and
# DEFAULTS.boxProtocol, together whenever the box-facing contract changes.
BOX_PROTOCOL = "7"
PROTOCOLS = {BOX_PROTOCOL}
RESULT_PATH = "/workspace/kf-result.json"
LINE_MARK = b"...[line truncated, %d bytes dropped]"
UNKNOWN_HW = {"gpu_name": "unknown", "compute_cap": 0, "driver_version": "unknown"}
EPOCH_LO, EPOCH_HI = 1_000_000_000, 4_102_444_800
SUITE_NAME = re.compile(r"[a-z][a-z0-9_]{0,47}")
GPU_NAME = re.compile(r"[A-Za-z0-9_. -]{1,100}")
DRIVER = re.compile(r"unknown|[0-9]{3}\.[0-9]{1,3}(?:\.[0-9]{1,2})?")
REASONS = {"payload_missing"}


def _int(lo, hi):
    return lambda v: re.fullmatch(r"[1-9][0-9]{0,15}", v) is not None and lo <= int(v) <= hi


def _broker_url(value):
    pattern = r"https://[A-Za-z0-9.-]+(?::[0-9]{1,5})?(?:/[A-Za-z0-9._~-]+)*"
    if re.fullmatch(pattern, value) is None or len(value) > 300:
        return False
    return not any(part in (".", "..") for part in value.split("/")[3:])


def _match(pattern):
    return lambda v: re.fullmatch(pattern, v) is not None


# The run token is a compact HS256 JWT minted and verified only by the Worker (protocol 7).
TOKEN = r"[A-Za-z0-9_-]{1,64}\.[A-Za-z0-9_-]{1,512}\.[A-Za-z0-9_-]{43}"
# (name, required, check). Optional entries are validated only when present.
SPEC = [
    ("KF_ACCESS_ID", False, _match(r"[A-Za-z0-9_.-]{1,256}")),
    ("KF_ACCESS_SECRET", False, _match(r"[A-Za-z0-9_.-]{1,256}")),
    ("KF_PROTOCOL", True, lambda v: v in PROTOCOLS),
    ("KF_BROKER_URL", True, _broker_url),
    ("KF_RUN_ID", True, _match(r"[1-9][0-9]{0,19}-[1-9][0-9]{0,19}")),
    ("KF_RUN_TOKEN", True, _match(TOKEN)),
    ("KF_NONCE", True, _match(r"[0-9a-f]{32}")),
    ("KF_SHA", True, _match(r"[0-9a-f]{40}")),
    ("KF_REPO", True, _match(r"[-\w.]+/[-\w.]+")),
    ("KF_RUNNER_NONCE", True, _match(r"[0-9a-f]{64}")),
    ("KF_BOOT_DEADLINE", False, _int(EPOCH_LO, EPOCH_HI)),
    ("KF_HARD_DEADLINE", False, _int(EPOCH_LO, EPOCH_HI)),
    ("KF_SOFT_DEADLINE", True, _int(EPOCH_LO, EPOCH_HI)),
    ("KF_HEARTBEAT_SECONDS", True, _int(1, 900)),
    ("KF_GRACE_PASS", False, _int(1, 3600)),
    ("KF_GRACE_FAIL", True, _int(1, 3600)),
]


def validate_env(env):
    """Return 'NAME: problem' strings (never values) for every contract violation."""
    problems = []
    for name, required, check in SPEC:
        value = env.get(name, "")
        if value == "":
            if required:
                problems.append("%s: missing" % name)
        elif not check(value):
            problems.append("%s: malformed" % name)
    bad = {p.split(":")[0] for p in problems}
    soft, hard = env.get("KF_SOFT_DEADLINE", ""), env.get("KF_HARD_DEADLINE", "")
    if not {"KF_SOFT_DEADLINE", "KF_HARD_DEADLINE"} & bad and soft and hard:
        if int(soft) >= int(hard):
            problems.append("KF_SOFT_DEADLINE: not before KF_HARD_DEADLINE")
    return problems


def write_env_file(path, env):
    """Write the contract variables (only) to a 0600 file in a 0700 directory."""
    directory = os.path.dirname(os.path.abspath(path))
    os.makedirs(directory, mode=0o700, exist_ok=True)
    os.chmod(directory, 0o700)
    lines = ["export %s=%s\n" % (n, shlex.quote(env[n])) for n, _, _ in SPEC if env.get(n)]
    temporary = path + ".tmp"
    descriptor = os.open(temporary, os.O_WRONLY | os.O_CREAT | os.O_TRUNC, 0o600)
    with os.fdopen(descriptor, "w") as handle:
        handle.write("".join(lines))
    os.replace(temporary, path)


# --- result ------------------------------------------------------------------------------


class ResultError(Exception):
    pass


def _unique(pairs):
    out = {}
    for key, value in pairs:
        if key in out:
            raise ValueError("duplicate field")
        out[key] = value
    return out


def _count(value):
    return type(value) is int and 0 <= value <= 10_000_000


def validate_diagnostics(records, *, budget=True):
    """Local result metadata only; never an input to a verdict or a wire extension."""
    if type(records) is not list or len(records) > 128:
        raise ValueError("diagnostic records")
    fields = {"arm", "mode", "rc", "admitted", "text", "sha256", "source_bytes", "dropped_bytes"}
    for item in records:
        if type(item) is not dict or set(item) != fields:
            raise ValueError("diagnostic shape")
        if (
            not isinstance(item["arm"], str)
            or re.fullmatch(r"--[a-z0-9-]{1,40}|guest", item["arm"]) is None
            or item["mode"] not in ("native", "guest")
            or type(item["rc"]) is not int
            or not 0 <= item["rc"] <= 255
            or type(item["admitted"]) is not bool
            or not isinstance(item["text"], str)
            or len(item["text"].encode("utf-8")) > 2048
            or not isinstance(item["sha256"], str)
            or re.fullmatch(r"[0-9a-f]{64}", item["sha256"]) is None
            or type(item["source_bytes"]) is not int
            or not 0 <= item["source_bytes"] <= 8 * 1024 * 1024
            or type(item["dropped_bytes"]) is not int
            or not 0 <= item["dropped_bytes"] <= item["source_bytes"]
        ):
            raise ValueError("diagnostic field")
    if budget and len(json.dumps(records, separators=(",", ":")).encode()) > 49152:
        raise ValueError("diagnostic budget")


def diagnostic_record(arm, mode, rc, admitted, text):
    """Keep a small head/tail plus identity of the complete original arm log."""
    raw = text.encode("utf-8")
    limit = 256 if admitted else 2048
    if len(raw) <= limit:
        preview, dropped = text, 0
    else:
        marker = "\n[diagnostic preview: head and tail]\n"
        half = (limit - len(marker)) // 2
        head = raw[:half].decode("utf-8", errors="ignore")
        tail = raw[-half:].decode("utf-8", errors="ignore")
        preview = head + marker + tail
        dropped = len(raw) - len((head + tail).encode())
    item = dict(
        arm=arm,
        mode=mode,
        rc=rc,
        admitted=admitted,
        text=preview,
        sha256=hashlib.sha256(raw).hexdigest(),
        source_bytes=len(raw),
        dropped_bytes=dropped,
    )
    validate_diagnostics([item])
    return item


def compact_diagnostics(records):
    """Bound the local manifest, keeping the first refused arm's preview."""
    validate_diagnostics(records, budget=False)
    result = [dict(item) for item in records]
    first_fault = next((i for i, item in enumerate(result) if not item["admitted"]), None)
    order = [i for i, item in enumerate(result) if item["admitted"]]
    order += [i for i, item in enumerate(result) if not item["admitted"] and i != first_fault]
    for i in order:
        if len(json.dumps(result, separators=(",", ":")).encode()) <= 49152:
            break
        result[i]["text"] = ""
        result[i]["dropped_bytes"] = result[i]["source_bytes"]
    validate_diagnostics(result)
    return result


def read_result_file(path):
    """Strictly parse local results, including optional bounded arm diagnostics."""
    with open(path, "rb") as source:
        raw = source.read(65537)
    try:
        if len(raw) > 65536:
            raise ValueError("size")
        doc = json.loads(raw.decode("utf-8"), object_pairs_hook=_unique)
        allowed = {
            "verdict",
            "suites",
            "hardware",
            "nonce",
            "sha",
            "runner_nonce",
            "diagnostics",
            "host_reference",
        }
        if type(doc) is not dict or not {"verdict", "suites"} <= set(doc) <= allowed:
            raise ValueError("shape")
        if doc["verdict"] not in ("PASS", "FAIL", "UNAVAILABLE", "PROVIDER_FAULT"):
            raise ValueError("verdict")
        if "host_reference" in doc and doc["host_reference"] not in (
            "GREEN",
            "NOT_GREEN",
            "MISSING",
        ):
            raise ValueError("host reference")
        rows = doc["suites"]
        if type(rows) is not list or not 1 <= len(rows) <= 19:
            raise ValueError("suites")
        for row in rows:
            if type(row) is not dict or set(row) != {"name", "pass", "fail", "skip"}:
                raise ValueError("row")
            if not isinstance(row["name"], str) or not SUITE_NAME.fullmatch(row["name"]):
                raise ValueError("name")
            if not all(_count(row[k]) for k in ("pass", "fail", "skip")):
                raise ValueError("count")
        if len({row["name"] for row in rows}) != len(rows):
            raise ValueError("duplicate suite")
        if "diagnostics" in doc:
            validate_diagnostics(doc["diagnostics"])
        hardware = doc.get("hardware")
        if hardware is not None:
            ok = type(hardware) is dict and set(hardware) == set(UNKNOWN_HW)
            ok = ok and isinstance(hardware["gpu_name"], str)
            ok = ok and GPU_NAME.fullmatch(hardware["gpu_name"]) is not None
            ok = ok and type(hardware["compute_cap"]) is int
            ok = ok and 0 <= hardware["compute_cap"] <= 2000
            ok = ok and isinstance(hardware["driver_version"], str)
            ok = ok and DRIVER.fullmatch(hardware["driver_version"]) is not None
            if not ok:
                raise ValueError("hardware")
        return doc
    except (ValueError, UnicodeError) as error:
        raise ResultError(str(error)) from None


def _ensure_fail(rows, name):
    for row in rows:
        if row["name"] == name:
            row["fail"] = max(row["fail"], 1)
            return
    rows.append({"name": name, "pass": 0, "fail": 1, "skip": 0})


def final_result(env, rc, timed_out, reason, path):
    """Build the structured result. Exit status and log text never supply a PASS."""
    rows, hardware, verdict = [], dict(UNKNOWN_HW), "FAIL"
    reference = {}
    if reason in REASONS:
        _ensure_fail(rows, reason)
    else:
        try:
            doc = read_result_file(path)
            bound = {"nonce": env["KF_NONCE"], "sha": env["KF_SHA"]}
            bound["runner_nonce"] = env["KF_RUNNER_NONCE"]
            if any(key in doc and doc[key] != value for key, value in bound.items()):
                raise ResultError("binding")
            verdict, rows = doc["verdict"], [dict(row) for row in doc["suites"]]
            hardware = dict(doc.get("hardware") or UNKNOWN_HW)
            if "host_reference" in doc:
                reference["host_reference"] = doc["host_reference"]
        except FileNotFoundError:
            if not timed_out:
                _ensure_fail(rows, "result_missing")
        except ResultError:
            _ensure_fail(rows, "result_invalid")
    complete = bool(rows) and all(r["fail"] == 0 and r["skip"] == 0 and r["pass"] > 0 for r in rows)
    known = hardware["gpu_name"] != "unknown" and hardware["driver_version"] != "unknown"
    if timed_out:
        verdict = "FAIL"
        _ensure_fail(rows, "timeout")
    elif rc != 0 and verdict == "PASS":
        verdict = "FAIL"
        _ensure_fail(rows, "app_exit")
    elif verdict == "PASS" and not (complete and known and hardware["compute_cap"] >= 750):
        verdict = "FAIL"
        _ensure_fail(rows, "incomplete")
    if verdict != "PASS" and not any(row["fail"] > 0 for row in rows):
        _ensure_fail(rows, "unavailable" if verdict == "UNAVAILABLE" else "payload_fail")
    return {
        "nonce": env["KF_NONCE"],
        "sha": env["KF_SHA"],
        "runner_nonce": env["KF_RUNNER_NONCE"],
        "verdict": verdict,
        "suites": rows,
        "hardware": hardware,
        **reference,
    }


# --- transport ---------------------------------------------------------------------------


class _NoRedirect(urllib.request.HTTPRedirectHandler):
    def redirect_request(self, *args, **kwargs):
        return None


USER_AGENT = "kayfabe-vast-box/7"


class HttpsTransport:
    """HTTPS POST to one fixed URL; no proxies, no redirects; token only in that header."""

    def __init__(self, token, timeout=10.0, access=None):
        self.token = token
        ident, secret = (access or {}).get("KF_ACCESS_ID"), (access or {}).get("KF_ACCESS_SECRET")
        # The Cloudflare Access pair the Worker injected, sent only when both halves are set.
        self.access = (
            {"CF-Access-Client-Id": ident, "CF-Access-Client-Secret": secret}
            if ident and secret
            else {}
        )
        self.timeout = timeout
        self.opener = urllib.request.build_opener(
            urllib.request.ProxyHandler({}),
            _NoRedirect(),
            urllib.request.HTTPSHandler(context=ssl.create_default_context()),
        )

    def post(self, url, body):
        """Return (status, parsed JSON or None); status None means no HTTP answer."""
        headers = {
            "Content-Type": "application/json",
            "Authorization": "Bearer " + self.token,
            # Cloudflare answers Python's default urllib User-Agent with 403 error 1010 before
            # Access sees the request (measured on a real box, 2026-10-25).
            "User-Agent": USER_AGENT,
        }
        headers.update(self.access)
        request = urllib.request.Request(url, data=body, headers=headers, method="POST")
        try:
            with self.opener.open(request, timeout=self.timeout) as response:
                status, raw = response.status, response.read(65536)
        except urllib.error.HTTPError as error:
            status, raw = error.code, error.read(65536)
        except (urllib.error.URLError, OSError, ValueError):
            return None, None
        try:
            return status, json.loads(raw)
        except ValueError:
            return status, None


# --- reporter ----------------------------------------------------------------------------


class Limits:
    def __init__(self, **kw):
        self.chunk = 8192  # bytes of log text per post
        self.flush = 3.0  # seconds before pending text is posted
        self.min_gap = 1.0  # seconds between request starts (Worker: 20 per 10 s)
        self.poll = 0.5
        self.retry_base, self.retry_cap, self.rate_backoff = 1.0, 30.0, 10.0
        self.final_budget = 300.0
        self.max_posts, self.max_bytes = 1500, 3_000_000
        self.max_requests, self.hard_requests = 1500, 1950
        self.line_cap = 4000
        self.buffer_cap = 1 << 20
        # When a cap binds, the END of the log is what survives (gate visibility, 2026-10-26): the
        # last tail_posts chunks are reserved for it, and so are tail_requests requests.
        self.tail_posts = 2
        self.tail_requests = 8
        self.__dict__.update(kw)


GAP_MARK = "[kf-report] %d bytes omitted\n"


class LineCapper:
    """Cap every line at `cap` bytes, marking truncation; state survives chunk boundaries."""

    def __init__(self, cap):
        self.cap, self.col, self.dropped = cap, 0, 0

    def feed(self, data):
        out = bytearray()
        pieces = data.split(b"\n")
        for index, piece in enumerate(pieces):
            room = self.cap - self.col
            if len(piece) <= room:
                out += piece
                self.col += len(piece)
            else:
                out += piece[: max(room, 0)]
                self.dropped += len(piece) - max(room, 0)
                self.col = self.cap
            if index < len(pieces) - 1:
                if self.dropped:
                    out += LINE_MARK % self.dropped
                out += b"\n"
                self.col, self.dropped = 0, 0
        return bytes(out)


class Reporter:
    def __init__(self, base, run, token, heartbeat, limits=None, transport=None, clock=None):
        self.url = "%s/report/%s" % (base, run)
        self.heartbeat = heartbeat
        self.limits = limits or Limits()
        self.transport = transport or HttpsTransport(token)
        self.clock = clock or time.monotonic
        self.sleep = time.sleep
        self.capper = LineCapper(self.limits.line_cap)
        self.decoder = codecs.getincrementaldecoder("utf-8")("replace")
        self.text, self.omitted = "", 0
        self.seq = self.posts = self.bytes = self.requests = self.attempts = 0
        self.pending = self.ack = self.last_req = None
        self.next_try, self.dead, self.fallback_used = 0.0, False, False
        self.progress_acked = False
        self.progress_refused = False
        self.gap = 0  # bytes dropped since the last post; announced at the head of the next one
        self.tail_mode = False  # a cap binds: only the newest tail_keep bytes are kept
        self.finishing = False
        self.last_ok = self.last_post = self.clock()

    def say(self, message):
        print("[kf-report] " + message, flush=True)

    def feed(self, data):
        self.text += self.decoder.decode(self.capper.feed(data))
        self._drop_oldest(self._tail_keep() if self.tail_mode else self.limits.buffer_cap)

    def _tail_keep(self):
        return self.limits.chunk * self.limits.tail_posts

    def _drop_oldest(self, keep):
        """Keep at most `keep` bytes of unsent text, the NEWEST ones; the gap is announced later."""
        if len(self.text) * 4 <= keep:
            return
        raw = self.text.encode()
        if len(raw) <= keep:
            return
        kept = raw[len(raw) - keep :].decode("utf-8", "ignore")
        dropped = len(raw) - len(kept.encode())
        self.text = kept
        self.omitted += dropped
        self.gap += dropped

    def _over(self, body_size, reserve):
        """Would this post break a cap? `reserve` keeps room for the end of the log."""
        limits = self.limits
        posts = min(limits.tail_posts, limits.max_posts // 2) if reserve else 0
        requests = min(limits.tail_requests, limits.max_requests // 4) if reserve else 0
        size = (
            min(limits.tail_posts * (limits.chunk + 256), limits.max_bytes // 4) if reserve else 0
        )
        return (
            self.posts + posts >= limits.max_posts
            or self.bytes + body_size + size > limits.max_bytes
            or self.requests + requests >= limits.max_requests
        )

    def _fit_tail(self):
        """At the end: the newest text that the remaining post, byte and request caps still allow."""
        limits = self.limits
        posts = max(0, min(limits.max_posts - self.posts, limits.max_requests - self.requests))
        room = max(0, limits.max_bytes - self.bytes)
        self._drop_oldest(min(posts * limits.chunk, room))
        while self.text:
            size = len(self.text.encode()) + len(GAP_MARK % (self.gap + 10**9))
            needed = -(-size // limits.chunk)
            if needed <= posts and len(json.dumps(self.text)) + 80 * needed <= room:
                return
            self._drop_oldest(max(0, len(self.text.encode()) * 7 // 8 - 1))

    def _message(self, kind, text=None, result=None):
        self.seq += 1
        payload = {"seq": self.seq, "kind": kind}
        if text is not None:
            payload["text"] = text
        if result is not None:
            payload["result"] = result
        body = json.dumps(payload, separators=(",", ":"), ensure_ascii=True).encode()
        self.pending = {"kind": kind, "body": body, "text": text, "result": result}
        self.attempts, self.next_try = 0, 0.0
        self.last_post = self.clock()

    def progress(self, text):
        self._message("progress", text)

    def _build_log(self, force):
        limits = self.limits
        if not self.text or not (force or len(self.text) >= limits.chunk or self._flush_due()):
            return
        if self.finishing:
            self._fit_tail()
            if not self.text:
                return
        prefix = GAP_MARK % self.gap if self.gap else ""
        room = limits.chunk - len(prefix)
        chunk = self.text[:room]
        if len(chunk.encode()) > room:
            chunk = chunk.encode()[:room].decode("utf-8", "ignore")
        body_size = len(json.dumps({"seq": self.seq + 1, "kind": "log", "text": prefix + chunk}))
        if self._over(body_size, reserve=not self.finishing):
            if self.finishing:
                self.omitted += len(self.text.encode())
                self.text = ""
            else:
                self.tail_mode = True
                self._drop_oldest(self._tail_keep())
            return
        self.text = self.text[len(chunk) :]
        self.posts += 1
        self._message("log", prefix + chunk)
        self.gap = 0
        self.bytes += len(self.pending["body"])

    def _flush_due(self):
        return self.clock() - self.last_post >= self.limits.flush

    def pump(self, force=False):
        """Send, retry or build at most one request; never blocks beyond one HTTP call."""
        if self.dead:
            self.pending = None
            return
        if self.pending is None:
            self._build_log(force)
        if self.pending is None and self.clock() - self.last_ok >= self.heartbeat:
            self._message("log", "")
        if self.pending is not None:
            self._attempt()

    def _attempt(self):
        now, limits = self.clock(), self.limits
        if now < self.next_try:
            return
        if self.last_req is not None and now - self.last_req < limits.min_gap:
            return
        if self.requests >= limits.hard_requests:
            self.say("request budget exhausted; reporting stopped")
            self.dead, self.pending = True, None
            return
        self.last_req, self.requests = now, self.requests + 1
        status, doc = self.transport.post(self.url, self.pending["body"])
        message, state = self.pending, (doc or {}).get("state") if isinstance(doc, dict) else None
        if status == 200:
            self.pending, self.attempts, self.last_ok = None, 0, self.clock()
            if message["kind"] == "progress":
                self.progress_acked = True
            if message["kind"] == "result":
                self.ack = doc if isinstance(doc, dict) else {}
            return
        if status in (401, 403, 404):
            self.say("broker refused the report (HTTP %d); reporting stopped" % status)
            self.dead, self.pending = True, None
            return
        if status in (400, 413):
            if message["kind"] == "progress":
                self.progress_refused = True
            self.say("broker rejected a %s report (HTTP %d)" % (message["kind"], status))
            self.pending = None
            if message["kind"] == "result" and not self.fallback_used and self.fallback:
                self.fallback_used = True
                self._message("result", self.fallback_text, self.fallback)
            else:
                self.omitted += len((message["text"] or "").encode())
            return
        self.attempts += 1
        delay = min(limits.retry_cap, limits.retry_base * 2 ** min(self.attempts - 1, 10))
        if status == 429:
            delay = max(delay, limits.rate_backoff)
        elif status == 409 and state == "REPORT_SEQUENCE":
            self.say("sequence conflict; continuing with the next number")
            body = json.loads(message["body"])
            self.pending = None
            self._message(body["kind"], body.get("text"), body.get("result"))
            return
        self.next_try = now + delay

    fallback = None
    fallback_text = "Result was rejected; sending a conservative failure."

    def finish(self, result, note, fallback=None):
        """Drain the text, then deliver `result` (if any). Returns the broker's ack or None."""
        self.fallback = fallback
        self.finishing = True
        end = self.clock() + self.limits.final_budget
        built = result is None
        while not self.dead and self.clock() < end:
            if self.pending is None:
                self._build_log(True)
            if self.pending is None and not self.text:
                if built:
                    break
                text = note
                if self.omitted:
                    text += " Sender omitted %d raw bytes beyond its bounds." % self.omitted
                self._message("result", text, result)
                built = True
            if self.pending is not None:
                self._attempt()
            self.sleep(min(self.limits.poll, 0.05))
        if self.pending is not None or self.dead:
            return None
        return self.ack if result is not None else {}


# --- daemon ------------------------------------------------------------------------------


class LogTail:
    def __init__(self, path):
        self.path, self.offset = path, 0

    def read(self):
        try:
            with open(self.path, "rb") as handle:
                size = os.fstat(handle.fileno()).st_size
                if size < self.offset:
                    self.offset = 0
                handle.seek(self.offset)
                data = handle.read(1 << 20)
        except OSError:
            return b""
        self.offset += len(data)
        return data


def read_done(path):
    try:
        with open(path, "r", encoding="utf-8") as handle:
            lines = handle.read(512).split("\n")
    except OSError:
        return None
    fields = dict(line.split("=", 1) for line in lines if "=" in line)
    try:
        rc = int(fields["rc"])
    except (KeyError, ValueError):
        return None
    reason = fields.get("reason")
    return {"rc": rc, "timeout": fields.get("timeout") == "1", "reason": reason}


def fail_result(env, name):
    return {
        "nonce": env["KF_NONCE"],
        "sha": env["KF_SHA"],
        "runner_nonce": env["KF_RUNNER_NONCE"],
        "verdict": "FAIL",
        "suites": [{"name": name, "pass": 0, "fail": 1, "skip": 0}],
        "hardware": dict(UNKNOWN_HW),
    }


def make_limits(args):
    names = "poll flush min_gap retry_base retry_cap rate_backoff final_budget".split()
    names += "max_posts max_bytes line_cap".split()
    return Limits(**{n: getattr(args, n) for n in names if getattr(args, n) is not None})


def make_reporter(env, args):
    heartbeat = env.get("KF_HEARTBEAT_SECONDS", "")
    seconds = int(heartbeat) if heartbeat.isdigit() and int(heartbeat) > 0 else 600
    return Reporter(
        env["KF_BROKER_URL"],
        env["KF_RUN_ID"],
        env["KF_RUN_TOKEN"],
        seconds,
        make_limits(args),
        transport=HttpsTransport(env["KF_RUN_TOKEN"], access=env),
    )


def run_daemon(args, env):
    problems = validate_env(env)
    if problems:
        print("[kf-report] environment contract violated: " + "; ".join(problems))
        return 2
    reporter = make_reporter(env, args)
    tail, soft = LogTail(args.log), int(env["KF_SOFT_DEADLINE"])
    reporter.progress("Box ready; payload starting")
    ready_fd = getattr(args, "ready_fd", None)
    done = None
    while done is None:
        data = tail.read()
        if data:
            reporter.feed(data)
        done = read_done(args.done)
        if done is None and time.time() > soft + 180:
            reporter.say("payload overran the soft deadline; reporting a timeout")
            done = {"rc": 124, "timeout": True, "reason": None}
        if done is None:
            reporter.pump()
            if ready_fd is not None and (
                reporter.progress_acked or reporter.dead or reporter.progress_refused
            ):
                os.write(ready_fd, b"READY\n" if reporter.progress_acked else b"REFUSED\n")
                os.close(ready_fd)
                ready_fd = None
            time.sleep(reporter.limits.poll)
    for data in iter(tail.read, b""):
        reporter.feed(data)
    result = final_result(env, done["rc"], done["timeout"], done["reason"], args.result)
    fallback = fail_result(env, "result_rejected")
    ack = reporter.finish(result, "Suite complete.", fallback)
    if ack is None:
        reporter.say("result not delivered; the backup log is %s" % args.log)
        return 3
    grace = max(int(env.get("KF_GRACE_PASS") or 0), int(env["KF_GRACE_FAIL"]))
    seconds = ack.get("grace_remaining_seconds")
    left = (
        str(math.ceil(seconds / 60))
        if type(seconds) is int and 0 <= seconds <= grace
        else "unknown"
    )
    print(
        "Use kf-box-kill [reason] to end this box early; Worker reports %s minutes remaining "
        "(hard TTL included)." % left,
        flush=True,
    )
    return 0 if result["verdict"] == "PASS" and not reporter.fallback_used else 1


def load_provider_env(env, path="/etc/environment"):
    """Read missing contract fields from Vast VM data; never source shell code.

    Process values win. Provider account/instance credentials and all unrelated
    fields are ignored. The ordinary strict validator still decides acceptance.
    """
    result = dict(env)
    missing = {name for name, _, _ in SPEC if not result.get(name)}
    if not missing:
        return result
    try:
        with open(path, "rb") as source:
            raw = source.read(65537)
    except FileNotFoundError:
        return result
    if len(raw) > 65536:
        raise ValueError("provider environment too large")
    seen = set()
    for line in raw.decode("utf-8").splitlines():
        name, separator, value = line.partition("=")
        name = name.strip()
        if not separator or name not in missing:
            continue
        if name in seen:
            raise ValueError("duplicate provider contract field")
        seen.add(name)
        words = shlex.split(value, comments=False, posix=True)
        if len(words) > 1:
            raise ValueError("invalid provider contract field")
        result[name] = words[0] if words else ""
    return result


def run_check_env(args, env):
    try:
        env = load_provider_env(env, getattr(args, "provider_env", "/etc/environment"))
    except (OSError, ValueError, UnicodeError):
        sys.stderr.write("onstart: provider environment is invalid\n")
        return 2
    problems = validate_env(env)
    if not problems:
        if args.write:
            write_env_file(args.write, env)
        return 0
    text = "onstart: environment contract violated:\n" + "".join("  %s\n" % p for p in problems)
    sys.stderr.write(text)
    bad = {p.split(":")[0] for p in problems}
    if not bad & {"KF_BROKER_URL", "KF_RUN_ID", "KF_RUN_TOKEN"}:
        reporter = make_reporter(env, args)
        reporter.feed(text.encode())
        binding = {"KF_NONCE", "KF_SHA", "KF_RUNNER_NONCE"}
        result = None if bad & binding else fail_result(env, "env_contract")
        reporter.finish(result, "Environment contract violated.")
    return 2


def parser():
    top = argparse.ArgumentParser(description=__doc__.split("\n")[0])
    sub = top.add_subparsers(dest="command", required=True)
    for name in ("check-env", "daemon"):
        p = sub.add_parser(name)
        for option in ("poll", "flush", "min-gap", "retry-base", "retry-cap", "rate-backoff"):
            p.add_argument("--" + option, type=float)
        p.add_argument("--final-budget", type=float)
        for option in ("max-posts", "max-bytes", "line-cap"):
            p.add_argument("--" + option, type=int)
        if name == "check-env":
            p.add_argument("--write")
            p.add_argument("--provider-env", default="/etc/environment")
        else:
            p.add_argument("--log", required=True)
            p.add_argument("--done", required=True)
            p.add_argument("--result", default=RESULT_PATH)
            p.add_argument("--ready-fd", type=int)
    return top


def main(argv=None, env=None):
    args = parser().parse_args(argv)
    env = os.environ if env is None else env
    return (run_daemon if args.command == "daemon" else run_check_env)(args, env)


if __name__ == "__main__":
    sys.exit(main())
