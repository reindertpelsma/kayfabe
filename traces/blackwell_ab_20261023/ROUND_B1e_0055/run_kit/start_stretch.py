"""Exactly one create attempt in this round; no provider retry or second lease."""

from pathlib import Path
import json
import subprocess
import sys

D = Path(__file__).resolve().parent


def ops(mode, name):
    with (D / name).open("w") as out:
        return subprocess.run(
            [sys.executable, str(D / "ops.py"), mode], stdout=out, stderr=subprocess.STDOUT
        ).returncode


assert ops("rent", "rent_1.txt") == 0, "single create failed; no automatic continuation"
iid = int((D / "own-id").read_text())
print("OWN_REPLACEMENT_CREATED", iid, flush=True)
rc = ops("ready", "ready_" + str(iid) + ".txt")
if rc:
    assert ops("destroy", "destroy_" + str(iid) + ".txt") == 0
    raise SystemExit("readiness stopped; exact-id teardown complete; one lease exhausted")
print("OWN_STRETCH_READY " + str(iid), flush=True)
