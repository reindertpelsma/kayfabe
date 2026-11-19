#!/usr/bin/env python3
"""Mutation checks of kfml in the offline guest (2026-11-17): every mutant needs two named killers (stdlib, GPU-free).

Each mutant is one exact-once text replacement in a private copy of kfml_guest.py, kfml_stage.py or vast_report.py (the whole of
scripts/ci and scripts/kfci is copied, plus the harness files the stage imports); the named tests of test_kfml_guest.py run against
the copy and the failing ones are the killers. The unmutated copy must be green first.

    python3 scripts/ci/kfml_guest_mutants.py [--jobs N] [substring-of-mutant-name]
"""
import argparse
import concurrent.futures
import json
import os
from pathlib import Path
import shutil
import subprocess
import sys
import tempfile

HERE = Path(__file__).resolve().parent
ROOT = HERE.parents[1]
FILES = {
    "kg": "scripts/ci/kfml_guest.py",
    "ks": "scripts/ci/kfml_stage.py",
    "report": "scripts/ci/vast_report.py",
    "payload": "scripts/ci/vast_payload.py",
}
SOURCES = {key: (ROOT / path).read_text() for key, path in FILES.items()}
SUITE = "test_kfml_guest."
COPY_DIRS = ["scripts/ci", "scripts/kfci", "scripts/kfml"]
COPY_FILES = [
    "scripts/bench/native_xid_guard.py",
    "scripts/bench/gssh_nv",
    "scripts/bench/native_fault_names.json",
    "scripts/bench/specviewperf/lanes.json",
    "ci/vast-broker/planner/schemas/instruction.schema.json",
    "ci/vast-broker/release-matrix.json",
    "ci/vast-broker/defaults.json",
]
COLLECT = """import io, json, sys, unittest
suite = unittest.defaultTestLoader.loadTestsFromNames(sys.argv[1:])
r = unittest.TextTestRunner(stream=io.StringIO()).run(suite)
print('KF_MUTATION_RESULT=' + json.dumps({'tests': r.testsRun, 'killers': [t.id() for t, _ in r.failures + r.errors]}))
"""

F, FA, FC = (SUITE + n for n in ("Flow.", "Facts.", "Transport."))


def M(name, key, old, new, *tests):
    return (name, key, old, new, list(tests))


MUTANTS = [
    M("flavour-chosen-without-the-guest-facts", "kg",
      '"--flavor", "auto", "--flavor-for",\n           "%s:%d.%d" % (facts["driver_cuda"], facts["cc"][0], facts["cc"][1]), "--guest-python", facts["python"],\n           "--guest-glibc", facts["glibc"]]',
      '"--flavor", "auto", "--guest-python", facts["python"], "--guest-glibc", facts["glibc"]]',
      F + "test_the_flavour_comes_from_the_guests_facts_not_the_hosts", F + "test_wheelhouse_command_carries_the_guest_facts_of_the_default_fake"),
    M("wheelhouse-for-the-host-interpreter", "kg", '"--guest-python", facts["python"],\n           "--guest-glibc", facts["glibc"]]', '"--guest-glibc", facts["glibc"]]',
      F + "test_the_flavour_comes_from_the_guests_facts_not_the_hosts", F + "test_wheelhouse_command_carries_the_guest_facts_of_the_default_fake"),
    M("no-flavour-is-a-failure", "kg", "    if p.returncode == 3:", "    if False:",
      F + "test_no_flavour_fits_is_a_not_run_verdict_with_the_reason", F + "test_no_flavour_fits_still_says_which_guest_it_was"),
    M("gate-on-guest-facts-skipped", "kg", "    if probs:\n        return finish(", "    if False:\n        return finish(",
      F + "test_missing_guest_pieces_are_named_fails_before_any_download", F + "test_unreadable_nvidia_smi_output_stops_before_the_host_does_any_work"),
    M("disk-check-removed", "kg", 'if facts["disk_free_bytes"] is not None and facts["disk_free_bytes"] < need:', "if False:",
      F + "test_a_guest_disk_that_cannot_hold_it_is_refused_before_copying", F + "test_the_disk_need_scales_with_the_wheelhouse"),
    M("wheelhouse-not-copied", "kg", 'b = tr.put_tree(wh_dir, GUEST_ROOT + "/wheelhouse", STEP_BUDGET_S["copy"])', 'b = {"bytes": 0, "s": 0}',
      F + "test_green_flow_order_and_commands", F + "test_transfer_record_has_the_sizes_and_times"),
    M("manifest-verify-result-ignored", "kg", '    step("verify", rc, t0)\n    if rc != 0:', '    step("verify", rc, t0)\n    if False:',
      F + "test_a_copy_that_fails_the_manifest_check_never_reaches_pip", F + "test_the_verify_reason_carries_the_guests_output"),
    M("setup-not-offline", "kg", '--venv %s/venv --wheelhouse %s/wheelhouse --nvcc-if-needed 2>&1" % (GUEST_ROOT, GUEST_ROOT)',
      '--venv %s/venv --nvcc-if-needed 2>&1" % GUEST_ROOT',
      F + "test_green_flow_order_and_commands", F + "test_setup_is_an_offline_wheelhouse_install_of_the_hashed_lock"),
    M("setup-allows-unhashed", "kg", '--wheelhouse %s/wheelhouse --nvcc-if-needed 2>&1"', '--wheelhouse %s/wheelhouse --nvcc-if-needed --allow-unhashed 2>&1"',
      F + "test_setup_is_an_offline_wheelhouse_install_of_the_hashed_lock", F + "test_green_flow_order_and_commands"),
    M("flavour-mismatch-hidden", "kg", 'if "has no flavour" in out else', 'if False else',
      F + "test_a_guest_that_derives_another_flavour_is_a_named_fail", F + "test_the_mismatch_reason_names_what_the_host_shipped"),
    M("guest-no-flavour-is-a-failure", "kg", "    if rc == 3:\n        return finish(ks.not_run(", "    if False:\n        return finish(ks.not_run(",
      F + "test_guest_setup_exit_3_is_not_run", F + "test_guest_setup_exit_3_names_the_reason_and_validates"),
    M("data-from-the-network", "kg", 'fetch-data --from %s/wheelhouse/data 2>&1" % (py, GUEST_ROOT)', 'fetch-data 2>&1" % py',
      F + "test_guest_data_comes_from_the_wheelhouse_not_the_network", F + "test_green_flow_order_and_commands"),
    M("failed-offline-data-ignored", "kg", '    step("fetch-data", rc, t0)\n    if rc != 0:', '    step("fetch-data", rc, t0)\n    if False:',
      F + "test_failed_offline_data_is_a_fail_and_nothing_runs", F + "test_failed_offline_data_step_is_recorded_with_its_rc"),
    M("empty-rows-accepted", "kg", "or not isinstance(rows, list) or not rows:", "or not isinstance(rows, list):",
      F + "test_no_rows_is_a_fail", F + "test_the_no_rows_record_validates_and_says_why"),
    M("crashing-run-can-pass", "kg", 'if steps[-1]["rc"] not in (0, 1) and doc["verdict"] == "PASS":', "if False:",
      F + "test_a_crashing_run_cannot_pass", F + "test_a_run_that_timed_out_cannot_pass"),
    M("guest-kfml-run-not-in-guest-mode", "kg", "export PYTHONPATH=%s/src/kfml KF_ML_GUEST=1 KF_ML_RUN_ID", "export PYTHONPATH=%s/src/kfml KF_ML_RUN_ID",
      F + "test_every_kfml_command_in_the_guest_carries_guest_mode", F + "test_green_flow_order_and_commands"),
    M("transport-copy-failure-ignored", "kg", '        if tar.wait() != 0 or rc != 0:', '        if False:',
      FC + "test_put_tree_ssh_failure_alone_raises", FC + "test_put_tree_tar_failure_alone_raises"),
    M("validator-accepts-an-unknown-transfer-key", "report",
      'ok = ok and type(transfer) is dict and set(transfer) <= {"wheelhouse_bytes", "wheelhouse_build_s", "src_bytes", "copy_bytes", "copy_s", "install_s"}',
      'ok = ok and type(transfer) is dict',
      "test_kfml_guest.Main.test_the_guest_record_validates_in_the_reporter", "test_kfml_guest.Main.test_transfer_keys_are_an_allowlist"),
]


