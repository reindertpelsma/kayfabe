#!/usr/bin/env python3
"""GPU-free UUID scanner mutants; every regression must have two distinct killers.

Run copied source/tests in temporary directories. Refuse missing/repeated anchors,
a red baseline, a harness error or fewer than two failing test methods per mutant.
"""

import json
from pathlib import Path
import subprocess
import sys
import tempfile

HERE = Path(__file__).resolve().parent
MUTANTS = [
    (
        "base64-label-shifts-alignment",
        "for alignment in range(4):",
        "for alignment in range(1):",
    ),
    ("base64-labelled-tail-dropped", "used = len(encoded) // 4 * 4", "used = 0"),
    (
        "base64-repeated-alignments",
        "for alignment in range(4):",
        "for alignment in range(8):",
    ),
    (
        "marked-compact-hidden",
        "COMPACT_UUID = hex_group(32)",
        'COMPACT_UUID = rb"(?!)"',
    ),
    (
        "wrapped-uuid-hidden",
        'WRAPPED_UUID = (WS + b"-" + WS).join(hex_group(n) for n in (8, 4, 4, 4, 12))',
        "WRAPPED_UUID = _UUID",
    ),
    ("marker-newline-hidden", "    + WS\n", '    + rb"[ \\t]{0,64}"\n'),
    (
        "short-wrapped-base64-hidden",
        "(?P<short>[A-Za-z0-9+/]{2,15}",
        "(?P<short>[A-Za-z0-9+/]{16,16}",
    ),
    (
        "compact-hex-prefiltered",
        "hex_candidates = HEX.finditer(data)",
        'hex_candidates = HEX.finditer(data) if b"2d" in low else ()',
    ),
    (
        "formatted-identity-not-normalized",
        "uuid = canonical_uuid(match.group(1))",
        "uuid = match.group(1)",
    ),
    (
        "marked-format-limit-ignored",
        "if match.end() - match.start() > MAX_MARKED_BYTES:",
        "if False:",
    ),
    (
        "skippable-zstd-hidden",
        'and 0x184D2A50 <= int.from_bytes(data[:4], "little") <= 0x184D2A5F',
        "and False",
    ),
    (
        "tracked-roots-omitted",
        '"ls-files", "-z", "--stage"',
        '"ls-files", "-z", "--stage", "--", "traces", "docs"',
    ),
    (
        "unversioned-roots-omitted",
        "os.walk(root, followlinks=False)",
        'os.walk(root / "traces", followlinks=False)',
    ),
    (
        "gitlinks-treated-as-files",
        'if mode == b"160000":',
        "if False:",
    ),
    (
        "symlink-text-omitted",
        "return os.fsencode(os.readlink(parts[-1], dir_fd=directory))",
        'return b""',
    ),
    (
        "symlink-target-followed",
        "return os.fsencode(os.readlink(parts[-1], dir_fd=directory))",
        "return (root / rel).read_bytes()",
    ),
    (
        "symlink-parent-followed",
        "part, os.O_RDONLY | os.O_DIRECTORY | os.O_NOFOLLOW, dir_fd=directory",
        "part, os.O_RDONLY | os.O_DIRECTORY, dir_fd=directory",
    ),
    (
        "nonregular-file-accepted",
        'if not stat.S_ISREG(info.st_mode):\n            raise ScanError("nonregular tracked file")',
        'if not stat.S_ISREG(info.st_mode):\n            return b""',
    ),
    ("gzip-hidden", 'return "gzip"', "return None"),
    ("tar-hidden", 'elif kind == "tar":', "elif False:"),
    ("zip-hidden", 'elif kind == "zip":', "elif False:"),
    ("xz-hidden", 'return "xz"', "return None"),
    ("zstd-hidden", 'return "zstd"', "return None"),
    ("hex-hidden", "for match in hex_candidates:", "for match in []:"),
    ("base64-hidden", "for match in B64.finditer(data):", "for match in []:"),
    ("utf16-hidden", "for match in re.finditer(pattern, data):", "for match in []:"),
    ("raw-container-coverage-lost", "hits |= text_scan(data, _uuids)", "pass"),
    ("depth-unbounded", "if _depth >= MAX_DEPTH:", "if False:"),
    ("count-unbounded", "if self.members > MAX_MEMBERS:", "if False:"),
    (
        "member-size-unbounded",
        "remaining = min(MAX_MEMBER_BYTES, MAX_EXPANDED_BYTES - self.expanded)",
        "remaining = MAX_EXPANDED_BYTES - self.expanded",
    ),
    (
        "total-size-unbounded",
        "remaining = min(MAX_MEMBER_BYTES, MAX_EXPANDED_BYTES - self.expanded)",
        "remaining = MAX_MEMBER_BYTES",
    ),
    (
        "corruption-is-clean",
        'raise ScanError("invalid or unsupported archive") from None',
        "return Counter()",
    ),
    (
        "cli-incomplete-passes",
        "        return 2\n    if args.init:",
        "        return 0\n    if args.init:",
    ),
    ("root-unbounded", "if len(data) > MAX_FILE_BYTES:", "if False:"),
    ("xz-dictionary-unbounded", "memlimit=128 * 1024 * 1024", "memlimit=None"),
    (
        "zstd-window-unbounded",
        "zstd.DecompressionParameter.window_log_max: 27",
        "zstd.DecompressionParameter.window_log_max: 31",
    ),
    ("zstd-cli-window-unbounded", '"--memory=128MB"', '"--memory=2GB"'),
    (
        "marker-prefilter-loses-gpu",
        '(b"gpu-", b"gpu_uuid", b"deviceuuid")',
        '(b"gpu_uuid", b"deviceuuid")',
    ),
    (
        "marker-prefilter-loses-device",
        '(b"gpu-", b"gpu_uuid", b"deviceuuid")',
        '(b"gpu-", b"gpu_uuid")',
    ),
]
COLLECT = """import io,json,sys,unittest
suite=unittest.defaultTestLoader.discover('.',pattern='test_gpu_uuid*.py')
r=unittest.TextTestRunner(stream=io.StringIO()).run(suite)
print(json.dumps({'tests':r.testsRun,'killers':sorted(set(t.id().split(' (',1)[0] for t,_ in r.failures+r.errors))}))
"""


