"""B1b native fuzz repeats: benchmark findings never gate the Windows observation arm."""

import argparse
import hashlib
import json
from pathlib import Path
import re
import subprocess
import sys


def parse(text, arm, returncode):
    lines = [line for line in text.splitlines() if line.startswith("FUZZ_ARM=")]
    fields = {}
    if len(lines) == 1:
        fields = dict(item.split("=", 1) for item in lines[0].split() if "=" in item)
    valid = (
        fields.get("FUZZ_ARM") == arm
        and fields.get("threads") == "9"
        and fields.get("cores") == "3"
        and fields.get("pin") == ("crowd" if arm == "crowd" else "unpinned")
        and fields.get("verdict") in ("PASS", "FAIL")
    )
    revoked = any(
        re.fullmatch(r"FUZZ_VIOLATION=ALIAS_REVOKED arm=crowd n=[1-9][0-9]*", line)
        for line in text.splitlines()
    )
    return {
        "arm": arm,
        "returncode": returncode,
        "valid_sample": valid,
        "alias_revoked": valid and arm == "crowd" and revoked,
        "fields": fields,
        "raw_sha256": hashlib.sha256(text.encode()).hexdigest(),
        "experiment_mode": "OBSERVATION",
        "attestation": False,
    }


def batch(execute, safety):
    records = []
    for index, arm in enumerate(("crowd", "crowd", "crowd", "crowd-free")):
        safety(index)
        returncode, text = execute(arm, index)
        records.append(parse(text, arm, returncode))
    safety(4)
    pinned = records[:3]
    valid = sum(row["valid_sample"] for row in pinned)
    recurrences = sum(row["alias_revoked"] for row in pinned)
    return {
        "records": records,
        "pinned_samples": 3,
        "valid_pinned_samples": valid,
        "alias_revoked_recurrences": recurrences,
        "native_finding": recurrences >= 2 if valid == 3 else None,
        "root_cause_claimed": False,
        "continue_windows": True,
        "experiment_mode": "OBSERVATION",
        "attestation": False,
    }


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("binary", type=Path)
    parser.add_argument("output", type=Path)
    args = parser.parse_args()
    import os

    sys.path.insert(0, str(Path(__file__).parent / "guard/blackwell"))
    import phase_xid_guard as G
    import windows_observation as O

    root = Path("/root/prov")
    receipt = json.loads((root / "native_receipt.json").read_text())
    accepted = tuple(receipt["approved_native_xid_lines"])

    def safety(index):
        if (root / "XID_STOP").exists():
            raise G.N.GuardRefused("prior fatal marker")
        if G.N.xid_lines(G.N.kernel_snapshot()) != accepted:
            raise G.N.GuardRefused("new Xid in native findings repeat")
        O.check_host_health(receipt["pci"])

    def execute(arm, index):
        log = root / f"native_fuzz_repeat_{index}.log"
        env = {**os.environ, "KF_BW3_FUZZ_ARM": arm}
        command = [str(args.binary), "--concurrent-fuzz", "--fuzz-cores", "3"]
        with log.open("w") as out:
            try:
                rc = subprocess.run(
                    command, env=env, stdout=out, stderr=subprocess.STDOUT, timeout=180
                ).returncode
            except subprocess.TimeoutExpired:
                rc = 124
        text = log.read_text()
        print(json.dumps({"index": index, "arm": arm, "returncode": rc}), flush=True)
        return rc, text

    try:
        result = batch(execute, safety)
    except Exception as error:
        G.record_fatal(root / "XID_STOP", {"STOP": str(error), "stage": "native repeat safety"})
        raise
    result["lease_id"] = json.loads(Path("/root/sweep2/lease.json").read_text())["id"]
    args.output.write_text(json.dumps(result, indent=2) + "\n")
    print("NATIVE_FINDINGS_RECORDED_CONTINUE_WINDOWS", flush=True)


if __name__ == "__main__":
    main()
