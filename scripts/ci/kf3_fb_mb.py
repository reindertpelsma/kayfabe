#!/usr/bin/env python3
"""Gate: every launcher that starts the kf3 device passes `fb-mb=` explicitly.

Owner decision 2026-10-24: the kayfabe store size (`fb-mb`, MiB) is REQUIRED on the QEMU command
line, exactly like the RAM size. `qemu/hw/misc/kf3/kf3.c` has no default and refuses an unset (or 0)
size at realize, and `kf_qemu::device::Config::check` says the same. A launcher whose emitted
`-device kf3-gpu,...` string lacks `fb-mb=` would therefore no longer start; this static scan
finds it before a hardware lane does.

A launcher MAY keep its own named variable with a script-level default (`fb-mb=${WIN_FB_MB:-4096}`):
the command line it emits still contains `fb-mb=`, which is all this gate checks. It does not run
anything and needs no GPU.

    python3 scripts/ci/kf3_fb_mb.py          # exit 1 and one line per offender
"""
import re
import subprocess
import sys
from dataclasses import dataclass
from pathlib import Path

ROOTS = ("scripts", "tools", ".github")
# The gate and its tests quote bad examples on purpose.
SKIP = {"scripts/ci/kf3_fb_mb.py", "scripts/ci/test_kf3_fb_mb.py"}
MAX_BYTES = 4 << 20
# Prose and data quote device strings; only launchers (code) are scanned.
NOT_CODE = {".md", ".json", ".jsonl", ".txt", ".csv", ".log"}

# `kf3-gpu` + optionally `,<property list>` (up to the closing quote or whitespace).
_NAME = re.compile(r"kf3-gpu(?P<props>,[^\"'\s]*)?")
# What must precede a BARE `kf3-gpu` for it to be a `-device` argument.
_BARE_BEFORE = re.compile(r"""(-device[ =]\s*|['"]-device['"]\s*,\s*)["']?$""")
_FB_MB = re.compile(r"(^|,)fb-mb=")


@dataclass(frozen=True)
class Violation:
    path: str
    line: int
    text: str

    def __str__(self):
        return f"{self.path}:{self.line}: kf3 device without fb-mb=: {self.text.strip()[:160]}"


def _devices(text):
    """Yield (line number, line, has_fb_mb) for each device definition in `text`."""
    for no, line in enumerate(text.splitlines(), 1):
        if line.lstrip().startswith("#"):
            continue
        for m in _NAME.finditer(line):
            end = m.end()
            props = m.group("props")
            if props is None:
                nxt = line[end : end + 1]
                if nxt == "." or (nxt and (nxt.isalnum() or nxt in "-_")):
                    continue  # `-global kf3-gpu.prop=`, `kf3-gpu.rom`, a longer name
                if not _BARE_BEFORE.search(line[: m.start()]):
                    continue  # a name in a grep, an xorg Identifier, ...
                yield no, line, False
            elif len(props) > 1:
                yield no, line, bool(_FB_MB.search(props[1:]))
            # `kf3-gpu,` alone is a prefix test (`startswith('kf3-gpu,')`), not a launcher


def scan_text(path, text):
    """The violations in one file's text."""
    if "kf3-gpu.fb-mb=" in text:  # `-global kf3-gpu.fb-mb=N` supplies it for every device
        return []
    return [Violation(path, no, line) for no, line, ok in _devices(text) if not ok]


def _files(root):
    root = Path(root)
    try:
        out = subprocess.run(
            ["git", "-C", str(root), "ls-files", "-z", "--", *ROOTS],
            capture_output=True,
            check=True,
        ).stdout.decode()
        names = [n for n in out.split("\0") if n]
    except (OSError, subprocess.CalledProcessError):
        names = []
    if not names:  # not a git tree (the unit tests build scratch trees)
        names = [
            str(p.relative_to(root))
            for r in ROOTS
            if (root / r).is_dir()
            for p in (root / r).rglob("*")
            if p.is_file()
        ]
    for n in sorted(names):
        p = root / n
        if n in SKIP or p.suffix in NOT_CODE or not p.is_file() or p.is_symlink():
            continue
        try:
            if p.stat().st_size > MAX_BYTES:
                continue
            yield n, p.read_bytes().decode("utf-8")
        except (OSError, UnicodeDecodeError):
            continue


def scan_root(root):
    """All violations under `root` (scripts, tools, .github)."""
    return [v for n, text in _files(root) for v in scan_text(n, text)]


def count_emitters(root):
    """How many files define a kf3 device (so a blind scanner is visible)."""
    return sum(1 for _, text in _files(root) if any(True for _ in _devices(text)))


def main():
    root = Path(__file__).resolve().parents[2]
    bad = scan_root(root)
    for v in bad:
        print(v)
    n = count_emitters(root)
    if bad:
        print(f"kf3 fb-mb gate: {len(bad)} launcher line(s) without fb-mb= ({n} launcher files scanned)")
        return 1
    print(f"kf3 fb-mb gate: clean ({n} launcher files carry fb-mb=)")
    return 0


if __name__ == "__main__":
    sys.exit(main())
