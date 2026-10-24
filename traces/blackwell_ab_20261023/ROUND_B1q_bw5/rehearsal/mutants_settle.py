"""Mutant check for console_pin.settle_pin (B1q): each mutant must be killed by >= 2 tests."""

from pathlib import Path
import re
import subprocess
import sys

BW = Path(__file__).resolve().parents[4] / "scripts/bench/blackwell"
LIM = "limit = deadline if window_end is None else min(deadline, window_end)"
M = [
    ("one check enough", "SETTLE_CONSECUTIVE = 2", "SETTLE_CONSECUTIVE = 1", 1),
    ("window 360", "SETTLE_WINDOW = 180", "SETTLE_WINDOW = 360", 1),
    ("interval 5", "SETTLE_SECONDS = 15", "SETTLE_SECONDS = 5", 1),
    ("no restart", "if prev_match and latest == prev_latest", "if prev_match", 1),
    ("no flap", 'state, consecutive = "flap", 0', 'state, consecutive = "mismatch", 0', 1),
    (
        "unknown accepted",
        'raise PinUnknownType("console fingerprint with an unlabelled or unknown key type")',
        "pins.extend(found)",
        1,
    ),
    (
        "first not latest",
        "latest = published[-1] if published else None",
        "latest = published[0] if published else None",
        1,
    ),
    ("ci-info kept", 'if "ci-info" in line:', "if False:", 1),
    (
        "absent is mismatch",
        'raise PinExpired("console host pin absent past readiness bound")\n    raise PinMismatch',
        'raise PinMismatch("x")\n    raise PinMismatch',
        1,
    ),
    (
        "malformed scan ok",
        'raise PinMismatch("malformed scanned ed25519 host key") from error',
        "scanned = None",
        1,
    ),
    ("window past deadline", LIM, "limit = deadline if window_end is None else window_end", 2),
    (
        "never re-scan",
        "            key = scan()\n",
        "            key = scan() if checks == 1 else key\n",
        1,
    ),
]


def killers():
    r = subprocess.run(
        [sys.executable, "-m", "unittest", "test_console_pin_settle"],
        cwd=BW,
        capture_output=True,
        text=True,
    )
    m = re.search(r"FAILED \((?:failures=(\d+))?(?:, )?(?:errors=(\d+))?", r.stderr)
    return int(m[1] or 0) + int(m[2] or 0) if m else 0


f = BW / "console_pin.py"
orig = f.read_text()
try:
    for label, a, b, n in M:
        assert orig.count(a) == n, label
        f.write_text(orig.replace(a, b))
        print(f"console_pin.py {label:22s} killers={killers()}", flush=True)
finally:
    f.write_text(orig)
