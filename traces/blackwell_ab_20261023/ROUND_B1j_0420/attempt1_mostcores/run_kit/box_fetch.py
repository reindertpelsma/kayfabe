"""Box-side media fetch from pinned public HTTPS sources (B1i, controller 04:00).

Runs on the rental. Reads the pinned table (file, url, expected_bytes, expected_sha256) from the
uploaded media manifest, downloads each file as parallel byte ranges with curl, verifies size and
sha256, and only then publishes <cache>/<file>.part (the name the existing whole-set verifier
renames). A mismatch, an unhonoured range, a short body or the per-file cutoff publishes nothing
and removes every fragment; the operator then uses the SSH upload for that file. Public URLs
only; nothing secret is sent to or read on the box.

Usage: box_fetch.py <manifest.json> <cache_dir> <cutoff_seconds> <status.json> [--workers N]
"""

import argparse
from concurrent.futures import ThreadPoolExecutor
import hashlib
import json
from pathlib import Path
import subprocess
import threading
import time

MIN_RANGED_BYTES = 64 * 1024
ALLOWED_PREFIXES = (
    "https://go.microsoft.com/fwlink/",
    "https://us.download.nvidia.com/Windows/",
    "https://fedorapeople.org/groups/virt/virtio-win/",
    "https://github.com/PowerShell/Win32-OpenSSH/releases/download/",
)


def allowed_url(url, allow_http_for_test=False):
    if allow_http_for_test and url.startswith("http://127.0.0.1:"):
        return True
    return isinstance(url, str) and url.startswith(ALLOWED_PREFIXES)


def ranges(size, workers):
    n = 1 if size < MIN_RANGED_BYTES else max(1, min(workers, 16))
    q, r = divmod(size, n)
    out, offset = [], 0
    for i in range(n):
        count = q + (i < r)
        out.append((offset, offset + count - 1))
        offset += count
    return out


def _curl(url, start, end, out, deadline, ranged, stop):
    cmd = [
        "curl",
        "-fsSL",
        "--proto",
        "=https,http",
        "--retry",
        "2",
        "--max-time",
        str(max(1, int(deadline - time.monotonic()))),
        "-o",
        str(out),
    ]
    if ranged:
        cmd += ["-r", "%d-%d" % (start, end)]
    cmd.append(url)
    child = subprocess.Popen(cmd, stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
    try:
        while child.poll() is None:
            if stop.exists() or time.monotonic() > deadline:
                child.kill()
                child.wait(timeout=5)
                return None
            time.sleep(0.2)
        return child.returncode
    finally:
        if child.poll() is None:
            child.kill()
            child.wait(timeout=5)


def fetch_file(row, cache, deadline, workers=8, allow_http_for_test=False, progress=None):
    cache = Path(cache)
    cache.mkdir(parents=True, exist_ok=True)
    name, size, digest = row["file"], int(row["expected_bytes"]), row["expected_sha256"]
    if Path(name).name != name or not allowed_url(row["url"], allow_http_for_test):
        return {"state": "failed", "reason": "name or url not allowed", "ranges": 0}
    plan = ranges(size, workers)
    parts = [cache / ("%s.boxfetch.%d" % (name, i)) for i in range(len(plan))]
    status = {"state": "running", "ranges": len(plan), "bytes_total": size, "bytes_done": 0}
    stop = cache / (name + ".boxfetch.stop")  # written by the operator when it falls back

    def cleanup():
        for p in parts + [cache / (name + ".boxfetch")]:
            p.unlink(missing_ok=True)

    def one(i):
        start, end = plan[i]
        remaining = deadline - time.monotonic()
        if remaining <= 0 or stop.exists():
            return False
        rc = _curl(row["url"], start, end, parts[i], deadline, len(plan) > 1, stop)
        return rc == 0 and parts[i].exists() and parts[i].stat().st_size == end - start + 1

    try:
        with ThreadPoolExecutor(max_workers=len(plan)) as pool:
            jobs = [pool.submit(one, i) for i in range(len(plan))]
            while not all(j.done() for j in jobs):
                if stop.exists():
                    break
                status["bytes_done"] = sum(p.stat().st_size for p in parts if p.exists())
                if progress:
                    progress(dict(status))
                time.sleep(0.5)
            ok = not stop.exists() and all(j.result() for j in jobs)
        if not ok or time.monotonic() > deadline:
            cleanup()
            return dict(status, state="failed", reason="range download failed or cutoff")
        whole = cache / (name + ".boxfetch")
        h = hashlib.sha256()
        with whole.open("wb") as out:
            for p in parts:
                with p.open("rb") as src:
                    for block in iter(lambda: src.read(8 * 1024 * 1024), b""):
                        h.update(block)
                        out.write(block)
        if stop.exists():
            cleanup()
            return dict(status, state="failed", reason="operator fell back to upload")
        if whole.stat().st_size != size or h.hexdigest() != digest:
            cleanup()
            return dict(status, state="mismatch", reason="size or sha256 differs from the pin")
        whole.rename(cache / (name + ".part"))
        cleanup()
        return dict(status, state="verified", bytes_done=size, sha256=digest)
    except OSError as error:
        cleanup()
        return dict(status, state="failed", reason=type(error).__name__)


def main(argv=None):
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("manifest")
    parser.add_argument("cache")
    parser.add_argument("cutoff", type=float)
    parser.add_argument("status")
    parser.add_argument("--workers", type=int, default=8)
    parser.add_argument("--allow-http-for-test", action="store_true")
    args = parser.parse_args(argv)
    manifest = json.loads(Path(args.manifest).read_text())
    if manifest.get("all_verified") is not True:
        raise SystemExit("manifest is not a verified pin table")
    deadline = time.monotonic() + args.cutoff
    lock = threading.Lock()
    files = {row["file"]: {"state": "pending"} for row in manifest["files"]}
    status_path = Path(args.status)

    def write(done=False):
        with lock:
            tmp = status_path.with_suffix(".tmp")
            tmp.write_text(json.dumps({"files": files, "done": done}, indent=2) + "\n")
            tmp.replace(status_path)

    def run(row):
        def progress(st):
            files[row["file"]] = st
            write()

        files[row["file"]] = fetch_file(
            row, args.cache, deadline, args.workers, args.allow_http_for_test, progress
        )
        write()

    write()
    with ThreadPoolExecutor(max_workers=len(manifest["files"])) as pool:
        list(pool.map(run, manifest["files"]))
    write(done=True)
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
