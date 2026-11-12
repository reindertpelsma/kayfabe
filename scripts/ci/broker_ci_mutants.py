#!/usr/bin/env python3
"""Mutation checks for the broker-in-CI plumbing (docs/design/V3_BROKER_IN_CI.md): every mutant needs >=2 killers.

Targets scripts/ci/broker_ci.py and the broker_* functions of scripts/ci/vast_payload.py, judged by
scripts/ci/test_broker_ci.py in an owned temporary copy of scripts/. Owner ruling 2026-11-12: a broker failure is a
FAIL; the mutants that turn one into a retry class, a provider fault, a skip or a pass are the point.
Falsifier: baseline red, an anchor absent or duplicated, or a mutant with fewer than two killing tests.

    python3 scripts/ci/broker_ci_mutants.py [substring-of-mutant-name]
"""

import re
import shutil
import subprocess
import sys
import tempfile
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
BC, VP = "scripts/ci/broker_ci.py", "scripts/ci/vast_payload.py"

# (name, file, old text (exactly once), new text)
MUTANTS = [
    ("small-tier-gets-the-broker", BC, 'return tier in ("middle", "large"), "tier"', 'return tier in ("small", "middle", "large"), "tier"'),
    ("large-tier-loses-the-broker", BC, 'return tier in ("middle", "large"), "tier"', 'return tier in ("middle",), "tier"'),
    ("override-ignored", BC, 'return value == "1", "env"', 'return tier in ("middle", "large"), "env"'),
    ("override-not-validated", BC, 'if value not in ("0", "1"):', 'if False:'),
    ("override-not-popped", BC, 'value = environ.pop("KF_BROKER", None)', 'value = environ.get("KF_BROKER")'),
    ("death-ignored", BC, '    if d["died"]:\n        return "broker_died"', '    if False:\n        return "broker_died"'),
    ("errors-ignored", BC, '    if d["errors"]:\n        return "broker_error"', '    if False:\n        return "broker_error"'),
    ("no-attach-ignored", BC, '    if d["accepted"] == 0 or d["attaches"] == 0:', '    if False:'),
    ("no-frames-ignored", BC, '    if d["commits"] == 0:', '    if False:'),
    ("late-attach-ignored", BC, 'd["first_attach_s"] > ATTACH_LIMIT_S', 'False'),
    ("leftover-ignored", BC, '    if not d["stopped"]:', '    if False:'),
    ("start-failure-ignored", BC, '    if not d["started"]:\n        return "broker_start"', '    if False:\n        return "broker_start"'),
    ("build-failure-ignored", BC, '    if not d["built"]:\n        return "broker_build"', '    if False:\n        return "broker_build"'),
    ("missing-summary-excused", BC, '    if d is None:\n        return "broker_start"', '    if d is None:\n        return None'),
    ("two-summaries-trusted", BC, "    if len(found) != 1:\n        return None\n    m = found[0]", "    if not found:\n        return None\n    m = found[-1]"),
    ("stage-rows-skip-instead-of-fail", BC, "    if cls is not None:\n        raise BrokerFailure(cls)\n    return [row((", "    if cls is not None:\n        return [row('broker_' + name, skipped=1)]\n    return [row(("),
    ("stage-rows-pass-regardless", BC, "    if cls is not None:\n        raise BrokerFailure(cls)\n    return [row((", "    return [row(("),
    ("class-renamed-retry", BC, '"broker_late_attach", ', '"broker_retry_late_attach", '),
    ("child-nonblocking-stdin", BC, "child_in = os.open(self.fifo, os.O_RDWR)", "child_in = os.open(self.fifo, os.O_RDWR | os.O_NONBLOCK)"),
    ("harness-write-can-block", BC, "self.fifo_fd = os.open(self.fifo, os.O_RDWR | os.O_NONBLOCK)", "self.fifo_fd = os.open(self.fifo, os.O_RDWR)"),
    ("broker-not-in-own-session", BC, "start_new_session=True, preexec_fn=_pdeathsig", "start_new_session=False, preexec_fn=None"),
    ("command-runs-without-broker", BC, "    if not session.start():\n        session.stop()\n        print(format_summary(name, session.summary()), file=out, flush=True)\n        return 1", "    session.start()"),
    ("exit-code-not-relayed", BC, "        rc = child.wait()\n    finally:", "        rc = child.wait()\n        rc = 0\n    finally:"),
    ("environment-not-passed", BC, "child = popen(command, env=child_environment(environ, session.sock))", "child = popen(command, env=dict(environ))"),
    ("broker-left-running", BC, "        session.stop()\n        print(format_summary(name, session.summary()), file=out, flush=True)\n    return rc", "        print(format_summary(name, session.summary()), file=out, flush=True)\n    return rc"),
    ("socket-dir-world-readable", BC, "os.chmod(self.dir, 0o700)", "os.chmod(self.dir, 0o755)"),
    ("kf3-uid-dropped", BC, '",display-broker-uid=" + str(int(uid))', '""'),
    ("blank-frame-accepted", BC, "    return sd < BLANK_SD", "    return False"),
    ("blank-threshold-too-high", BC, "BLANK_SD = 0.01", "BLANK_SD = 0.9"),
    ("first-frame-counter-not-required", BC, '["commits"] >= 1, frame_wait', '["commits"] >= 0, frame_wait'),
    ("resize-attach-not-required", BC, "    if not (arrived and restored):", "    if False:"),
    ("resize-restore-not-required", BC, "    if not (arrived and restored):", "    if not arrived:"),
    ("input-not-required", BC, "    if not wait_for(lambda: input_arrived(guest.logger_text()), input_wait, clock, sleep):", "    if False:"),
    ("pointer-tolerance-huge", BC, "INPUT_TOLERANCE_PX = 2", "INPUT_TOLERANCE_PX = 5000"),
    ("key-up-not-required", BC, '    return "up" in dirs[dirs.index("down"):]', "    return True"),
    ("explicit-no-line-passes", BC, "    if len(found) != 1:\n        raise BrokerFailure(\"broker_start\")\n    frames,", "    if len(found) < 1:\n        raise BrokerFailure(\"broker_start\")\n    frames,"),
    ("explicit-fail-passes", BC, '        if verdict != "PASS":\n            raise BrokerFailure(step, rows)', '        if verdict == "NOT_RUN":\n            raise BrokerFailure(step, rows)'),
    ("explicit-class-dropped", BC, "    if cls and cls in CLASSES and", "    if False and"),
    # --- the payload side: the ruling itself ---------------------------------------------------------
    ("payload-broker-failure-becomes-retry", VP, "            raise EvidenceError(failure.name, rows) from None\n\n    return guarded", '            raise EvidenceError(failure.name + "_retry_operator", rows) from None\n\n    return guarded'),
    ("payload-guest-path-becomes-retry", VP, "                raise EvidenceError(cls, [*guest.rows, row(guest.name, failed=1)]) from None", '                raise EvidenceError(cls + "_retry_operator", [*guest.rows, row(guest.name, failed=1)]) from None'),
    ("payload-broker-failure-is-provider-fault", VP, "            raise EvidenceError(failure.name, rows) from None\n\n    return guarded", "            raise ProviderFault(failure.name, rows) from None\n\n    return guarded"),
    ("payload-guest-path-is-provider-fault", VP, "                raise EvidenceError(cls, [*guest.rows, row(guest.name, failed=1)]) from None", "                raise ProviderFault(cls, [*guest.rows, row(guest.name, failed=1)]) from None"),
    ("payload-broker-failure-skipped", VP, "            return rows + broker_ci.stage_rows(text, name)\n        except broker_ci.BrokerFailure as failure:\n            raise EvidenceError(failure.name, rows) from None", "            return rows + broker_ci.stage_rows(text, name)\n        except broker_ci.BrokerFailure as failure:\n            return rows + [row(failure.name, skipped=1)]"),
    ("payload-guest-error-hides-broker", VP, "            if cls:\n                raise EvidenceError(cls,", "            if False:\n                raise EvidenceError(cls,"),
    ("payload-provider-fault-lost", VP, "        except ProviderFault:\n            raise\n        except EvidenceError as guest:", "        except EvidenceError as guest:"),
    ("payload-preboot-exemption-for-everything", VP, "and guest_name.endswith(broker_ci.PREBOOT_SUFFIXES):", "and True:"),
    ("payload-exempts-any-class", VP, 'if cls == "broker_no_attach" and', "if cls is not None and"),
    ("payload-timeout-converted", VP, "cls = None if rc == 124 else broker_class(text, name, guest.name)", "cls = broker_class(text, name, guest.name)"),
    ("payload-missing-output-is-infrastructure", VP, '            text = ""  # no output file means', '            raise  # no output file means'),
    ("payload-guest-stage-not-wrapped", VP, "            if broker and name in BROKER_GUEST_STAGES:", "            if False and name in BROKER_GUEST_STAGES:"),
    ("payload-merge-bar-wrapped", VP, '"windows_gate_2", "specviewperf")', '"windows_gate_2", "specviewperf", "merge_bar")'),
    ("payload-broker-always-on", VP, "    try:\n        return broker_ci.broker_wanted(environ, tier)", "    try:\n        return True, 'tier'"),
    ("payload-explicit-stage-dropped", VP, "        if windows and broker:\n            # The short explicit", "        if False:\n            # The short explicit"),
    ("payload-explicit-failure-is-retry", VP, "        raise EvidenceError(failure.name, failure.rows) from None", '        raise EvidenceError(failure.name + "_retry_operator", failure.rows) from None'),
]