def tree(parent, name, overrides):
    root = Path(tempfile.mkdtemp(prefix=name + "-", dir=parent))
    for rel in COPY_DIRS:
        shutil.copytree(ROOT / rel, root / rel, ignore=shutil.ignore_patterns("__pycache__"))
    for rel in COPY_FILES:
        (root / rel).parent.mkdir(parents=True, exist_ok=True)
        shutil.copyfile(ROOT / rel, root / rel)
    for key, text in overrides.items():
        (root / FILES[key]).write_text(text)
    return root


def run_tests(root, names):
    done = subprocess.run([sys.executable, "-B", "-c", COLLECT, *names], cwd=root / "scripts/ci", capture_output=True, text=True,
                          timeout=1800, env={"PATH": os.environ["PATH"], "HOME": os.environ.get("HOME", "/tmp"), "PYTHONDONTWRITEBYTECODE": "1"})
    line = next((s for s in done.stdout.splitlines() if s.startswith("KF_MUTATION_RESULT=")), None)
    if line is None:
        raise SystemExit("harness failure: " + done.stderr[-600:] + done.stdout[-300:])
    return json.loads(line.split("=", 1)[1])


def one(parent, mutant):
    name, key, old, new, tests = mutant
    text = SOURCES[key]
    pairs = list(zip(old, new)) if isinstance(old, list) else [(old, new)]
    for anchor, _ in pairs:
        if text.count(anchor) != 1:
            return {"mutant": name, "count": 0, "killers": [], "error": "anchor x%d: %r" % (text.count(anchor), anchor[:60])}
    for anchor, replacement in pairs:
        text = text.replace(anchor, replacement)
    root = tree(parent, "m", {key: text})
    killers = sorted(set(run_tests(root, tests)["killers"]))
    shutil.rmtree(root, ignore_errors=True)
    print("%s: %d named killers" % (name, len(killers)), file=sys.stderr, flush=True)
    return {"mutant": name, "count": len(killers), "killers": killers}


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--jobs", type=int, default=2)
    parser.add_argument("only", nargs="?", default="")
    args = parser.parse_args()
    chosen = [m for m in MUTANTS if args.only in m[0]]
    with tempfile.TemporaryDirectory(prefix="kf-kfml-guest-mutants-") as parent:
        baseline = run_tests(tree(parent, "base", {}), [SUITE.rstrip(".")])
        if baseline["killers"]:
            print("BASELINE RED: %s" % baseline["killers"], file=sys.stderr)
            return 1
        print("baseline green: %d tests" % baseline["tests"], file=sys.stderr, flush=True)
        with concurrent.futures.ThreadPoolExecutor(args.jobs) as pool:
            rows = list(pool.map(lambda m: one(parent, m), chosen))
    bad = [r for r in rows if r["count"] < 2]
    print(json.dumps({"cases": rows}, indent=2))
    for row in bad:
        print("TOO FEW KILLERS: %(mutant)s (%(count)d) %(error)s" % dict(row, error=row.get("error", "")), file=sys.stderr)
    return 1 if bad else 0


if __name__ == "__main__":
    sys.exit(main())
