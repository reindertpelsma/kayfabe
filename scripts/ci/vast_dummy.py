"""Disposable non-VM acceptance payload; no kayfabe build or attestation."""

import hashlib
import json
import os
from pathlib import Path
import re
import subprocess
import sys
import time

LOCK = {
    "blob-a.txt": ("437f93a9c1e7fa5a3f84fbb02e9a382e98553f25cce789410285e9064fcf8c28", 49),
    "blob-b.txt": ("ac4cd519d42d0635dad2158dff7950bc6ac2caede47f127b95339b10b780b61e", 270177),
}


def hardware():
    row = (
        subprocess.check_output(
            ["nvidia-smi", "--query-gpu=name,compute_cap,driver_version", "--format=csv,noheader"],
            text=True,
        )
        .strip()
        .splitlines()
    )
    if len(row) != 1:
        raise ValueError("GPU count")
    name, cc, driver = [v.strip() for v in row[0].split(",")]
    if (
        name == "unknown"
        or not re.fullmatch(r"[A-Za-z0-9_. -]{1,100}", name)
        or not re.fullmatch(r"\d{1,2}\.\d", cc)
        or not re.fullmatch(r"\d{3}\.\d{1,3}(?:\.\d{1,2})?", driver)
    ):
        raise ValueError("GPU fields")
    cap = round(float(cc) * 100)
    if cap < 750:
        raise ValueError("compute capability")
    return {"gpu_name": name, "compute_cap": cap, "driver_version": driver}


def verify_blob(data, name):
    digest, size = LOCK[name]
    if len(data) != size or hashlib.sha256(data).hexdigest() != digest:
        raise ValueError("blob integrity")


def download(url, path):
    status = subprocess.check_output(
        [
            "curl",
            "--silent",
            "--show-error",
            "--fail",
            "--max-time",
            "60",
            "--proto",
            "=https",
            "--max-redirs",
            "0",
            "--output",
            str(path),
            "--write-out",
            "%{http_code}",
            url,
        ],
        text=True,
    )
    if status != "200":
        raise ValueError("download status")


def main():
    sha, nonce = sys.argv[1:]
    if sha != os.environ["KF_SHA"] or nonce != os.environ["KF_NONCE"]:
        raise ValueError("binding")
    mode = Path("scripts/ci/vast_dummy_mode").read_text().strip()
    if mode not in {"A", "B"}:
        raise ValueError("reporter mode")
    print("DUMMY_" + mode + " START", flush=True)
    print("::error::dummy untrusted log", flush=True)
    print("Bearer " + "FAKE" * 12, flush=True)
    subprocess.run(["nvidia-smi"], check=True)
    info = hardware()
    start = time.monotonic()
    iterations = 0
    while time.monotonic() - start < 30:
        assert (
            hashlib.sha256(b"abc").hexdigest()
            == "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        )
        iterations += 1
    print(
        "SELF_TEST seconds="
        + str(round(time.monotonic() - start, 3))
        + " iterations="
        + str(iterations),
        flush=True,
    )
    if mode == "B":
        first = None
        url_a = None
        for name in LOCK:
            raw = subprocess.check_output(["kf-box-blob-url", "dummy-blobs-20261022", name])
            locator = json.loads(raw)
            if set(locator) != {"url", "name", "size"} or locator["name"] != name:
                raise ValueError("locator schema")
            target = Path("/workspace") / name
            download(locator["url"], target)
            verify_blob(target.read_bytes(), name)
            print(
                "BLOB " + name + " bytes=" + str(target.stat().st_size) + " sha256_ok=1", flush=True
            )
            if name == "blob-a.txt":
                url_a = locator["url"]
                first = time.monotonic()
        time.sleep(max(0, 60 - (time.monotonic() - first)))
        download(url_a, Path("/workspace/blob-a-reuse.txt"))
        verify_blob(Path("/workspace/blob-a-reuse.txt").read_bytes(), "blob-a.txt")
        print(
            "SIGNED_URL_REUSE after_seconds="
            + str(round(time.monotonic() - first, 3))
            + " status=200 sha256_ok=1",
            flush=True,
        )
    result = {
        "nonce": nonce,
        "sha": sha,
        "verdict": "PASS" if mode == "B" else "FAIL",
        "suites": [
            {
                "name": "dummy_self_test",
                "pass": 1 if mode == "B" else 0,
                "fail": 0 if mode == "B" else 1,
                "skip": 0,
            }
        ],
        "hardware": info,
    }
    Path("/workspace/kf-ci-result.json").write_text(json.dumps(result))
    print("DUMMY_" + mode + " COMPLETE", flush=True)
    return 0 if mode == "B" else 7


if __name__ == "__main__":
    try:
        raise SystemExit(main())
    except Exception as error:
        print("DUMMY infrastructure failure " + type(error).__name__, flush=True)
        raise SystemExit(1)
