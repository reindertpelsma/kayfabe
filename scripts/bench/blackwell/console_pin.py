"""Owner 13:05 readiness: await console publication, never accept an unattested key."""

import base64
import hashlib
import math
import re
import struct
import subprocess
import time

POLL_SECONDS = 20
CREATE_CEILING = 1200
PUBLIC_KEY = re.compile(r"ssh-ed25519\s+([A-Za-z0-9+/]+={0,2})")
FINGERPRINT = re.compile(r"SHA256:[A-Za-z0-9+/]{43}(?![A-Za-z0-9+/])")


class PinMismatch(ValueError):
    """The authenticated console published a different host key."""


class PinExpired(TimeoutError):
    """The console did not publish a matching key before its readiness deadline."""


def stage_seconds(factor):
    if type(factor) not in (int, float) or not math.isfinite(factor) or factor <= 0:
        raise ValueError("invalid console readiness factor")
    return 60 * min(10, max(4, factor))


def fingerprint(key):
    raw = base64.b64decode(key, validate=True)
    if len(raw) != 51 or raw[:19] != struct.pack(">I", 11) + b"ssh-ed25519" + struct.pack(">I", 32):
        raise ValueError("invalid ed25519 host key")
    return "SHA256:" + base64.b64encode(hashlib.sha256(raw).digest()).decode().rstrip("=")


def console_pins(text, expected_key):
    if not isinstance(text, str) or len(text.encode()) > 1024 * 1024:
        raise ValueError("console text exceeds readiness byte bound")
    pins = set()
    for line in text.splitlines():
        for key in PUBLIC_KEY.findall(line):
            try:
                pins.add(fingerprint(key))
            except ValueError as error:
                raise PinMismatch("malformed published ed25519 host pin") from error
        # A console may also print its RSA/ECDSA/DSA fingerprints (measured lease 57443671).
        # They do not attest the ed25519 key selected by the unchanged ssh-keyscan invocation.
        # Unlabelled fingerprints still count, so an unknown type stays fail-closed.
        upper = line.upper()
        if "ED25519" in upper or not any(kind in upper for kind in ("RSA", "ECDSA", "DSA")):
            pins.update(FINGERPRINT.findall(line))
    # Preserve the earlier guard's acceptance of the full base64 key in console text.
    if expected_key in text:
        pins.add(fingerprint(expected_key))
    return pins


def wait_for_pin(
    expected_key,
    fetch_console,
    *,
    elapsed_since_create,
    factor,
    stage_started_elapsed=None,
    clock=time.monotonic,
    sleep=time.sleep,
    record=lambda receipt: None,
):
    if (
        type(elapsed_since_create) not in (int, float)
        or not math.isfinite(elapsed_since_create)
        or elapsed_since_create < 0
    ):
        raise ValueError("invalid lease age")
    expected = fingerprint(expected_key)
    start = clock()
    stage_start = elapsed_since_create if stage_started_elapsed is None else stage_started_elapsed
    if not 0 <= stage_start <= elapsed_since_create:
        raise ValueError("console stage start is outside this lease life")
    deadline = (
        start + min(stage_start + stage_seconds(factor), CREATE_CEILING) - elapsed_since_create
    )
    polls = 0
    while True:
        at = clock()
        remaining = deadline - at
        if remaining <= 0:
            raise PinExpired("console host pin absent past readiness bound")
        transport_error = None
        try:
            text = fetch_console(min(POLL_SECONDS, remaining))
        except (OSError, subprocess.SubprocessError) as error:
            text = ""
            transport_error = type(error).__name__
        # A result arriving after either absolute deadline is never readiness.
        if clock() >= deadline:
            raise PinExpired("console host pin arrived after readiness bound")
        pins = console_pins(text, expected_key)
        polls += 1
        # Preserve the original guard: the scanned key must appear in the authenticated
        # console. Other published fingerprints do not negate that exact attestation.
        if expected in pins:
            state = "match"
        elif pins:
            state = "mismatch"
        else:
            state = "absent"
        receipt = {
            "state": state,
            "fingerprints": sorted(pins),
            "expected_fingerprint": expected,
            "console_sha256": hashlib.sha256(text.encode()).hexdigest(),
            "polls": polls,
            "wait_seconds": clock() - start,
            "elapsed_since_create": elapsed_since_create + clock() - start,
            "deadline_since_create": elapsed_since_create + deadline - start,
            "transport_error": transport_error,
        }
        record(receipt)
        if state == "mismatch":
            raise PinMismatch("authenticated console host pin differs from scanned ed25519 key")
        if state == "match":
            return {**receipt, "fingerprint": expected}
        sleep(min(max(0, POLL_SECONDS - (clock() - at)), max(0, deadline - clock())))


# Controller 12:10 (BW5 B1q): settle-and-retry, still fail-closed. Lease 57498790 refused on a
# single ssh-keyscan taken ~23 s before the console published its ED25519 pin (MEASURED in its
# guard receipt). The settled guard re-polls the console AND re-scans every check.
SETTLE_SECONDS = 15
SETTLE_WINDOW = 180
SETTLE_CONSECUTIVE = 2
HOST_KEY_LINES = 64
HOST_KEY_LINE_BYTES = 400


class PinFlap(PinMismatch):
    """The scanned host key changed between checks without matching an unchanged console pin."""


class PinUnknownType(PinMismatch):
    """The console printed a fingerprint whose key type is unlabelled or unknown."""


