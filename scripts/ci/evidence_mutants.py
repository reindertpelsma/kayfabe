#!/usr/bin/env python3
"""Mutation checks of the box evidence bundle (vast_evidence.py): every mutant needs two killers.

Each mutant is one exact-once text replacement in a private copy of vast_evidence.py; the tests of
test_vast_evidence.py run against the copy and the failing test names are the killers. Stdlib only,
GPU-free. The Worker route has its own mutants (ci/vast-broker/evidence-mutants.mjs).

    python3 scripts/ci/evidence_mutants.py [--jobs N] [substring-of-mutant-name]
"""

import argparse
import concurrent.futures
import json
import os
import shutil
import subprocess
import sys
import tempfile
from pathlib import Path

HERE = Path(__file__).resolve().parent
ROOT = HERE.parents[1]
SOURCE = (HERE / "vast_evidence.py").read_text()

# (name, old text (exactly once), new text)
MUTANTS = [
    (
        "scan-of-data-skipped",
        "        scanner.scan(item.name, item.lead + item.data)\n",
        "        pass\n",
    ),
    (
        "lead-not-scanned",
        "        scanner.scan(item.name, item.lead + item.data)\n",
        "        scanner.scan(item.name, item.data)\n",
    ),
    (
        "size-cut-before-the-scan",
        "chosen, dropped = select(candidates, cap, shrink=False)",
        "chosen, dropped = select(candidates, cap, shrink=True)",
    ),
    ("fragments-off", "if label in FRAGMENT_LABELS and len(value) >= 2 * FRAGMENT:", "if False:"),
    (
        "name-hit-printed",
        '            raise ScanHit(hit.label, "name-" + digest) from None',
        "            raise",
    ),
    ("manifest-not-scanned", '        scanner.scan("MANIFEST.json", body)', "        pass"),
    (
        "skipped-names-raw",
        '            skipped.append(safe_name(prefix + "/" + name))',
        '            skipped.append(prefix + "/" + name)',
    ),
    ("jwt-pattern-gone", '    ("jwt", _jwt),', '    ("jwt", lambda data: False),'),
    (
        "bearer-needs-forty",
        'rb"(?i)\\bBearer[ \\t]+[A-Za-z0-9._~+/=-]{20,256}"',
        'rb"(?i)\\bBearer[ \\t]+[A-Za-z0-9._~+/=-]{40,256}"',
    ),
    (
        "github-token-pattern-gone",
        'rb"\\b(?:gh[pousr]_[A-Za-z0-9]{20,255}|github_pat_[A-Za-z0-9_]{20,255})"',
        'rb"\\bNEVER_GH_TOKEN"',
    ),
    (
        "prov-symlink-file-followed",
        "top = sorted(p for p in roots.prov.iterdir() if is_plain_file(p))",
        "top = sorted(p for p in roots.prov.iterdir() if p.is_file())",
    ),
    (
        "walk-follows-symlinked-files",
        "            elif is_plain_file(entry):",
        "            elif entry.is_file():",
    ),
    (
        "walk-follows-symlinked-dirs",
        "            if is_plain_dir(entry):\n                if level < depth:",
        "            if entry.is_dir():\n                if level < depth:",
    ),
    (
        "walk-root-symlink-followed",
        "    if not is_plain_dir(root):\n        return\n    stack",
        "    if not root.is_dir():\n        return\n    stack",
    ),
    (
        "vm-logs-symlink-followed",
        "if expired() or not is_plain_dir(vm) or not is_plain_dir(logs):",
        "if expired() or not logs.is_dir():",
    ),
    (
        "open-follows-symlinks",
        "os.O_RDONLY | os.O_NOFOLLOW | os.O_NONBLOCK | os.O_CLOEXEC",
        "os.O_RDONLY | os.O_NONBLOCK | os.O_CLOEXEC",
    ),
    (
        "open-accepts-any-file-type",
        '        if not stat.S_ISREG(os.fstat(fd).st_mode):\n            raise OSError("not a regular file")',
        "        pass",
    ),
    (
        "log-tail-takes-the-head",
        "        start = max(0, size - cap)\n        first",
        "        start = 0\n        first",
    ),
    (
        "tar-member-unbounded",
        "                yield name, data[start:], start, data[max(0, start - LEAD) : start]",
        '                yield name, data, 0, b""',
    ),
    (
        "command-not-in-its-own-group",
        "                start_new_session=True,\n",
        "",
    ),
    (
        "command-phase-ignored",
        "        if expired() or time.monotonic() > until:",
        "        if expired():",
    ),
    (
        "commands-before-files",
        "    items, skipped = [], []\n\n    def mtime(path):",
        "    skipped = []\n    items = host_items(runner, expired, skipped)\n\n    def mtime(path):",
    ),
    (
        "busy-409-not-retried",
        "        if status not in (None, 409, 429, 500, 502, 503, 504):",
        "        if status not in (None, 429, 500, 502, 503, 504):",
    ),
    (
        "any-409-is-stored",
        '        if status == 409 and state == "EVIDENCE_EXISTS":',
        "        if status == 409:",
    ),
    (
        "client-errors-retried",
        "        if status not in (None, 409, 429, 500, 502, 503, 504):",
        "        if False:",
    ),
    (
        "refusal-changes-exit-status",
        '            % (hit.label, hit.member if hit.member.startswith("name-") else safe_name(hit.member))\n        )\n        return 0',
        '            % (hit.label, hit.member if hit.member.startswith("name-") else safe_name(hit.member))\n        )\n        return 1',
    ),
    ("exact-minimum-too-long", "EXACT_MIN = 6", "EXACT_MIN = 60"),
    (
        "guest-secrets-not-read",
        'files += [p for p in root.glob("*/secrets/*") if p.is_file() and p.suffix != ".pub"]',
        "files += []",
    ),
    (
        "broker-path-not-a-secret",
        '            found.append(("broker_path", part))\n',
        "            pass\n",
    ),
    (
        "presigned-pattern-gone",
        'rb"(?i)X-Amz-(?:Signature|Credential|Security-Token)=|',
        'rb"(?i)NEVER-X-Amz=|',
    ),
    (
        "private-key-pattern-gone",
        'rb"-----BEGIN [A-Z0-9 ]{0,30}PRIVATE KEY-----"',
        'rb"-----NEVER PRIVATE KEY-----"',
    ),
    (
        "vast-key-pattern-gone",
        'rb"(?i)(?:instance_api_key|vast[_-]?api[_-]?key)',
        'rb"(?i)(?:never_instance_api_key)',
    ),
    ("uuid-scrub-off", "        data = _UUID.sub(self._uuid, data)\n", ""),
    (
        "uuid-all-equal",
        "n = self.uuids.setdefault(value, len(self.uuids) + 1)",
        "n = 1",
    ),
    (
        "serial-scrub-off",
        "        data = _SERIAL_LINE.sub(self._serial, data)\n        data = _SERIAL_INLINE.sub(self._serial, data)\n",
        "",
    ),
    ("mac-scrub-off", "        data = _MAC.sub(self._mac, data)\n", ""),
    ("ipv4-scrub-off", "        data = _IPV4.sub(self._v4, data)\n", ""),
    ("ipv6-scrub-off", "        return _IPV6.sub(self._v6, data)\n", "        return data\n"),
    (
        "private-ipv4-scrubbed-too",
        "if not ipaddress.IPv4Address(match.group(1).decode()).is_global:",
        "if False:",
    ),
    (
        "forbidden-directories-readable",
        'FORBIDDEN_PARTS = {"secrets", "tpm", "firmware", "disk", ".ssh", "unattend.d"}',
        "FORBIDDEN_PARTS = set()",
    ),
    (
        "forbidden-names-readable",
        '    return not FORBIDDEN_NAME.search(parts[-1] if parts else "")',
        "    return True",
    ),
    (
        "cap-never-binds",
        "    return raw <= RAW_CAP and sum(estimate(i) for i in items) + 8192 <= cap * FIT",
        "    return True",
    ),
    ("unpacked-cap-removed", "    return raw <= RAW_CAP and", "    return True and"),
    (
        "drops-the-highest-tier-first",
        "        low = max(kept, key=Item.key)\n        if low.tier >= 2:\n            kept.remove(low)",
        "        low = min(kept, key=Item.key)\n        if low.tier >= 2:\n            kept.remove(low)",
    ),
    (
        "tier-zero-droppable",
        "        if low.tier >= 2:\n            kept.remove(low)",
        "        if low.tier >= 0:\n            kept.remove(low)",
    ),
    (
        "log-never-halved",
        "        shrinkable = [i for i in kept if i.shrink and len(i.data) > 4096]",
        "        shrinkable = []",
    ),
    (
        "timeout-screendump-low-tier",
        "tier = 2 if path in timeouts else (3 if index < 3 else 5)",
        "tier = 5",
    ),
    ("screenshot-count-unbounded", "SHOT_COUNT, SHOT_MAX = 6,", "SHOT_COUNT, SHOT_MAX = 600,"),
    (
        "deadline-ignored",
        '        if left < 3:\n            return False, "time"',
        '        if False:\n            return False, "time"',
    ),
    ("environment-not-checked", "        if problems and not args.no_upload:", "        if False:"),
    ("budget-not-enforced", "    if budget < 10:", "    if False:"),
    (
        "name-not-sanitised",
        '        parts.append(re.sub(r"[^A-Za-z0-9._-]", "_", part)[:60])',
        "        parts.append(part)",
    ),
    ("success-line-without-hash", 'built["sha256"][:12],', '"",'),
    (
        "token-printed",
        '    say("KF_EVIDENCE_SKIPPED reason=%s" % reason)\n    return 0',
        '    say("KF_EVIDENCE_SKIPPED reason=%s %s" % (reason, env["KF_RUN_TOKEN"]))\n    return 0',
    ),
    (
        "redirects-followed",
        "    def redirect_request(self, *args, **kwargs):\n        return None",
        "    def redirect_request(self, req, fp, code, msg, headers, newurl):\n        return urllib.request.Request(newurl)",
    ),
    (
        "access-half-pair-sent",
        "            if ident and secret\n            else {}",
        "            if ident or secret\n            else {}",
    ),
    (
        "png-scrubbed-as-text",
        '        if is_text(item.data) and not item.name.endswith(".png"):',
        "        if True:",
    ),
    (
        "manifest-not-first",
        '[("MANIFEST.json", body)] + [(i.name, i.data) for i in items]',
        '[(i.name, i.data) for i in items] + [("MANIFEST.json", body)]',
    ),
]
COLLECT = """import io,json,sys,unittest
s=unittest.defaultTestLoader.discover('.',pattern='test_vast_evidence.py')
r=unittest.TextTestRunner(stream=io.StringIO()).run(s)
print(json.dumps({'tests':r.testsRun,'killers':[t.id() for t,_ in r.failures+r.errors]}))
"""


