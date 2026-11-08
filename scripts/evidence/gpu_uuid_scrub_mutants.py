#!/usr/bin/env python3
"""Scrubber regressions, each killed by two independent test methods (GPU-free)."""

import json
from pathlib import Path
import subprocess
import sys
import tempfile

HERE = Path(__file__).resolve().parent
CI = HERE.parent / "ci"
MUTANTS = [
    ("logical-stream-digests-ignored", "if extra_tables is not None:", "if False:"),
    (
        "key-ignored",
        "hmac.new(key, original.lower(), hashlib.sha256)",
        'hmac.new(b"ignored", original.lower(), hashlib.sha256)',
    ),
    (
        "identity-ignored",
        "hmac.new(key, original.lower(), hashlib.sha256)",
        'hmac.new(key, b"ignored", hashlib.sha256)',
    ),
    (
        "ascii-crossrefs-unscrubbed",
        "data = BARE.sub(lambda m: mapper.values.get(m.group().lower(), m.group()), data)",
        "pass",
    ),
    (
        "encoded-crossrefs-unscrubbed",
        "for (start, end), encoding, decoded in gate.encoded_views(data):",
        "for (start, end), encoding, decoded in []:",
    ),
    (
        "unmarked-binary-corrupted",
        "    return data\n\n\ndef rewrite_text",
        '    for old, new in mapper.values.items():\n        data = data.replace(bytes.fromhex(old.decode().replace("-", "")), bytes.fromhex(new.decode().replace("-", "")))\n    return data\n\n\ndef rewrite_text',
    ),
    (
        "unmarked-hex-counter-corrupted",
        "    return data\n\n\ndef rewrite_text",
        '    for old, new in mapper.values.items():\n        data = data.replace(old.replace(b"-", b""), new.replace(b"-", b""))\n    return data\n\n\ndef rewrite_text',
    ),
    (
        "md5-stale",
        "for algorithm in (hashlib.md5, hashlib.sha256):",
        "for algorithm in (hashlib.sha256,):",
    ),
    (
        "sha256-stale",
        "for algorithm in (hashlib.md5, hashlib.sha256):",
        "for algorithm in (hashlib.md5,):",
    ),
    ("tsv-size-stale", "fields[-2] = sizes[fields[-1].lower()]", "pass"),
    (
        "tar-member-order-reversed",
        "for i, member in enumerate(infos):\n                body = rewritten.get(i)",
        "for i, member in reversed(list(enumerate(infos))):\n                body = rewritten.get(i)",
    ),
    (
        "zip-member-order-reversed",
        "for i, info in enumerate(new_infos):",
        "for i, info in reversed(list(enumerate(new_infos))):",
    ),
]
COLLECT = """import io,json,unittest
suite=unittest.defaultTestLoader.discover('.',pattern='test_scrub_gpu_uuids.py')
r=unittest.TextTestRunner(stream=io.StringIO()).run(suite)
print(json.dumps({'tests':r.testsRun,'killers':sorted(set(t.id() for t,_ in r.failures+r.errors))}))
"""


def run(directory):
    done = subprocess.run(
        [sys.executable, "-B", "-c", COLLECT],
        cwd=directory,
        capture_output=True,
        text=True,
        timeout=120,
    )
    if done.returncode:
        raise RuntimeError("scrub mutant harness failure")
    return json.loads(done.stdout.splitlines()[-1])


def main():
    source = (HERE / "scrub_gpu_uuids.py").read_text()
    rows = []
    with tempfile.TemporaryDirectory(prefix="kf-uuid-scrub-mutants-") as parent:
        directory = Path(parent) / "scripts/evidence"
        directory.mkdir(parents=True)
        ci = directory.parent / "ci"
        ci.mkdir()
        for name in ("no_gpu_uuids.py", "test_gpu_uuid_archives.py"):
            (ci / name).write_bytes((CI / name).read_bytes())
        (directory / "test_scrub_gpu_uuids.py").write_bytes(
            (HERE / "test_scrub_gpu_uuids.py").read_bytes()
        )
        target = directory / "scrub_gpu_uuids.py"
        target.write_text(source)
        baseline = run(directory)
        if baseline["killers"]:
            raise RuntimeError("baseline red")
        for name, old, new in MUTANTS:
            if source.count(old) != 1:
                raise RuntimeError("mutant anchor is not unique: " + name)
            target.write_text(source.replace(old, new))
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
