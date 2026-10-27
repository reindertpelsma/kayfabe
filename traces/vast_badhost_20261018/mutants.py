#!/usr/bin/env python3
"""Run isolated provider/host mutants against the actual Node and workerd tests."""

import json
import os
from pathlib import Path
import re
import shutil
import subprocess
import tempfile
import time

ROOT = Path(__file__).resolve().parents[2]
BROKER = "ci/vast-broker/"
NODE = [BROKER + "test/badhost.test.js"]
RUNTIME = [BROKER + "workerd/green-provider.test.js"]
RUNNER = [BROKER + "test/green-provider.test.js"]
CLIENT = [BROKER + "test/client.test.js"]
CASES = []


def add(name, relative, old, new, tests=NODE, pattern="badhost"):
    path = BROKER + relative if not relative.startswith(".github/") else relative
    original = (ROOT / path).read_text()
    assert original.count(old) == 1, (name, old)
    CASES.append((name, path, original, original.replace(old, new), tests, pattern))


for name, expression in [
    ("kvm", r"/no \/dev\/kvm\b/i.test(details)"),
    ("initialization", r"driver && /RmInitAdapter failed|rm_init_adapter failed/.test(details)"),
    ("unusable", r"/GPU unusable/i.test(details)"),
    ("not-ready", r"/\bNOT_READY\b/.test(details) && unusable"),
]:
    add(
        "classification-" + name,
        "src/provider-fault.js",
        "if (" + expression + ")",
        "if (false && (" + expression + "))",
    )
add(
    "product-stage-admitted",
    "src/provider-fault.js",
    "!['cuda_host', 'merge_bar', 'cuda_guest'].every(",
    "false && !['cuda_host', 'merge_bar', 'cuda_guest'].every(",
)
add(
    "same-machine-condemns-host",
    "src/ledger.js",
    "k !== key &&",
    "true &&",
    RUNTIME,
    "repeated one-machine",
)
add(
    "host-learning-removed",
    "src/ledger.js",
    "if (!other) return;",
    "return;",
    NODE,
    "two distinct machines",
)
add(
    "machine-index-unbounded",
    "src/ledger.js",
    "if (!machines.has(key) && machines.size >= 257) return;",
    "if (false) return;",
    RUNTIME,
    "provider indexes stay bounded",
)
add(
    "host-index-unbounded",
    "src/ledger.js",
    "if (!hosts.has(hostKey) && hosts.size >= 257) return;",
    "if (false) return;",
    RUNTIME,
    "host index stays bounded",
)
add(
    "offer-host-index-ignored",
    "src/offers.js",
    "const badHosts = hosts.map(([k]) => Number(k.slice('bad-host:'.length)));",
    "const badHosts = [];",
    NODE,
    "dynamic host denies cached",
)
add(
    "offer-host-cap-ignored",
    "src/offers.js",
    "bad.length > 256 || hosts.length > 256",
    "bad.length > 256",
    NODE,
    "overflow refuses offers",
)
add(
    "offer-host-rejection-ignored",
    "src/policy.js",
    "|| badHosts.includes(offer.host_id)",
    "|| false",
    NODE,
    "dynamic host denies cached",
)
add(
    "reserve-host-cap-ignored",
    "src/ledger.js",
    "badHosts.size > 256 ||",
    "false ||",
    RUNTIME,
    "rejects bad-host-cap",
)
add(
    "reserve-host-index-ignored",
    "src/ledger.js",
    "badHosts.has('bad-host:' + record.hostId) ||",
    "false ||",
    RUNTIME,
    "rejects bad-host[12]",
)
add(
    "reserve-static-host-ignored",
    "src/ledger.js",
    "cfg.badHosts.includes(record.hostId) ||",
    "false ||",
    RUNTIME,
    "rejects configured-host",
)
add(
    "reserve-same-host-admitted",
    "src/ledger.js",
    "record.hostId === prior.hostId ||",
    "false ||",
    RUNTIME,
    "rejects same-host",
)
add(
    "reserve-unknown-host-admitted",
    "src/ledger.js",
    "!Number.isSafeInteger(record.hostId) ||\n          record.hostId <= 0 ||",
    "false ||",
    RUNTIME,
    "rejects unknown-host",
)
add(
    "reserve-unknown-prior-host-admitted",
    "src/ledger.js",
    "!Number.isSafeInteger(prior.hostId) ||\n          prior.hostId <= 0 ||",
    "false ||",
    RUNTIME,
    "rejects unknown-prior-host",
)
add(
    "retry-other-host-filter-removed",
    "src/index.js",
    "offer.host_id !== retrySource.hostId &&",
    "true &&",
    NODE,
    "retry requires a different known host",
)
add(
    "static-host-seed-removed",
    "defaults.json",
    '"badHosts": [224600]',
    '"badHosts": []',
    NODE,
    "static seed",
)
add(
    "runner-host-hidden",
    "runner.py",
    '+ " host "\n                        + shown_id(data["host_id"])',
    '+ ""',
    RUNNER,
    "BASE runner provider replacement",
)
add(
    "summary-notice-hidden",
    "runner.py",
    'self.summary.write("\\n" + provider_notice + "\\n")',
    "pass",
    RUNNER,
    "BASE runner provider replacement",
)
add(
    "provider-status-description-hidden",
    ".github/workflows/gpu-vast.yml",
    'description="GPU provider fault; no kayfabe verdict"',
    'description="GPU broker $state"',
    CLIENT,
    "provider exhaustion",
)