def run(directory):
    done = subprocess.run(
        [sys.executable, "-c", COLLECT],
        cwd=directory,
        capture_output=True,
        text=True,
        timeout=120,
    )
    if done.returncode:
        raise RuntimeError("mutant harness failure")
    return json.loads(done.stdout.splitlines()[-1])


def main():
    source = (HERE / "no_gpu_uuids.py").read_text()
    rows = []
    with tempfile.TemporaryDirectory(prefix="kf-uuid-mutants-") as parent:
        directory = Path(parent)
        (directory / "test_gpu_uuid_archives.py").write_bytes(
            (HERE / "test_gpu_uuid_archives.py").read_bytes()
        )
        (directory / "test_gpu_uuid_review.py").write_bytes(
            (HERE / "test_gpu_uuid_review.py").read_bytes()
        )
        target = directory / "no_gpu_uuids.py"
        target.write_text(source)
        baseline = run(directory)
        if baseline["killers"]:
            raise RuntimeError("baseline red")
        for name, old, new in MUTANTS:
            if source.count(old) != 1:
                raise RuntimeError("mutant anchor is not unique: " + name)
            target.write_text(source.replace(old, new))
            # Disable bytecode caching: same size/mtime mutations must not load
            # the previous mutant's pyc on a fast filesystem.
            for cache in directory.glob("__pycache__/*"):
                cache.unlink()
            result = run(directory)
            rows.append(
                {
                    "mutant": name,
                    "count": len(result["killers"]),
                    "killers": result["killers"],
                }
            )
    print(json.dumps({"tests": baseline["tests"], "cases": rows}, indent=2))
    return int(any(row["count"] < 2 for row in rows))


if __name__ == "__main__":
    sys.exit(main())
