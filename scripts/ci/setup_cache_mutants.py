#!/usr/bin/env python3
"""Setup cache mutations: every changed boundary needs two named failing contracts."""

import json
import os
from pathlib import Path
import shutil
import subprocess
import sys
import tempfile

HERE = Path(__file__).resolve().parent
CASES = [
    (
        "cache-starts-apt-before-guard",
        "vast_setup.py",
        '    configure_vendor(cache / "vendor", Path.home() / ".cargo")',
        '    apt(cache, ["update"])\n    apt(cache, ["install", "python3"])\n    configure_vendor(cache / "vendor", Path.home() / ".cargo")',
        "guard_order",
    ),
    (
        "cache-bypasses-apt-shim",
        "vast_setup.py",
        '    configure_vendor(cache / "vendor", Path.home() / ".cargo")',
        '    subprocess.run(["/usr/bin/apt-get", "update"], check=True)\n    subprocess.run(["/usr/bin/apt-get", "install", "python3"], check=True)\n    configure_vendor(cache / "vendor", Path.home() / ".cargo")',
        "guard_order",
    ),
    (
        "legacy-unittest-headings-ignored",
        "vast_box_mutants.py",
        'r"(?:\\.(?P<inner>\\w+))?\\)',
        'r"(?:\\.(?P<inner>\\w+))\\)',
        "mutation_output",
    ),
    (
        "modern-unittest-headings-ignored",
        "vast_box_mutants.py",
        'match["inner"] is None or match["inner"] == match["method"]',
        'match["inner"] is None',
        "mutation_output",
    ),
    (
        "wrong-inner-method-counted",
        "vast_box_mutants.py",
        'match["inner"] is None or match["inner"] == match["method"]',
        "True",
        "mutation_output",
    ),
    ("blob-retry-disabled", "kf-box-blob-url", "--retry 8", "--retry 0", "blob_retry"),
    ("blob-401-not-retried", "kf-box-blob-url", "--retry-all-errors", "", "blob_retry"),
    (
        "blob-backoff-flattened",
        "kf-box-blob-url",
        "--retry 8",
        "--retry 8 --retry-delay 1",
        "blob_retry",
    ),
    (
        "blob-total-budget-raised",
        "kf-box-blob-url",
        "--signal=KILL 299s",
        "--signal=KILL 999s",
        "blob_retry",
    ),
    (
        "blob-total-deadline-removed",
        "kf-box-blob-url",
        "timeout --signal=KILL 299s",
        "",
        "blob_retry",
    ),
    (
        "permanent-progress-refusal-event",
        "vast_report.py",
        "self.progress_refused = True",
        "self.progress_refused = False",
        "readiness",
    ),
    (
        "restore-requires-ack-event",
        "vast_setup.py",
        "    wait_for_ready()\n    cache.mkdir",
        "    cache.mkdir",
        "readiness",
    ),
    (
        "skip-ack-event",
        "vast_setup.py",
        "    fd = None\n    try:",
        "    return\n    fd = None\n    try:",
        "readiness",
    ),
    (
        "refusal-is-not-ready",
        "vast_setup.py",
        'os.read(fd, 16) != b"READY\\n"',
        'os.read(fd, 16) not in (b"READY\\n", b"REFUSED\\n")',
        "readiness",
    ),
    (
        "progress-marked-before-ack",
        "vast_report.py",
        "self.progress_acked = False",
        "self.progress_acked = True",
        "readiness",
    ),
    (
        "refused-ack-sends-ready",
        "vast_report.py",
        'b"READY\\n" if reporter.progress_acked else b"REFUSED\\n"',
        'b"READY\\n"',
        "readiness",
    ),
    (
        "payload-missing-ready-channel",
        "vast_onstart.sh",
        '"KF_BROKER_READY_FD=$ready_fd"',
        '"KF_BROKER_READY_FD="',
        "readiness",
    ),
    (
        "provision-local-apt-hook",
        "vast_payload_hooks.sh",
        'if [ -n "${KF_APT_CACHE_ROOT:-}" ]; then',
        "if false; then",
        "apt_hook",
    ),
    (
        "image-binding",
        "vast_setup.py",
        'lock["schema"] != 1 or lock["image"] != TARGET_IMAGE',
        'lock["schema"] != 1',
        "cache",
    ),
    (
        "cargo-lock-binding",
        "vast_setup.py",
        'any(lock["locks"][name] != digest(root / name) for name in LOCK_PATHS)',
        "False",
        "cache",
    ),
    (
        "platform-binding",
        "vast_setup.py",
        'facts != PLATFORM or lock["platform"] != facts',
        "False",
        "cache",
    ),
    (
        "package-policy-binding",
        "vast_setup.py",
        'lock["package_policy_sha256"] != policy',
        "False",
        "cache",
    ),
    (
        "asset-hash",
        "vast_setup.py",
        'size != asset["size"] or h.hexdigest() != asset["sha256"]',
        'size != asset["size"]',
        "cache",
    ),
    (
        "locator-binding",
        "vast_setup.py",
        'locator["size"] != asset["size"] or locator["name"] != asset["name"]',
        "False",
        "cache",
    ),
    (
        "download-origin",
        "vast_setup.py",
        'url.scheme != "https"\n            or url.hostname not in GITHUB_HOSTS',
        "False",
        "cache",
    ),
    ("no-redirect", "vast_setup.py", "        return None\n", "        return newurl\n", "cache"),
    (
        "archive-paths",
        "vast_setup.py",
        'path.is_absolute()\n                    or ".." in path.parts\n                    or "\\\\" in member.name\n                    or path.parts[0] != kind',
        "False",
        "cache",
    ),
    ("archive-expansion", "vast_setup.py", "total > limit", "False", "cache"),
    (
        "preserve-existing-directory",
        "vast_setup.py",
        "        if created:\n",
        "        if True:\n",
        "cache",
    ),
    (
        "local-apt-sources",
        "vast_setup.py",
        '"Dir::Etc::sourceparts=-"',
        '"Dir::Etc::sourceparts=/etc/apt/sources.list.d"',
        "cache",
    ),
    (
        "no-recommendations",
        "vast_setup.py",
        '"APT::Install-Recommends=false"',
        '"APT::Install-Recommends=true"',
        "cache",
    ),
    (
        "cargo-source-override",
        "vast_setup.py",
        '(home / "config").exists() or ("[source" in before and not before.endswith(block))',
        "False",
        "cache",
    ),
    (
        "deb-index-binding",
        "setup_cache_build.py",
        'int(control["Size"]) != int(size)',
        "False",
        "build",
    ),
    (
        "deb-download-sha",
        "setup_cache_build.py",
        'size != row["size"] or h.hexdigest() != row["sha256"]',
        'size != row["size"]',
        "build",
    ),
    (
        "deb-epoch-filename",
        "setup_cache_build.py",
        'name = PurePosixPath(control["Filename"]).name',
        "name = name",
        "build",
    ),
    ("mirror-status", "setup_cache_build.py", 'row["status"] in (200, 206)', "True", "build"),
    (
        "install-blob-helper",
        "vast_onstart.sh",
        'install -m 700 "$here/kf-box-blob-url" "$bin_dir/kf-box-blob-url"',
        ":",
        "blob",
    ),
    (
        "blob-path-guard",
        "kf-box-blob-url",
        "''|.|..|*[!A-Za-z0-9._-]*) exit 2",
        "''|.|..) exit 2",
        "blob",
    ),
    (
        "blob-token-header",
        "kf-box-blob-url",
        "Authorization: Bearer %s",
        "Wrong-Header: %s",
        "blob",
    ),
    (
        "setup-before-work",
        "vast_payload.sh",
        'python3 -I "$here/vast_setup.py" restore "$(cd "$here/../.." && pwd)"',
        ":",
        "entry",
    ),
]
COLLECT = """import io,json,sys,unittest
s=unittest.defaultTestLoader.discover('scripts/ci',pattern='test_vast_setup_'+sys.argv[1]+'.py')
r=unittest.TextTestRunner(stream=io.StringIO()).run(s)
print(json.dumps({'tests':r.testsRun,'killers':[t.id() for t,_ in r.failures+r.errors]}))
"""


