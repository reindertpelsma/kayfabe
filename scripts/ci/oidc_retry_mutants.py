#!/usr/bin/env python3
"""Mutation checks for the runner's OIDC token retry (2026-11-11): every mutant needs two killers.

Target: ci/vast-broker/runner.py `oidc_token`, driven through the real `main()` by
ci/vast-broker/test/oidc_retry_fixture.py (fake token endpoint, fake clock) and judged by
ci/vast-broker/test/oidc-retry.test.js (KF_RUNNER_PATH). Stdlib and node only; GPU-free.
Falsifier: baseline red, an anchor absent or duplicated, or a mutant with fewer than two killers.

    python3 scripts/ci/oidc_retry_mutants.py [substring-of-mutant-name]
"""

import json
import os
import re
import subprocess
import sys
import tempfile
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
BROKER = ROOT / "ci/vast-broker"
RUNNER = BROKER / "runner.py"

RETRY = "            retry = status in {408, 429} or 500 <= status <= 599\n"
STOP = "        if not retry or attempt >= OIDC_ATTEMPTS or left <= 0:\n"
AFTER = "            if status == 429:\n"
CATCH = "        except (OSError, http.client.HTTPException, ValueError) as cause:\n"
DELAY = (
    "        delay = min(max(OIDC_FIRST_DELAY if attempt == 1 else OIDC_STEP, wait or 0), left)\n"
)

# (name, old text (exactly once), new text)
MUTANTS = [
    # --- retry removed -----------------------------------------------------------------------------
    ("retry-removed-everywhere", STOP, "        if True:\n"),
    ("retry-removed-for-status", RETRY, "            retry = False\n"),
    (
        "retry-removed-for-network",
        '"network " + clean(type(cause).__name__), True, True\n',
        '"network " + clean(type(cause).__name__), False, True\n',
    ),
    (
        "retry-removed-for-body",
        'what, retry = "HTTP 200 body not accepted", True\n',
        'what, retry = "HTTP 200 body not accepted", False\n',
    ),
    ("retry-drops-5xx", RETRY, "            retry = status in {408, 429}\n"),
    ("retry-drops-429-and-408", RETRY, "            retry = 500 <= status <= 599\n"),
    # --- a refusal retried -------------------------------------------------------------------------
    ("4xx-retried", RETRY, "            retry = 400 <= status <= 599\n"),
    ("every-status-retried", RETRY, "            retry = True\n"),
    (
        "401-403-retried",
        RETRY,
        "            retry = status in {401, 403, 408, 429} or 500 <= status <= 599\n",
    ),
    (
        "404-retried",
        RETRY,
        "            retry = status in {404, 408, 429} or 500 <= status <= 599\n",
    ),
    (
        "300-retried",
        RETRY,
        "            retry = status in {300, 408, 429} or 500 <= status <= 599\n",
    ),
    (
        "304-retried",
        RETRY,
        "            retry = status in {304, 408, 429} or 500 <= status <= 599\n",
    ),
    (
        "3xx-retried",
        RETRY,
        "            retry = 300 <= status <= 308 or status in {408, 429} or status >= 500\n",
    ),
    ("redirect-retried", 'what, retry = "redirect", False\n', 'what, retry = "redirect", True\n'),
    # --- bound removed -----------------------------------------------------------------------------
    ("bound-removed", STOP, "        if not retry:\n"),
    ("attempt-cap-removed", STOP, "        if not retry or left <= 0:\n"),
    ("time-budget-removed", STOP, "        if not retry or attempt >= OIDC_ATTEMPTS:\n"),
    (
        "delay-not-clipped-to-the-budget",
        DELAY,
        "        delay = max(OIDC_FIRST_DELAY if attempt == 1 else OIDC_STEP, wait or 0)\n",
    ),
    ("retry-after-uncapped", "wait = min(int(after), OIDC_RETRY_AFTER)", "wait = int(after)"),
    ("budget-doubled", "20, 180, 2, 10, 60\n", "20, 360, 2, 10, 60\n"),
    ("attempts-doubled", "20, 180, 2, 10, 60\n", "40, 180, 2, 10, 60\n"),
    # --- the token check and the line --------------------------------------------------------------
    (
        "token-shape-unchecked",
        "                        and re.fullmatch(\n"
        '                            r"[A-Za-z0-9_-]+\\.[A-Za-z0-9_-]+\\.[A-Za-z0-9_-]+", data["value"]\n'
        "                        )\n",
        "",
    ),
    ("token-length-unchecked", '                        and len(data["value"]) <= 8192\n', ""),
    ("status-missing-from-the-line", 'what = "HTTP " + str(status)\n', 'what = "HTTP"\n'),
    ("attempt-missing-from-the-line", '                + ", "\n                + counter\n', ""),
    # --- Retry-After honoured outside a 429 ---------------------------------------------------------
    ("retry-after-honoured-on-503", AFTER, "            if status in {429, 503}:\n"),
    ("retry-after-honoured-on-5xx", AFTER, "            if status == 429 or status >= 500:\n"),
    ("retry-after-honoured-on-408", AFTER, "            if status in {408, 429}:\n"),
    ("retry-after-honoured-on-200", AFTER, "            if status in {200, 429}:\n"),
    ("retry-after-honoured-on-every-status", AFTER, "            if status != 0:\n"),
    # --- the network class --------------------------------------------------------------------------
    (
        "truncated-reply-fatal",
        CATCH,
        "        except (OSError, ValueError) as cause:\n",
    ),
    (
        "malformed-reply-fatal",
        CATCH,
        "        except (OSError, http.client.HTTPException) as cause:\n",
    ),
    ("every-exception-retried", CATCH, "        except Exception as cause:\n"),
    (
        "invalid-retried",
        CATCH,
        "        except (OSError, http.client.HTTPException, Invalid) as cause:\n",
    ),
    ("retry-after-ignored", "                    wait = min(", "                    _ = min("),
]


def killers(runner):
    env = dict(os.environ, KF_RUNNER_PATH=str(runner))
    p = subprocess.run(
        ["node", "--test", "--test-reporter=tap", "test/oidc-retry.test.js"],
        cwd=BROKER,
        env=env,
        capture_output=True,
        text=True,
        timeout=900,
    )
    return re.findall(r"^not ok \d+ - (.*)$", p.stdout, re.M)


def main():
    only = sys.argv[1] if len(sys.argv) > 1 else ""
    source = RUNNER.read_text()
    rows, bad = [], 0
    with tempfile.TemporaryDirectory(prefix="kf-oidc-retry-") as directory:
        work = Path(directory) / "base-runner.py"
        work.write_text(source)
        baseline = killers(work)
        if baseline:
            print("BASELINE RED: %s" % baseline, file=sys.stderr)
            return 1
        for name, old, new in MUTANTS:
            if only not in name:
                continue
            assert source.count(old) == 1, (name, source.count(old))
            work = Path(directory) / ("mut-" + name + "-runner.py")
            work.write_text(source.replace(old, new))
            found = killers(work)
            rows.append({"mutant": name, "killers": found, "count": len(found)})
            print("%s: %d killers" % (name, len(found)), file=sys.stderr, flush=True)
            bad += len(found) < 2
    print(json.dumps({"cases": rows}, indent=2))
    return 1 if bad else 0


if __name__ == "__main__":
    sys.exit(main())
