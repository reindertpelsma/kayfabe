#!/usr/bin/env python3
"""Replay retained public text through the unchanged guard, without hardware."""
import json
from pathlib import Path
import subprocess
import sys
import tempfile

ROOT = Path(__file__).resolve().parents[2]
GUARD = ROOT / "scripts/bench/native_xid_guard.py"
FIX = ROOT / "traces/sweep_windows_20261021/lease_57081143"
NATIVE = FIX / "native"


def run(*args):
    p = subprocess.run([sys.executable, str(GUARD), *map(str, args)],
                       capture_output=True, text=True, timeout=20, check=False)
    return {"exit_code": p.returncode, "stdout": p.stdout, "stderr": p.stderr}


def main():
    baseline = json.loads((NATIVE / "native_xid_baseline.json").read_text())
    native_lines = baseline["approved_native_xid_lines"]
    records = (FIX / "linux_stop/kernel_xids.log").read_text().splitlines()
    assert records[:5] == native_lines
    guest_defer = records[5:9]
    assert len(guest_defer) == 4
    assert all("pid=91947, name=qemu-system-x86" in line for line in guest_defer)
    assert all(": 31," in line for line in guest_defer)
    with tempfile.TemporaryDirectory(prefix="s2-guard-preflight-") as tmp:
        tmp = Path(tmp)
        native = tmp / "native.log"
        receipt = tmp / "baseline.json"
        guest = tmp / "native_plus_guest_defer.log"
        native.write_text("\n".join(native_lines) + "\n")
        guest.write_text("\n".join(native_lines + guest_defer) + "\n")
        approval = run("native", native,
                       NATIVE / "sweep74aa78ab_ad106_native595_defer-liveness.log",
                       NATIVE / "sweep74aa78ab_ad106_native595_missing-page-fault.log")
        assert approval["exit_code"] == 0, approval
        assert json.loads(approval["stdout"])["approved_native_xid_lines"] == native_lines
        receipt.write_text(approval["stdout"])
        unchanged = run("check", native, receipt)
        assert unchanged["exit_code"] == 0, unchanged
        after_guest = run("check", guest, receipt)
        help_result = run("--help")
        result = {
            "kind": "MEASURED GPU-free historical-text replay; no hardware verdict",
            "native_fault_count": len(native_lines),
            "guest_defer_fault_count": len(guest_defer),
            "guest_missing_page_included": False,
            "native_approval": approval,
            "unchanged_snapshot": unchanged,
            "after_guest_defer": after_guest,
            "supported_cli": help_result,
            "STOP": "Required guest exception is absent; no rental or workaround",
        }
        print(json.dumps(result, indent=2))
        return after_guest["exit_code"]


if __name__ == "__main__":
    sys.exit(main())
