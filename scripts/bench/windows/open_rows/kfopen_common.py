# SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later
"""Shared helpers of the open Windows rows (STATUS: DESIGN-ONLY, 2026-11-17; GPU-free logic, nothing here ran on a Windows guest).

Standard library only, Python 3.9+, so the same files run under the guest's python.org install, a standalone
user's Python and the Linux CI tests. No function here uses a shell: every command is an argument list
(docs/design/V3_WINDOWS_OPEN_ROWS.md, "Shell is glue, logic is Python").
"""
import datetime
import hashlib
import json
import os
import re
import subprocess
import sys
import time

HERE = os.path.dirname(os.path.abspath(__file__))
FAKE_UUID_PREFIX = "fa4efa4e-fa4e-4fa4-8fa4-fa4efa4e"
UUID_RX = re.compile(r"(?i)(?:GPU-)?[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}")


class OpenRowError(Exception):
    """A refusal or a failed precondition; the message is one line a person can act on."""


def utc():
    return datetime.datetime.now(datetime.timezone.utc).strftime("%Y-%m-%dT%H:%M:%SZ")


def say(key, value="", stream=None):
    """The one output protocol of the rows: `KFOPEN <key> <value>` (the same shape as the KFGL / KFSURVIVE lines of the matrix)."""
    line = ("KFOPEN %s %s" % (key, value)).rstrip()
    print(line, file=stream or sys.stdout, flush=True)
    return line


def check_line(name, ok, why=""):
    """`CHECK <name> ok` or `CHECK <name> FAIL <why>`: the second form is what the matrix's default fail_rx catches."""
    line = ("CHECK %s ok" % name) if ok else ("CHECK %s FAIL %s" % (name, why)).rstrip()
    print(line, flush=True)
    return line


def scrub_uuids(text):
    """Replace every UUID (GPU board or device UUIDs that deqp and Vulkan print) by a fake one that keeps equalities.

    The repository is public; a board identifier never belongs in committed evidence (scripts/ci/no_gpu_uuids.py)."""
    seen = {}

    def sub(m):
        key = m.group(0).lower().replace("gpu-", "")
        seen.setdefault(key, FAKE_UUID_PREFIX + "%04x" % len(seen))
        return seen[key]
    return UUID_RX.sub(sub, text)


def sha256_file(path):
    h = hashlib.sha256()
    with open(path, "rb") as f:
        for block in iter(lambda: f.read(1 << 20), b""):
            h.update(block)
    return h.hexdigest()


def sha256_text(text):
    return hashlib.sha256(text.encode("utf-8")).hexdigest()


def canonical(obj):
    return json.dumps(obj, sort_keys=True, separators=(",", ":"))


def load_json(path):
    with open(path, encoding="utf-8") as f:
        return json.load(f)


def write_json(path, obj):
    os.makedirs(os.path.dirname(os.path.abspath(path)), exist_ok=True)
    tmp = path + ".tmp"
    with open(tmp, "w", encoding="utf-8", newline="\n") as f:
        json.dump(obj, f, indent=1, sort_keys=True)
        f.write("\n")
    os.replace(tmp, path)


class Result:
    def __init__(self, rc, out="", secs=0.0, timed_out=False):
        self.rc, self.out, self.secs, self.timed_out = rc, out, secs, timed_out

    def __repr__(self):
        return "Result(rc=%r, timed_out=%r, secs=%.1f)" % (self.rc, self.timed_out, self.secs)


class Runner:
    """Runs argument lists. The tests replace it by a recorder (tests/helpers.py FakeRunner); nothing else spawns processes."""

    def run(self, argv, cwd=None, timeout=None, env=None, tee=None):
        if not isinstance(argv, (list, tuple)) or not all(isinstance(a, str) for a in argv):
            raise OpenRowError("argv must be a list of strings (no shell strings)")
        t0 = time.time()
        try:
            p = subprocess.Popen(list(argv), cwd=cwd, env=env, stdout=subprocess.PIPE, stderr=subprocess.STDOUT)
        except OSError as e:
            return Result(127, "START_FAILED %s: %s" % (argv[0], e), time.time() - t0)
        try:
            out, _ = p.communicate(timeout=timeout)
            timed_out = False
        except subprocess.TimeoutExpired:
            kill_tree(p)
            out, _ = p.communicate()
            timed_out = True
        text = (out or b"").decode("utf-8", "replace")
        if tee:
            with open(tee, "a", encoding="utf-8", errors="replace") as f:
                f.write(text)
        return Result(124 if timed_out else p.returncode, text, time.time() - t0, timed_out)


def kill_tree(p):
    """taskkill /T /F on Windows (the matrix's supervisor does the same); plain kill elsewhere."""
    if os.name == "nt":
        subprocess.run(["taskkill.exe", "/PID", str(p.pid), "/T", "/F"], stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
    else:
        p.kill()


class Trust:
    """Standalone defaults to `git-only`. Everything that lifts it is explicit and ends up in the receipt (owner, 2026-11-17)."""

    def __init__(self, accept_unanchored=None, commit_overrides=None, user_tools=None, prebuilt=None):
        self.accept_unanchored = accept_unanchored            # reason string, or None
        self.commit_overrides = dict(commit_overrides or {})  # {"cts": sha}
        self.user_tools = dict(user_tools or {})              # {"git": path}: always trusted, hash recorded, never verified
        self.prebuilt = prebuilt                              # a directory the user built; trusted, hash recorded

    @classmethod
    def from_args(cls, args):
        over, tools = {}, {}
        for item in getattr(args, "source_commit_override", None) or []:
            name, sep, sha = item.partition("=")
            if not sep or not re.fullmatch(r"[0-9a-f]{40}", sha):
                raise OpenRowError("--source-commit-override wants NAME=<40 hex commit>")
            over[name] = sha
        for item in getattr(args, "tool", None) or []:
            name, sep, path = item.partition("=")
            if not sep or not path:
                raise OpenRowError("--tool wants NAME=PATH")
            tools[name] = path
        reason = getattr(args, "accept_unanchored_downloads", None)
        if reason is not None and not reason.strip():
            raise OpenRowError("--accept-unanchored-downloads needs a reason (it is written into the receipt)")
        return cls(reason, over, tools, getattr(args, "prebuilt", None))

    def record(self):
        return {"mode": "git-only" if not (self.accept_unanchored or self.commit_overrides or self.user_tools or self.prebuilt) else "lifted",
                "accept_unanchored_downloads": self.accept_unanchored, "source_commit_override": self.commit_overrides,
                "user_tools": sorted(self.user_tools), "prebuilt": self.prebuilt}


def add_trust_args(ap):
    ap.add_argument("--accept-unanchored-downloads", metavar="REASON", help="explicitly lift the download anchors (recorded in the receipt); sources stay pinned")
    ap.add_argument("--source-commit-override", action="append", metavar="NAME=SHA", help="build another commit of cts|dxs (explicit, recorded)")
    ap.add_argument("--tool", action="append", metavar="NAME=PATH", help="use this tool instead of fetching it (trusted, sha256 recorded, not verified)")
    ap.add_argument("--prebuilt", metavar="DIR", help="use a tree you built yourself instead of building (trusted, sha256 recorded)")