def published_ed25519(text):
    """Every ED25519 pin the console printed, in console order (the last one is the latest)."""
    if not isinstance(text, str) or len(text.encode()) > 1024 * 1024:
        raise ValueError("console text exceeds readiness byte bound")
    pins = []
    for line in text.splitlines():
        for key in PUBLIC_KEY.findall(line):
            try:
                pins.append(fingerprint(key))
            except ValueError as error:
                raise PinMismatch("malformed published ed25519 host pin") from error
        found = FINGERPRINT.findall(line)
        if not found:
            continue
        upper = line.upper()
        if "ED25519" in upper:
            pins.extend(found)
        elif not any(kind in upper for kind in ("RSA", "ECDSA", "DSA")):
            raise PinUnknownType("console fingerprint with an unlabelled or unknown key type")
    return pins


def host_key_lines(text):
    """The console's host-key lines for the evidence; cloud-init's ci-info key table is omitted."""
    kept, omitted = [], 0
    for line in text.splitlines():
        if not (
            "ssh-ed25519" in line
            or FINGERPRINT.search(line)
            or PUBLIC_KEY.search(line)
            or "host key" in line.lower()
        ):
            continue
        if "ci-info" in line:
            omitted += 1
        elif len(kept) < HOST_KEY_LINES:
            kept.append(line[:HOST_KEY_LINE_BYTES])
    return kept, omitted


def settle_pin(
    scan,
    fetch_console,
    *,
    elapsed_since_create,
    factor,
    stage_started_elapsed=None,
    clock=time.monotonic,
    sleep=time.sleep,
    record=lambda receipt: None,
):
    """Accept the scanned ed25519 key only when two consecutive checks equal the latest pin.

    Each check polls the console, then scans. Before the first publication checks run every
    POLL_SECONDS until the console stage deadline (absent => PinExpired). From the first check
    that sees a pin, checks run every SETTLE_SECONDS for at most SETTLE_WINDOW seconds (and never
    past the stage deadline); no settled match by then => PinMismatch.
    """
    if (
        type(elapsed_since_create) not in (int, float)
        or not math.isfinite(elapsed_since_create)
        or elapsed_since_create < 0
    ):
        raise ValueError("invalid lease age")
    start = clock()
    stage_start = elapsed_since_create if stage_started_elapsed is None else stage_started_elapsed
    if not 0 <= stage_start <= elapsed_since_create:
        raise ValueError("console stage start is outside this lease life")
    deadline = (
        start + min(stage_start + stage_seconds(factor), CREATE_CEILING) - elapsed_since_create
    )
    window_end = None
    checks = consecutive = 0
    prev_scan = prev_latest = None
    prev_match = False
    while True:
        at = clock()
        limit = deadline if window_end is None else min(deadline, window_end)
        if at >= limit:
            break
        transport_error = scan_error = None
        try:
            text = fetch_console(min(POLL_SECONDS, limit - at))
        except (OSError, subprocess.SubprocessError) as error:
            text = ""
            transport_error = type(error).__name__
        if clock() >= deadline:
            break  # a result arriving after the absolute deadline is never readiness
        checks += 1
        lines, omitted = host_key_lines(text)
        receipt = {
            "check": checks,
            "polls": checks,
            "console_sha256": hashlib.sha256(text.encode()).hexdigest(),
            "host_key_lines": lines,
            "ci_info_lines_omitted": omitted,
            "transport_error": transport_error,
        }
        try:
            published = published_ed25519(text)
        except PinUnknownType:
            record({**receipt, "state": "unknown_type", "consecutive": 0})
            raise
        latest = published[-1] if published else None
        try:
            key = scan()
        except (OSError, subprocess.SubprocessError) as error:
            key, scan_error = None, type(error).__name__
        try:
            scanned = fingerprint(key) if key else None
        except ValueError as error:
            record({**receipt, "state": "malformed_scan", "consecutive": 0})
            raise PinMismatch("malformed scanned ed25519 host key") from error
        if latest is not None and window_end is None:
            window_end = at + SETTLE_WINDOW
        if latest is None:
            state, consecutive = "absent", 0
        elif scanned == latest:
            state = "match"
            consecutive = consecutive + 1 if prev_match and latest == prev_latest else 1
        elif (
            scanned is not None
            and prev_scan is not None
            and scanned != prev_scan
            and latest == prev_latest
        ):
            state, consecutive = "flap", 0
        else:
            state, consecutive = "mismatch", 0
        receipt.update(
            state=state,
            consecutive=consecutive,
            published=published,
            latest=latest,
            scanned_fingerprint=scanned,
            scan_error=scan_error,
            wait_seconds=clock() - start,
            elapsed_since_create=elapsed_since_create + clock() - start,
            deadline_since_create=elapsed_since_create + deadline - start,
            window_end_since_create=(
                None if window_end is None else elapsed_since_create + window_end - start
            ),
        )
        record(receipt)
        if state == "flap":
            raise PinFlap("scanned host key changed between checks without matching the console")
        if consecutive >= SETTLE_CONSECUTIVE:
            return {**receipt, "fingerprint": scanned, "key": key}
        prev_match = state == "match"
        prev_latest = latest
        if scanned is not None:
            prev_scan = scanned
        interval = POLL_SECONDS if window_end is None else SETTLE_SECONDS
        limit = deadline if window_end is None else min(deadline, window_end)
        sleep(min(max(0, interval - (clock() - at)), max(0, limit - clock())))
    if window_end is None:
        raise PinExpired("console host pin absent past readiness bound")
    raise PinMismatch("scanned ed25519 never settled on the latest console pin in the window")