def run_tests(tree):
    p = subprocess.run(
        [sys.executable, "-B", "-I", "-m", "unittest", "discover", "-s", str(tree / "scripts/ci"), "-p", "test_broker_ci.py"],
        capture_output=True, text=True, timeout=900, cwd=tree,
    )
    text = p.stdout + p.stderr
    killers = sorted({re.sub(r" \[.*", "", m.split(" (")[0]) for m in re.findall(r"^(?:FAIL|ERROR): (.*)$", text, re.M)})
    return p.returncode, killers, text


WORKERS = 4


def judge(args):
    tree, name, rel, old, new = args
    path = tree / rel
    original = path.read_text()
    if original.count(old) != 1:
        return name, None, "ANCHOR absent or duplicated (%d)" % original.count(old)
    path.write_text(original.replace(old, new))
    try:
        compile(path.read_text(), str(path), "exec")
        rc, killers, text = run_tests(tree)
    finally:
        path.write_text(original)
    return name, killers, ""


def main():
    only = sys.argv[1] if len(sys.argv) > 1 else ""
    from concurrent.futures import ThreadPoolExecutor
    from queue import Queue
    with tempfile.TemporaryDirectory() as t:
        trees = Queue()
        for i in range(WORKERS):
            tree = Path(t) / ("t%d" % i)
            shutil.copytree(ROOT / "scripts", tree / "scripts", ignore=shutil.ignore_patterns("__pycache__", "*.pyc", "node_modules"))
            trees.put(tree)
        tree = trees.get()
        rc, killers, text = run_tests(tree)
        trees.put(tree)
        if rc != 0:
            print("BASELINE RED\n" + text[-3000:])
            return 2

        def work(m):
            tree = trees.get()
            try:
                return judge((tree, *m))
            finally:
                trees.put(tree)

        chosen = [m for m in MUTANTS if only in m[0]]
        bad = 0
        with ThreadPoolExecutor(WORKERS) as pool:
            for name, killers, why in pool.map(work, chosen):
                if killers is None or len(killers) < 2:
                    bad += 1
                print("%-48s killers=%s %s %s" % (name, "?" if killers is None else len(killers),
                                                  "ok" if killers is not None and len(killers) >= 2 else "WEAK",
                                                  why or (killers if len(killers or []) < 2 else "")))
        print("MUTANTS %s" % ("FAILED (%d)" % bad if bad else "ALL KILLED BY >=2 TESTS (%d)" % len(chosen)))
        return 1 if bad else 0


if __name__ == "__main__":
    raise SystemExit(main())
