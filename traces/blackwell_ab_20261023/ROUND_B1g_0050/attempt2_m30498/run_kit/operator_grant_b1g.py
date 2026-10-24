"""Controller 02:30 B1g selection override for THIS grant only (recorded in the evidence).

Order: machine 30498 (GB205, default profile), then machine 99805 (GB206, 7-visible-core
profile), then the next qualifying offer in the harness's die order (GB206/GB205/GB203 before
GB202). The pin for an attempt workspace is read from grant_pin.json next to this file.
"""

import json
from pathlib import Path

ORDER = (30498, 99805)
PIN_FILE = Path(__file__).resolve().parent / "grant_pin.json"


def pin():
    value = json.loads(PIN_FILE.read_text())["machine"]
    if value is not None and value not in ORDER:
        raise ValueError("pin is not one of the granted machines")
    return value


def select(eligible, pinned=None):
    pinned = pin() if pinned is None and PIN_FILE.exists() else pinned
    if pinned is None:
        return list(eligible)
    return [x for x in eligible if int(x["machine_id"]) == pinned]
