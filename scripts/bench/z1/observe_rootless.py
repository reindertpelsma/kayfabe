"""Root-only kernel observer; never grants the observed QEMU privileges.

The observed suite and every QEMU run as the throwaway uid. This observer only
reads evidence and stops its own Popen child on refusal. The source-matched
negative-control oracle is reused unchanged from native_xid_guard.py.
"""

import importlib.util
import json
import os
from pathlib import Path
import subprocess
import sys
import time

from host_guard import identity
from kernel_scope import snapshot

REPO = Path(__file__).resolve().parents[3]
spec = importlib.util.spec_from_file_location(
    "xid_guard", REPO / "scripts/bench/native_xid_guard.py"
)
guard = importlib.util.module_from_spec(spec)
spec.loader.exec_module(guard)


def descendant(pid, anchor, anchor_start):
    if identity(anchor) != anchor_start:
        raise guard.GuardRefused("owned suite anchor changed")
    seen = set()
    while pid != anchor:
        if pid <= 1 or pid in seen or len(seen) >= 64:
            raise guard.GuardRefused("QEMU is not descended from owned suite")
        seen.add(pid)
        record = guard.process_identity(pid)
        if record is None:
            raise guard.GuardRefused("owned launch chain disappeared")
        pid = record[0]


def main():
    manifest_path, receipt_path, process_dir, ack_dir, *command = sys.argv[1:]
    manifest = json.loads(Path(manifest_path).read_text())
    revision = manifest["revision"]
    binary = str(Path(manifest["qemu"]["path"]).resolve())
    pins = guard.pin_suite(REPO, revision)
    state = json.loads(Path(receipt_path).read_text())
    baseline = tuple(state.get("accepted_xid_lines", state["approved_native_xid_lines"]))
    guard.check_unchanged(snapshot(), baseline)
    ack = Path(ack_dir)
    ack.mkdir(parents=True, exist_ok=True)
    child = subprocess.Popen(command, start_new_session=True)
    anchor_start = identity(child.pid)
    context = record = None
    try:
        while True:
            current = Path(process_dir) / "current.json"
            if context is None and current.exists():
                candidate = json.loads(current.read_text())
                if (ack / f"{candidate['pid']}.pass").exists():
                    candidate = None
                if candidate:
                    descendant(candidate["pid"], child.pid, anchor_start)
                    if identity(candidate["pid"]) != candidate["starttime"]:
                        raise guard.GuardRefused("recorded QEMU start time differs")
                    context = {
                        "pid": candidate["pid"],
                        "starttime": candidate["starttime"],
                        "exe": binary,
                        "revision": revision,
                        "source_pin": pins,
                        "arm": guard.guest_launch_arm(candidate["argv"]),
                    }
                    guard.verify_guest_identity(context)
                    record = candidate
            journal = snapshot()
            if context:
                snapshot = Path(process_dir) / f"qemu-{context['pid']}.json"
                latest = json.loads(snapshot.read_text())
                finished = "rc" in latest
                if not finished:
                    guard.verify_guest_identity(context)
                argv = record["argv"]
                serials = [
                    arg.split("path=", 1)[1]
                    for arg in argv
                    if arg.startswith("file,id=kfcon0,path=")
                ]
                if len(serials) != 1:
                    raise guard.GuardRefused("exactly one console proof required")
                serial = Path(serials[0]).read_text(errors="strict")
                qlog = Path(record["qlog"]).read_text(errors="strict")
                if context["arm"] in ("--defer-liveness", "--missing-page-fault"):
                    accepted = guard.approve_guest(
                        journal, baseline, serial, qlog, context, final=finished
                    )
                else:
                    guard.check_unchanged(journal, baseline)
                    accepted = baseline
                if finished:
                    if latest["rc"]:
                        raise guard.GuardRefused("QEMU exit was nonzero")
                    state.setdefault("completed_rootless_arms", []).append(context)
                    if accepted != baseline:
                        state.setdefault("guest_proofs", []).append(
                            {**context, "accepted_faults": len(accepted) - len(baseline)}
                        )
                    baseline = tuple(accepted)
                    state["accepted_xid_lines"] = baseline
                    Path(receipt_path).write_text(json.dumps(state, indent=2) + "\n")
                    (ack / f"{context['pid']}.pass").write_text("PASS\n")
                    context = record = None
            else:
                guard.check_unchanged(journal, baseline)
            if child.poll() is not None:
                if child.returncode or context:
                    raise guard.GuardRefused("suite failed or an arm lacks final proof")
                guard.check_unchanged(snapshot(), baseline)
                print("ROOTLESS_OBSERVER_PASS", flush=True)
                return 0
            time.sleep(0.1)
    except BaseException as error:
        (ack / "STOP").write_text(str(error) + "\n")
        if child.poll() is None:
            # Root session created above, with exact live anchor identity checked.
            if identity(child.pid) == anchor_start:
                os.killpg(child.pid, 15)
                try:
                    child.wait(timeout=5)
                except subprocess.TimeoutExpired:
                    os.killpg(child.pid, 9)
                    child.wait()
        raise


if __name__ == "__main__":
    sys.exit(main())
