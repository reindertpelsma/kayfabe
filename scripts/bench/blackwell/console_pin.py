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
        # A console may also print its RSA/ECDSA fingerprints. They do not attest
        # the ed25519 key selected by the unchanged ssh-keyscan invocation.
        upper = line.upper()
        if "ED25519" in upper or not any(kind in upper for kind in ("RSA", "ECDSA")):
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
