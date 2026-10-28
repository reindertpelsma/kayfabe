"""Same-GPU two-VM experiment, n>=3 solo and concurrent with retained logs."""

import hashlib
import json
import os
from pathlib import Path
import re
import subprocess
import sys
import time

from host_guard import Refused, preflight, qemus
from kernel_scope import snapshot

REPO = Path(__file__).resolve().parents[3]
sys.path.insert(0, str(REPO / "scripts/bench"))
import native_xid_guard as xid_guard


def run_case(manifest, image, output, label, pair, tool, owned):
    baseline = xid_guard.xid_lines(snapshot())
    jobs = []
    start = int(time.time()) + 20
    try:
        for slot in range(2 if pair else 1):
            ram_mib = 4096 if tool else 2048
            resources = preflight(ram_mib, owned)
            tag = f"{label}_{slot}"
            bench = output / tag
            bench.mkdir()
            env = dict(os.environ)
            env.update(
                BENCH_DIR=str(bench),
                KF_FASTGUEST_DIR=str(image),
                QEMU_BIN=manifest["qemu"]["path"],
                KF_LOCK=str(bench / "vm.lock"),
                KF_VM_NAME=f"kf-z1-{tag}",
                KF3_FB_MB="4096",
                NVKVM_RAM_MB=str(ram_mib),
                KF3_DEV_EXTRA=f"gpu-minor=0,guest-driver={manifest['driver']}",
                KF_GPU_IDLE_MIB="11264",
                KF_DEVICE="kf3",
                KF_ARMS="--timer" if tool else "--timer --engines --ce-client --guest-ram-pin",
                KF_APPEND=f"KF_TOOL=1 KF_Z1_START={start} " if tool else "",
            )
            log = (bench / "launcher.log").open("w")
            child = subprocess.Popen(
                ["bash", str(REPO / "scripts/fastguest/run_fast_guest.sh"), tag, "180"],
                env=env,
                stdout=log,
                stderr=subprocess.STDOUT,
            )
            jobs.append((child, log, bench, resources))
            # Record only the new QEMU descended from this owned shell. Do not
            # admit arbitrary provider/owner processes into the allowed set.
            until = time.monotonic() + 10
            while child.poll() is None and time.monotonic() < until:
                actual = qemus()
                new = set(actual) - set(owned)
                for pid in new:
                    cursor = pid
                    for _ in range(32):
                        if cursor == child.pid:
                            owned[pid] = actual[pid]
                            break
                        record = Path(f"/proc/{cursor}/stat").read_text()
                        cursor = int(record.rpartition(")")[2].split()[1])
                        if cursor <= 1:
                            raise Refused("foreign QEMU appeared while launching owned child")
                if new:
                    break
                time.sleep(0.05)
            if child.poll() is not None:
                raise Refused("guest exited before both slots could be launched")
        deadline = time.monotonic() + 190
        while any(child.poll() is None for child, _, _, _ in jobs):
            xid_guard.check_unchanged(snapshot(), baseline)
            if time.monotonic() >= deadline:
                raise Refused("owned guest exceeded the unchanged 180-second oracle")
            time.sleep(0.2)
        xid_guard.check_unchanged(snapshot(), baseline)
        results = []
        for child, log, bench, resources in jobs:
            rc = child.wait(timeout=190)
            log.close()
            text = (bench / "launcher.log").read_text()
            if rc or "FAST_VERDICT=PASS" not in text:
                raise Refused("guest failed: " + bench.name)
            serial = next(bench.glob("*_serial.log")).read_text()
            qlog = next(bench.glob("*_qemu.log")).read_text()
            if "host GPU minor=0 " not in qlog:
                raise Refused("same host GPU binding line missing")
            row = {
                "slot": bench.name,
                "rc": rc,
                "resources": resources,
                "host_bindings": [
                    line for line in qlog.splitlines() if "kf3: host GPU minor=" in line
                ],
            }
            if tool:
                if (
                    "KERNEL rv=43 want=43 -> PASS" not in serial
                    or "BENCH_TOTAL_BAD=0" not in serial
                    or "BENCH_VERDICT: PASS" not in serial
                ):
                    raise Refused("CUDA output differs from bare-metal oracle")
                match = re.search(r"^BSUM N=1024 .*?gflops=([0-9.]+)", serial, re.M)
                if not match:
                    raise Refused("matmul throughput row missing")
                row["gflops_med"] = float(match[1])
                origin = re.search(
                    r"BENCH_CLOCK_ORIGIN mono_ms=([0-9.]+) real_ms=([0-9.]+)", serial
                )
                window = re.search(r"B1024_WINDOW_MONO_MS=([0-9.]+)\.\.([0-9.]+)", serial)
                if not origin or not window:
                    raise Refused("matrix execution window missing")
                offset = float(origin[2]) - float(origin[1])
                row["matrix_begin_ms"] = float(window[1]) + offset
                row["matrix_end_ms"] = float(window[2]) + offset
                for key in ("BEGIN", "END"):
                    values = re.findall(r"^Z1_WORK_" + key + r" epoch=([0-9]+)", serial, re.M)
                    if len(values) != 1:
                        raise Refused("workload execution window missing")
                    row[key.lower()] = int(values[0])
            results.append(row)
        if pair and tool:
            overlap = min(row["end"] for row in results) - max(row["begin"] for row in results)
            matrix_overlap = min(row["matrix_end_ms"] for row in results) - max(
                row["matrix_begin_ms"] for row in results
            )
            if overlap <= 0 or matrix_overlap <= 0:
                raise Refused("the two matrix execution windows did not overlap")
        else:
            overlap = matrix_overlap = None
        return {
            "label": label,
            "pair": pair,
            "tool": tool,
            "results": results,
            "work_overlap_s": overlap,
            "matrix_overlap_ms": matrix_overlap,
        }
    finally:
        for child, log, _, _ in jobs:
            if child.poll() is None:
                # timeout is a child of this exact shell; terminate only our tree.

                # The shell has not been made a process-group leader, so use the
                # descendant identities tracked below rather than a broad signal.
                for pid, starttime in list(owned.items()):
                    from host_guard import identity

                    try:
                        if identity(pid) == starttime:
                            os.kill(pid, 15)
                    except FileNotFoundError:
                        pass
                child.terminate()
                child.wait(timeout=5)
            log.close()
        owned.clear()
        preflight(0)


def main():
    manifest_path, base, cuda, folder = sys.argv[1:]
    manifest = json.loads(Path(manifest_path).read_text())
    binary = Path(manifest["qemu"]["path"])
    if hashlib.file_digest(binary.open("rb"), "sha256").hexdigest() != manifest["qemu"]["sha256"]:
        raise Refused("binary hash changed")
    output = Path(folder)
    output.mkdir(parents=True)
    results = []
    for rep in range(1, 4):
        for label, pair, tool, image in (
            (f"solo{rep}", False, True, cuda),
            (f"pair{rep}", True, True, cuda),
            (f"subset{rep}", True, False, base),
        ):
            results.append(run_case(manifest, Path(image), output, label, pair, tool, {}))
            (output / "results.json").write_text(json.dumps(results, indent=2) + "\n")
            print("TWO_VM_CASE_PASS", label, flush=True)
    print("TWO_VM_SHARE_PASS", flush=True)


if __name__ == "__main__":
    sys.exit(main())
