#!/usr/bin/env python3
"""Trusted BASE-branch controller. Run with python3 -I; stdlib only.
Worker text is display-only. No checkout, cache, binary/artifact input or upload.
"""

import contextlib, dataclasses, hashlib, http.client, io, json, math, os, re, secrets, signal, sys, time, traceback, urllib.parse
from pathlib import Path

MAX_RESPONSE = 262144
# Status protocol: bump with defaults.json statusProtocol whenever the status shape changes.
STATUS_PROTOCOL = 6
MAX_LOG = 4194304
# Display policy for the box log (gate visibility, 2026-10-26): the job log shows the HEAD and the
# TAIL of the box output inside the one display budget (65000 bytes, 200 lines, see Runner.show),
# with an explicit marker for the middle. The tail is where a payload's FINAL SUMMARY lives.
HEAD_BYTES, HEAD_LINES = 24000, 100
TAIL_BYTES, TAIL_LINES = 16000, 70
STATES = {
    "NONE",
    "CREATING",
    "BOOTING",
    "TESTING",
    "GRACE",
    "NOT_ELIGIBLE",
    "RENTING_DISABLED",
    "BUSY",
    "UNAVAILABLE",
    "UNAVAILABLE_BUDGET",
    "UNAUTHORIZED",
    "BAD_REQUEST",
    "NOT_FOUND",
    "OWNERSHIP_MISMATCH",
    "RATE_LIMIT",
    "RATE_LIMITED",
}
VERDICTS = {"PASS", "FAIL", "UNAVAILABLE"}
ZERO = "0" * 64
# Review B1: the runner's budget is 200 minutes (Runner.until). A run needs the hard limit
# (defaults.json hardMinutes) plus the Worker's admission margin (admissionMarginMinutes);
# after a definite BUSY the runner stops asking once less than that is left.
ADMISSION_NEED_SECONDS = (180 + 5) * 60


class Invalid(Exception):
    pass


class Diagnosed(Invalid):
    """An Invalid that carries its one trusted diagnostic line.

    The line is built only from an integer status, a sanitised media type, an exception class
    name, a setting NAME and a fixed reason. Never a value, URL, path, header or body.
    """

    def __init__(self, line):
        super().__init__()
        self.line = line


def clean(value, limit=64):
    """Truncate to `limit` characters; anything outside [A-Za-z0-9/+.;=-] becomes '?'."""
    return re.sub(r"[^A-Za-z0-9/+.;=-]", "?", str(value)[:limit])


def reply_line(status, content_type):
    code = status if type(status) is int else "?"
    return (
        "runner: Worker reply not accepted: HTTP "
        + str(code)
        + ", content-type "
        + clean(content_type or "none")
    )


def rejected_line(error):
    """The one trusted line for a 200 reply that failed the runner's own check.

    Only the line number of the raise inside runner.py (an int taken from the traceback of our
    own code): never a field name, value, header or body.
    """
    frames = traceback.extract_tb(error.__traceback__)
    number = frames[-1].lineno if frames else None
    return "runner: Worker reply rejected by runner.py check at line " + str(
        number if type(number) is int else 0
    )


def setting_line(name, reason):
    return "runner: setting rejected before any request: " + name + " " + reason


def network_line(error):
    return "runner: network failure: " + clean(type(error).__name__)


class WaitBudget(Exception):
    """BUSY until the remaining budget can no longer hold a run: nothing was rented."""


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


# A run of 32+ [A-Za-z0-9_-] characters is blanked as a possible token, EXCEPT a snake_case test name
# (2026-10-26, gate run 40263112941: libtest prints `<name> --- FAILED` and 60-character names were
# blanked, leaving `[redacted] --- FAILED`). Kept only if: lowercase words joined by `_`, at least
# three, each at most 20 characters, at most 128 characters in all, at most one digit in five. Hex,
# base64url, mixed-case, hyphenated and digit-heavy strings still match the generic rule; configured
# secrets and token families are replaced by value or pattern regardless. src/sanitize.js and
# scripts/ci/vast_payload.py (is_test_identifier) hold the same grammar; test/fixtures/scrub-vectors.json
# pins all three.
SNAKE_NAME = re.compile(r"[a-z][a-z0-9]{0,19}(?:_[a-z0-9]{1,20}){2,}")


