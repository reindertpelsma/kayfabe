"""B1g evidence: the console's host-key lines with key types; never changes the guard decision.

Raw console text stays private in the operator workspace (console_raw_<id>_<n>.txt); only
host-key lines (public keys / fingerprints and their type labels) are exported.
"""

import datetime as dt
import hashlib
import json
from pathlib import Path
import base64
import re
import struct

import console_pin as C

# A valid but never-published ed25519 blob, so console_pins skips its full-key branch.
_SENTINEL = base64.b64encode(
    struct.pack(">I", 11) + b"ssh-ed25519" + struct.pack(">I", 32) + bytes(32)
).decode()

KEY = re.compile(r"ssh-ed25519\s+([A-Za-z0-9+/]+={0,2})")
OTHER_KEY = re.compile(r"(ssh-rsa|ecdsa-sha2-nistp\d+|ssh-dss)\s+[A-Za-z0-9+/]+={0,2}")
FPR = re.compile(r"SHA256:[A-Za-z0-9+/]{43}(?![A-Za-z0-9+/])")
TYPE = re.compile(r"\((ED25519|ECDSA|RSA|DSA)\)", re.I)
LINE_BOUND = 300


def _bounded(line, anchor):
    line = line.strip()
    if len(line) <= LINE_BOUND:
        return line
    at = max(0, line.find(anchor) - 60)
    return line[at:at + LINE_BOUND]


def key_lines(text):
    try:
        guard = C.console_pins(text, _SENTINEL) if _SENTINEL not in text else set()
    except (ValueError, C.PinMismatch):
        guard = set()
    rows = []
    for line in text.splitlines():
        for key in KEY.findall(line):
            try:
                fp = C.fingerprint(key)
                kind = "ed25519_key"
            except ValueError:
                fp, kind = None, "ed25519_key_malformed"
            rows.append({"kind": kind, "fingerprint": fp, "line": _bounded(line, key),
                         "counted_by_guard": fp in guard})
        for match in OTHER_KEY.finditer(line):
            rows.append({"kind": "other_key:" + match.group(1), "fingerprint": None,
                         "line": _bounded(line, match.group(0)), "counted_by_guard": False})
        for fp in FPR.findall(line):
            typed = TYPE.search(line)
            kind = "fingerprint_typed:" + typed.group(1).upper() if typed else "fingerprint_untyped"
            rows.append({"kind": kind, "fingerprint": fp, "line": _bounded(line, fp),
                         "counted_by_guard": fp in guard})
    return rows


def capture(root, iid, text):
    root = Path(root)
    n = len(list(root.glob(f"console_raw_{iid}_*.txt")))
    (root / f"console_raw_{iid}_{n}.txt").write_text(text)
    row = {
        "utc": dt.datetime.now(dt.timezone.utc).isoformat(),
        "lease_id": iid,
        "poll": n,
        "console_sha256": hashlib.sha256(text.encode()).hexdigest(),
        "key_lines": key_lines(text),
    }
    with (root / f"console_keylines_{iid}.jsonl").open("a") as out:
        out.write(json.dumps(row) + "\n")
    return row
