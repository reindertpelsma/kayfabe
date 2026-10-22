#!/usr/bin/env python3
"""Trusted BASE-branch controller. Run with python3 -I; stdlib only.
Worker text is display-only. No checkout, cache, binary/artifact input or upload.
"""
import contextlib, dataclasses, hashlib, http.client, io, json, math, os, re, secrets, signal, sys, time, urllib.parse
from pathlib import Path

MAX_RESPONSE = 262144
MAX_LOG = 4194304
STATES = {
    "NONE",
    "CREATING",
    "BOOTING",
    "TESTING",
    "GRACE",
    "MANUAL",
    "NOT_NEEDED",
    "NOT_ELIGIBLE",
    "NEEDS_APPROVAL",
    "RENTING_DISABLED",
    "BUSY",
    "UNAVAILABLE",
    "UNAVAILABLE_BUDGET",
    "UNAUTHORIZED",
    "BAD_REQUEST",
    "NOT_FOUND",
    "OWNERSHIP_MISMATCH",
    "RATE_LIMIT",
}
VERDICTS = {"PASS", "FAIL", "UNAVAILABLE"}
ZERO = "0" * 64


class Invalid(Exception):
    pass


@dataclasses.dataclass
class Reply:
    status: int
    headers: dict
    body: bytes


def strict_json(raw):
    if not isinstance(raw, bytes) or len(raw) > MAX_RESPONSE:
        raise Invalid()

    def pairs(items):
        result = {}
        for key, value in items:
            if key in result:
                raise Invalid()
            result[key] = value
        return result

    def bad(_):
        raise Invalid()

    try:
        value = json.loads(
            raw.decode("utf-8", errors="strict"), object_pairs_hook=pairs, parse_constant=bad
        )
    except (ValueError, UnicodeError, RecursionError):
        raise Invalid() from None

    def depth(v, n=0):
        if n > 8:
            raise Invalid()
        if isinstance(v, dict):
            for k, x in v.items():
                if not isinstance(k, str) or len(k) > 80:
                    raise Invalid()
                depth(x, n + 1)
        elif isinstance(v, list):
            if len(v) > 100:
                raise Invalid()
            for x in v:
                depth(x, n + 1)
        elif isinstance(v, str):
            try:
                v.encode("utf-8", errors="strict")
            except UnicodeError:
                raise Invalid() from None

    depth(value)
    if not isinstance(value, dict):
        raise Invalid()
    return value


