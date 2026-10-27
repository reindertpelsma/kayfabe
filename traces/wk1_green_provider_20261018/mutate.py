#!/usr/bin/env python3
"""Run alone from repository root. Restore source and mtime even after a failed mutant."""
import json
import os
from pathlib import Path
import subprocess
import sys
import time

NODE = [
    "nice",
    "-n19",
    "node",
    "--test",
    "ci/vast-broker/test/green-provider.test.js",
    "ci/vast-broker/workerd/green-provider.test.js",
    "ci/vast-broker/workerd/green-budget-window.test.js",
    "ci/vast-broker/workerd/green-incident.test.js",
    "ci/vast-broker/test/green-attempt-budget.test.js",
]
COLLECT = """import io,json,unittest
s=unittest.defaultTestLoader.discover('scripts/ci',pattern='test_vast_payload*.py')
s.addTests(unittest.defaultTestLoader.discover('scripts/ci',pattern='test_vast_provider_fault.py'))
r=unittest.TextTestRunner(stream=io.StringIO()).run(s)
print('KF_PROVIDER_KILLS='+json.dumps({'tests':r.testsRun,
'killers':sorted(set(t.id() for t,_ in r.failures+r.errors))}))
"""
PYTHON = ["nice", "-n19", "python3", "-B", "-c", COLLECT]
L = "ci/vast-broker/src/ledger.js"
I = "ci/vast-broker/src/index.js"
R = "ci/vast-broker/runner.py"
P = "scripts/ci/vast_payload.py"
S = "scripts/ci/vast_report.py"
# Name, source, unique old/new anchor, test family.
CASES = [
    ("retry-once", L, "(prior.providerRetries ?? 0) !== 0 ||", "false ||", "node"),
    ("fresh-machine", L, "record.machineId === prior.machineId ||", "false ||", "node"),
    ("fixed-deadline", L, "record.hardDeadline !== prior.hardDeadline ||", "false ||", "node"),
    ("fixed-allowance", L, "record.runBudgetUsd !== prior.runBudgetUsd ||", "false ||", "node"),
    ("total-allowance", L, "spent + cost > prior.runBudgetUsd ||", "false ||", "node"),
    (
        "settled-proof",
        L,
        "const prior = await ledger.granted(record.key),\n"
        "          spent = prior ? rentalCost(prior, prior.ended) : NaN;",
        "const prior = await ledger.granted(record.key);\n"
        "      if (prior) prior.ended = previous?.ended;\n"
        "      const spent = prior ? rentalCost(prior, prior.ended) : NaN;",
        "node",
    ),
    ("prior-nonce", L, "prior.nonce !== retryNonce ||", "false ||", "node"),
    (
        "archive-spend",
        L,
        "await ledger.put('cost:' + record.key + ':provider:0', {\n"
        "          ...previous,\n          ...costAttempt(prior),\n          created: true,\n        });",
        "void previous;",
        "node",
    ),
    (
        "dollar-count-caps",
        L,
        "await ledger.budgets(cfg, now, cost, null, { credit, sameAttempt: retryNonce !== undefined });",
        "void cost;",
        "node",
    ),
    (
        "contract-allowance",
        I,
        "record.providerRetries === 1 && record.retryCostUsd + total > record.runBudgetUsd",
        "false",
        "node",
    ),
    ("box-nonce", I, "KF_NONCE: r.nonce,", "KF_NONCE: n,", "node"),
    (
        "run-allowance-reset",
        I,
        "r.runBudgetUsd =\n"
        "      retrySource?.runBudgetUsd ?? (profile === 'sweep' ? cfg.sweepDaily : cfg.daily);",
        "r.runBudgetUsd = cost;",
        "node",
    ),
    (
        "fresh-market",
        I,
        "const market = retrySource ? new OfferBook(now) : offers;",
        "const market = offers;",
        "node",
    ),
    (
        "boot-classification",
        I,
        "const providerFault =\n        reason === 'BOOT_TIMEOUT' &&",
        "const providerFault =\n        false &&",
        "node",
    ),
    (
        "runner-replacement",
        R,
        'if data["verdict"] == "PROVIDER_FAULT" or (',
        "if False and (",
        "node",
    ),
    (
        "runner-retries-again",
        R,
        'if not self.provider_retry and data.get("provider_retries", 0) == 0:',
        "if True:",
        "node",
    ),
    (
        "runner-budget-classification",
        R,
        'if self.provider_retry and self.metadata.get("verdict") == "PROVIDER_FAULT":',
        "if False:",
        "node",
    ),
    (
        "runner-second-header",
        R,
        'if self.rental_shown != data["id"] and data["id"] is not None:',
        'if not self.rental_shown and data["id"] is not None:',
        "node",
    ),
    ("runner-run-budget-reason", R, '        "run-usd",\n', "", "node"),
    (
        "terminal-rental-fault",
        R,
        "self.provider_retry\n"
        '                    and data["state"] == "NONE"\n'
        '                    and data["verdict"] == "UNAVAILABLE"',
        "False",
        "node",
    ),
    (
        "actual-rejected-spend",
        I,
        "        record.computeStoragePrice = computeStorage;\n"
        "        record.bandwidthCost = bandwidth;",
        "        record.bandwidthCost = bandwidth;",
        "node",
    ),
    ("replacement-busy-window", R, "                    and not self.provider_retry\n", "", "node"),
    (
        "retry-count-waiver",
        L,
        "if (!options.sameAttempt && values.count_today >= cfg.dailyCount)",
        "if (values.count_today >= cfg.dailyCount)",
        "node",
    ),
    ("attempt-dedup", L, "!counted[profile].has(attemptKey)", "true", "node"),
    ("different-host", I, "evaluation.allowed = otherHosts;", "void otherHosts;", "node"),
    ("missing-kvm", P, 'raise ProviderFault("provider_kvm_missing")', "return", "python"),
    ("unusable-gpu", P, 'return "provider_gpu_unavailable"', "return None", "python"),
    (
        "stage-classification",
        P,
        'verdict = "PROVIDER_FAULT" if isinstance(error, ProviderFault) else "FAIL"',
        'verdict = "FAIL"',
        "python",
    ),
    ("main-classification", P, "if isinstance(error, ProviderFault):", "if False:", "python"),
    (
        "unmatched-xid-stop",
        P,
        "if watch_delta(bounded_text(log), rc, failed_provider=True):",
        "if False:",
        "python",
    ),
    (
        "reporter-classification",
        S,
        '("PASS", "FAIL", "UNAVAILABLE", "PROVIDER_FAULT")',
        '("PASS", "FAIL", "UNAVAILABLE")',
        "python",
    ),
    ("attempt-original-day", L, "if (attemptStarted >= day &&", "if (v.started >= day &&", "node"),
    ("public-provider-attribution", R, ", not a kayfabe verdict\\n", ", provider fault\\n", "node"),
    ("failed-provider-wrapper", P, "failed_provider=True", "failed_provider=False", "python"),
    ("failed-provider-exit", P, "command_rc == rc", "True", "python"),
    ("failed-provider-scope", P, "failed_provider and command_rc", "command_rc", "python"),
    ("failed-provider-reserved-exit", P, "rc not in (0, 6, 7, 124)", "True", "python"),
    ("failed-provider-counts", P, "and before == after and", "and True and", "python"),
]


