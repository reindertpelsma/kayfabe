"""Run already-built, hash-verified native gates as an ordinary user (no cargo stub)."""

import hashlib
import json
import os
from pathlib import Path
import re
import subprocess
import sys

from host_guard import Refused, check_privilege, preflight, status
import grp


def main():
    own = status(os.getpid())
    check_privilege(
        os.geteuid(),
        int(own["CapEff"], 16),
        int(own["CapPrm"], 16),
        set(os.getgroups()),
        grp.getgrnam("kvm").gr_gid,
        os.getegid(),
    )
    preflight()
    manifest = json.loads(Path(sys.argv[1]).read_text())
    print("ROOTLESS_GATES_START", manifest["revision"], flush=True)
    passed = 0
    births = user = 0
    for name in [f"kf-gate{i}" for i in range(1, 10)] + ["kf-micro-reserve-probe"]:
        entry = manifest["gates"][name]
        binary = Path(entry["path"])
        digest = hashlib.file_digest(binary.open("rb"), "sha256").hexdigest()
        if digest != entry["sha256"]:
            raise Refused("gate binary hash changed: " + name)
        args = [str(binary)] + (["reserve"] if name == "kf-micro-reserve-probe" else [])
        env = {**os.environ, "KF3_WIN_USER_CHANNELS_PASSTHROUGH": "1"}
        result = subprocess.run(args, env=env, capture_output=True, text=True, timeout=180)
        print(result.stdout, result.stderr, sep="", flush=True)
        text = result.stdout + result.stderr
        lines = [line for line in text.splitlines() if "kf-host: channel birth " in line]
        births += len(lines)
        user += sum(" PRIVILEGED_CHANNEL=0 privilege=USER " in line for line in lines)
        refused = re.search("CHANNEL (?:BIRTH |CLASS )?REFUSED|CUDA THREAD REFUSED", text)
        if name == "kf-micro-reserve-probe":
            match = re.search(r"^MICRO_RESERVE_VERDICT arm=reserve (\w+)", text, re.M)
            verdict = match[1] if match else "MISSING"
            ok = verdict == "PASS" or (
                verdict == "FALLBACK" and "CHECK fallback_flat_fb_alias_placeable PASS" in text
            )
            print("GATE10_VERDICT=" + verdict, flush=True)
        else:
            ok = re.search(r"^GATE[0-9]+_VERDICT=PASS$", text, re.M) is not None
            passed += bool(ok and not result.returncode and not refused)
        if not ok or result.returncode or refused:
            raise Refused("rootless native gate failed: " + name)
    if passed != 9 or births < 1 or births != user:
        raise Refused("gate count or USER birth proof failed")
    print(f"ROOTLESS_GATES_PASS gates={passed} births={births} user={user}", flush=True)
    return 0


if __name__ == "__main__":
    sys.exit(main())
