"""Operator side of the B1i media transfer: box-side fetch first, SSH upload per-file fallback.

The whole transfer phase keeps its 1200 s bound. The box fetches every file from its pinned
public URL (box_fetch.py) with a 600 s per-file cutoff; a file falls back to the existing checked
SSH upload as soon as the box reports failure or a sha256/size mismatch, when its measured rate
projects past the cutoff (checked after 90 s), or at the cutoff. The fallback therefore always has
at least 600 s of the original bound. Nothing secret is sent to the box.
"""

import json
import subprocess
import time

TOTAL_SECONDS = 1200
BOX_CUTOFF_SECONDS = 600
MIN_FALLBACK_SECONDS = TOTAL_SECONDS - BOX_CUTOFF_SECONDS
PROJECTION_AFTER_SECONDS = 90
POLL_SECONDS = 15
CACHE = "/workspace/bw-capc-validation/cache"
STATUS = "/root/prov/box_fetch_status.json"


def decide(status, elapsed, total_bytes):
    """Per file: 'done', 'fallback' or 'wait'."""
    state = (status or {}).get("state")
    if state == "verified":
        return "done"
    if state in ("failed", "mismatch"):
        return "fallback"
    if elapsed >= BOX_CUTOFF_SECONDS:
        return "fallback"
    if status and elapsed >= PROJECTION_AFTER_SECONDS:
        done = float(status.get("bytes_done", 0) or 0)
        total = float(status.get("bytes_total", total_bytes) or total_bytes)
        if done <= 0 or elapsed * total / done > BOX_CUTOFF_SECONDS:
            return "fallback"
    return "wait"


def ssh_runner(ssh):
    def run(cmd, timeout):
        r = subprocess.run(ssh + [cmd], capture_output=True, text=True, timeout=timeout, check=True)
        return r.stdout

    return run


def box_phase(
    runner,
    manifest,
    *,
    clock=time.monotonic,
    sleep=time.sleep,
    total=TOTAL_SECONDS,
    on_fallback=None
):
    assert total == TOTAL_SECONDS
    start = clock()
    sizes = {row["file"]: int(row["expected_bytes"]) for row in manifest["files"]}
    try:
        started = runner(
            "set -eu; mkdir -p /root/prov %s; nohup python3 /root/sweep2/box_fetch.py "
            "/root/sweep2/media_manifest.json %s %d %s >/root/prov/box_fetch.log 2>&1 "
            "</dev/null & echo $!" % (CACHE, CACHE, BOX_CUTOFF_SECONDS, STATUS),
            30,
        )
    except (subprocess.SubprocessError, OSError):
        for name in sizes:
            if on_fallback:
                on_fallback(name)
        return {
            "verified": set(),
            "fallback": list(sizes),
            "box_pid": None,
            "elapsed_s": round(clock() - start, 1),
            "status": {},
            "start_failed": True,
            "cutoff_s": BOX_CUTOFF_SECONDS,
            "total_s": TOTAL_SECONDS,
        }
    verified, fallback, last = set(), [], {}
    while True:
        elapsed = clock() - start
        try:
            last = json.loads(runner("cat %s" % STATUS, 20)).get("files", {})
        except (subprocess.SubprocessError, ValueError, OSError):
            pass
        for name, size in sizes.items():
            if name in verified or name in fallback:
                continue
            verdict = decide(last.get(name), elapsed, size)
            if verdict == "wait" and elapsed >= BOX_CUTOFF_SECONDS:
                verdict = "fallback"  # hard bound, independent of decide()
            if verdict == "done":
                verified.add(name)
            elif verdict == "fallback":
                fallback.append(name)
                try:  # stop the box-side fetch of this file before the upload starts
                    runner("touch %s/%s.boxfetch.stop" % (CACHE, name), 20)
                except (subprocess.SubprocessError, OSError):
                    pass
                if on_fallback:
                    on_fallback(name)
        if len(verified) + len(fallback) == len(sizes):
            break
        sleep(min(POLL_SECONDS, max(1, BOX_CUTOFF_SECONDS - (clock() - start))))
    return {
        "verified": verified,
        "fallback": fallback,
        "box_pid": started.strip(),
        "elapsed_s": round(clock() - start, 1),
        "status": last,
        "cutoff_s": BOX_CUTOFF_SECONDS,
        "total_s": TOTAL_SECONDS,
    }