def run(family):
    r = subprocess.run(NODE if family == "node" else PYTHON, capture_output=True, text=True)
    if family == "node":
        return sum(line.startswith("not ok ") for line in r.stdout.splitlines()), r.returncode
    line = next(x for x in r.stdout.splitlines() if x.startswith("KF_PROVIDER_KILLS="))
    return len(json.loads(line.split("=", 1)[1])["killers"]), r.returncode


for name, filename, old, new, family in CASES:
    assert Path(filename).read_text().count(old) == 1, (name, "anchor preflight")
for family in ("node", "python"):
    kills, code = run(family)
    assert kills == 0 and code == 0, ("baseline", family, kills, code)
start = int(sys.argv[1]) if len(sys.argv) > 1 else 0
for i, (name, filename, old, new, family) in enumerate(CASES):
    if i < start:
        continue
    source = Path(filename)
    original = source.read_text()
    timestamp = source.stat().st_mtime_ns
    assert original.count(old) == 1, (name, original.count(old))
    try:
        source.write_text(original.replace(old, new))
        # Distinct source mtimes prevent stale Python bytecode from deciding a mutant result.
        stamp = time.time_ns() + (i + 1) * 2000000000
        os.utime(source, ns=(stamp, stamp))
        kills, code = run(family)
        print(json.dumps({"mutant": name, "kills": kills, "exit": code}), flush=True)
        assert kills >= 2, (name, kills)
    finally:
        source.write_text(original)
        os.utime(source, ns=(timestamp, timestamp))