def blank_run(match):
    run = match[0]
    keep = (
        len(run) <= 128
        and SNAKE_NAME.fullmatch(run)
        and sum(c.isdigit() for c in run) * 5 <= len(run)
    )
    return run if keep else "[redacted]"


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
    text = re.sub(r"[A-Za-z0-9_-]{32,}", blank_run, text)
    text = re.sub(
        r":{2,}", lambda x: (": " * (len(x[0]) // 2) + (":" if len(x[0]) % 2 else "")), text
    ).replace("##[", "# [")
    text = re.sub(r"""https?://[^\s<>"']+""", "[url redacted]", text, flags=re.I)  # signed-url
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


def line_cost(line, max_line=1000):
    """Display cost of one line as scrub renders it: "[untrusted] " + the line (<= max_line) + LF."""
    body = line[:-1] if line.endswith("\n") else line
    return len("[untrusted] ") + min(len(body.encode()), max_line) + 1


def take_head(text, max_bytes, max_lines):
    """Split `text` after the last whole line that fits both budgets: (shown, rest)."""
    used = count = cut = 0
    for line in text.splitlines(keepends=True):
        cost = line_cost(line)
        if count >= max_lines or used + cost > max_bytes:
            break
        used, count, cut = used + cost, count + 1, cut + len(line)
    return text[:cut], text[cut:]


def take_tail(text, max_bytes, max_lines):
    """Split `text` before the first of the last whole lines that fit both budgets: (rest, shown)."""
    used = count = 0
    cut = len(text)
    for line in reversed(text.splitlines(keepends=True)):
        cost = line_cost(line)
        if count >= max_lines or used + cost > max_bytes:
            break
        used, count, cut = used + cost, count + 1, cut - len(line)
    return text[:cut], text[cut:]


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
        or not isinstance(value["reason"], str)
        or value["reason"] not in SELECTION_REASONS
    ):
        raise Invalid()


# Every offer rejection code the Worker can emit (src/policy.js offerRejections, src/offers.js).
REJECTION_CODES = frozenset(
    {
        "VERIFICATION",
        "CAPABILITY",
        "MACHINE",
        "BAD_HOST",
        "IN_USE",
        "DIE",
        "RELIABILITY",
        "CPU_RAM",
        "PERFORMANCE",
        "NETWORK",
        "DISK_BW",
        "PRICE_FIELDS",
        "COUNTRY",
        "PRICE",
    }
)


def provider_id(v):
    """A public Vast machine_id or host_id (item 9): a positive safe integer or null."""
    return v is None or integer(v, 1, 2**53 - 1)


def validate_availability(data, profile):
    if (
        set(data)
        != {
            "profile",
            "offers",
            "thresholds",
            "rejected",
            "rejected_offers",
            "verified_counts",
            "cache",
        }
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
                "machine_id",
                "host_id",
            }
            or not class_token(row["die"])
            or row["die"] in seen
            or not isinstance(row["gpu_name"], str)
            or not re.fullmatch("[A-Z0-9_]{1,100}", row["gpu_name"])
            or not integer(row["compute_cap"], 750, 2000)
            or not integer(row["count"], 1, 100)
            or row["kvm"] is not True
            or not provider_id(row["machine_id"])
            or not provider_id(row["host_id"])
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
        or not set(data["rejected"]) <= REJECTION_CODES
        or not all(integer(v, 0, 100) for v in data["rejected"].values())
        or not isinstance(data["rejected_offers"], list)
        or len(data["rejected_offers"]) > 100
    ):
        raise Invalid()
    counts = {}
    for row in data["rejected_offers"]:
        if (
            not isinstance(row, dict)
            or set(row) != {"machine_id", "host_id", "reasons"}
            or not provider_id(row["machine_id"])
            or not provider_id(row["host_id"])
            or not isinstance(row["reasons"], list)
            or not 1 <= len(row["reasons"]) <= len(REJECTION_CODES)
            or not all(isinstance(x, str) and x in REJECTION_CODES for x in row["reasons"])
            or len(set(row["reasons"])) != len(row["reasons"])
        ):
            raise Invalid()
        for reason in row["reasons"]:
            counts[reason] = counts.get(reason, 0) + 1
    # The counts and the rows come from one Worker pass; they must agree exactly.
    if counts != data["rejected"]:
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
    "machine_id",
    "host_id",
    "sha",
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
    "last_post_at",
    "result_deadline_at",
}
OPTIONAL_STATUS_KEYS = {
    "country",
    "instance_id",
    "geolocation",
    "dph_total",
    "host_driver_version",
    "host_reference",
}
ERROR_KEYS = {"state", "verdict", "log_tail_sanitised", "bytes_dropped"}


