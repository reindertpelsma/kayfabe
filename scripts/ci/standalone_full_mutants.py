#!/usr/bin/env python3
"""Mutation checks of the standalone full run (2026-11-17): every mutant needs two named killers (stdlib, GPU-free).

Each mutant is one exact-once text replacement in a private copy of standalone_full.py, apps_discovery_stage.py, apps_gate.py, vast_payload.py, vast_report.py or matlab_stage.py
(the whole of scripts/ci and scripts/kfci is copied, plus the harness files the payload imports); the named tests of test_standalone_full.py run against the copy and
the failing ones are the killers. The unmutated copy must be green first.

    python3 scripts/ci/standalone_full_mutants.py [--jobs N] [substring-of-mutant-name]
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
    "sf": "scripts/ci/standalone_full.py",
    "ads": "scripts/ci/apps_discovery_stage.py",
    "payload": "scripts/ci/vast_payload.py",
    "report": "scripts/ci/vast_report.py",
    "matlab": "scripts/ci/matlab_stage.py",
    "gate": "scripts/kfci/apps_gate.py",
}
SOURCES = {key: (ROOT / path).read_text() for key, path in FILES.items()}
SUITE = "test_standalone_full."
COPY_DIRS = ["scripts/ci", "scripts/kfci", "scripts/kfml", "scripts/apps", "scripts/bench/linux", "scripts/bench/windows"]
COPY_FILES = [
    "scripts/bench/native_xid_guard.py",
    "scripts/bench/native_fault_names.json",
    "scripts/bench/specviewperf/lanes.json",
    "scripts/bench/windows/appmatrix/gate_profiles.json",
    "ci/vast-broker/planner/schemas/instruction.schema.json",
    "ci/vast-broker/release-matrix.json",
    "ci/vast-broker/defaults.json",
    "ci/vast-broker/src/results.js",
    "ci/vast-broker/test/fixtures/propimages/ad102-large-slot1-even.instruction.json",
]
COLLECT = """import io, json, sys, unittest
suite = unittest.defaultTestLoader.loadTestsFromNames(sys.argv[1:])
r = unittest.TextTestRunner(stream=io.StringIO()).run(suite)
print('KF_MUTATION_RESULT=' + json.dumps({'tests': r.testsRun, 'killers': [t.id() for t, _ in r.failures + r.errors]}))
"""

MF, RF, CL, MT, WO, AD, RV = (SUITE + n for n in ("ManifestTests.", "RefusalTests.", "CommandLineTests.", "MatlabInFullRunTests.", "WindowsOpenRowsTests.", "AppsDiscoveryTests.", "ReportValidatorTests."))


def M(name, key, old, new, *tests):
    return (name, key, old, new, list(tests))


MUTANTS = [
    M("propimages-requirement-dropped", "sf", '("propimage_apps", "propimages", lambda i, a: bool(i.propimages)),', '("propimage_apps", "propimages", lambda i, a: True),',
      RF + "test_each_missing_input_is_named_by_stage_and_input", CL + "test_each_missing_input_is_one_named_refusal_line_and_no_plan"),
    M("strict-not-required-of-images", "sf", '    if instruction.propimages and not instruction.propimages_strict:\n        out.append("propimage_apps:strict")\n', "",
      RF + "test_the_full_run_is_large_tier_and_strict"),
    M("tier-not-checked", "sf", '    if instruction.tier != "large":\n        out.append("tier:large")\n', "", RF + "test_the_full_run_is_large_tier_and_strict"),
    M("a-skip-is-not-recorded", "sf", '    skipped = [s for s in FULL_STAGES if s in skip]\n', "    skipped = []\n",
      RF + "test_a_skip_removes_the_check_and_is_recorded", CL + "test_skips_are_printed_and_the_run_says_it_is_not_complete"),
    M("a-run-with-a-skip-says-complete", "sf", '"complete": not skipped and [s for s in FULL_STAGES if s in planned] == list(FULL_STAGES),', '"complete": True,',
      RF + "test_a_skip_removes_the_check_and_is_recorded"),
    M("a-skipped-stage-is-still-checked", "sf", "        if stage in skip:\n            continue\n        if not ok(instruction, args):", "        if not ok(instruction, args):",
      RF + "test_a_skip_removes_the_check_and_is_recorded", CL + "test_skips_are_printed_and_the_run_says_it_is_not_complete"),
    M("only-allowed-with-full", "sf", '    if only:\n        return "full:with_only"\n', "", RF + "test_flag_combinations_that_would_make_a_different_run_are_refused", CL + "test_full_with_only_and_full_without_a_file_are_refused"),
    M("full-without-an-instruction-file", "sf", '    if not instruction_given:\n        return "full:needs_instruction_file"\n', "",
      RF + "test_flag_combinations_that_would_make_a_different_run_are_refused", CL + "test_full_with_only_and_full_without_a_file_are_refused"),
    M("a-prerequisite-can-be-skipped", "sf", '    base = [s for s in skip if s in ("provision", "merge_bar")]', "    base = []", RF + "test_flag_combinations_that_would_make_a_different_run_are_refused"),
    M("a-stage-can-outlive-what-it-needs", "sf", '    for stage, needs in NEEDS.items():\n        if needs in skip and stage not in skip:\n            return "skip:needs:%s:%s" % (stage, needs)\n', "",
      RF + "test_flag_combinations_that_would_make_a_different_run_are_refused"),
    M("offline-host-not-named", "sf", '    if online is False and "windows_open_rows" not in skip:\n        out.append("windows_open_rows:online_host")\n', "",
      RF + "test_an_offline_host_names_the_online_windows_rows"),
    M("matlab-image-not-required", "sf", '("matlab", "matlab_image", lambda i, a: i.matlab is not None or bool(getattr(a, "matlab_image", None))),', '("matlab", "matlab_image", lambda i, a: True),',
      RF + "test_matlab_needs_an_image_from_the_flag_or_the_instruction", CL + "test_a_full_dry_run_without_a_matlab_image_fails_by_name"),
    M("spec-lane-a-only-accepted", "sf", '"specviewperf" in i.stages and i.spec == "both"', '"specviewperf" in i.stages', RF + "test_each_missing_input_is_named_by_stage_and_input", CL + "test_each_missing_input_is_one_named_refusal_line_and_no_plan"),
    M("a-manifest-stage-leaves-the-table", "sf", '"windows_gate_2", "windows_open_rows",', '"windows_gate_2",',
      MF + "test_the_full_plan_is_the_manifest_in_the_manifest_order", MF + "test_the_result_row_bound_fits_one_full_run"),
    M("full-does-not-plan-the-manual-stages", "payload", "    only = (*only, *MANUAL_ONLY_STAGES) if full else tuple(only)", "    only = tuple(only)",
      MF + "test_the_full_plan_is_the_manifest_in_the_manifest_order", CL + "test_a_full_dry_run_with_every_input_prints_the_whole_manifest"),
    M("the-default-plan-gets-the-new-stages", "payload", '(APPS_DISCOVERY_IN_LARGE or "apps_discovery" in only)', '(True or "apps_discovery" in only)',
      MF + "test_the_default_plan_does_not_change", CL + "test_a_dry_run_without_full_plans_none_of_the_manual_stages"),
    M("a-new-stage-leaves-the-manual-table", "payload", '"apps_discovery": "APPS_DISCOVERY_IN_LARGE", ', "",
      MF + "test_every_manifest_stage_is_a_payload_stage_and_the_new_ones_are_manual_only"),
    M("a-small-tier-plans-the-open-windows-rows", "payload", 'if i.tier == "large" and (WINDOWS_OPEN_ROWS_IN_LARGE or "windows_open_rows" in only):', 'if (WINDOWS_OPEN_ROWS_IN_LARGE or "windows_open_rows" in only):',
      MF + "test_only_names_a_new_stage_and_a_small_tier_never_gets_one"),
    M("matlab-standalone-stays-permissive-in-a-full-run", "payload", '                if args.full:\n                    matlab_argv.append("--strict-rows")', '                if False:\n                    matlab_argv.append("--strict-rows")',
      MT + "test_the_full_run_asks_the_matlab_stage_for_zero_skips"),
    M("strict-rows-flag-not-defined-by-the-stage", "matlab", '    r.add_argument("--strict-rows", action="store_true", help=', '    r.add_argument("--strict-rows-x", action="store_true", help=',
      MT + "test_the_stage_has_the_flag_and_the_payload_refusal_accepts_it_with_full", MT + "test_the_stage_accepts_the_flag_on_its_command_line"),
    M("open-windows-rows-run-offline", "payload", '                    "WINVM_NET=online",\n', "", WO + "test_the_stage_command_is_online_and_names_the_gate_overlay"),
    M("an-unlisted-row-that-did-not-run-passes", "payload", '    if unexpected:\n        raise EvidenceError("windows_open_rows_not_run")\n', "",
      WO + "test_any_other_row_that_did_not_run_fails_by_name"),
    M("a-boot-failure-passes", "payload", 'if by_class.get("boot_fail") or len(apps) != expected', 'if len(apps) != expected', WO + "test_a_guest_that_did_not_boot_fails_the_stage"),
    M("a-failing-open-row-fails-the-stage", "payload", '    if by_class.get("boot_fail") or len(apps) != expected or rc not in (0, 1, 3) or exits[0] not in ("0", "1", "3"):\n        raise EvidenceError("windows_open_rows")\n',
      '    if by_class.get("boot_fail") or len(apps) != expected or rc not in (0, 1, 3) or exits[0] not in ("0", "1", "3"):\n        raise EvidenceError("windows_open_rows")\n    if by_class.get("guest_fail"):\n        raise EvidenceError("windows_open_rows")\n',
      WO + "test_a_row_that_ran_and_failed_is_a_listed_breakage_not_a_failure"),
    M("by-design-rows-not-named", "payload", '"not_run_by_design": sorted(n for n in not_ran if n in WINDOWS_OPEN_ROWS_NOT_RUN_BY_DESIGN)', '"not_run_by_design": []',
      WO + "test_only_the_three_by_design_rows_not_running_passes_and_is_named"),
    M("a-truncated-open-rows-log-passes", "payload", 'if len(grade) != 1 or len(exits) != 1 or not apps or len({a[0] for a in apps}) != len(apps):', 'if False:',
      WO + "test_a_truncated_log_and_a_duplicate_app_are_not_a_pass", WO + "test_a_retry_or_refusal_without_app_lines_is_a_retry_name"),
    M("discovery-not-run-passes-the-grader", "payload", '    if doc.get("not_run"):\n        raise EvidenceError("apps_discovery_not_run")\n', "",
      AD + "test_the_payload_grader_maps_the_record"),
    M("discovery-retry-is-a-pass", "payload", '    if doc.get("verdict") == "RETRY":\n        raise EvidenceError("apps_discovery_retry")\n', "",
      AD + "test_the_payload_grader_maps_the_record"),
    M("a-row-that-ran-and-failed-counts-as-not-run", "ads", '            status, reason = "KNOWN_BREAKAGE", verdict', '            status, reason = "NOT_RUN", verdict',
      AD + "test_ran_and_failed_is_a_breakage_and_not_ran_is_named", AD + "test_hang_and_a_dead_guest_are_findings_that_ran"),
    M("a-missing-row-is-a-pass", "ads", '            status, reason = "NOT_RUN", "no_guest_row"', '            status, reason = "PASS", ""',
      AD + "test_a_missing_row_is_not_run_never_a_pass", AD + "test_a_missing_guest_row_fails_the_stage_by_name"),
    M("not-run-rows-do-not-fail-a-strict-stage", "ads", '        return "FAIL" if strict else "PARTIAL"', '        return "PARTIAL"',
      AD + "test_ran_and_failed_is_a_breakage_and_not_ran_is_named", AD + "test_a_row_that_did_not_run_fails_the_stage_by_name"),
    M("the-guest-runs-before-the-host-control", "ads", "            if host_ok:\n                step(\"host\"", "            if False:\n                step(\"host\"",
      AD + "test_the_steps_run_in_order_and_the_host_runs_first_when_it_exists", AD + "test_a_stage_where_every_row_ran_passes_and_names_the_host_control"),
    M("no-bundle-is-not-a-retry", "ads", '        return finish("RETRY", "no_bundle")', '        return finish("PASS", "no_bundle")',
      AD + "test_no_bundle_is_a_retry_and_never_a_pass"),
    M("no-time-left-is-ignored", "ads", "    if deadline and now() + budget > deadline:", "    if False:", AD + "test_not_enough_time_is_a_retry_before_anything_starts"),
    M("unknown-verdict-word-is-a-result", "ads", '            status, reason = "NOT_RUN", "unknown_verdict_" + reason_of(verdict)', '            status, reason = "KNOWN_BREAKAGE", verdict',
      AD + "test_an_unknown_verdict_word_is_not_a_result"),
    # 2026-11-20 (first run on a box): the stage booted the base image, booted rows the host setup had refused, and graded unjudgeable rows as rows that ran
    M("the-stage-boots-the-image-itself-writable", "ads", 'APPS_PER_BOOT="1", KF_GUEST_IMG=str(overlay))', 'APPS_PER_BOOT="1", KF_GUEST_IMG=image)',
      AD + "test_every_step_gets_a_throwaway_overlay_and_never_an_image_itself", AD + "test_the_overlay_is_removed_when_the_stage_ends"),
    M("the-overlay-step-is-never-run", "ads", 'if step("overlay", ["qemu-img", "create"', 'if 0 and step("overlay", ["qemu-img", "create"',
      AD + "test_every_step_gets_a_throwaway_overlay_and_never_an_image_itself", AD + "test_the_overlay_is_backed_by_the_apps_image_the_gate_names"),
    M("the-overlay-is-never-removed", "ads", "    finally:\n        cleanup()\n", "    finally:\n        pass\n",
      AD + "test_the_overlay_is_removed_when_the_stage_ends", AD + "test_the_overlay_is_removed_when_a_step_raises"),
    M("the-overlay-is-not-removed-after-an-exception", "ads", "    finally:\n        cleanup()\n", "    except OSError:\n        cleanup()\n",
      AD + "test_the_overlay_is_removed_when_the_stage_ends", AD + "test_the_overlay_is_removed_when_a_step_raises"),
    M("a-foreign-kf-guest-img-is-ignored", "ads", "    if (requested and os.path.realpath(requested) != os.path.realpath(image)) or os.path.realpath(image) ==", "    if os.path.realpath(image) ==",
      AD + "test_a_kf_guest_img_that_names_the_base_image_is_refused_by_name_before_any_boot", AD + "test_any_other_image_in_kf_guest_img_is_refused_too"),
    M("an-apps-image-linked-to-the-base-is-accepted", "ads", "or os.path.realpath(image) == os.path.realpath(base_image(bench_env)):", ":",
      AD + "test_an_apps_image_that_is_a_link_to_the_base_image_is_refused", AD + "test_the_link_is_refused_even_when_kf_guest_img_names_the_link_itself"),
    M("a-missing-apps-image-boots-anyway", "ads", "    if not Path(image).is_file():\n", "    if False:\n",
      AD + "test_no_apps_image_is_a_retry_before_anything_boots", AD + "test_the_base_guest_alone_is_not_an_apps_image"),
    M("an-overlay-failure-boots-anyway", "ads", 'return finish("RETRY", "overlay_failed")', "pass",
      AD + "test_an_overlay_that_could_not_be_made_is_a_retry_and_nothing_boots", AD + "test_an_overlay_failure_is_a_retry_in_the_payload_grader"),
    M("the-driver-receipt-is-not-copied-to-the-overlay", "ads", '                shutil.copyfile(receipt, str(overlay) + ".STAGED")', "                pass",
      AD + "test_the_boot_helper_finds_the_driver_receipt_beside_the_overlay", AD + "test_the_receipt_beside_the_overlay_is_a_copy_of_the_images_own"),
    M("the-apps-image-is-the-base-image", "gate", "/ APPS_IMAGE_NAME)", "/ BASE_IMAGE_NAME)",
      AD + "test_the_apps_gate_names_the_image_in_one_place", AD + "test_every_step_gets_a_throwaway_overlay_and_never_an_image_itself"),
    M("refused-rows-are-booted", "ads", "        boot_rows = tuple(r for r in rows if r not in refused)", "        boot_rows = tuple(rows)",
      AD + "test_rows_the_host_setup_refused_are_not_booted_and_say_why", AD + "test_when_every_row_is_refused_nothing_boots_and_the_stage_fails_by_name"),
    M("a-refusal-loses-its-reason", "ads", '            out[row] = (re.sub(r"[^A-Za-z0-9_.:=<>-]+", "_", tidy).strip("_")[:80]) or "setup_refused"', '            out[row] = "setup_refused"',
      AD + "test_setup_refusals_are_read_with_their_real_reason", AD + "test_rows_the_host_setup_refused_are_not_booted_and_say_why"),
    M("a-failed-setup-counts-as-a-refusal", "ads", 'if status == "NOT_RUN" and row in rows and row in ML_ROWS:', 'if status in ("NOT_RUN", "FAIL") and row in rows and row in ML_ROWS:',
      AD + "test_only_a_refusal_is_a_refusal", AD + "test_a_setup_that_failed_is_not_a_refusal_the_guest_names_it"),
    M("all-rows-refused-is-a-retry", "ads", "        if boot_rows and not guest:", "        if not guest:",
      AD + "test_when_every_row_is_refused_nothing_boots_and_the_stage_fails_by_name", AD + "test_all_refused_is_a_failure_of_the_record_and_never_a_retry"),
    M("a-config-row-is-a-breakage", "ads", '        elif verdict == "FAIL" and note.startswith(CONFIG_NOTE):', "        elif False:",
      AD + "test_a_config_row_is_not_run_and_never_a_breakage", AD + "test_a_config_row_fails_the_stage_by_name"),
    M("a-hang-without-a-result-is-a-breakage", "ads", '        elif verdict == "HANG" and NO_RESULT_NOTE in note:', "        elif False:",
      AD + "test_a_hang_without_a_result_line_is_not_run_and_a_dead_guest_is_not", AD + "test_a_hang_without_a_result_line_fails_the_stage_by_name"),
    M("a-dead-guest-is-not-run", "ads", '        elif verdict == "HANG" and NO_RESULT_NOTE in note:', '        elif verdict in ("HANG", "GUEST_DEAD") and NO_RESULT_NOTE in note:',
      AD + "test_a_hang_without_a_result_line_is_not_run_and_a_dead_guest_is_not", AD + "test_a_dead_guest_is_a_listed_breakage_in_the_stage"),
    M("row-bound-back-to-nineteen", "report", "MAX_SUITE_ROWS = 32", "MAX_SUITE_ROWS = 19", MF + "test_the_result_row_bound_fits_one_full_run", RV + "test_the_reader_takes_twenty_five_rows_one_full_run_reports"),
    M("full-record-with-skip-says-complete-is-accepted", "report", 'ok = ok and (not value["complete"] or not value["skipped"])', "pass", RV + "test_full_record_is_accepted_and_bad_ones_are_not"),
    M("open-rows-record-with-a-not-run-row-is-accepted", "report", 'ok = ok and value["passed"] <= value["ran"] <= value["expected"]', "pass", RV + "test_windows_open_rows_record"),
    # 2026-11-20: the private Python 3.12 step of the stage and its record field
    M("the-python-step-never-runs", "ads", '            rc_py = step("python312", ["python3", "-B", "-I", str(apps / "mlrows/private_python.py"), "ensure", "--if-needed", "--root", ml_root], "python312")',
      "            rc_py = 0",
      AD + "test_the_private_python_is_ensured_before_the_ml_setup_and_both_get_the_same_root", AD + "test_the_record_says_what_the_private_python_did"),
    M("the-python-step-runs-after-the-setup", "ads",
      '            rc_py = step("python312", ["python3", "-B", "-I", str(apps / "mlrows/private_python.py"), "ensure", "--if-needed", "--root", ml_root], "python312")\n'
      '            try:\n'
      '                record["private_python"] = private_python_summary((log_dir / "python312.log").read_text(errors="replace")[-20000:], rc_py)\n'
      '            except OSError:\n'
      '                record["private_python"] = "FAIL:no_result_line_rc%d" % rc_py\n',
      '            step("setup_mlrows", ["python3", "-B", "-I", str(apps / "mlrows/mlrows.py"), "setup", "--root", ml_root, "--rows", ",".join(ml)], "setup_mlrows")\n'
      '            rc_py = step("python312", ["python3", "-B", "-I", str(apps / "mlrows/private_python.py"), "ensure", "--if-needed", "--root", ml_root], "python312")\n'
      '            try:\n'
      '                record["private_python"] = private_python_summary((log_dir / "python312.log").read_text(errors="replace")[-20000:], rc_py)\n'
      '            except OSError:\n'
      '                record["private_python"] = "FAIL:no_result_line_rc%d" % rc_py\n',
      AD + "test_the_private_python_is_ensured_before_the_ml_setup_and_both_get_the_same_root", AD + "test_the_steps_run_in_order_and_the_host_runs_first_when_it_exists"),
    M("the-python-step-is-not-conditional", "ads", '"ensure", "--if-needed", "--root", ml_root]', '"ensure", "--root", ml_root]',
      AD + "test_the_private_python_is_ensured_before_the_ml_setup_and_both_get_the_same_root", AD + "test_the_python_step_asks_only_when_needed_and_for_the_same_root_as_the_venvs"),
    M("the-python-step-uses-another-root", "ads", '"ensure", "--if-needed", "--root", ml_root]', '"ensure", "--if-needed", "--root", args.apps_root]',
      AD + "test_the_private_python_is_ensured_before_the_ml_setup_and_both_get_the_same_root", AD + "test_the_python_step_asks_only_when_needed_and_for_the_same_root_as_the_venvs"),
    M("a-failed-python-build-is-a-retry", "ads", '            try:\n                record["private_python"] = private_python_summary(',
      '            if rc_py:\n                return finish("RETRY", "python_build_failed")\n            try:\n                record["private_python"] = private_python_summary(',
      AD + "test_a_failed_python_build_is_not_a_retry_the_rows_say_python_too_old", AD + "test_the_record_says_what_the_private_python_did"),
    M("a-silent-python-step-is-ok", "ads", '        return "FAIL:no_result_line_rc%d" % rc\n', '        return "ok:no_result_line"\n',
      AD + "test_a_step_that_printed_nothing_is_a_failure_in_the_record_and_the_summary_is_bounded_and_printable", AD + "test_the_record_says_what_the_private_python_did"),
    M("the-python-summary-is-unbounded", "ads", '"%s:%s" % (status, reason))[:100]', '"%s:%s" % (status, reason))[:100000]',
      AD + "test_a_step_that_printed_nothing_is_a_failure_in_the_record_and_the_summary_is_bounded_and_printable", AD + "test_the_record_says_what_the_private_python_did"),
    M("the-plan-forgets-the-python-step", "ads", '        steps += ["python312", "setup_mlrows"]', '        steps += ["setup_mlrows"]',
      AD + "test_the_plan_names_the_build_and_the_setup_before_the_matrix", AD + "test_the_plan_is_the_list_of_steps_the_stage_really_runs"),
    M("the-validator-refuses-the-host-control-field", "report", '"guest_rc", "private_python", "nvcodec_host_control"}', '"guest_rc", "private_python"}',
      AD + "test_a_case_the_host_passes_and_the_guest_fails_is_a_guest_only_breakage", AD + "test_the_record_validator_takes_the_control_fields_and_refuses_anything_else"),
    M("the-validator-refuses-the-findings-of-a-row", "report", '"dcgm", "class", "findings"}', '"dcgm"}',
      AD + "test_a_case_the_host_passes_and_the_guest_fails_is_a_guest_only_breakage", AD + "test_the_record_validator_takes_the_control_fields_and_refuses_anything_else"),
    M("the-validator-refuses-the-private-python-field", "report", '"guest_rc", "private_python", "nvcodec_host_control"}', '"guest_rc", "nvcodec_host_control"}',
      AD + "test_the_record_says_what_the_private_python_did", AD + "test_the_record_validator_takes_a_short_printable_private_python_and_nothing_else"),
    # 2026-11-20: the host control of nvcodec_matrix (the bare host, before any guest boot) and the classification of a guest failure by the host
    M("host-pass-is-agreement", "ads", '        return "GUEST_ONLY", "host_pass"', '        return "AGREEMENT", "host_pass"',
      AD + "test_a_case_the_host_passes_and_the_guest_fails_is_a_guest_only_breakage", AD + "test_the_classification_of_one_case_is_a_pure_table"),
    M("host-failure-is-guest-only", "ads", '        return "AGREEMENT", "host_" + h.lower()', '        return "GUEST_ONLY", "host_" + h.lower()',
      AD + "test_a_case_that_fails_on_the_host_too_is_agreement_and_not_a_breakage", AD + "test_the_classification_of_one_case_is_a_pure_table"),
    M("host-not-run-is-guest-only", "ads", '        return "UNCLASSIFIED", "host_not_run:" + tidy(host.get("reason"))', '        return "GUEST_ONLY", "host_not_run:" + tidy(host.get("reason"))',
      AD + "test_a_guest_failure_the_host_can_not_judge_stays_a_plain_breakage_with_the_hosts_own_reason", AD + "test_the_classification_of_one_case_is_a_pure_table"),
    M("a-case-the-host_does_not_list-is-guest-only", "ads", '        return "UNCLASSIFIED", "host_case_missing"', '        return "GUEST_ONLY", "host_case_missing"',
      AD + "test_a_host_that_does_not_know_the_case_or_disagrees_in_kind_is_unclassified", AD + "test_the_classification_of_one_case_is_a_pure_table"),
    M("agreement-stays-a-breakage", "ads", '            if control["cls"] == "AGREEMENT":', "            if False:",
      AD + "test_a_case_that_fails_on_the_host_too_is_agreement_and_not_a_breakage", AD + "test_a_host_where_every_case_that_ran_failed_still_ran_and_the_guest_is_compared_with_it"),
    M("guest-only-has-no-named-reason", "ads", '            elif control["cls"] == "GUEST_ONLY":', "            elif False:",
      AD + "test_a_case_the_host_passes_and_the_guest_fails_is_a_guest_only_breakage", AD + "test_the_findings_are_bounded_and_the_record_stays_valid"),
    M("the-row-forgets-its-class-and-findings", "ads", '            entry["class"], entry["findings"] = control["cls"], control["findings"][:MAX_FINDINGS]', "            pass",
      AD + "test_a_case_the_host_passes_and_the_guest_fails_is_a_guest_only_breakage", AD + "test_a_case_that_fails_on_the_host_too_is_agreement_and_not_a_breakage"),
    M("the-findings-are-unbounded", "ads", 'control["findings"][:MAX_FINDINGS]', 'control["findings"]',
      AD + "test_the_findings_are_bounded_and_the_record_stays_valid", AD + "test_a_row_with_many_guest_only_failures_is_a_valid_record_for_the_payload"),
    M("the-control-needs-the-bundle", "ads", "            if NVCODEC in boot_rows:\n                # the HOST CONTROL", "            if NVCODEC in boot_rows and host_ok:\n                # the HOST CONTROL",
      AD + "test_the_control_needs_neither_the_apps_bundle_nor_a_probe_from_it", AD + "test_no_ffmpeg_on_the_host_is_a_named_not_run_of_the_control_and_the_row_still_runs"),
    M("a-guest-boots-before-the-control", "ads", "            if NVCODEC in boot_rows:\n                # the HOST CONTROL",
      '            guest_rc = step("guest", ["bash", str(apps / "apps_matrix.sh"), "guest", args.run_id, *boot_rows], "guest")\n            if NVCODEC in boot_rows:\n                # the HOST CONTROL',
      AD + "test_the_control_runs_after_the_overlay_and_before_any_guest_boot_and_before_the_host_matrix", AD + "test_the_steps_run_in_order_and_the_host_runs_first_when_it_exists"),
    M("the-control-sees-the-image", "ads", "                ctl_env = {k: v for k, v in env.items() if k not in CONTROL_ENV_DROPPED}", "                ctl_env = dict(env)",
      AD + "test_the_control_never_sees_an_image", AD + "test_the_control_needs_neither_the_apps_bundle_nor_a_probe_from_it"),
    M("the-control-sees-the-bench-dir", "ads", 'CONTROL_ENV_DROPPED = ("KF_GUEST_IMG", "BENCH_DIR")', 'CONTROL_ENV_DROPPED = ("KF_GUEST_IMG",)',
      AD + "test_the_control_never_sees_an_image", AD + "test_the_control_needs_neither_the_apps_bundle_nor_a_probe_from_it"),
    M("the-control-uses-the-bundles-probe", "ads", '"--probe", ctl_root + "/bin/nvcodec_probe"], "nvcodec_control", ctl_env)', '"--bundle", "/opt/apps/bundle"], "nvcodec_control", ctl_env)',
      AD + "test_the_control_needs_neither_the_apps_bundle_nor_a_probe_from_it", AD + "test_the_control_never_sees_an_image"),
    M("a-failed-provision-still-runs-the-matrix", "ads", "                if rc_p == 0 and not PROVISION_FAIL.search(provision_text):", "                if True:",
      AD + "test_no_ffmpeg_on_the_host_is_a_named_not_run_of_the_control_and_the_row_still_runs", AD + "test_a_probe_that_could_not_be_built_is_named_too"),
    M("a-host-that-attempted-nothing-is-a-control", "ads", '    if not any(r["result"] in HOST_RAN for r in rows):', "    if False:",
      AD + "test_a_host_without_nvenc_is_a_not_run_whose_reason_is_the_cases_own", AD + "test_a_host_that_not_supports_where_the_guest_agrees_is_no_finding_at_all"),
    M("the-hosts-failure-is-not-its-verdict", "ads", '    verdict = "FAIL" if any(r["result"] in GUEST_FAILED for r in rows) else "PASS"', '    verdict = "PASS"',
      AD + "test_a_case_that_fails_on_the_host_too_is_agreement_and_not_a_breakage", AD + "test_a_passing_guest_row_is_never_reclassified_by_the_host"),
    M("the-control-runs-for-every-row-set", "ads", "            if NVCODEC in boot_rows:\n                # the HOST CONTROL", "            if True:\n                # the HOST CONTROL",
      AD + "test_a_stage_without_the_nvcodec_row_runs_no_control_and_says_nothing_about_it", AD + "test_the_plan_is_the_list_of_steps_the_stage_really_runs"),
    M("the-plan-forgets-the-control", "ads", "        steps += list(CONTROL_STEPS)", "        pass",
      AD + "test_the_plan_names_the_build_and_the_setup_before_the_matrix", AD + "test_the_plan_is_the_list_of_steps_the_stage_really_runs"),
    M("the-per-case-table-is-not-kept", "ads", '                (result_dir / "nvcodec_host_vs_guest.tsv").write_text(table_tsv(control["table"]))', "                pass",
      AD + "test_the_hosts_verdict_per_case_is_written_next_to_the_guests", AD + "test_a_hostile_guests_huge_log_is_read_only_at_its_tail"),
    M("the-matrix-host-side-is-overruled", "ads", '        if not host_verdict and name == NVCODEC and control and control["status"] == "ran":', '        if name == NVCODEC and control and control["status"] == "ran":',
      AD + "test_the_apps_matrix_host_side_still_wins_the_host_column_when_it_ran", AD + "test_the_apps_matrix_host_side_wins_in_the_other_direction_too"),
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
    if text.count(old) != 1:
        return {"mutant": name, "count": 0, "killers": [], "error": "anchor x%d: %r" % (text.count(old), old[:60])}
    root = tree(parent, "m", {key: text.replace(old, new)})
    killers = sorted(set(run_tests(root, [SUITE.rstrip(".")])["killers"]))      # every test of the suite runs; `tests` says which ones are expected to be among the killers
    missing = [t for t in tests if not any(k.startswith(t) for k in killers)]
    if missing and len(killers) >= 2:
        print("%s: expected killer(s) did not fail: %s" % (name, missing), file=sys.stderr, flush=True)
    shutil.rmtree(root, ignore_errors=True)
    print("%s: %d named killers" % (name, len(killers)), file=sys.stderr, flush=True)
    return {"mutant": name, "count": len(killers), "killers": killers}


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--jobs", type=int, default=2)
    parser.add_argument("only", nargs="?", default="")
    args = parser.parse_args()
    chosen = [m for m in MUTANTS if args.only in m[0]]
    with tempfile.TemporaryDirectory(prefix="kf-full-mutants-") as parent:
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