def scrub(raw, max_bytes=65536, max_lines=200, max_line=1000, redactions=()):
    """Allow TAB/LF, printable ASCII and precomposed Latin letters U+00C0..FF
    excluding multiplication/division signs. No other Unicode is displayed.
    Remove complete terminal control sequences before the character allow-list.
    Caps include runner prefixes; the fixed drop marker is additional <=48 bytes.
    """
    if isinstance(raw, str):
        raw = raw.encode("utf-8", errors="replace")
    original = len(raw)
    text = raw.decode("utf-8", errors="replace")
    for secret in sorted(set(redactions), key=len, reverse=True):
        if secret:
            text = text.replace(secret, "[redacted]")
    text = re.sub(r"(?:\x1b\]|\x9d)[\s\S]*?(?:\x07|\x1b\\|\x9c|$)", "", text)
    text = re.sub(r"(?:\x1b[P^_]|[\x90\x98\x9e\x9f])[\s\S]*?(?:\x1b\\|\x9c|$)", "", text)
    text = re.sub(r"(?:\x1b\[|\x9b)[0-?]*[ -/]*[@-~]", "", text)
    text = re.sub(r"\x1b[ -/]*[@-~]", "", text)
    text = "".join(
        c
        for c in text
        if c in "\t\n" or 32 <= ord(c) <= 126 or 192 <= ord(c) <= 255 and ord(c) not in {215, 247}
    )
    text = re.sub(r"\bBearer\s+[^\s]+", "Bearer [redacted]", text, flags=re.I)
    text = re.sub(
        r"\b(?:gh[pousr]_[A-Za-z0-9]{20,255}|github_pat_[A-Za-z0-9_]{20,255})\b", "[redacted]", text
    )
    text = re.sub(r"[A-Za-z0-9_-]{32,}", "[redacted]", text)
    text = re.sub(
        r":{2,}", lambda x: (": " * (len(x[0]) // 2) + (":" if len(x[0]) % 2 else "")), text
    ).replace("##[", "# [")
    rows = []
    size = 0
    kept = 0
    for line in text.splitlines(keepends=True):
        had_lf = line.endswith("\n")
        line = line[:-1] if had_lf else line
        if len(rows) >= max_lines:
            break
        safe = line.encode()[:max_line].decode("utf-8", errors="ignore")
        rendered = "[untrusted] " + safe + "\n"
        length = len(rendered.encode())
        if size + length > max_bytes:
            break
        rows.append(rendered)
        size += length
        kept += len(safe.encode()) + int(had_lf)
    dropped = max(0, original - kept)
    return (
        "".join(rows) + ("[" + str(dropped) + " bytes dropped by runner]\n" if dropped else ""),
        dropped,
    )


def integer(v, lo=0, hi=MAX_LOG):
    return type(v) is int and lo <= v <= hi


def hex_value(v, n):
    return isinstance(v, str) and re.fullmatch("[0-9a-f]{" + str(n) + "}", v) is not None


def chain(previous, chunk):
    raw = chunk.encode("utf-8", errors="strict")
    return hashlib.sha256(bytes.fromhex(previous) + len(raw).to_bytes(4, "big") + raw).hexdigest()


def validate_suites(rows, verdict):
    if not isinstance(rows, list) or not 1 <= len(rows) <= 20:
        raise Invalid()
    seen = set()
    for row in rows:
        if (
            not isinstance(row, dict)
            or set(row) != {"name", "pass", "fail", "skip"}
            or not isinstance(row["name"], str)
            or re.fullmatch("[a-z][a-z0-9_]{0,47}", row["name"]) is None
            or row["name"] in seen
            or not all(integer(row[k], 0, 10000000) for k in ["pass", "fail", "skip"])
        ):
            raise Invalid()
        seen.add(row["name"])
    if verdict == "PASS":
        if any(r["fail"] or r["skip"] or not r["pass"] for r in rows):
            raise Invalid()
    elif not any(r["fail"] for r in rows):
        raise Invalid()


def class_token(value):
    return isinstance(value, str) and re.fullmatch("[A-Z][A-Z0-9_]{1,23}", value) is not None


SELECTION_REASONS = {"DEFAULT_ORDER", "PREFERENCE", "ROTATE", "NONE"}


def validate_selection(value):
    if (
        not isinstance(value, dict)
        or set(value) != {"die", "ignored", "reason"}
        or not class_token(value["die"])
        or not integer(value["ignored"], 0, 16)
        or value["reason"] not in SELECTION_REASONS
    ):
        raise Invalid()


def validate_availability(data, profile):
    if (
        set(data) != {"profile", "offers", "thresholds", "rejected", "verified_counts", "cache"}
        or data["profile"] != profile
    ):
        raise Invalid()
    if (
        not isinstance(data["offers"], list)
        or len(data["offers"]) > 100
        or not isinstance(data["verified_counts"], list)
        or len(data["verified_counts"]) > 100
    ):
        raise Invalid()
    seen = set()
    for row in data["offers"]:
        if (
            not isinstance(row, dict)
            or set(row)
            != {
                "die",
                "gpu_name",
                "compute_cap",
                "count",
                "effective_hourly",
                "estimated_total",
                "kvm",
            }
            or not class_token(row["die"])
            or row["die"] in seen
            or not isinstance(row["gpu_name"], str)
            or not re.fullmatch("[A-Z0-9_]{1,100}", row["gpu_name"])
            or not integer(row["compute_cap"], 750, 2000)
            or not integer(row["count"], 1, 100)
            or row["kvm"] is not True
        ):
            raise Invalid()
        seen.add(row["die"])
        for k in ["effective_hourly", "estimated_total"]:
            if (
                type(row[k]) not in {int, float}
                or not math.isfinite(row[k])
                or not 0 <= row[k] <= 1000
            ):
                raise Invalid()
    for row in data["verified_counts"]:
        if (
            not isinstance(row, dict)
            or set(row) != {"die", "total", "verified"}
            or not class_token(row["die"])
            or not integer(row["total"], 0, 100)
            or not integer(row["verified"], 0, row["total"])
        ):
            raise Invalid()
    keys = {
        "reliability",
        "cores",
        "cpuGhz",
        "ramMb",
        "downMbps",
        "upMbps",
        "diskMbps",
        "medianRatio",
        "price",
        "bandwidth_down_gb",
        "bandwidth_up_gb",
        "verified",
    }
    if not isinstance(data["thresholds"], dict) or set(data["thresholds"]) != keys:
        raise Invalid()
    if (
        any(
            type(v) not in {int, float} or not math.isfinite(v) or not 0 < v <= 1000000
            for v in data["thresholds"].values()
        )
        or data["thresholds"]["verified"] != 1
    ):
        raise Invalid()
    if (
        not isinstance(data["rejected"], dict)
        or not set(data["rejected"])
        <= {
            "VERIFICATION",
            "CAPABILITY",
            "MACHINE",
            "DIE",
            "RELIABILITY",
            "CPU_RAM",
            "PERFORMANCE",
            "NETWORK",
            "DISK_BW",
            "DRIVER",
            "PRICE_FIELDS",
            "PRICE",
        }
        or not all(integer(v, 0, 100) for v in data["rejected"].values())
    ):
        raise Invalid()
    if (
        not isinstance(data["cache"], dict)
        or set(data["cache"]) != {"hit", "age_ms"}
        or type(data["cache"]["hit"]) is not bool
        or not integer(data["cache"]["age_ms"], 0, 60000)
    ):
        raise Invalid()
    return data


STATUS_KEYS = {
    "state",
    "verdict",
    "run",
    "id",
    "sha",
    "pr",
    "cleanup_pending",
    "result_at",
    "ended_at",
    "started_at",
    "deadline_at",
    "gpu_model",
    "cost_estimate_usd",
    "log_cursor",
    "log_tail_sanitised",
    "bytes_dropped",
    "tail_bytes_dropped",
    "runner_nonce",
    "log_sha256",
    "protocol_version",
    "worker_version",
    "bundle_sha256",
    "profile",
    "die",
    "compute_cap",
    "driver_version",
    "selection_reason",
    "preferences_ignored",
    "suites",
    "boot_deadline_at",
    "hard_deadline_at",
    "soft_deadline_at",
    "manual_deadline_at",
    "last_post_at",
    "result_deadline_at",
}
ERROR_KEYS = {"state", "verdict", "log_tail_sanitised", "bytes_dropped"}


def validate_status(data, ctx, manifest, http_status):
    if (
        not set(data) <= STATUS_KEYS
        or data.get("state") not in STATES
        or data.get("verdict") not in VERDICTS
    ):
        raise Invalid()
    for key in ["log_tail_sanitised", "gpu_model"]:
        if key in data and (
            not isinstance(data[key], str)
            or len(data[key].encode()) > (8192 if key == "log_tail_sanitised" else 100)
        ):
            raise Invalid()
    for key in ["bytes_dropped", "tail_bytes_dropped"]:
        if key in data and not integer(data[key], 0, MAX_LOG * 16):
            raise Invalid()
    if http_status != 200:
        if (
            not ERROR_KEYS <= set(data) <= ERROR_KEYS | {"reason", "selection"}
            or data["verdict"] != "UNAVAILABLE"
        ):
            raise Invalid()
        if "reason" in data and data["reason"] not in {
            "NO_ALLOWED_PREFERENCE",
            "NO_ALLOWED_OFFERS",
        }:
            raise Invalid()
        if "selection" in data:
            validate_selection(data["selection"])
        return data
    if set(data) != STATUS_KEYS:
        raise Invalid()
    if (
        data["run"] != ctx["run"]
        or data["sha"] != ctx["sha"]
        or data["pr"] != ctx["pr"]
        or type(data["pr"]) not in {int, type(None)}
    ):
        raise Invalid()
    if data["runner_nonce"] != ctx["nonce"] or not hex_value(data["log_sha256"], 64):
        raise Invalid()
    for key in manifest:
        if type(data[key]) is not type(manifest[key]) or data[key] != manifest[key]:
            raise Invalid()
    if data["id"] is not None and (
        not isinstance(data["id"], str) or not re.fullmatch("[1-9][0-9]{0,19}", data["id"])
    ):
        raise Invalid()
    if type(data["cleanup_pending"]) is not bool or not integer(data["log_cursor"], 0, 10000):
        raise Invalid()
    for key in ["result_at", "ended_at", "started_at", "deadline_at"]:
        if data[key] is not None and not integer(data[key], 0, 4102444800000):
            raise Invalid()
    if (
        data["started_at"] is None
        or data["deadline_at"] is None
        or data["deadline_at"] < data["started_at"]
    ):
        raise Invalid()
    if data["result_at"] is not None and data["result_at"] < data["started_at"]:
        raise Invalid()
    if (
        type(data["cost_estimate_usd"]) not in {int, float}
        or not math.isfinite(data["cost_estimate_usd"])
        or not 0 <= data["cost_estimate_usd"] <= 100
    ):
        raise Invalid()
    if (
        data["profile"] != ctx.get("profile", "check")
        or data["selection_reason"] not in SELECTION_REASONS
        or not integer(data["preferences_ignored"], 0, 16)
    ):
        raise Invalid()
    if data["die"] != "unknown" and not class_token(data["die"]):
        raise Invalid()
    if (
        not integer(data["compute_cap"], 0, 2000)
        or not isinstance(data["driver_version"], str)
        or not re.fullmatch(r"unknown|\d{3}\.\d{1,3}(?:\.\d{1,2})?", data["driver_version"])
    ):
        raise Invalid()
    for key in [
        "boot_deadline_at",
        "hard_deadline_at",
        "soft_deadline_at",
        "manual_deadline_at",
        "last_post_at",
        "result_deadline_at",
    ]:
        if data[key] is not None and (not integer(data[key], data["started_at"], 4102444800000)):
            raise Invalid()
    if data["state"] != "NOT_NEEDED" and (
        data["soft_deadline_at"] is None
        or data["boot_deadline_at"] is None
        or data["manual_deadline_at"] is None
    ):
        raise Invalid()
    if (
        data["hard_deadline_at"] is not None
        and data["soft_deadline_at"] is not None
        and data["soft_deadline_at"] >= data["hard_deadline_at"]
    ):
        raise Invalid()
    if data["verdict"] == "PASS" and data["state"] not in {"GRACE", "MANUAL", "NONE", "NOT_NEEDED"}:
        raise Invalid()
    if data["state"] == "GRACE" and data["result_at"] is None:
        raise Invalid()
    if data["result_at"] is not None and data["state"] != "NOT_NEEDED":
        validate_suites(data["suites"], data["verdict"])
    elif data["suites"] != []:
        raise Invalid()
    if (
        data["verdict"] == "PASS"
        and data["state"] != "NOT_NEEDED"
        and (
            data["result_at"] is None
            or data["driver_version"] == "unknown"
            or data["compute_cap"] < 750
        )
    ):
        raise Invalid()
    return data


class Runner:
    def __init__(self, ctx, manifest, client, emit=print, summary=None):
        self.ctx = ctx
        self.manifest = manifest
        self.client = client
        self.emit = emit
        self.summary = summary
        self.metadata = None
        self.cursor = 0
        self.hash = ZERO
        self.raw = []
        self.raw_size = 0
        self.displayed = 0
        self.display_lines = 0
        self.display_dropped = 0

    def read(self, route, method="GET", body=None):
        reply = self.client(route, method, body)
        if (
            reply.status in {301, 302, 303, 307, 308}
            or reply.headers.get("content-type", "").split(";")[0].strip() != "application/json"
        ):
            raise Invalid()
        return reply.status, strict_json(reply.body)

    def show(self, text):
        if self.displayed >= 65000 or self.display_lines >= 200:
            self.display_dropped += len(text.encode())
            return
        safe, dropped = scrub(
            text,
            max_bytes=max(0, 65000 - self.displayed - 256),
            max_lines=max(0, 199 - self.display_lines),
            redactions=self.ctx.get("redactions", ()),
        )
        self.display_dropped += dropped
        self.display_lines += len(safe.splitlines())
        # Tokens originate here; they never come from Worker/box strings.
        token = secrets.token_hex(24)
        frame = (
            "::stop-commands::"
            + token
            + "\nUNTRUSTED Worker/box output\n"
            + safe
            + "::"
            + token
            + "::\n"
        )
        self.displayed += len(frame.encode())
        self.emit(frame)

    def pages(self, target):
        while self.cursor < target:
            code, data = self.read("/logs/" + self.ctx["run"] + "?cursor=" + str(self.cursor))
            if code != 200 or set(data) != {
                "cursor",
                "next_cursor",
                "chunks",
                "log_increment_sanitised",
                "log_sha256",
                "bytes_dropped",
            }:
                raise Invalid()
            chunks = data["chunks"]
            if (
                not isinstance(chunks, list)
                or not 1 <= len(chunks) <= 4
                or not integer(data["cursor"], 0, 10000)
                or data["cursor"] != self.cursor
                or not integer(data["next_cursor"], 0, 10000)
                or data["next_cursor"] != self.cursor + len(chunks)
            ):
                raise Invalid()
            if not integer(data["bytes_dropped"], 0, MAX_LOG * 16):
                raise Invalid()
            for chunk in chunks:
                if not isinstance(chunk, str) or not 0 < len(chunk.encode()) <= 16384:
                    raise Invalid()
            if data["log_increment_sanitised"] != "".join(chunks):
                raise Invalid()
            for chunk in chunks:
                self.hash = chain(self.hash, chunk)
                self.raw_size += len(chunk.encode())
                if self.raw_size > MAX_LOG:
                    raise Invalid()
                self.raw.append(chunk)
            if data["log_sha256"] != self.hash:
                raise Invalid()
            self.cursor = data["next_cursor"]
            self.show("".join(chunks))

    def execute(self):
        result = {"state": "UNAVAILABLE", "verdict": "UNAVAILABLE", "verified": 0}
        terminal = False
        try:
            profile = self.ctx.get("profile", "check")
            code, availability = self.read("/availability?profile=" + profile)
            if code != 200:
                validate_status(availability, self.ctx, self.manifest, code)
                raise Invalid()
            validate_availability(availability, profile)
            lines = [
                "Availability: " + profile,
                "die gpu compute_cap count effective_hourly estimated_total kvm",
            ]
            for row in availability["offers"]:
                lines.append(
                    " ".join(
                        [
                            row["die"],
                            row["gpu_name"],
                            str(row["compute_cap"]),
                            str(row["count"]),
                            format(row["effective_hourly"], ".6f"),
                            format(row["estimated_total"], ".6f"),
                            "1",
                        ]
                    )
                )
            lines.append(
                "Thresholds: "
                + " ".join(k + "=" + str(v) for k, v in sorted(availability["thresholds"].items()))
            )
            lines.append(
                "Rejected counts: "
                + " ".join(k + "=" + str(v) for k, v in sorted(availability["rejected"].items()))
            )
            lines.append(
                "Verified counts: "
                + " ".join(
                    r["die"] + "=" + str(r["verified"]) + "/" + str(r["total"])
                    for r in availability["verified_counts"]
                )
            )
            self.show("\n".join(lines))
            request = {
                "runner_nonce": self.ctx["nonce"],
                "tested_sha": self.ctx["sha"],
                "pr_number": self.ctx["pr"],
                "profile": profile,
            }
            if "preferences" in self.ctx:
                request["preferences"] = self.ctx["preferences"]
            code, data = self.read("/rent", "POST", request)
            for attempt in range(600):
                data = validate_status(data, self.ctx, self.manifest, code)
                if code != 200:
                    self.show(data["log_tail_sanitised"])
                    result["state"] = data["state"]
                    break
                self.pages(data["log_cursor"])
                if (
                    data["state"] == "NOT_NEEDED"
                    or data["state"] in {"GRACE", "MANUAL", "NONE"}
                    and data["result_at"] is not None
                ):
                    if self.cursor != data["log_cursor"] or self.hash != data["log_sha256"]:
                        raise Invalid()
                    self.metadata = data
                    terminal = True
                    result = {"state": data["state"], "verdict": data["verdict"], "verified": 1}
                    self.show(data["gpu_model"])
                    break
                if data["state"] not in {"CREATING", "BOOTING", "TESTING", "MANUAL"}:
                    break
                time.sleep(20)
                code, data = self.read("/status/" + self.ctx["run"])
        except (Invalid, ValueError, KeyError, TypeError, UnicodeError, OSError, TimeoutError):
            result = {"state": "UNAVAILABLE", "verdict": "UNAVAILABLE", "verified": 0}
        finally:
            if not terminal:
                try:
                    self.read("/stop/" + self.ctx["run"], "POST")
                except Exception:
                    pass
        safe, dropped = scrub(
            "".join(self.raw), max_bytes=65536, redactions=self.ctx.get("redactions", ())
        )
        self.emit("::group::Final bounded log: UNTRUSTED Worker/box output\n")
        self.show("".join(self.raw))
        self.emit(
            "["
            + str(self.display_dropped)
            + " bytes dropped by runner display budget]\n::endgroup::\n"
        )
        if self.summary is not None:
            # Every interpolated Worker text passed the local allow-list. All
            # decision fields below are locally validated enums/expected values.
            self.summary.write(
                "### UNTRUSTED Worker/box output\n\n| Field | Value |\n| --- | --- |\n"
            )
            for key, value in [
                ("State", result["state"]),
                ("Verdict", result["verdict"]),
                ("Tested SHA", self.ctx["sha"]),
                ("Worker version", self.manifest["worker_version"]),
                ("Expected bundle SHA256", self.manifest["bundle_sha256"]),
                ("Received log hash", self.hash),
                ("Displayed log SHA256", hashlib.sha256(safe.encode()).hexdigest()),
                ("Runner dropped bytes", str(dropped)),
            ]:
                self.summary.write("| " + key + " | " + value + " |\n")
            if self.metadata is not None:
                d = self.metadata
                gpu, _ = scrub(
                    d["gpu_model"],
                    max_bytes=512,
                    max_lines=1,
                    redactions=self.ctx.get("redactions", ()),
                )
                gpu = (
                    gpu.strip()
                    .replace("&", "&amp;")
                    .replace("<", "&lt;")
                    .replace(">", "&gt;")
                    .replace("|", "&#124;")
                    .replace("`", "&#96;")
                    .replace("\n", " ")
                )
                elapsed = (
                    (d["result_at"] or d["ended_at"] or d["started_at"]) - d["started_at"]
                ) / 1000
                for key, value in [
                    ("Profile", d["profile"]),
                    ("Die class", d["die"]),
                    ("Driver (untrusted)", d["driver_version"]),
                    ("Selection reason", d["selection_reason"]),
                    ("Preferences ignored", str(d["preferences_ignored"])),
                    ("GPU (untrusted)", gpu),
                    ("Elapsed seconds", format(elapsed, ".3f")),
                    ("Estimated USD", format(d["cost_estimate_usd"], ".6f")),
                ]:
                    self.summary.write("| " + key + " | " + value + " |\n")
                for row in d["suites"]:
                    self.summary.write(
                        "| Suite "
                        + row["name"]
                        + " | pass="
                        + str(row["pass"])
                        + " fail="
                        + str(row["fail"])
                        + " skip="
                        + str(row["skip"])
                        + " |\n"
                    )
                if d["profile"] == "sweep":
                    self.summary.write(
                        "| Coverage | sweep:"
                        + d["die"]
                        + ","
                        + d["driver_version"]
                        + ","
                        + time.strftime("%Y-%m-%d", time.gmtime())
                        + " |\n"
                    )
            fence = "`" * max(3, 1 + max([len(x) for x in re.findall("`+", safe)] + [0]))
            self.summary.write("\n" + fence + "text\n" + safe + fence + "\n")
        return result


@contextlib.contextmanager
def deadline(seconds):
    def expired(*_):
        raise TimeoutError()

    old = signal.signal(signal.SIGALRM, expired)
    signal.setitimer(signal.ITIMER_REAL, seconds)
    try:
        yield
    finally:
        signal.setitimer(signal.ITIMER_REAL, 0)
        signal.signal(signal.SIGALRM, old)


def https_request(url, method="GET", headers=None, body=None):
    u = urllib.parse.urlsplit(url)
    if u.scheme != "https" or not u.hostname or u.username or u.password or u.fragment:
        raise Invalid()
    connection = http.client.HTTPSConnection(u.hostname, u.port or 443, timeout=20)
    try:
        with deadline(20):
            connection.request(
                method,
                u.path + ("?" + u.query if u.query else ""),
                body=body,
                headers=headers or {},
            )
            response = connection.getresponse()
            if response.status in {301, 302, 303, 307, 308}:
                raise Invalid()
            if int(response.getheader("Content-Length", "0")) > MAX_RESPONSE:
                raise Invalid()
            raw = response.read(MAX_RESPONSE + 1)
            if len(raw) > MAX_RESPONSE:
                raise Invalid()
            return Reply(response.status, {k.lower(): v for k, v in response.getheaders()}, raw)
    finally:
        connection.close()


def main():
    output = os.environ["GITHUB_OUTPUT"]
    summary_path = os.environ["GITHUB_STEP_SUMMARY"]
    result = {"state": "UNAVAILABLE", "verdict": "UNAVAILABLE", "verified": 0}
    try:
        manifest = strict_json(Path(sys.argv[1]).read_bytes())
        if (
            set(manifest) != {"protocol_version", "worker_version", "bundle_sha256"}
            or type(manifest["protocol_version"]) is not int
            or manifest["protocol_version"] != 4
            or not re.fullmatch("v[0-9A-Za-z.-]{1,31}", manifest["worker_version"])
            or not hex_value(manifest["bundle_sha256"], 64)
        ):
            raise Invalid()
        event = strict_json(Path(os.environ["GITHUB_EVENT_PATH"]).read_bytes())
        repo = os.environ["GITHUB_REPOSITORY"]
        base = os.environ["GITHUB_SHA"]
        event_name = os.environ["GITHUB_EVENT_NAME"]
        sha = (
            event.get("pull_request", {}).get("head", {}).get("sha")
            if event_name == "pull_request_target"
            else base
        )
        pr = event.get("number") if event_name == "pull_request_target" else None
        if (
            not hex_value(sha, 40)
            or not hex_value(base, 40)
            or event_name not in {"pull_request_target", "workflow_dispatch"}
        ):
            raise Invalid()
        if event_name == "pull_request_target" and (
            event["pull_request"]["head"]["repo"]["full_name"] != repo
            or event["pull_request"]["base"]["ref"] != "master"
        ):
            result["state"] = "NOT_ELIGIBLE"
            raise Invalid()
        if event_name == "workflow_dispatch" and os.environ["GITHUB_REF"] != "refs/heads/master":
            raise Invalid()
        if sys.argv[2:] == ["--profile", "sweep"]:
            selected = event.get("inputs", {}).get("tested_sha", "")
            if selected:
                if not hex_value(selected, 40):
                    raise Invalid()
                sha = selected
        run = os.environ["GITHUB_RUN_ID"] + "-" + os.environ["GITHUB_RUN_ATTEMPT"]
        if (
            not re.fullmatch("[1-9][0-9]{0,19}-[1-9][0-9]{0,19}", run)
            or pr is not None
            and not integer(pr, 1, 2147483647)
        ):
            raise Invalid()
        origin = os.environ["WORKER_URL"].rstrip("/")
        prefix = os.environ.get("WORKER_PATH", "")
        if (
            not re.fullmatch(r"https://[A-Za-z0-9.-]+", origin)
            or prefix
            and not re.fullmatch("[A-Za-z0-9_-]{32,128}", prefix)
        ):
            raise Invalid()
        for value in [
            origin,
            urllib.parse.urlsplit(origin).hostname,
            prefix,
            *urllib.parse.urlsplit(origin).hostname.split("."),
        ]:
            if value:
                print("::add-mask::" + value, flush=True)
        print(
            "Trusted BASE "
            + base
            + "; runner nonce generated before rent; Worker "
            + manifest["worker_version"]
            + " expected bundle SHA256 "
            + manifest["bundle_sha256"],
            flush=True,
        )
        profile = "sweep" if sys.argv[2:] == ["--profile", "sweep"] else "check"
        if sys.argv[2:] and profile != "sweep":
            raise Invalid()
        preferences = event.get("inputs", {}).get(
            "preferences", "rotate" if profile == "sweep" else ""
        )
        if not isinstance(preferences, str) or len(preferences) > 400:
            raise Invalid()
        preferences = preferences.split(",") if preferences else None
        if preferences is not None and (
            len(preferences) > 16
            or not all(class_token(v) or profile == "sweep" and v == "rotate" for v in preferences)
        ):
            raise Invalid()
        ctx = {
            "profile": profile,
            **({"preferences": preferences} if preferences is not None else {}),
            "sha": sha,
            "run": run,
            "pr": pr,
            "nonce": secrets.token_hex(32),
            "redactions": [
                origin,
                urllib.parse.urlsplit(origin).hostname,
                prefix,
                urllib.parse.quote(origin, safe=""),
                *urllib.parse.urlsplit(origin).hostname.split("."),
            ],
        }

        def client(route, method="GET", body=None):
            audience = urllib.parse.quote(origin, safe="")
            print("::add-mask::" + audience, flush=True)
            token_reply = https_request(
                os.environ["ACTIONS_ID_TOKEN_REQUEST_URL"] + "&audience=" + audience,
                headers={"Authorization": "Bearer " + os.environ["ACTIONS_ID_TOKEN_REQUEST_TOKEN"]},
            )
            token_data = strict_json(token_reply.body)
            if (
                token_reply.status != 200
                or set(token_data) != {"value"}
                or not isinstance(token_data["value"], str)
                or not re.fullmatch(
                    r"[A-Za-z0-9_-]+\.[A-Za-z0-9_-]+\.[A-Za-z0-9_-]+", token_data["value"]
                )
                or len(token_data["value"]) > 8192
            ):
                raise Invalid()
            token = token_data["value"]
            ctx["redactions"].append(token)
            print("::add-mask::" + token, flush=True)
            return https_request(
                origin + ("/" + prefix if prefix else "") + route,
                method,
                {"Authorization": "Bearer " + token, "Content-Type": "application/json"},
                None if body is None else json.dumps(body, separators=(",", ":")).encode(),
            )

        with open(summary_path, "a", encoding="utf-8") as summary:
            result = Runner(
                ctx, manifest, client, emit=lambda s: print(s, end="", flush=True), summary=summary
            ).execute()
    except Exception:
        pass  # Raw exceptions may contain endpoint/Worker input.
    with open(output, "a", encoding="ascii") as outputs:
        for key in ["state", "verdict", "verified"]:
            outputs.write(key + "=" + str(result[key]) + "\n")
    return 0 if result["verified"] == 1 and result["verdict"] == "PASS" else 1


if __name__ == "__main__":
    raise SystemExit(main())
