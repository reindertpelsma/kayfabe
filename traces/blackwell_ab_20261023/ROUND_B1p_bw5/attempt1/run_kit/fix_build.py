"""BW5 round B1p: box-side checks for the pinned fix revision (fix_pin.json); used by build_fix.sh.

The pinned revision is the merge of the relay-route fix and the second-life fix (controller 11:50):
HEAD's two parents are the two pinned fix heads, each two commits (RED, fix) above the pinned base.

source  PIN REPO                                  the worktree is the exact clean pinned chain
tests   PIN LOG                                   the GPU-free test log passed (no failure)
receipt PIN REPO BENCH TESTLOG BUILDLOG OUT        binary present, carries the route log line;
                                                  writes the sha256 receipt the controller pins
"""

import datetime as dt
import hashlib
import json
from pathlib import Path
import re
import subprocess
import sys

HEX40 = re.compile(r"[0-9a-f]{40}")
PASSED = re.compile(r"test result: ok\. (\d+) passed; 0 failed; (\d+) ignored;")


def load_pin(path):
    pin = json.loads(Path(path).read_text())
    revs = [pin.get("source"), pin.get("base"), *(pin.get("parents") or [])]
    if len(revs) != 4 or not all(HEX40.fullmatch(str(rev)) for rev in revs):
        raise ValueError("pin needs a full source, base and exactly two parents")
    if pin.get("bin_dir") != pin["source"][:8]:
        raise ValueError("pin bin_dir is not the source's kf3-bins directory")
    line = pin.get("log_line")
    if type(line) is not str or len(line) < 8 or not line.isascii():
        raise ValueError("pin log_line missing")
    if pin.get("log_source") != "crates/kf-qemu/src/userrelay.rs":
        raise ValueError("pin log_source is not the route module")
    markers = pin.get("source_markers")
    if not markers or not all(type(m) is list and len(m) == 2 and all(m) for m in markers):
        raise ValueError("pin source_markers missing")
    return pin


def _git(repo, *args, check=True):
    r = subprocess.run(["git", "-C", str(repo), *args], capture_output=True, text=True)
    if check and r.returncode:
        raise ValueError("git " + " ".join(args) + " failed: " + r.stderr.strip()[-200:])
    return r.stdout.strip()


def check_source(repo, pin):
    head = _git(repo, "rev-parse", "HEAD")
    if head != pin["source"]:
        raise ValueError("checked-out revision is not the pinned fix")
    for n, parent in enumerate(pin["parents"], 1):
        if _git(repo, "rev-parse", f"HEAD^{n}") != parent:
            raise ValueError(f"merge parent {n} is not the pinned fix head")
        if _git(repo, "rev-parse", f"HEAD^{n}~2") != pin["base"]:
            raise ValueError(f"merge parent {n} is not two commits above the pinned base")
    if _git(repo, "rev-parse", "--verify", "-q", "HEAD^3", check=False):
        raise ValueError("the pinned revision has more than two parents")
    if _git(repo, "status", "--porcelain", "--untracked-files=no"):
        raise ValueError("fix worktree is dirty")
    if pin["log_line"] not in _git(repo, "show", "HEAD:" + pin["log_source"]):
        raise ValueError("the pinned source does not carry the route log line")
    for path, marker in pin["source_markers"]:
        if marker not in _git(repo, "show", "HEAD:" + path):
            raise ValueError("the pinned source lacks a fix marker in " + path)
    return head


def tests_ok(text):
    if "test result: FAILED" in text:
        raise ValueError("a GPU-free test failed")
    counts = [(int(a), int(b)) for a, b in PASSED.findall(text)]
    passes = sum(a for a, _ in counts)
    if not counts or passes == 0:
        raise ValueError("no passing GPU-free test result")
    return passes, sum(b for _, b in counts)


def receipt(pin, repo, bench, test_log, build_log):
    source = check_source(repo, pin)
    passes, ignored = tests_ok(Path(test_log).read_text())
    binary = Path(bench) / "kf3-bins" / pin["bin_dir"] / "qemu-system-x86_64"
    if not binary.is_file():
        raise ValueError("pinned kf3 binary missing: " + str(binary))
    body = binary.read_bytes()
    if pin["log_line"].encode() not in body:
        raise ValueError("kf3 binary does not carry the route log line")
    return {
        "source": source,
        "parents": pin["parents"],
        "base": pin["base"],
        "branch": pin.get("branch"),
        "binary": str(binary),
        "binary_sha256": hashlib.sha256(body).hexdigest(),
        "route_log_line": pin["log_line"],
        "route_log_line_in_binary": True,
        "tests_passed": True,
        "test_passes": passes,
        "test_ignored": ignored,
        "test_log": str(test_log),
        "build_log": str(build_log),
        "utc": dt.datetime.now(dt.timezone.utc).isoformat(),
        "scope": "relay-route fix b85d02bae + second-life fix 38fd948fd, merged on cb80cbd29",
        "host_verification": "UNVERIFIED-HOST",
        "experiment_mode": "OBSERVATION",
        "attestation": False,
    }


def main(argv):
    if argv[:1] == ["source"] and len(argv) == 3:
        print("OBSERVED_FIX_SOURCE", check_source(argv[2], load_pin(argv[1])))
    elif argv[:1] == ["tests"] and len(argv) == 3:
        print("OBSERVED_FIX_TESTS passes=%d ignored=%d" % tests_ok(Path(argv[2]).read_text()))
    elif argv[:1] == ["receipt"] and len(argv) == 7:
        row = receipt(load_pin(argv[1]), argv[2], argv[3], argv[4], argv[5])
        out = Path(argv[6])
        tmp = out.with_suffix(".tmp")
        tmp.write_text(json.dumps(row, indent=2) + "\n")
        tmp.replace(out)
        print(json.dumps(row))
    else:
        raise SystemExit(__doc__)


if __name__ == "__main__":
    main(sys.argv[1:])
