"""Native-first raw baseline; preserve the existing exact-Xid negative oracle."""

import importlib.util
import json
import os
from pathlib import Path
import re
import subprocess
import sys

from kernel_scope import snapshot

REPO = Path(__file__).resolve().parents[3]
sys.path.insert(0, str(REPO / "scripts/bench"))
spec = importlib.util.spec_from_file_location("vast_payload", REPO / "scripts/ci/vast_payload.py")
payload = importlib.util.module_from_spec(spec)
spec.loader.exec_module(payload)
guard = payload.xid_guard


def main():
    binary, folder, receipt_path = sys.argv[1:]
    out = Path(folder)
    out.mkdir(parents=True, exist_ok=True)
    source = (REPO / "scripts/fastguest/fast_suite.sh").read_text()
    block = source.split("ARMS=(--timer", 1)[1].split(")", 1)[0]
    arms = ["--timer"] + re.findall(r"--[a-z0-9-]+", block)
    if len(arms) != 30 or len(set(arms)) != 30:
        raise guard.GuardRefused("expected unchanged 30-arm source list")
    initial = guard.xid_lines(snapshot())
    accepted = initial
    for arm in arms:
        guard.check_unchanged(snapshot(), accepted)
        name = arm.removeprefix("--")
        watched = out / f"{name}.watch.log"
        env = {**os.environ, "XID_OUTDIR": str(out)}
        with watched.open("w") as log:
            result = subprocess.run(
                [
                    "bash",
                    str(REPO / "scripts/bench/host_xid_watch.sh"),
                    name,
                    "--",
                    "timeout",
                    "--kill-after=5",
                    "180",
                    binary,
                    arm,
                ],
                env=env,
                stdout=log,
                stderr=subprocess.STDOUT,
            )
        text = watched.read_text()
        if not payload.xid_ok(text, result.returncode, arm, text):
            raise guard.GuardRefused("native arm or exact Xid proof refused: " + arm)
        accepted = guard.xid_lines(snapshot())
        print("Z1_BARE_ARM_PASS", arm, flush=True)
    delta = accepted[len(initial) :]
    guard.approve_native(
        "\n".join(delta),
        (out / "defer-liveness.watch.log").read_text(),
        (out / "missing-page-fault.watch.log").read_text(),
    )
    Path(receipt_path).write_text(
        json.dumps(
            {
                "initial_xid_lines": initial,
                "approved_native_xid_lines": accepted,
                "native_delta": len(delta),
                "bare_pass": 30,
            },
            indent=2,
        )
        + "\n"
    )
    print("BARE_SUITE_PASS=30 BARE_SUITE_FAIL=0 BARE_SUITE_CRASH=0 ARMS=30", flush=True)


if __name__ == "__main__":
    main()
