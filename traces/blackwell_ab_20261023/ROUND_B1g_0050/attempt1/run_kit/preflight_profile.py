"""Judge the unchanged host preflight under the lease profile (B1f "7-visible-core deviation").

Usage: preflight_profile.py <preflight.log> <lease.json> <verdict.json> [nproc]
Default (no profile in lease.json): exactly the original rule, VERDICT=SOUND required.
Profile: HOST_FAULT is accepted only if the sole failed check is "only 7 cores" and nproc is 7;
every other failed check, 6 or fewer cores, or an nproc mismatch still refuses (exit 2).
"""

import json
import os
from pathlib import Path
import re
import sys

PROFILE = "7-visible-core deviation"


def judge(text, row, nproc):
    lines = text.splitlines()
    verdicts = [line for line in lines if line.startswith("VERDICT=")]
    failed = [
        re.sub(r"^\s*✘\s*HOST FAULT:\s*", "", line).strip()
        for line in lines
        if line.lstrip().startswith("✘")
    ]
    out = {"verdict_line": verdicts, "failed_checks": failed, "cpu_visible": nproc}
    profile = row.get("profile")
    if profile is None:
        out.update(accepted=verdicts == ["VERDICT=SOUND"], accepted_deviation=[])
        return out
    if profile != PROFILE or row.get("cpu_profile_floor") != 7:
        raise ValueError("unknown lease profile")
    out["profile"] = PROFILE
    if verdicts == ["VERDICT=SOUND"] and not failed:
        out.update(accepted=nproc >= 7, accepted_deviation=[])
        return out
    ok = verdicts == ["VERDICT=HOST_FAULT"] and failed == ["only 7 cores"] and nproc == 7
    out.update(accepted=ok, accepted_deviation=["only 7 cores"] if ok else [])
    return out


def main():
    log, lease, dest = (Path(a) for a in sys.argv[1:4])
    nproc = int(sys.argv[4]) if len(sys.argv) > 4 else len(os.sched_getaffinity(0))
    row = json.loads(lease.read_text())
    out = judge(log.read_text(), row, nproc)
    out.update(experiment_mode="OBSERVATION", attestation=False)
    dest.write_text(json.dumps(out, indent=2) + "\n")
    print("PREFLIGHT_PROFILE", json.dumps(out), flush=True)
    return 0 if out["accepted"] else 2


if __name__ == "__main__":
    sys.exit(main())