def validate_status(data, ctx, manifest, http_status):
    # A refusal may carry reason/selection (checked below); before edits3 this first test
    # refused them, so every reasoned refusal ended as Invalid.
    refusal = {"reason", "selection"} if http_status != 200 else set()
    if (
        not set(data) <= STATUS_KEYS | OPTIONAL_STATUS_KEYS | refusal
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
        # A str test first: a list or dict reason is Invalid, not an unhashable TypeError.
        if "reason" in data and (
            not isinstance(data["reason"], str)
            or data["reason"] not in {"NO_ALLOWED_PREFERENCE", "NO_ALLOWED_OFFERS", "RUNNER_TIME"}
        ):
            raise Invalid()
        if "selection" in data:
            validate_selection(data["selection"])
        return data
    if set(data) - OPTIONAL_STATUS_KEYS != STATUS_KEYS:
        raise Invalid()
    if data["run"] != ctx["run"] or data["sha"] != ctx["sha"]:
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
    if not provider_id(data["machine_id"]) or not provider_id(data["host_id"]):
        raise Invalid()
    if "instance_id" in data and data["instance_id"] != data["id"]:
        raise Invalid()
    location = data.get("geolocation", "unknown")
    if not isinstance(location, str) or len(location) > 128:
        raise Invalid()
    if any(ord(c) < 32 or 127 <= ord(c) <= 159 for c in location):
        raise Invalid()
    price = data.get("dph_total")
    if price is not None and (
        type(price) not in {int, float} or not math.isfinite(price) or not 0 <= price <= 1000
    ):
        raise Invalid()
    host_driver = data.get("host_driver_version", "unknown")
    if not isinstance(host_driver, str) or not re.fullmatch(
        r"unknown|\d{3}\.\d{1,3}(?:\.\d{1,2})?", host_driver
    ):
        raise Invalid()
    if data.get("host_reference", "MISSING") not in {"GREEN", "NOT_GREEN", "MISSING"}:
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
    if "country" in data and (
        not isinstance(data["country"], str)
        or not re.fullmatch(r"UNKNOWN|[A-Z]{2}", data["country"])
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
        "last_post_at",
        "result_deadline_at",
    ]:
        if data[key] is not None and (not integer(data[key], data["started_at"], 4102444800000)):
            raise Invalid()
    if data["soft_deadline_at"] is None or data["boot_deadline_at"] is None:
        raise Invalid()
    if (
        data["hard_deadline_at"] is not None
        and data["soft_deadline_at"] is not None
        and data["soft_deadline_at"] >= data["hard_deadline_at"]
    ):
        raise Invalid()
    # A PASS needs a GPU result; the Worker never decides a skip (review W2).
    if data["verdict"] == "PASS" and data["state"] not in {"GRACE", "NONE"}:
        raise Invalid()
    if data["state"] == "GRACE" and data["result_at"] is None:
        raise Invalid()
    if data["result_at"] is not None:
        validate_suites(data["suites"], data["verdict"])
    elif data["suites"] != []:
        raise Invalid()
    if data["verdict"] == "PASS" and (
        data["result_at"] is None
        or data["driver_version"] == "unknown"
        or data["compute_cap"] < 750
    ):
        raise Invalid()
    return data


def shown_id(value):
    return "-" if value is None else str(value)


def public_gpu(value):
    prefix = r"(?:NVIDIA[ _])?(?:(?:GeForce|Quadro)[ _])?"
    model = r"(?:RTX|GTX)[ _](?:PRO[ _])?[A-Z]{0,2}\d{3,4}[A-Z]{0,2}"
    suffix = r"(?:[ _-][A-Z0-9]{1,16}){0,4}"
    quadro = r"(?:NVIDIA[ _])?Quadro[ _][A-Z]?\d{3,4}"
    die = r"(?:TU|GA|AD|GB)\d{3}|CC_\d{2,3}"
    pattern = r"(?:" + prefix + model + suffix + "|" + quadro + suffix + "|" + die + ")"
    if isinstance(value, str) and len(value) <= 100 and re.fullmatch(pattern, value, re.I | re.A):
        return value
    return "unknown"


def rental_line(data):
    price = data.get("dph_total")
    return (
        "RENTAL machine="
        + shown_id(data["machine_id"])
        + " host="
        + shown_id(data["host_id"])
        + " instance="
        + shown_id(data.get("instance_id", data["id"]))
        + " gpu="
        + public_gpu(data["gpu_model"]).replace(" ", "_")
        + " price="
        + (format(price, ".6f") if price is not None else "unknown")
    )


def summary_cell(value):
    return (
        str(value)
        .replace("&", "&amp;")
        .replace("<", "&lt;")
        .replace(">", "&gt;")
        .replace("|", "&#124;")
        .replace("`", "&#96;")
        .replace("\n", " ")
    )


class Runner:
    def __init__(self, ctx, manifest, client, emit=print, summary=None):
        self.ctx = ctx
        self.manifest = manifest
        self.client = client
        self.emit = emit
        self.summary = summary
        self.metadata = None
        self.rental_shown = False
        self.cursor = 0
        self.hash = ZERO
        self.raw = []
        self.raw_size = 0
        self.displayed = 0
        self.display_lines = 0
        self.display_dropped = 0
        # Head of the box log: shown live until it is full; the rest is kept in self.raw only.
        self.head_bytes = 0
        self.head_lines = 0
        self.head_full = False
        self.head_end = 0
        self.infrastructure_failure = False
        self.rent_busy = False
        # A configured RENTING_DISABLED refusal of /rent: terminal, the Worker never touched Vast.
        self.renting_disabled = False
        # Edits4 F1: sticky for the whole /rent sequence. Once any /rent reply was ambiguous
        # (5xx, timeout, connection error, garbled page) a box may exist for this run.
        self.rent_ambiguous = False
        self.until = time.monotonic() + 600 * 20
        # The newest unrecovered failure of a Worker call, as one trusted line (see Diagnosed).
        self.diagnostic = None

    def read(self, route, method="GET", body=None):
        return self.decode(self.client(route, method, body))

    def decode(self, reply):
        if (
            reply.status in {301, 302, 303, 307, 308}
            or reply.headers.get("content-type", "").split(";")[0].strip() != "application/json"
        ):
            raise Diagnosed(reply_line(reply.status, reply.headers.get("content-type", "")))
        return reply.status, strict_json(reply.body)

    def retry_read(self, route, method="GET", body=None):
        last = None
        for attempt in range(6):
            if time.monotonic() >= self.until:
                self.infrastructure_failure = True
                raise TimeoutError()
            if route == "/rent":
                # Review B1: each request declares the remaining budget; after a definite BUSY
                # the runner stops once a run would no longer fit. After an ambiguous reply it
                # keeps asking: the Worker returns a prior admission or refuses (RUNNER_TIME).
                # Edits4 F1: a BUSY after an ambiguous reply is not definite either (the
                # Worker's busy flag answers a retry while the first create is still running).
                remaining = int(self.until - time.monotonic())
                if (
                    self.rent_busy
                    and not self.rent_ambiguous
                    and remaining < ADMISSION_NEED_SECONDS
                ):
                    raise WaitBudget()
                body["remaining_seconds"] = remaining
                self.rent_busy = False
            try:
                reply = self.client(route, method, body)
                if route.startswith(("/status/", "/logs/")) and reply.status != 200:
                    # Infrastructure error bodies are not a protocol or cleanup instruction.
                    code, data = 503, {
                        "state": "UNAVAILABLE",
                        "verdict": "UNAVAILABLE",
                        "log_tail_sanitised": "",
                        "bytes_dropped": 0,
                    }
                else:
                    try:
                        code, data = self.decode(reply)
                    except Invalid:
                        self.rent_ambiguous |= route == "/rent"
                        raise
            except Diagnosed as error:
                self.diagnostic = error.line
                raise
            except (OSError, TimeoutError) as error:
                self.diagnostic = network_line(error)
                self.infrastructure_failure = True
                self.rent_ambiguous |= route == "/rent"
                if attempt == 5:
                    raise
            else:
                if code == 200:
                    self.infrastructure_failure = False
                    self.diagnostic = None
                    return code, data
                try:
                    validate_status(data, self.ctx, self.manifest, code)
                except Invalid:
                    # A bare refusal such as {"state": "UNAUTHORIZED"} is not a status object.
                    # Name its HTTP status and, only when it is a known enum value, its state.
                    state = data.get("state") if isinstance(data, dict) else None
                    line = reply_line(code, "application/json")
                    if type(state) is str and state in STATES:
                        line += ", state " + state
                    self.diagnostic = line
                    self.rent_ambiguous |= route == "/rent"
                    raise
                if route == "/rent" and code == 503 and data["state"] == "RENTING_DISABLED":
                    # A deliberate setting, not an outage: the Worker did not touch Vast, so
                    # nothing is ambiguous and nothing is owed. Static text, no Worker string.
                    self.renting_disabled = True
                    self.infrastructure_failure = False
                    self.diagnostic = "runner: renting is disabled on the Worker"
                    return code, data
                transient = code in {429, 500, 502, 503, 504} or (
                    code == 409 and data["state"] == "BUSY"
                )
                self.rent_busy = route == "/rent" and code == 409 and data["state"] == "BUSY"
                self.rent_ambiguous |= route == "/rent" and code in {500, 502, 503, 504}
                self.infrastructure_failure = transient or route.startswith(("/status/", "/logs/"))
                if not transient:
                    return code, data
                last = (code, data)
                if attempt == 5:
                    return last
            time.sleep(min(2**attempt, 20))
        raise Invalid()

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

    def show_head(self, text):
        """Live display of the box log: only its first HEAD_BYTES / HEAD_LINES are shown."""
        if self.head_full:
            return
        shown, rest = take_head(text, HEAD_BYTES - self.head_bytes, HEAD_LINES - self.head_lines)
        if shown:
            self.show(shown)
            self.head_end += len(shown.encode())
            self.head_bytes += sum(line_cost(x) for x in shown.splitlines(keepends=True))
            self.head_lines += len(shown.splitlines(keepends=True))
        if rest:
            self.head_full = True

    def show_tail(self):
        """The end of the box log, after the head, with the size of the gap in between."""
        raw = "".join(self.raw).encode()
        if not self.head_full:
            # The whole log fit in the head budget; the bounded final log repeats it as before.
            self.show("".join(self.raw))
            return
        rest = raw[self.head_end :].decode("utf-8", errors="ignore")
        room_bytes = max(0, min(TAIL_BYTES, 65000 - self.displayed - 512))
        room_lines = max(0, min(TAIL_LINES, 199 - self.display_lines))
        skipped, tail = take_tail(rest, room_bytes, room_lines)
        middle = len(raw) - self.head_end - len(tail.encode())
        if middle > 0:
            self.emit("[" + str(middle) + " bytes dropped from the middle]\n")
        self.show(tail)

    def pages(self, target):
        while self.cursor < target:
            code, data = self.retry_read("/logs/" + self.ctx["run"] + "?cursor=" + str(self.cursor))
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
            self.show_head("".join(chunks))

    def execute(self):
        result = {"state": "UNAVAILABLE", "verdict": "UNAVAILABLE", "verified": 0}
        terminal = final = admitted = False
        try:
            profile = self.ctx.get("profile", "check")
            code, availability = self.retry_read("/availability?profile=" + profile)
            if code != 200:
                validate_status(availability, self.ctx, self.manifest, code)
                raise Invalid()
            validate_availability(availability, profile)
            lines = [
                "Availability: " + profile,
                "die gpu compute_cap count effective_hourly estimated_total kvm machine_id host_id",
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
                            shown_id(row["machine_id"]),
                            shown_id(row["host_id"]),
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
            # Packed a few per line: the display budget is 200 lines for the whole run.
            line = ""
            for row in availability["rejected_offers"]:
                entry = (
                    shown_id(row["machine_id"])
                    + "/"
                    + shown_id(row["host_id"])
                    + ":"
                    + ",".join(row["reasons"])
                )
                if line and len(line) + len(entry) > 800:
                    lines.append("Rejected offers (machine_id/host_id:reasons): " + line)
                    line = ""
                line += (" " if line else "") + entry
            if line:
                lines.append("Rejected offers (machine_id/host_id:reasons): " + line)
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
                "profile": profile,
            }
            if "preferences" in self.ctx:
                request["preferences"] = self.ctx["preferences"]
            code, data = self.retry_read("/rent", "POST", request)
            for attempt in range(600):
                try:
                    data = validate_status(data, self.ctx, self.manifest, code)
                except Diagnosed:
                    raise
                except Invalid as error:
                    # Emitted now, before the /stop the caller sends, so the job log keeps it
                    # even when the run ends right after.
                    self.emit(rejected_line(error) + "\n")
                    raise
                if code != 200:
                    if self.infrastructure_failure:
                        time.sleep(20)
                        route = "/status/" + self.ctx["run"] if admitted else "/rent"
                        code, data = self.retry_read(
                            route, "GET" if admitted else "POST", None if admitted else request
                        )
                        continue
                    self.show(data["log_tail_sanitised"])
                    result["state"] = data["state"]
                    break
                admitted = True
                self.metadata = data
                if not self.rental_shown and data["id"] is not None:
                    self.emit(rental_line(data) + "\n")
                    self.rental_shown = True
                final = data["state"] in {"GRACE", "NONE"} and data["result_at"] is not None
                self.pages(data["log_cursor"])
                if final:
                    if self.cursor != data["log_cursor"] or self.hash != data["log_sha256"]:
                        raise Invalid()
                    self.metadata = data
                    terminal = True
                    result = {"state": data["state"], "verdict": data["verdict"], "verified": 1}
                    self.show(data["gpu_model"])
                    break
                if data["state"] not in {"CREATING", "BOOTING", "TESTING"}:
                    break
                time.sleep(20)
                code, data = self.retry_read("/status/" + self.ctx["run"])
        except WaitBudget:
            self.show("BUSY until the remaining budget could no longer hold a run: not renting")
            result = {"state": "BUSY", "verdict": "UNAVAILABLE", "verified": 0}
        except (Invalid, ValueError, KeyError, TypeError, UnicodeError, OSError, TimeoutError):
            result = {"state": "UNAVAILABLE", "verdict": "UNAVAILABLE", "verified": 0}
        finally:
            # Owner ruling 2026-10-24 item 4: with the verified final result in hand, stop the box
            # now (best effort; the Worker's grace timer is the fallback). Edits2 D: an announced
            # result whose final fetch failed (hash, cursor or read) is stopped too. Only an
            # infrastructure fault before any result skips the stop, so paid work survives it.
            # Edits4 F1: give-up after an ambiguous /rent reply with nothing admitted as far as
            # the runner knows: a box may exist for this run, so ask the Worker to stop it
            # (harmless 404 when nothing exists).
            # A configured RENTING_DISABLED refusal needs no stop: the Worker never touched Vast
            # (unless an earlier /rent reply of this run was ambiguous).
            if (
                terminal
                or final
                or not self.infrastructure_failure
                or (self.rent_ambiguous and not admitted)
            ) and not (self.renting_disabled and not self.rent_ambiguous):
                try:
                    self.read("/stop/" + self.ctx["run"], "POST")
                except Exception:
                    pass
        safe, dropped = scrub(
            "".join(self.raw), max_bytes=65536, redactions=self.ctx.get("redactions", ())
        )
        if self.diagnostic and result["verified"] != 1:
            self.emit(self.diagnostic + "\n")
        self.emit("::group::Final bounded log: UNTRUSTED Worker/box output\n")
        self.show_tail()
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
                self.summary.write("| " + key + " | " + summary_cell(value) + " |\n")
            if self.metadata is not None:
                d = self.metadata
                gpu = public_gpu(d["gpu_model"])
                elapsed = (
                    (d["result_at"] or d["ended_at"] or d["started_at"]) - d["started_at"]
                ) / 1000
                for key, value in [
                    ("Profile", d["profile"]),
                    ("Die class", d["die"]),
                    ("Provider country (untrusted)", d.get("country", "UNKNOWN")),
                    ("Vast instance id", shown_id(d.get("instance_id", d["id"]))),
                    ("Vast machine id", shown_id(d["machine_id"])),
                    ("Vast host id", shown_id(d["host_id"])),
                    ("Offer geolocation (untrusted)", d.get("geolocation", "unknown")),
                    (
                        "Offer USD/hour",
                        (
                            format(d["dph_total"], ".6f")
                            if d.get("dph_total") is not None
                            else "unknown"
                        ),
                    ),
                    ("Host driver label (untrusted)", d.get("host_driver_version", "unknown")),
                    ("Host reference (untrusted)", d.get("host_reference", "MISSING")),
                    ("Driver (untrusted)", d["driver_version"]),
                    ("Selection reason", d["selection_reason"]),
                    ("Preferences ignored", str(d["preferences_ignored"])),
                    ("GPU (untrusted)", gpu),
                    ("Elapsed seconds", format(elapsed, ".3f")),
                    ("Estimated USD", format(d["cost_estimate_usd"], ".6f")),
                ]:
                    self.summary.write("| " + key + " | " + summary_cell(value) + " |\n")
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


def access_headers(environ):
    ident = environ.get("CF_ACCESS_CLIENT_ID", "")
    secret = environ.get("CF_ACCESS_CLIENT_SECRET", "")
    if not ident and not secret:
        return {}
    for name, value, pattern, least in [
        ("CF_ACCESS_CLIENT_ID", ident, r"[A-Za-z0-9_.-]{1,256}", 1),
        ("CF_ACCESS_CLIENT_SECRET", secret, r"[A-Za-z0-9_.-]{32,256}", 32),
    ]:
        if not value:
            raise Diagnosed(setting_line(name, "empty"))
        if not re.fullmatch(pattern, value):
            short = re.fullmatch(r"[A-Za-z0-9_.-]*", value) and len(value) < least
            raise Diagnosed(setting_line(name, "too short" if short else "bad characters"))
    return {"CF-Access-Client-Id": ident, "CF-Access-Client-Secret": secret}


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
                raise Diagnosed(reply_line(response.status, response.getheader("Content-Type", "")))
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
            or manifest["protocol_version"] != STATUS_PROTOCOL
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
        profile = "sweep" if sys.argv[2:] == ["--profile", "sweep"] else "check"
        if sys.argv[2:] and profile != "sweep":
            raise Invalid()
        # Check runs start only from pull_request_target or a push to master; the sweep is a
        # dispatch of its own pinned workflow. The tested commit is always the run head.
        if (
            not hex_value(sha, 40)
            or not hex_value(base, 40)
            or event_name
            not in (
                {"workflow_dispatch"} if profile == "sweep" else {"pull_request_target", "push"}
            )
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
        run = os.environ["GITHUB_RUN_ID"] + "-" + os.environ["GITHUB_RUN_ATTEMPT"]
        if not re.fullmatch("[1-9][0-9]{0,19}-[1-9][0-9]{0,19}", run):
            raise Invalid()
        origin = os.environ.get("WORKER_URL", "").rstrip("/")
        prefix = os.environ.get("WORKER_PATH", "")
        if not origin:
            raise Diagnosed(setting_line("WORKER_URL", "empty"))
        if not re.fullmatch(r"https://[A-Za-z0-9.-]+", origin):
            path = re.fullmatch(r"https://[A-Za-z0-9.-]+[/?#].*", origin)
            raise Diagnosed(
                setting_line("WORKER_URL", "trailing slash or path" if path else "bad characters")
            )
        if prefix and not re.fullmatch("[A-Za-z0-9_-]{32,128}", prefix):
            if "/" in prefix:
                reason = "trailing slash or path"
            else:
                short = re.fullmatch("[A-Za-z0-9_-]*", prefix) and len(prefix) < 32
                reason = "too short" if short else "bad characters"
            raise Diagnosed(setting_line("WORKER_PATH", reason))
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
            "nonce": secrets.token_hex(32),
            "redactions": [
                origin,
                urllib.parse.urlsplit(origin).hostname,
                prefix,
                urllib.parse.quote(origin, safe=""),
                *urllib.parse.urlsplit(origin).hostname.split("."),
            ],
        }

        access = access_headers(os.environ)
        for value in access.values():
            ctx["redactions"].append(value)
            print("::add-mask::" + value, flush=True)

        def client(route, method="GET", body=None):
            audience = urllib.parse.quote(origin, safe="")
            print("::add-mask::" + audience, flush=True)
            if not os.environ.get("ACTIONS_ID_TOKEN_REQUEST_URL") or not os.environ.get(
                "ACTIONS_ID_TOKEN_REQUEST_TOKEN"
            ):
                raise Diagnosed(setting_line("GITHUB_OIDC", "empty"))
            try:
                token_reply = https_request(
                    os.environ["ACTIONS_ID_TOKEN_REQUEST_URL"] + "&audience=" + audience,
                    headers={
                        "Authorization": "Bearer " + os.environ["ACTIONS_ID_TOKEN_REQUEST_TOKEN"]
                    },
                )
            except Diagnosed:
                # A redirect from the token endpoint is not a Worker reply.
                raise Invalid() from None
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
                {
                    **access,
                    "Authorization": "Bearer " + token,
                    "Content-Type": "application/json",
                    "X-KF-Protocol": str(STATUS_PROTOCOL),
                },
                None if body is None else json.dumps(body, separators=(",", ":")).encode(),
            )

        with open(summary_path, "a", encoding="utf-8") as summary:
            result = Runner(
                ctx, manifest, client, emit=lambda s: print(s, end="", flush=True), summary=summary
            ).execute()
    except Diagnosed as error:
        # Reached only by a setting rejected in this function (a setting NAME and a fixed
        # reason, see setting_line); Worker calls run inside Runner.execute.
        print(error.line, flush=True)
    except Exception:
        pass  # Raw exceptions may contain endpoint/Worker input.
    with open(output, "a", encoding="ascii") as outputs:
        for key in ["state", "verdict", "verified"]:
            outputs.write(key + "=" + str(result[key]) + "\n")
    return 0 if result["verified"] == 1 and result["verdict"] == "PASS" else 1


if __name__ == "__main__":
    raise SystemExit(main())
