#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later
"""Mutation checks of the `matlab` payload stage (2026-11-17): every mutant needs two named killers (stdlib, GPU-free).

Each mutant is one exact-once text replacement in a private copy of matlab_stage.py, vast_report.py, vast_payload.py or payload_instruction.py (the whole of scripts/ci,
scripts/kfci and scripts/bench/linux is copied, plus the harness files the stage imports); the named tests of test_matlab_stage.py run against the copy and the failing
ones are the killers. The unmutated copy must be green first.

    python3 scripts/ci/matlab_stage_mutants.py [--jobs N] [substring-of-mutant-name]
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
FILES = {"ms": "scripts/ci/matlab_stage.py", "report": "scripts/ci/vast_report.py", "payload": "scripts/ci/vast_payload.py", "pinst": "scripts/ci/payload_instruction.py"}
SOURCES = {key: (ROOT / path).read_text() for key, path in FILES.items()}
SUITE = "test_matlab_stage."
COPY_DIRS = ["scripts/ci", "scripts/kfci", "scripts/bench/linux"]
COPY_FILES = [
    "scripts/bench/native_xid_guard.py", "scripts/bench/native_fault_names.json", "scripts/bench/specviewperf/lanes.json",
    "scripts/bench/windows/propimage.py", "scripts/bench/windows/propimage_check.py", "scripts/bench/windows/clocknet_assert.py", "scripts/bench/windows/clockrule.py",
    "scripts/bench/gpu_use.py", "scripts/bench/windows/gpu_assert.py", "scripts/bench/windows/kf3_census.py", "scripts/bench/windows/propimage_plain.py", "scripts/bench/windows/propimage_registry.py",
    "scripts/bench/windows/propimage_runspec.py", "scripts/bench/windows/app_defs/core_apps.py", "scripts/bench/windows/propimage_autounattend.xml.in",
    "scripts/bench/windows/kf3_flags.sample.env", "ci/vast-broker/planner/schemas/instruction.schema.json", "ci/vast-broker/release-matrix.json",
    "ci/vast-broker/defaults.json", "ci/vast-broker/propimages.json", "ci/vast-broker/test/fixtures/matlab/ad102-large-matlab.instruction.json",
]
COLLECT = """import io, json, sys, unittest
suite = unittest.defaultTestLoader.loadTestsFromNames(sys.argv[1:])
r = unittest.TextTestRunner(stream=io.StringIO()).run(suite)
print('KF_MUTATION_RESULT=' + json.dumps({'tests': r.testsRun, 'killers': [t.id() for t, _ in r.failures + r.errors]}))
"""
P, I, R, MN, PW, IS, RP, MA, SF, GB = (SUITE + n for n in ("PinsTests.", "InstructionTests.", "RunTests.", "ManifestTests.", "PayloadWiringTests.", "InstructionSchemaTests.",
                                                           "ReportTests.", "MainTests.", "StageFileTests.", "GuestBinTests."))


def M(name, key, old, new, *tests):
    return (name, key, old, new, list(tests))


MUTANTS = [
    M("image-sha-not-compared", "ms", 'if obj["image"]["sha256"] != item["sha256"]:', "if False:",
      I + "test_named_refusals", R + "test_a_changed_pin_is_a_strict_failure_with_every_row_not_run"),
    M("driver-pin-not-required", "ms", 'if driver_pin(obj["driver"]["version"], matrix) is None:', "if False:",
      I + "test_named_refusals", R + "test_an_unpinned_driver_in_the_instruction_refuses_the_whole_stage"),
    M("image-from-a-mirror", "ms", '"sources": [{"kind": "signed", "ref": image_item["ref"]}]}]', '"sources": [{"kind": "mirror", "url": "https://x.invalid/" + image_item["ref"]}]}]',
      MN + "test_manifest_is_valid_for_the_downloader_and_pins_both_objects", MN + "test_the_image_is_only_ever_a_signed_ref_never_a_mirror"),
    M("driver-created-time-dropped", "ms", 'driver_sha, driver_created = dpin["sha256"], dpin["checked"] + "T00:00:00Z"', "driver_sha, driver_created = dpin[\"sha256\"], None",
      R + "test_the_pinned_drivers_created_time_is_the_matrix_checked_date", R + "test_strict_run_fetches_the_pinned_objects_and_runs_the_listed_rows"),
    M("standalone-image-pin-enforced", "ms", "            image_sha_pin = None\n", "            image_sha_pin = image_sha\n",
      R + "test_standalone_uses_the_callers_image_without_any_pin_and_records_its_sha256", R + "test_standalone_never_enforces_a_pin_on_the_users_image"),
    M("image-left-on-the-box", "ms", "        if started:\n            pa.cleanup(pctx)", "        if False:\n            pa.cleanup(pctx)",
      R + "test_the_image_never_stays_on_the_box", R + "test_standalone_url_sources_are_streamed_and_hashed"),
    M("download-failure-is-a-skip", "ms", 'harness = r.reason in ("DOWNLOAD",) or r.reason.startswith("KERNEL_LOG")', "harness = False",
      R + "test_a_failed_download_is_a_retry_not_an_application_failure", R + "test_a_kernel_log_that_cannot_be_read_is_a_harness_problem"),
    M("instruction-lanes-ignored", "ms", 'lanes.update({r: v["lane"] for r, v in instruction["rows"].items()} if strict else {})', "pass",
      R + "test_lanes_and_isolation_come_from_the_instruction", R + "test_the_listed_gate_lane_decides_the_stage"),
    M("standalone-source-not-validated", "ms", '            if not Source(a.matlab_image).valid():\n                raise pa.Refused("IMAGE_SOURCE_BAD")', "            if False:\n                raise pa.Refused(\"IMAGE_SOURCE_BAD\")",
      R + "test_standalone_sources_must_be_absolute_paths_or_https", R + "test_a_relative_image_path_is_not_a_source"),
    M("gate-failure-allowed-under-pass", "report", ' or (r["verdict"] == "FAIL" and r["lane"] == "gate") for r in rows)', " for r in rows)",
      RP + "test_pass_needs_no_gate_failure", RP + "test_a_red_gate_record_cannot_be_relabelled_pass"),
    M("strict-skip-allowed-under-pass", "report", 'and not (value["strict"] and any(r["verdict"] == "NOT_RUN" for r in rows))', "",
      RP + "test_a_strict_pass_with_a_skipped_row_is_refused_and_a_permissive_one_is_not", RP + "test_a_strict_pass_that_skipped_the_preamble_is_refused"),
    M("row-names-not-checked", "report", '        ok = names == [n for n in MATLAB_ROWS if n in names] and len(set(names)) == len(names)', "        ok = True",
      RP + "test_row_names_must_be_known_and_in_table_order", RP + "test_shape_and_bounds"),
    M("breakages-not-compared", "report", 'ok = ok and type(value["breakages"]) is list and [b.get("row") if type(b) is dict else None for b in value["breakages"]] == [r["row"] for r in bad]',
      'ok = ok and type(value["breakages"]) is list',
      RP + "test_a_breakage_for_a_passing_row_is_refused", RP + "test_a_discovery_failure_is_fine_when_listed_as_a_breakage"),
    M("harness-row-allowed-under-pass", "report", 'not any(r["verdict"] == "HARNESS" or', "not any(False or",
      RP + "test_pass_needs_no_harness_row", RP + "test_a_retry_record_may_hold_harness_rows_but_a_pass_may_not"),
    M("stage-in-the-default-plan", "payload", "MATLAB_IN_LARGE = False", "MATLAB_IN_LARGE = True",
      PW + "test_off_by_default_and_a_manual_only_stage", PW + "test_the_stage_is_built_in_main_and_is_not_budgeted"),
    M("stage-planned-on-any-tier", "payload", 'if i.tier == "large" and (i.matlab is not None or MATLAB_IN_LARGE or "matlab" in only):', 'if i.matlab is not None or MATLAB_IN_LARGE or "matlab" in only:',
      PW + "test_selectable_by_only_on_the_large_tier_only", PW + "test_an_instruction_with_matlab_on_a_middle_tier_is_not_planned"),
    M("result-verdict-not-required", "payload", '    result["matlab"] = doc\n    if type(doc) is not dict or doc.get("sha") != sha or rc or doc.get("verdict") != "PASS":',
      '    result["matlab"] = doc\n    if type(doc) is not dict or doc.get("sha") != sha or rc:',
      PW + "test_matlab_rows_requires_a_pass_verdict_and_the_sha", PW + "test_matlab_rows_function_grades_the_record"),
    M("flags-beside-an-instruction-allowed", "payload", '    if instruction.matlab is not None and flags:\n        return "INSTRUCTION_AND_IMAGE_FLAG"', "    if False:\n        return \"INSTRUCTION_AND_IMAGE_FLAG\"",
      PW + "test_refusals_of_the_standalone_flags", PW + "test_dry_run_of_the_golden_and_of_the_flags"),
    M("pin_check_skipped_in_the_payload", "payload", "            return matlab_stage.instruction_refusal(instruction.matlab)", "            return None",
      PW + "test_the_instruction_object_is_checked_against_the_pins", PW + "test_dry_run_of_the_golden_and_of_the_flags"),
    # 2026-11-18: the optional `bin` of the object (the image's own matlab binary: the name `matlab` is not on the guest's ssh PATH)
    M("bin-not-refused-when-invalid", "ms", '    if "bin" in obj and not bin_valid(obj["bin"], mr):\n        return "BIN_INVALID"\n', "",
      I + "test_a_bad_bin_is_refused_by_name", R + "test_an_invalid_bin_refuses_the_whole_stage_by_name_before_anything_is_fetched"),
    M("bin-pattern-loses-its-characters", "ms", "return type(value) is str and mr.BIN_RE.fullmatch(value) is not None", "return type(value) is str",
      I + "test_the_bin_rule_is_the_rows_own_pattern_not_a_copy", I + "test_a_bad_bin_is_refused_by_name"),
    M("bin-accepts-a-non-string", "ms", "return type(value) is str and mr.BIN_RE.fullmatch(value) is not None", "return mr.BIN_RE.fullmatch(str(value)) is not None",
      I + "test_a_bad_bin_is_refused_by_name", I + "test_the_bin_rule_is_the_rows_own_pattern_not_a_copy"),
    M("instruction-bin-ignored", "ms", "    if wanted is not None:\n        return wanted\n", "",
      R + "test_the_instruction_bin_is_the_binary_the_rows_run", R + "test_another_bin_in_the_instruction_is_another_binary"),
    M("bin-conflict-allowed", "ms", "if wanted is not None and flag is not None and flag != wanted:", "if False:",
      R + "test_a_flag_that_differs_from_the_instruction_bin_is_a_usage_error_before_anything_runs", MA + "test_a_flag_that_differs_from_the_instruction_bin_is_a_named_usage_error"),
    M("bin-conflict-also-for-an-equal-flag", "ms", "if wanted is not None and flag is not None and flag != wanted:", "if wanted is not None and flag is not None:",
      R + "test_a_flag_equal_to_the_instruction_bin_is_not_a_conflict", GB + "test_the_rule_table"),
    M("flag-ignored", "ms", "    return flag if flag is not None else mr.MATLAB_BIN", "    return mr.MATLAB_BIN",
      R + "test_the_flag_alone_still_names_the_binary_of_an_instruction_without_bin", R + "test_standalone_runs_the_flag_binary_and_without_a_flag_the_name_matlab"),
    M("default-binary-is-not-the-rows-name", "ms", "    return flag if flag is not None else mr.MATLAB_BIN", '    return flag if flag is not None else "/usr/local/bin/matlab"',
      R + "test_an_instruction_without_bin_runs_the_name_matlab_as_before", R + "test_standalone_runs_the_flag_binary_and_without_a_flag_the_name_matlab"),
    M("flag-default-is-the-name-matlab", "ms", 'r.add_argument("--matlab", default=None,', 'r.add_argument("--matlab", default="matlab",',
      MA + "test_the_flag_is_not_given_by_default_and_is_passed_as_given", MA + "test_a_strict_command_line_without_the_flag_has_no_conflict"),
    M("bin-conflict-error-loses-its-name", "ms", 'raise ValueError("BIN_CONFLICT the instruction\'s bin and --matlab differ")', 'raise ValueError("the instruction\'s bin and --matlab differ")',
      GB + "test_both_and_different_is_a_value_error_naming_the_conflict", MA + "test_a_flag_that_differs_from_the_instruction_bin_is_a_named_usage_error"),
    M("bin-dropped-from-the-stage-file", "payload", "instruction_file.write_text(json.dumps(instruction.matlab))",
      'instruction_file.write_text(json.dumps({k: v for k, v in instruction.matlab.items() if k != "bin"}))',
      SF + "test_the_stage_gets_the_object_with_its_bin", SF + "test_the_stage_gets_another_bin_as_it_is"),
    M("bin-as-a-second-spelling-on-the-command-line", "payload", '                matlab_argv += ["--instruction-matlab", str(instruction_file)]\n',
      '                matlab_argv += ["--instruction-matlab", str(instruction_file), "--matlab", "matlab"]\n',
      SF + "test_the_stage_gets_the_object_with_its_bin", SF + "test_the_stage_gets_another_bin_as_it_is"),
    M("matlab-on-the-middle-tier", "pinst", '    if i.matlab is not None and i.tier != "large":\n        raise InstructionError("matlab needs the large tier")', "    pass",
      IS + "test_refusals", PW + "test_an_instruction_with_matlab_on_a_middle_tier_is_not_planned"),
    M("matlab-not-emitted", "pinst", '        if self.matlab is not None:\n            out["matlab"] = json.loads(json.dumps(self.matlab))\n', "",
      IS + "test_the_golden_is_accepted_and_round_trips", IS + "test_emit_instruction_writes_the_object_back_byte_for_byte"),
]


def tree(parent, name, overrides):
    root = Path(tempfile.mkdtemp(prefix=name + "-", dir=parent))
    for rel in COPY_DIRS:
        shutil.copytree(ROOT / rel, root / rel, ignore=shutil.ignore_patterns("__pycache__"))
    for rel in COPY_FILES:
        (root / rel).parent.mkdir(parents=True, exist_ok=True)
        shutil.copyfile(ROOT / rel, root / rel)
    shutil.copytree(ROOT / "scripts/bench/linux/matlab", root / "scripts/bench/linux/matlab", dirs_exist_ok=True)
    for key, text in overrides.items():
        (root / FILES[key]).write_text(text)
    return root


def run_tests(root, names):
    done = subprocess.run([sys.executable, "-B", "-c", COLLECT, *names], cwd=root / "scripts/ci", capture_output=True, text=True, timeout=1800,
                          env={"PATH": os.environ["PATH"], "HOME": os.environ.get("HOME", "/tmp"), "PYTHONDONTWRITEBYTECODE": "1"})
    line = next((s for s in done.stdout.splitlines() if s.startswith("KF_MUTATION_RESULT=")), None)
    if line is None:
        raise SystemExit("harness failure: " + done.stderr[-600:] + done.stdout[-300:])
    return json.loads(line.split("=", 1)[1])


def one(parent, mutant):
    name, key, old, new, tests = mutant
    text = SOURCES[key]
    if text.count(old) != 1:
        return {"mutant": name, "count": 0, "killers": [], "error": "anchor x%d: %r" % (text.count(old), old[:60])}
    root = tree(parent, "m", {key: text.replace(old, new)})
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
    with tempfile.TemporaryDirectory(prefix="kf-matlab-stage-mutants-") as parent:
        baseline = run_tests(tree(parent, "base", {}), [SUITE.rstrip(".")])
        if baseline["killers"]:
            print("BASELINE RED: %s" % baseline["killers"], file=sys.stderr)
            return 1
        print("baseline green: %d tests" % baseline["tests"], file=sys.stderr, flush=True)
        with concurrent.futures.ThreadPoolExecutor(args.jobs) as pool:
            rows = list(pool.map(lambda m: one(parent, m), chosen))
    bad = [r for r in rows if r["count"] < 2 and not (r["mutant"] in SINGLE and r["count"] >= 1)]
    print(json.dumps({"cases": rows}, indent=2))
    for row in bad:
        print("TOO FEW KILLERS: %(mutant)s (%(count)d) %(error)s" % dict(row, error=row.get("error", "")), file=sys.stderr)
    return 1 if bad else 0


SINGLE = set()      # a mutant with one named killer is never accepted: the rule is two

if __name__ == "__main__":
    sys.exit(main())