def run_tests(directory):
    done = subprocess.run(
        [sys.executable, "-c", COLLECT],
        cwd=directory,
        capture_output=True,
        text=True,
        timeout=600,
        env={"PATH": os.environ["PATH"], "HOME": os.environ.get("HOME", "/tmp")},
    )
    if done.returncode != 0:
        raise SystemExit("harness failure: " + done.stderr[-400:])
    return json.loads(done.stdout.splitlines()[-1])


def workdir(parent, name, source):
    root = Path(tempfile.mkdtemp(prefix=name + "-", dir=parent))
    ci = root / "scripts" / "ci"
    ci.mkdir(parents=True)
    for f in ("test_vast_evidence.py", "vast_report.py"):
        shutil.copyfile(HERE / f, ci / f)
    (ci / "vast_evidence.py").write_text(source)
    # the cross-language meta check imports ci/vast-broker/src/evidence.js
    (root / "ci").mkdir()
    (root / "ci" / "vast-broker").symlink_to(ROOT / "ci" / "vast-broker")
    return ci


def one(parent, mutant):
    name, old, new = mutant
    if SOURCE.count(old) != 1:
        return {
            "mutant": name,
            "count": 0,
            "killers": [],
            "error": "anchor x%d" % SOURCE.count(old),
        }
    result = run_tests(workdir(parent, "m", SOURCE.replace(old, new)))
    killers = sorted(set(result["killers"]))
    print("%s: %d killers" % (name, len(killers)), file=sys.stderr, flush=True)
    return {"mutant": name, "count": len(killers), "killers": killers}


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--jobs", type=int, default=4)
    parser.add_argument("only", nargs="?", default="")
    args = parser.parse_args()
    chosen = [m for m in MUTANTS if args.only in m[0]]
    with tempfile.TemporaryDirectory(prefix="kf-evidence-mutants-") as parent:
        baseline = run_tests(workdir(parent, "base", SOURCE))
        if baseline["killers"]:
            print("BASELINE RED: %s" % baseline["killers"], file=sys.stderr)
            return 1
        with concurrent.futures.ThreadPoolExecutor(args.jobs) as pool:
            rows = list(pool.map(lambda m: one(parent, m), chosen))
    bad = [r for r in rows if r["count"] < 2]
    print(json.dumps({"tests": baseline["tests"], "cases": rows}, indent=2))
    for row in bad:
        print("TOO FEW KILLERS: %(mutant)s (%(count)d)" % row, file=sys.stderr)
    return 1 if bad else 0


if __name__ == "__main__":
    sys.exit(main())
