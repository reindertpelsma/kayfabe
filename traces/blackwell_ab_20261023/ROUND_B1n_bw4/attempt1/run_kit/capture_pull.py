"""Operator side: copy one arm's box capture into the private dev-box directory, sha256-verified.

The private root is outside every git worktree (default /data/bw4_private_capture/<lease>/).
Public evidence gets capture_<run>.json: names, sizes, sha256 and pull status only.
Usage: capture_pull.py WORKSPACE EVIDENCE_DIR RUN [RUN...]   (RUN 'ALL' pulls every packed arm)
"""

import hashlib
import json
from pathlib import Path, PurePosixPath
import re
import subprocess
import sys
import time

RUN = re.compile(r"[a-z0-9_]{1,40}")
PRIVATE = Path("/data/bw4_private_capture")
REPOS = (Path("/workspace"),)


def private_root(base, *, lease):
    base = Path(base).resolve()
    if any(part.endswith("-wt") or part == "traces" for part in base.parts):
        raise ValueError("private capture must never live in a git worktree")
    if not str(base).startswith("/data/"):
        raise ValueError("private capture root must be under /data")
    return base / str(int(lease))


def parse_manifest(text):
    rows = []
    for line in text.splitlines():
        if not line.strip():
            continue
        digest, name = line.split("  ", 1)
        path = PurePosixPath(name)
        if (
            not re.fullmatch(r"[0-9a-f]{64}", digest)
            or path.is_absolute()
            or ".." in path.parts
            or not name
        ):
            raise ValueError("unsafe or malformed manifest row")
        rows.append((digest, name))
    return rows


def sha256(path):
    digest = hashlib.sha256()
    with open(path, "rb") as handle:
        for block in iter(lambda: handle.read(1 << 20), b""):
            digest.update(block)
    return digest.hexdigest()


def pull_arm(run, manifest_text, dest_root, fetch, deadline_s, clock=time.monotonic):
    if not RUN.fullmatch(run):
        raise ValueError("invalid arm name")
    rows = parse_manifest(manifest_text)
    start = clock()
    files, failed, not_pulled, verified = {}, [], [], 0
    for digest, name in rows:
        target = Path(dest_root) / run / name
        remaining = deadline_s - (clock() - start)
        if remaining <= 0:
            not_pulled.append(name)
            files[name] = {"sha256": digest, "status": "NOT_PULLED_DEADLINE"}
            continue
        try:
            fetch(name, target, timeout=remaining)
            ok = target.is_file() and sha256(target) == digest
        except (OSError, subprocess.SubprocessError) as error:
            ok = False
            files[name] = {"sha256": digest, "status": "FETCH_ERROR " + type(error).__name__}
        if ok:
            verified += 1
            files[name] = {"sha256": digest, "bytes": target.stat().st_size, "status": "VERIFIED"}
        else:
            failed.append(name)
            files.setdefault(name, {"sha256": digest, "status": "HASH_MISMATCH"})
    return {
        "run": run,
        "private_dir": str(Path(dest_root) / run),
        "verified": verified,
        "failed": failed,
        "not_pulled": not_pulled,
        "files": files,
        "seconds": round(clock() - start, 1),
    }


def ssh_fetcher(workspace, run):
    conf = Path(workspace) / "ssh.conf"
    alias = (Path(workspace) / "alias").read_text().strip()

    def fetch(name, target, timeout):
        target.parent.mkdir(parents=True, exist_ok=True)
        remote = f"{alias}:/root/capture/{run}/{name}"
        subprocess.run(
            ["scp", "-q", "-F", str(conf), remote, str(target)],
            check=True,
            timeout=max(1, timeout),
            capture_output=True,
        )

    return fetch


def remote_text(workspace, command, timeout=60):
    conf = Path(workspace) / "ssh.conf"
    alias = (Path(workspace) / "alias").read_text().strip()
    return subprocess.run(
        ["ssh", "-F", str(conf), alias, command],
        check=True,
        capture_output=True,
        text=True,
        timeout=timeout,
    ).stdout


def main(argv):
    if len(argv) < 3:
        raise SystemExit(__doc__)
    workspace, evidence, runs = Path(argv[0]), Path(argv[1]), argv[2:]
    lease = int((workspace / "own-id").read_text())
    root = private_root(PRIVATE, lease=lease)
    if runs == ["ALL"]:
        listing = remote_text(workspace, "ls -1 /root/capture 2>/dev/null || true")
        runs = [r for r in listing.split() if RUN.fullmatch(r)]
    for run in runs:
        manifest = remote_text(workspace, f"cat /root/capture/{run}/MANIFEST.sha256")
        receipt = pull_arm(run, manifest, root, ssh_fetcher(workspace, run), deadline_s=1500)
        public = {k: v for k, v in receipt.items() if k != "private_dir"}
        public.update(private=True, lease=lease, raw_location="private dev-box capture dir")
        evidence.mkdir(parents=True, exist_ok=True)
        (evidence / f"capture_{run}.json").write_text(json.dumps(public, indent=1) + "\n")
        (root / f"capture_{run}_receipt.json").write_text(json.dumps(receipt, indent=1) + "\n")
        print(
            "CAPTURE_PULLED run=%s verified=%d failed=%d not_pulled=%d"
            % (run, receipt["verified"], len(receipt["failed"]), len(receipt["not_pulled"])),
            flush=True,
        )


if __name__ == "__main__":
    main(sys.argv[1:])