source = (ROOT / BROKER / "src/ledger.js").read_text()
start = source.index("  async markProviderFault(")
end = source.index("  async save(", start)
section = source[start:end]
without_transaction = section.replace(
    "await this.storage.transaction(async (storage) => {",
    "await (async (storage) => {",
).replace("\n    });\n  }", "\n    })(this.storage);\n  }")
add(
    "host-proof-not-atomic",
    "src/ledger.js",
    section,
    without_transaction,
    RUNTIME,
    "provider fault writes roll back",
)
add(
    "retry-unknown-host-filter-removed",
    "src/index.js",
    "providerId(offer.host_id) !== null &&",
    "true &&",
    NODE,
    "retry rejects unknown-host offer",
)


def run(work, files, pattern=None):
    command = ["node", "--test", "--test-concurrency=1"]
    if pattern:
        command += ["--test-name-pattern=" + pattern]
    return subprocess.run(command + files, cwd=work, capture_output=True, text=True, timeout=180)


def main():
    results = []
    with tempfile.TemporaryDirectory(prefix="wk1-badhost-mutants-") as directory:
        work = Path(directory)
        for relative in [BROKER + part for part in ["src", "test", "workerd", "node_modules"]]:
            shutil.copytree(
                ROOT / relative,
                work / relative,
                ignore=shutil.ignore_patterns("__pycache__", ".wrangler"),
            )
        for relative in [BROKER + name for name in ["runner.py", "package.json", "defaults.json"]]:
            shutil.copy2(ROOT / relative, work / relative)
        shutil.copytree(ROOT / ".github/workflows", work / ".github/workflows")
        shutil.copytree(
            ROOT / "scripts/ci", work / "scripts/ci", ignore=shutil.ignore_patterns("__pycache__")
        )
        (work / "scripts/bench").mkdir(parents=True)
        shutil.copy2(ROOT / "scripts/bench/native_xid_guard.py", work / "scripts/bench")
        baseline = run(work, NODE + RUNTIME + RUNNER + CLIENT)
        assert baseline.returncode == 0, baseline.stdout[-5000:] + baseline.stderr[-1000:]
        print("BASELINE PASS", flush=True)
        for index, (name, relative, original, mutant, files, pattern) in enumerate(CASES):
            target = work / relative
            try:
                target.write_text(mutant)
                # Each Python subprocess must see the current private-copy source.
                stamp = time.time() + index * 3
                os.utime(target, (stamp, stamp))
                process = run(work, files, pattern)
                kills = re.findall(
                    r"^not ok [0-9]+ - ((?:badhost|workerd|green|provider ).*)$",
                    process.stdout,
                    re.M,
                )
                assert process.returncode and len(kills) >= 2, (
                    name,
                    kills,
                    process.stdout[-4000:],
                    process.stderr[-1000:],
                )
                result = {"mutant": name, "kills": len(kills), "tests": kills}
                results.append(result)
                print(json.dumps(result), flush=True)
            finally:
                target.write_text(original)
                os.utime(target, (stamp + 1, stamp + 1))
    print(json.dumps({"baseline": "PASS", "mutants": results}, indent=2))


if __name__ == "__main__":
    main()
