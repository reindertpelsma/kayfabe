"""Execute only the unchanged inherited settled-pin branch with fresh owned paths."""

import datetime
import hashlib
import ipaddress
import json
import os
from pathlib import Path
import subprocess
import sys
import time
from types import SimpleNamespace

from execution import ready_body, ready_budget

PIN = "169f99eef6216e2ff0b69185877af20f3cc74794a875b73158c7495812b8208f"
OPS = "aa48a2fab7d4ae2c401f02f163087e456c668f92c168128238b396d2dff16733"


def main():
    config = json.loads(Path(sys.argv[1]).read_text())
    D, L = Path(config["workspace"]), Path(config["ledger"])
    guard = Path(config["bw"]) / "console_pin.py"
    legacy = Path(config["legacy_ops"])
    if hashlib.sha256(guard.read_bytes()).hexdigest() != PIN:
        raise ValueError("identity admission source changed")
    if hashlib.sha256(legacy.read_bytes()).hexdigest() != OPS:
        raise ValueError("legacy settled readiness source changed")
    sys.path[:0] = [config["bw"], config["provider_bw"], str(D)]
    import console_pin as C
    import provider_retry as P
    import pin_evidence as PE

    def api(args):
        process = subprocess.run(
            ["vastai", *args, "--raw"], capture_output=True, text=True, timeout=60, check=True
        )
        return json.loads(process.stdout)

    def now():
        return datetime.datetime.now(datetime.timezone.utc)

    def save(rows):
        temporary = L.with_suffix(".tmp")
        temporary.write_text(json.dumps(rows, indent=2) + "\n")
        temporary.replace(L)

    BO = ready_budget(config["plan"])
    environment = dict(globals(), D=D, L=L, C=C, P=P, PE=PE, api=api, now=now, save=save, BO=BO)
    exec(compile(ready_body(legacy.read_text()), "inherited_ready", "exec"), environment)


if __name__ == "__main__":
    main()
