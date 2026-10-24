"""Controller BW7 attempt 2: native-template-only staged identity admission.

The legacy console_pin.settle_pin stays byte-identical. A changing or missing scan never
attests anything; this policy merely waits within one fixed deadline for a console-bound
Windows identity with a continuous matching stability window.
"""

import hashlib
import math
import subprocess
import time

import console_pin as C
from ref_windows import CONSOLE, MARKER

STABILITY_SECONDS = 120
ABSOLUTE_SECONDS = 1200
CONSECUTIVE_POLLS = 2
POLL_SECONDS = 15


def windows_pin(text):
    lines = [CONSOLE.sub("", line.strip()) for line in text.splitlines()]
    found = None
    for index, line in enumerate(lines):
        if line.endswith(MARKER):
            following = lines[index + 1] if index + 1 < len(lines) else ""
            keys = C.PUBLIC_KEY.findall(following)
            if len(keys) != 1:
                raise C.PinFlap("native Windows marker lacks one valid ed25519 key")
            found = C.fingerprint(keys[0])
    return found


def settle_pin(scan, fetch_console, *, first_reachable_at, clock=time.monotonic,
               sleep=time.sleep, record=lambda receipt: None):
    if (type(first_reachable_at) not in (int, float)
            or not math.isfinite(first_reachable_at)
            or first_reachable_at > clock()):
        raise ValueError("invalid first SSH-reachable clock")
    deadline = first_reachable_at + ABSOLUTE_SECONDS
    stable_since = previous_pair = None
    consecutive = polls = 0
    while clock() < deadline:
        at = clock()
        console_error = scan_error = None
        try:
            text = fetch_console(min(20, deadline - clock()))
        except (OSError, subprocess.SubprocessError) as error:
            text, console_error = "", type(error).__name__
        if clock() >= deadline:
            break
        published = C.published_ed25519(text)
        latest = published[-1] if published else None
        windows = windows_pin(text)
        try:
            key = scan(min(10, deadline - clock()))
        except (OSError, subprocess.SubprocessError) as error:
            key, scan_error = None, type(error).__name__
        if clock() >= deadline:
            break
        scanned = C.fingerprint(key) if key else None
        pair = (latest, scanned)
        matched = latest is not None and scanned == latest and windows == latest
        if matched:
            if pair != previous_pair or stable_since is None:
                stable_since, consecutive = clock(), 1
            else:
                consecutive += 1
            state = "match"
        else:
            stable_since, consecutive = None, 0
            state = "absent" if latest is None else "mismatch"
        stable_seconds = 0 if stable_since is None else clock() - stable_since
        polls += 1
        lines, omitted = C.host_key_lines(text)
        receipt = {
            "policy": "BW7_NATIVE_STAGED_V1", "state": state, "polls": polls,
            "check": polls, "latest": latest, "published": published,
            "scanned_fingerprint": scanned, "windows_marker_pin": windows,
            "consecutive": consecutive, "stable_seconds": stable_seconds,
            "stability_seconds_required": STABILITY_SECONDS,
            "elapsed_since_first_reachable": clock() - first_reachable_at,
            "absolute_seconds": ABSOLUTE_SECONDS,
            "console_sha256": hashlib.sha256(text.encode()).hexdigest(),
            "host_key_lines": lines, "ci_info_lines_omitted": omitted,
            "console_error": console_error, "scan_error": scan_error,
        }
        record(receipt)
        if matched and consecutive >= CONSECUTIVE_POLLS and stable_seconds >= STABILITY_SECONDS:
            return {**receipt, "key": key, "fingerprint": scanned}
        previous_pair = pair if matched else None
        sleep(min(max(0, POLL_SECONDS - (clock() - at)), max(0, deadline - clock())))
    raise C.PinFlap("native SSH identity did not stabilize before the absolute deadline")
