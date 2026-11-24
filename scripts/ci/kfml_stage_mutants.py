#!/usr/bin/env python3
"""Mutation checks of the kfml stage (2026-11-16): every mutant needs two named killers (stdlib, GPU-free).

Each mutant is one exact-once text replacement in a private copy of kfml_stage.py, vast_report.py or vast_payload.py (the whole of
scripts/ci and scripts/kfci is copied, plus the harness files the stage imports); the named tests of test_kfml_stage.py run against
the copy and the failing ones are the killers. The unmutated copy must be green first.

    python3 scripts/ci/kfml_stage_mutants.py [--jobs N] [substring-of-mutant-name]
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
    "ks": "scripts/ci/kfml_stage.py",
    "report": "scripts/ci/vast_report.py",
    "payload": "scripts/ci/vast_payload.py",
    "codes": "scripts/ci/kfml_skip_codes.py",
    "errors": "scripts/kfml/kfml/errors.py",
}
SOURCES = {key: (ROOT / path).read_text() for key, path in FILES.items()}
SUITE = "test_kfml_stage."
COPY_DIRS = ["scripts/ci", "scripts/kfci", "scripts/kfml"]
COPY_FILES = [
    "scripts/bench/native_xid_guard.py",
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

S, PL, RU, RP, SM, SR = (SUITE + n for n in ("Summarize.", "Payload.", "Run.", "Report.", "StrictInMain.", "SkipRule."))


def M(name, key, old, new, *tests):
    return (name, key, old, new, list(tests))


MUTANTS = [
    M("stage-dropped-from-the-default-plan", "payload", "KFML_IN_MIDDLE_LARGE = True", "KFML_IN_MIDDLE_LARGE = False",
      PL + "test_in_the_default_plan_of_middle_and_large_only", PL + "test_dry_run_plans_it_by_default_and_skip_removes_it_and_touches_nothing"),
    M("stage-planned-in-the-small-tier", "payload", 'if i.tier in KFML_TIER and (KFML_IN_MIDDLE_LARGE or "kfml" in only):',
      'if (KFML_IN_MIDDLE_LARGE or "kfml" in only):',
      PL + "test_in_the_default_plan_of_middle_and_large_only", PL + "test_only_kfml_cannot_pull_it_into_small_or_adhoc"),
    M("stage-back-in-the-manual-only-table", "payload", 'MANUAL_ONLY_STAGES = {"propimage_apps": "PROPIMAGE_APPS_IN_LARGE", "propimage_apps_b": "PROPIMAGE_APPS_B_IN_LARGE", "matlab": "MATLAB_IN_LARGE", "apps_discovery": "APPS_DISCOVERY_IN_LARGE", "windows_open_rows": "WINDOWS_OPEN_ROWS_IN_LARGE", "install_flow": "INSTALL_FLOW_IN_LARGE"}',
      'MANUAL_ONLY_STAGES = {"propimage_apps": "PROPIMAGE_APPS_IN_LARGE", "propimage_apps_b": "PROPIMAGE_APPS_B_IN_LARGE", "matlab": "MATLAB_IN_LARGE", "apps_discovery": "APPS_DISCOVERY_IN_LARGE", "windows_open_rows": "WINDOWS_OPEN_ROWS_IN_LARGE", "install_flow": "INSTALL_FLOW_IN_LARGE", "kfml": "KFML_IN_MIDDLE_LARGE"}',
      PL + "test_it_is_not_manual_only_and_has_a_budget_row", PL + "test_the_live_budget_guard_accepts_kfml_as_a_budgeted_default_stage"),
    M("not-run-verdict-is-a-failing-stage", "payload", 'if rc == 0 and doc.get("verdict") == "NOT_RUN":', "if False:",
      PL + "test_not_run_with_rc0_is_a_skipped_row_never_a_pass_never_a_fail", PL + "test_stage_not_run_flows_through_to_a_skipped_row_and_a_valid_report"),
    M("not-run-counted-as-a-pass", "payload", 'return [row("kfml", skipped=1)]', 'return [row("kfml", 1)]',
      PL + "test_not_run_with_rc0_is_a_skipped_row_never_a_pass_never_a_fail", PL + "test_stage_not_run_flows_through_to_a_skipped_row_and_a_valid_report"),
    M("not-run-with-bad-exit-code-accepted", "payload", 'if rc == 0 and doc.get("verdict") == "NOT_RUN":', 'if doc.get("verdict") == "NOT_RUN":',
      PL + "test_not_run_line_with_a_bad_exit_code_is_still_a_failing_stage", PL + "test_not_run_line_from_a_killed_stage_is_a_failure"),
    # 2026-11-17 strict grading: under a strict instruction (propimages_strict) a row that did not run fails by name `kfml_not_run`
    M("strict-ignored-by-the-grader", "payload", '    result["kfml"] = doc\n', '    result["kfml"] = doc\n    strict = False\n',
      PL + "test_strict_a_not_run_row_fails_by_name_and_the_record_is_kept", PL + "test_strict_a_whole_stage_not_run_fails_by_name_and_the_record_is_kept",
      SM + "test_a_strict_instruction_fails_a_not_run_row_by_name_and_keeps_the_record"),
    M("strict-only-checks-the-whole-stage", "payload", '        if type(rows) is list and any(', '        if False and type(rows) is list and any(',
      PL + "test_strict_a_not_run_row_fails_by_name_and_the_record_is_kept", SM + "test_a_strict_instruction_fails_a_not_run_row_by_name_and_keeps_the_record"),
    M("strict-only-checks-the-rows", "payload", '        if strict:\n            raise EvidenceError("kfml_not_run")\n        return [row("kfml", skipped=1)]',
      '        return [row("kfml", skipped=1)]',
      PL + "test_strict_a_whole_stage_not_run_fails_by_name_and_the_record_is_kept", SM + "test_a_strict_instruction_fails_a_whole_stage_not_run_by_name_and_keeps_the_record"),
    M("strict-not-wired-to-the-instruction", "payload", "kfml_rows(bounded_text(output), rc, sha, result, strict=instruction.propimages_strict)",
      "kfml_rows(bounded_text(output), rc, sha, result)",
      SM + "test_a_strict_instruction_fails_a_not_run_row_by_name_and_keeps_the_record", SM + "test_a_strict_instruction_fails_a_whole_stage_not_run_by_name_and_keeps_the_record"),
    M("strict-on-for-every-instruction", "payload", "kfml_rows(bounded_text(output), rc, sha, result, strict=instruction.propimages_strict)",
      "kfml_rows(bounded_text(output), rc, sha, result, strict=True)",
      SM + "test_a_non_strict_instruction_still_passes_a_not_run_row", SM + "test_a_non_strict_instruction_still_skips_a_whole_stage_not_run"),
    M("strict-row-check-before-the-exit-code", "payload", '    if rc or doc.get("verdict") != "PASS":\n        raise EvidenceError("kfml")\n    rows = doc.get("rows")\n    if strict:',
      '    if (rc and not strict) or doc.get("verdict") != "PASS":\n        raise EvidenceError("kfml")\n    rows = doc.get("rows")\n    if strict:',
      PL + "test_strict_a_not_run_row_with_a_bad_exit_code_is_the_stage_failing_not_a_not_run", PL + "test_strict_a_not_run_row_from_a_killed_stage_is_the_stage_failing_not_a_not_run"),
    # skip rule: under a strict instruction a NOT_RUN row passes only with a code from the row runner's fixed vocabulary (kfml_skip_codes.SKIP_CODES)
    M("skip-rule-accepts-any-code", "payload", "and not kfml_skip_codes.permitted(r) for r in rows", 'and not r.get("skip_code") for r in rows',
      SR + "test_strict_an_unknown_code_fails_by_name", SR + "test_strict_a_code_outside_the_vocabulary_such_as_a_driver_too_old_fails_by_name",
      SM + "test_a_strict_instruction_fails_a_not_run_row_with_an_unknown_code_by_name"),
    M("skip-rule-accepts-a-missing-code", "payload", "and not kfml_skip_codes.permitted(r) for r in rows",
      'and not ("skip_code" not in r or kfml_skip_codes.permitted(r)) for r in rows',
      SR + "test_strict_a_row_without_a_code_fails_by_name", PL + "test_strict_a_not_run_row_fails_by_name_and_the_record_is_kept",
      SM + "test_a_strict_instruction_fails_a_not_run_row_by_name_and_keeps_the_record"),
    M("skip-rule-accepts-a-whole-stage-not-run-with-coded-rows", "payload", '        if strict:\n            raise EvidenceError("kfml_not_run")\n        return [row("kfml", skipped=1)]',
      '        import kfml_skip_codes\n        if strict and not all(kfml_skip_codes.permitted(r) for r in doc.get("rows") or [] if type(r) is dict):\n            raise EvidenceError("kfml_not_run")\n        return [row("kfml", skipped=1)]',
      SR + "test_strict_a_whole_stage_not_run_fails_by_name_even_when_every_row_carries_an_allowed_code",
      SM + "test_a_strict_instruction_fails_a_whole_stage_not_run_by_name_even_when_every_row_carries_an_allowed_code"),
    M("skip-rule-enumeration-widened", "errors", '    "DIE_LACKS_FEATURE",', '    "DIE_LACKS_FEATURE", "DRIVER_TOO_OLD",',
      SR + "test_the_vocabulary_is_the_row_runners_tuple_and_every_consumer_loads_it_from_there",
      SR + "test_strict_a_code_outside_the_vocabulary_such_as_a_driver_too_old_fails_by_name"),
    M("skip-rule-reads-the-reason-text-as-the-code", "codes", 'type(row.get("skip_code")) is str and row["skip_code"] in SKIP_CODES',
      'type(row.get("reason")) is str and row["reason"] in SKIP_CODES',
      SR + "test_strict_an_allowed_skip_passes_and_the_code_is_recorded_in_the_evidence", SR + "test_strict_an_allowed_code_with_any_free_text_reason_passes",
      SR + "test_strict_the_reason_text_is_never_read_as_a_code"),
    M("skip-rule-stage-keeps-any-code", "ks", "    if kfml_skip_codes.permitted(r):", '    if r.get("skip_code"):',
      SR + "test_summarize_drops_a_code_that_is_free_text_or_unknown", SR + "test_summarize_keeps_a_vocabulary_code_on_a_not_run_row_and_nowhere_else"),
    M("skip-rule-report-refuses-a-recorded-code", "report", '{"name", "status", "reason", "wall_s", "metrics"} <= set(r) <= {"name", "status", "reason", "wall_s", "metrics", "skip_code"}',
      'set(r) == {"name", "status", "reason", "wall_s", "metrics"}',
      SR + "test_the_report_validator_reads_a_code_on_a_not_run_row_and_refuses_the_rest", RP + "test_reads_a_record_with_a_coded_not_run_row"),
    M("result-for-another-sha-accepted", "payload", 'if type(doc) is not dict or doc.get("sha") != sha:\n        raise EvidenceError("kfml")',
      'if type(doc) is not dict:\n        raise EvidenceError("kfml")',
      PL + "test_a_result_for_another_sha_is_refused", PL + "test_rows_refuse_everything_else"),
    M("fail-row-leaves-the-verdict-pass", "ks", 'elif rows and "FAIL" not in statuses and "PASS" in statuses and all(s in ("PASS", "NOT_RUN") for s in statuses):', 'elif rows and "PASS" in statuses:',
      S + "test_pass_needs_one_pass_and_no_fail", RU + "test_a_fail_row_is_a_fail_verdict"),
    M("no-flavour-is-a-failed-setup", "ks", 'if s["rc"] == 3:', 'if s["rc"] == 99:',
      RU + "test_no_flavour_fits_is_a_not_run_verdict_with_the_named_reason", RU + "test_main_prints_one_result_line_and_exit_codes"),
    M("stage-pins-a-flavour", "ks", '[py, "-m", "kfml", "setup", "--venv", str(venv), "--nvcc-if-needed"]',
      '[py, "-m", "kfml", "setup", "--flavor", "cu128", "--venv", str(venv), "--nvcc-if-needed"]',
      RU + "test_green_run_does_setup_fetch_run_in_order_with_auto_flavour", S + "test_no_die_or_driver_is_named_in_the_stage"),
    M("secrets-reach-the-children", "ks", "            del env[k]", "            pass",
      RU + "test_provider_and_cloud_secrets_never_reach_the_children", RU + "test_generic_token_secret_password_key_names_never_reach_the_children"),
    M("uuid-scrub-removed-from-reasons", "ks", '    t = re.sub(r"(?i)\\b[0-9a-f]{8}(?:-[0-9a-f]{4}){3}-[0-9a-f]{12}\\b", "<uuid>", t)\n', "",
      S + "test_uuids_are_scrubbed", S + "test_failure_and_not_run_reasons_are_scrubbed_too"),
    M("failed-fetch-ignored", "ks", 'if done["fetch-data"]["rc"] != 0:', 'if False:',
      RU + "test_failed_setup_or_fetch_is_fail", RU + "test_main_prints_one_result_line_and_exit_codes"),
    M("crash-exit-code-can-pass", "ks", 'if s["rc"] not in (0, 1) and doc["verdict"] == "PASS":', 'if False:',
      RU + "test_crash_exit_code_cannot_pass", RU + "test_main_prints_one_result_line_and_exit_codes"),
    M("validator-lets-a-fail-row-sit-in-a-pass-verdict", "report", 'ok = "PASS" in status and "FAIL" not in status', 'ok = "PASS" in status',
      RP + "test_verdict_and_rows_must_agree", RP + "test_refuses_a_malformed_record"),
    M("validator-row-bound-removed", "report", 'ok = ok and type(rows) is list and len(rows) <= 8', 'ok = ok and type(rows) is list',
      RP + "test_row_and_step_counts_are_bounded", RP + "test_refuses_a_malformed_record"),
    M("report-accepts-pass-with-failed-kfml", "report", 'if doc["verdict"] == "PASS" and doc["kfml"]["verdict"] == "FAIL":', "if False:",
      RP + "test_a_passing_report_cannot_carry_a_failed_kfml", RP + "test_a_passing_report_cannot_carry_a_kfml_with_a_failing_row"),
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
    with tempfile.TemporaryDirectory(prefix="kf-kfml-mutants-") as parent:
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
