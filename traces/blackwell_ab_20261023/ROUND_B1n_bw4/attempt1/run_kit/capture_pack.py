"""Box side (owned rental): pack one arm's private capture with a sha256 manifest.

Inputs: the guest capture copied out by the collection boot (dumps, WER, evtx; it carries
GUEST_MANIFEST.txt with the guest's own sha256 per file), the arm's VM logs (the full kf3/QEMU
log of every boot of this arm) and the arm's /root/prov files (incl. the user-ring trace dir).
Output: /root/capture/<run>/{guest,vm_logs,prov}/... plus MANIFEST.sha256 and summary.json.
Nothing here is published; only names, sizes and hashes go to the public evidence.
Usage: capture_pack.py RUN
"""

import hashlib
import json
from pathlib import Path
import re
import shutil
import sys

RUN = re.compile(r"[a-z0-9_]{1,40}")
VM_LOGS = Path("/workspace/bw-capc-validation/kfwin/logs")
PROV = Path("/root/prov")
OUT = Path("/root/capture")


def sha256(path):
    digest = hashlib.sha256()
    with open(path, "rb") as handle:
        for block in iter(lambda: handle.read(1 << 20), b""):
            digest.update(block)
    return digest.hexdigest()


def guest_manifest(guest_dir):
    path = Path(guest_dir) / "GUEST_MANIFEST.txt"
    if not path.exists():
        return {}, False
    rows, done = {}, False
    for line in path.read_text(errors="replace").splitlines():
        line = line.strip().lstrip("﻿")
        if line == "CAPTURE_GUEST_DONE":
            done = True
        parts = line.split(" ", 3)
        if len(parts) == 4 and parts[0] == "GUESTFILE":
            rows[parts[3].replace("\\", "/")] = (parts[1].lower(), int(parts[2]))
    return rows, done


def copy_tree(source, target):
    source = Path(source)
    if not source.is_dir():
        return
    for path in sorted(source.rglob("*")):
        if path.is_file() and not path.is_symlink():
            dest = target / path.relative_to(source)
            dest.parent.mkdir(parents=True, exist_ok=True)
            shutil.copy2(path, dest)


def pack(run, *, guest_dir=None, vm_logs=VM_LOGS, prov=PROV, out=OUT):
    if not RUN.fullmatch(run):
        raise ValueError("invalid arm name")
    prov, vm_logs = Path(prov), Path(vm_logs)
    root = Path(out) / run
    guest_dir = Path(guest_dir) if guest_dir else root / "guest"
    if guest_dir != root / "guest":
        copy_tree(guest_dir, root / "guest")
    (root / "vm_logs").mkdir(parents=True, exist_ok=True)
    for path in sorted(vm_logs.glob(run + "_*")):
        if path.is_file() and re.fullmatch(re.escape(run) + r"_[A-Za-z0-9_.-]+", path.name):
            shutil.copy2(path, root / "vm_logs" / path.name)
    copy_tree(prov / "arms" / run, root / "prov" / "arms" / run)
    for path in sorted(prov.glob("*" + run + "*")):
        if path.is_file():
            (root / "prov").mkdir(parents=True, exist_ok=True)
            shutil.copy2(path, root / "prov" / path.name)
    expected, complete = guest_manifest(root / "guest")
    files, mismatches = {}, []
    for path in sorted(root.rglob("*")):
        if not path.is_file() or path.name in ("MANIFEST.sha256", "summary.json"):
            continue
        name = str(path.relative_to(root))
        files[name] = {"bytes": path.stat().st_size, "sha256": sha256(path)}
        if name.startswith("guest/"):
            want = expected.get(name[len("guest/") :])
            if want and (want[0] != files[name]["sha256"] or want[1] != files[name]["bytes"]):
                mismatches.append(name[len("guest/") :])
    missing = sorted(set(expected) - {n[len("guest/") :] for n in files if n.startswith("guest/")})
    (root / "MANIFEST.sha256").write_text(
        "".join(f"{row['sha256']}  {name}\n" for name, row in files.items())
    )
    summary = {
        "run": run,
        "files": files,
        "guest_manifest_complete": complete,
        "guest_hash_mismatches": mismatches,
        "guest_files_missing": missing,
        "total_bytes": sum(row["bytes"] for row in files.values()),
    }
    (root / "summary.json").write_text(json.dumps(summary, indent=1) + "\n")
    return summary


def public_summary(summary):
    return {
        "run": summary["run"],
        "private": True,
        "note": "raw capture is private (dev box, never in git); names, sizes and sha256 only",
        "guest_manifest_complete": summary["guest_manifest_complete"],
        "guest_hash_mismatches": summary["guest_hash_mismatches"],
        "guest_files_missing": summary["guest_files_missing"],
        "total_bytes": summary["total_bytes"],
        "files": {
            name: {"bytes": row["bytes"], "sha256": row["sha256"]}
            for name, row in summary["files"].items()
        },
    }


def main(argv):
    if len(argv) != 1:
        raise SystemExit(__doc__)
    summary = pack(argv[0], guest_dir=OUT / argv[0] / "guest")
    print(
        "CAPTURE_PACKED run=%s files=%d bytes=%d guest_complete=%s mismatches=%d"
        % (
            argv[0],
            len(summary["files"]),
            summary["total_bytes"],
            summary["guest_manifest_complete"],
            len(summary["guest_hash_mismatches"]),
        )
    )


if __name__ == "__main__":
    main(sys.argv[1:])
