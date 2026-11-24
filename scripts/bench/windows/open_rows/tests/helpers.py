# SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later
"""Fakes and fixtures of the open-rows tests (GPU-free, no network, no Windows). Fixtures carry only the fake UUID prefix fa4efa4e-fa4e-4fa4-8fa4-fa4efa4e."""
import json
import os
import sys

HERE = os.path.dirname(os.path.abspath(__file__))
OR = os.path.dirname(HERE)                                  # .../open_rows
AM = os.path.join(os.path.dirname(OR), "appmatrix")
FIX = os.path.join(HERE, "fixtures")
for p in (OR, AM):
    if p not in sys.path:
        sys.path.insert(0, p)

from kfopen_common import Result  # noqa: E402


def fixture(name):
    with open(os.path.join(FIX, name), encoding="utf-8") as f:
        return f.read()


def fixture_json(name):
    return json.loads(fixture(name))


class FakeRunner:
    """Records argument lists; `script` maps a predicate on argv to a Result or a callable(argv, cwd) -> Result."""

    def __init__(self, script=None, default=None):
        self.calls, self.script, self.default = [], list(script or []), default or Result(0, "")

    def run(self, argv, cwd=None, timeout=None, env=None, tee=None):
        assert isinstance(argv, (list, tuple)) and all(isinstance(a, str) for a in argv), argv
        self.calls.append({"argv": list(argv), "cwd": cwd, "timeout": timeout})
        for pred, res in self.script:
            if pred(argv):
                return res(argv, cwd) if callable(res) else res
        return self.default

    def argvs(self):
        return [c["argv"] for c in self.calls]

    def find(self, *tokens):
        return [c for c in self.calls if all(t in " ".join(c["argv"]) for t in tokens)]


class FakeFetcher:
    def __init__(self, texts=None, files=None):
        self.texts, self.files, self.downloads = dict(texts or {}), dict(files or {}), []

    def get_text(self, url, accept=None):
        if url not in self.texts:
            raise AssertionError("unexpected fetch of " + url)
        return self.texts[url]

    def download(self, url, dest):
        import hashlib
        data = self.files.get(url)
        if data is None:
            raise AssertionError("unexpected download of " + url)
        os.makedirs(os.path.dirname(os.path.abspath(dest)), exist_ok=True)
        with open(dest, "wb") as f:
            f.write(data)
        self.downloads.append(url)
        return hashlib.sha256(data).hexdigest()


class FakeProc:
    """subprocess.Popen stand-in driven by a FakeClock: exits when the clock reaches `exit_at` (None: never)."""

    def __init__(self, clock, exit_at=None, rc=0, pid=4242, exit_on_close=True):
        self.clock, self.exit_at, self.rc, self.pid, self.exit_on_close = clock, exit_at, rc, pid, exit_on_close
        self.killed, self.close_seen_at = False, None

    def poll(self):
        if self.killed:
            return -9
        if self.close_seen_at is not None and self.exit_on_close and self.clock.now() >= self.close_seen_at + 0.3:
            return self.rc
        if self.exit_at is not None and self.clock.now() >= self.exit_at:
            return self.rc
        return None

    def kill(self):
        self.killed = True


class FakeClock:
    def __init__(self):
        self.t = 0.0

    def now(self):
        return self.t

    def sleep(self, s):
        self.t += s


class FakeWin:
    """The Win32 layer: a window that appears after `appear_s`, titles from a function, captures from a list (cycled)."""

    def __init__(self, clock, proc, appear_s=1.0, title_fn=None, captures=None):
        self.clock, self.proc, self.appear_s = clock, proc, appear_s
        self.title_fn = title_fn or (lambda keys: "sample")
        self.captures, self.ci = captures or [], 0
        self.keys, self.closed = [], False

    def find_window(self, pid):
        return 1 if self.clock.now() >= self.appear_s else None

    def title(self, h):
        return self.title_fn(self.keys)

    def post_key(self, h, vk):
        self.keys.append(vk)

    def post_close(self, h):
        self.closed = True
        self.proc.close_seen_at = self.clock.now()

    def capture(self, h):
        c = self.captures[min(self.ci, len(self.captures) - 1)]
        self.ci += 1
        return c


def pixels(w, h, fn):
    """BGRA bytes of a w x h image; fn(x, y) -> (b, g, r)."""
    out = bytearray()
    for y in range(h):
        for x in range(w):
            b, g, r = fn(x, y)
            out += bytes((b, g, r, 255))
    return bytes(out)


def blank_capture(w=96, h=96):
    return (w, h, pixels(w, h, lambda x, y: (10, 20, 30)))


def busy_capture(seed, w=96, h=96):
    return (w, h, pixels(w, h, lambda x, y: ((x * 3 + seed) % 256, (y * 5 + seed * 7) % 256, (x * y + seed) % 256)))