def run(root, suite):
    p = subprocess.run(
        [sys.executable, "-c", COLLECT, suite],
        cwd=root,
        capture_output=True,
        text=True,
        timeout=45,
        env={**os.environ, "PYTHONDONTWRITEBYTECODE": "1"},
    )
    assert p.returncode == 0, p.stderr[-300:]
    return json.loads(p.stdout.splitlines()[-1])


def main():
    rows = []
    with tempfile.TemporaryDirectory(prefix="kf-v8-cache-mutants-") as d:
        root = Path(d)
        shutil.copytree(HERE, root / "scripts/ci", ignore=shutil.ignore_patterns("__pycache__"))
        shutil.copytree(HERE.parent / "bench", root / "scripts/bench")
        for suite in sorted({c[4] for c in CASES}):
            assert not run(root, suite)["killers"], suite
        for name, filename, old, new, suite in CASES:
            path = root / "scripts/ci" / filename
            source = path.read_text()
            assert source.count(old) == 1, (name, source.count(old))
            altered = source.replace(old, new)
            if name == "deb-index-binding":
                altered = altered.replace(
                    'urllib.parse.unquote(parsed.path) != "/ubuntu/" + control["Filename"]',
                    "False",
                )
            path.write_text(altered)
            try:
                result = run(root, suite)
                killers = sorted(set(result["killers"]))
                assert len(killers) >= 2, (name, killers)
                rows.append({"mutant": name, "count": len(killers), "killers": killers})
            finally:
                path.write_text(source)
    print(json.dumps({"run": "v8-setup-cache-mutants-20261023", "cases": rows}, indent=2))


if __name__ == "__main__":
    main()
