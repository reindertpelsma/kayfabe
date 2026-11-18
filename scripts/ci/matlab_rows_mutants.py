#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later
"""matlab_rows.py rule mutations, each requiring two independently named killers (the whole suite runs per mutant)."""
import argparse
import ast
import re
import shutil
import subprocess
import sys
import tempfile
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
REL = "scripts/bench/linux/matlab_rows.py"
SRC = (ROOT / REL).read_text()
COPIES = ("scripts/bench/linux/propimage_linux.py", "scripts/bench/linux/propimage_linux_pkgs.py", "scripts/bench/windows/propimage.py", "scripts/bench/windows/clockrule.py", "scripts/bench/windows/clocknet_assert.py",
          "scripts/bench/windows/gpu_assert.py", "scripts/bench/windows/kf3_census.py", "scripts/bench/linux/auth_stub_app.py", "scripts/bench/gpu_use.py")


def case(name, old, new, *tests, count=1):
    assert SRC.count(old) == count, (name, old, SRC.count(old))
    return name, SRC.replace(old, new), tests


CASES = [
    case("single-tolerance-loosened", 'TOLERANCE = {"single": 1e-4,', 'TOLERANCE = {"single": 1e-1,',
         "test_tolerance_miss_is_output_mismatch", "test_scripts_use_the_documented_tolerance_numbers"),
    case("exact-prefix-vecadd-dropped", 'EXACT_PREFIXES = ("vecadd_", ', 'EXACT_PREFIXES = (',
         "test_exact_checks_allow_no_difference", "test_exact_prefixes_cover_the_exact_checks_of_the_scripts"),
    case("fingerprint-ignored", "    ok = rel <= tol and fp <= tol\n", "    ok = rel <= tol\n",
         "test_fingerprint_disagreement_alone_fails", "test_first_and_last_elements_are_compared"),
    case("fingerprint-zero-scale", "scale = max(abs(x) for x in cpu) or 1.0", "scale = max(abs(x) for x in cpu) or 0.0",
         "test_an_all_zero_reference_fingerprint_uses_a_unit_scale", "test_zero_reference_uses_the_absolute_difference"),
    case("guest-tolerance-trusted", 'tol = tolerance_for(name, item.get("dtype"))\n    if None in (gpu', 'tol = num(item.get("tol"))\n    if None in (gpu',
         "test_the_same_miss_passes_in_double_only_below_its_own_tolerance", "test_guest_cannot_loosen_its_own_tolerance"),
    case("guest-flag-trusted", '        flag = raw.get("guest_pass")\n', '        flag = raw.get("guest_pass")\n        ok = flag if isinstance(flag, bool) else ok\n',
         "test_agreement_flag_is_recomputed_from_the_raw_numbers", "test_guest_cannot_loosen_its_own_tolerance"),
    case("count-trusts-guest-expectation", "    return value == 0.0, ", "    return value == expected, ",
         "test_counts_must_be_zero_whatever_the_guest_expected", "test_a_nonzero_mismatch_count_fails_the_cuda_row"),
    case("atleast-trusts-guest-minimum", '"atleast": host_atleast', '"atleast": check_atleast',
         "test_worker_minimums_come_from_the_host", "test_one_worker_fails_the_parpool_row"),
    case("residual-precision-from-guest", 'tolerance_for(item.get("name", ""), _dtype_of(item.get("name", "")))', 'tolerance_for(item.get("name", ""), item.get("dtype"))',
         "test_residual_precision_comes_from_the_name_not_the_guest", "test_good_documents_pass_for_every_row"),
    case("loss-fall-fraction", "LOSS_FALL = 0.5 ", "LOSS_FALL = 1.0 ",
         "test_loss_must_fall_on_both_devices", "test_a_loss_that_barely_falls_fails_and_the_pair_edges"),
    case("pair-tolerance-widened", "PAIR_REL, PAIR_ABS = 0.10, 2e-3", "PAIR_REL, PAIR_ABS = 10.0, 2e-3",
         "test_final_losses_must_agree", "test_a_loss_that_barely_falls_fails_and_the_pair_edges"),
    case("compute-capability-floor-dropped", "MIN_CC_X100 = 750 ", "MIN_CC_X100 = 0 ",
         "test_a_compute_capability_below_matlabs_floor_is_not_run", "test_the_preamble_gate_unit"),
    case("unreadable-counters-pass", 'if gpu is None or gpu.get("unreadable"):', 'if False:',
         "test_unreadable_counters_are_harness_never_pass", "test_missing_host_counters_make_the_row_harness_and_the_run_retry"),
    case("host-xid-ignored", '    if xid:\n        return "FAIL", "XID"', '    if False:\n        return "FAIL", "XID"',
         "test_host_xid_fails_a_row_whose_numbers_are_fine", "test_xids_in_the_rows_window_fail_it"),
    case("timeout-not-a-hang", '    if execution.get("timed_out"):\n        return "FAIL", "HANG"', '    if False:\n        return "FAIL", "HANG"',
         "test_timeout_is_a_hang_and_beats_the_numbers", "test_a_row_that_exceeds_its_timebox_is_a_hang"),
    case("strict-skip-allowed", "    if strict and skipped:", "    if False and skipped:",
         "test_strict_policy_fails_on_any_skip", "test_nothing_ran_is_skipped_and_still_exit_zero_and_strict_is_not"),
    case("gate-lane-ignored", 'r["lane"] == "gate" and r["verdict"] == "FAIL"]', 'False and r["verdict"] == "FAIL"]',
         "test_the_same_miss_in_the_gate_lane_fails_the_run", "test_gate_fail_beats_everything"),
    case("discovery-failure-fatal", 'r["lane"] == "gate" and r["verdict"] == "FAIL"]', 'r["verdict"] == "FAIL"]',
         "test_tolerance_miss_is_a_discovery_breakage_that_does_not_fail_the_run", "test_discovery_fail_is_listed_not_fatal"),
    case("default-lane-gate", 'lane = ctx.lanes.get(row, "discovery")', 'lane = ctx.lanes.get(row, "gate")',
         "test_all_rows_discovery_by_default", "test_tolerance_miss_is_a_discovery_breakage_that_does_not_fail_the_run"),
    case("inject-destination", 'GUEST_SCRIPTS = GUEST_ROOT + "/scripts"', 'GUEST_SCRIPTS = "/tmp/x"',
         "test_inject_needs_the_inject_verb", "test_remote_command_shape"),
    case("exit-line-forgeable", "    return int(m[-1]) if len(m) == 1 else None", "    return int(m[-1]) if m else None",
         "test_exit_code_parsing", "test_a_forged_second_exit_line_makes_the_guest_untrusted"),
    case("oversize-accepted", "    if len(raw) > MAX_JSON_BYTES:\n        raise BadResult(\"TOO_LARGE\")", "    if False:\n        raise BadResult(\"TOO_LARGE\")",
         "test_oversized_json_is_refused_before_parsing", "test_hostile_documents_at_run_level_name_their_reason"),
    case("nan-accepted", "parse_constant=_no_constant, ", "",
         "test_nan_infinity_and_duplicate_keys_are_refused", "test_hostile_documents_at_run_level_name_their_reason"),
    case("duplicate-keys-accepted", "object_pairs_hook=_no_dups)", "object_pairs_hook=None)",
         "test_nan_infinity_and_duplicate_keys_are_refused", "test_hostile_documents_at_run_level_name_their_reason"),
    case("evidence-keeps-raw-guest-fields", "json.dumps(sanitize_result(parsed), indent=1", "json.dumps(parsed, indent=1",
         "test_evidence_is_text_only_and_never_holds_raw_guest_bytes", "test_evidence_result_file_is_the_sanitized_form"),
    case("gpu-name-not-compared", 'if hw.get("gpu_name") and _norm_gpu_name', 'if False and _norm_gpu_name',
         "test_gpu_info_checks_the_host_facts", "test_a_wrong_gpu_name_fails_the_preamble_and_nothing_else_runs"),
    case("memory-fraction-bounds", "MEM_FRACTION = (0.70, 0.90)", "MEM_FRACTION = (0.0, 10.0)",
         "test_memory_row_bounds", "test_a_memory_row_that_allocated_too_little_fails"),
    case("driver-mismatch-is-harness", '("GUEST_DRIVER_MISMATCH", "NOT_RUN")', '("GUEST_DRIVER_MISMATCH", "HARNESS")',
         "test_refusal_mapping", "test_a_missing_driver_is_not_run_and_a_wrong_sha_is_harness"),
    case("boot-not-offline", 'net="offline", clock=clock, qemu=self.qemu', 'net="online", clock=clock, qemu=self.qemu',
         "test_boot_argv_is_offline_recorded_clock_with_the_kf3_device_and_the_runs_driver", "test_boot_refuses_an_identity_mismatch"),
    case("no-driver-clock-floor", "clock = pl.driver_clock(self.st, self.driver_run, self.created)", 'clock = pl.clock_for_mode(self.st, "run", "offline")',
         "test_the_boot_uses_the_driver_clock_floor", "test_boot_argv_is_offline_recorded_clock_with_the_kf3_device_and_the_runs_driver"),
    case("post-driver-identity-skipped", '            self.identity("post-driver")', "            pass",
         "test_the_driver_stage_re_checks_the_identity_so_the_gpu_uuid_must_match", "test_boot_and_driver_stage_each_run_the_identity_comparison"),
    case("one-shared-boot-for-row-isolation", 'return [[r] for r in ordered] if isolation == "row" else [ordered]', "return [ordered]",
         "test_each_row_boots_alone_installs_the_driver_then_injects_then_runs", "test_restricting_rows_keeps_the_preamble"),
    case("preamble-last", "    ordered = [PREAMBLE] + [r for r in rows if r != PREAMBLE]", "    ordered = [r for r in rows if r != PREAMBLE] + [PREAMBLE]",
         "test_each_row_boots_alone_installs_the_driver_then_injects_then_runs", "test_a_failed_preamble_stops_the_remaining_rows"),
    case("crash-keeps-the-boot", 'in ("CRASH", "HANG", "XID") and i + 1', 'in () and i + 1',
         "test_a_crash_in_a_shared_boot_sends_the_rest_to_a_fresh_boot", "test_a_hang_in_a_shared_boot_sends_the_rest_to_a_fresh_boot"),
    case("deadline-ignored", "    if ctx.deadline_epoch and ctx.now() + need", "    if False and ctx.now() + need",
         "test_deadline_gives_not_run_time_and_a_retry", "test_the_deadline_cuts_later_boots_only"),
    case("cuda-row-asserts-nothing", '"cuda_kernel": ("cuda_kernel.m", 900, ("gpu",),', '"cuda_kernel": ("cuda_kernel.m", 900, (),',
         "test_required_engines_per_row", "test_an_idle_gpu_fails_the_row_as_gpu_not_used"),
    case("harness-boot-failures-repeat", 'elif harness_refusals >= 2:', 'elif harness_refusals >= 99:',
         "test_a_harness_boot_failure_twice_stops_paying_for_boots", "test_repeated_harness_boots_end_in_harness_repeated"),
    case("user-pin-always-enforced", 'return "IMAGE_SHA_MISMATCH" if (strict and pin and image_sha != pin) else None',
         'return "IMAGE_SHA_MISMATCH" if (pin and image_sha != pin) else None',
         "test_user_supplied_pins_are_enforced_under_strict_only", "test_user_supplied_pins_are_enforced_under_strict_only_for_the_cli"),
    case("fetch-path-unrestricted", r'if not re.fullmatch(r"kfrows/out/[A-Za-z0-9_]{1,40}\.json", remote):', "if False:",
         "test_fetch_reads_only_the_rows_own_json_and_bounds_it", "test_fetch_path_whitelist_is_the_rows_output_directory"),
    case("shell-metacharacters-allowed", 'BIN_RE = re.compile(r"[A-Za-z0-9_./-]{1,120}")', 'BIN_RE = re.compile(r".*")',
         "test_nothing_unquoted_reaches_the_shell", "test_matlab_binary_option_is_validated"),
    # GPU use (owner requirement of 2026-11-17): gpuDevice present, a gpuArray operation, the host census window
    case("census-no-work-ignored", '    if census_verdict == "FAIL":\n        return "FAIL", "GPU_NOT_USED",', '    if False:\n        return "FAIL", "GPU_NOT_USED",',
         "test_the_host_census_saying_no_work_fails_a_row_whose_guest_numbers_and_engines_are_fine", "test_the_census_window_with_no_submissions_fails_the_row_end_to_end"),
    case("census-missing-passes", '    if census_verdict != "PASS":\n        return "HARNESS", None, "HOST_CENSUS_MISSING"', '    if False:\n        return "HARNESS", None, "HOST_CENSUS_MISSING"',
         "test_a_census_that_could_not_be_read_is_harness_never_a_pass", "test_a_missing_baseline_line_is_a_harness_matter_not_a_pass_and_not_a_failure"),
    case("census-window-not-passed-to-the-judge", "verified, gpu, census, xid, work)", "verified, gpu, census, xid, None)",
         "test_the_census_window_with_no_submissions_fails_the_row_end_to_end", "test_the_good_run_records_the_census_work"),
    case("baseline-line-not-read", "if qlog and edge0 else None\n    work = assess_work", "if False else None\n    work = assess_work",
         "test_the_census_window_with_no_submissions_fails_the_row_end_to_end", "test_a_guest_claiming_gpu_work_cannot_beat_a_host_that_saw_none_for_any_row"),
    case("on-gpu-flag-not-required", '        if raw["kind"] == "cmp" and not on_gpu:\n', '        if False:\n',
         "test_a_cpu_result_compared_with_the_cpu_result_agrees_numerically_and_still_fails", "test_one_comparison_on_the_cpu_among_gpu_ones_is_enough_to_fail"),
    case("gpuarray-item-always-ok", '    return on and exists, "on_gpu=%s exists_on_gpu=%s" % (on, exists)', '    return True, "on_gpu=%s exists_on_gpu=%s" % (on, exists)',
         "test_a_gpuarray_proof_item_that_says_cpu_fails_for_the_three_rows_that_have_one", "test_a_false_gpuarray_item_is_named_in_the_findings"),
    case("no-proof-item-needed", '    if not any(i.get("gpu_proof") for i in items):\n', '    if False:\n',
         "test_no_gpu_proof_item_at_all_is_a_failure_even_when_every_check_passes", "test_a_document_of_counts_only_has_no_gpu_proof_and_says_so"),
    case("gpu-device-block-ignored", "    found += verify_gpu_device(doc, hw)\n", "",
         "test_gpudevice_must_be_present_usable_and_the_hosts_gpu", "test_the_scripts_write_a_device_block_and_the_host_requires_it"),
    case("device-name-not-compared", '    if hw and hw.get("gpu_name") and _norm_gpu_name(hw["gpu_name"]) != _norm_gpu_name(name):\n        return [("GPU_NOT_USED"', '    if False:\n        return [("GPU_NOT_USED"',
         "test_gpudevice_must_be_present_usable_and_the_hosts_gpu", "test_gpu_not_used_outranks_a_numeric_miss_for_a_foreign_device"),
    case("numeric-miss-outranks-gpu-not-used", 'else "GPU_NOT_USED" if any(k == "GPU_NOT_USED" for k, _ in found)\n             else ("OUTPUT_MISMATCH" if found else None))', 'else ("OUTPUT_MISMATCH" if found else None))',
         "test_gpu_not_used_outranks_a_numeric_miss", "test_a_cpu_result_compared_with_the_cpu_result_agrees_numerically_and_still_fails"),
    # the breadth rows (docs/design/V3_PROPIMAGE_LINUX.md section 15)
    case("range-trusts-the-guests-bounds", 'value, bounds = num(item.get("value")), RANGES.get(item.get("name"))',
         'value, bounds = num(item.get("value")), ((num(item.get("min")), num(item.get("expected"))) if num(item.get("min")) is not None else RANGES.get(item.get("name")))',
         "test_the_guest_cannot_widen_its_own_range", "test_a_widened_range_at_run_level_still_fails_the_row"),
    case("range-table-widened", '"unif_mean_" + _g: (0.499, 0.501)', '"unif_mean_" + _g: (0.0, 1.0)',
         "test_a_range_statistic_outside_the_hosts_interval_fails_and_inside_passes", "test_the_host_range_table_matches_the_bounds_the_scripts_state"),
    case("range-without-entry-passes", '    if value is None or bounds is None:\n        return False, "malformed range"', '    if value is None or bounds is None:\n        return True, "malformed range"',
         "test_a_range_without_a_host_entry_is_malformed", "test_a_range_check_the_host_has_no_interval_for_fails_the_row_at_run_level"),
    case("range-kind-unregistered", '"gpuarray": check_gpuarray, "range": check_range}', '"gpuarray": check_gpuarray}',
         "test_a_range_statistic_outside_the_hosts_interval_fails_and_inside_passes", "test_all_new_rows_pass_through_the_runner_and_a_wrong_gpu_array_fails_each"),
    case("exact-prefix-dropped", '"content_mismatches", "exact_")', '"content_mismatches")',
         "test_an_exact_check_with_one_wrong_element_fails_whatever_the_precision_table_says", "test_exact_checks_use_a_zero_tolerance_in_the_script_and_the_host"),
    case("error-identifier-not-in-the-reason", '"value=%g (guest expected %g, host expects 0)%s" % (value, expected, tail)', '"value=%g (guest expected %g, host expects 0)" % (value, expected)',
         "test_a_step_that_threw_names_its_identifier_and_keeps_the_other_steps", "test_a_failed_step_names_its_identifier_in_the_rows_reason"),
    case("error-identifier-not-in-evidence", '"device_index", "device_name", "env_requested", "err_id")', '"device_index", "device_name", "env_requested")',
         "test_evidence_keeps_the_error_identifier_and_the_renderer_record_and_nothing_else_of_them", "test_the_evidence_file_of_a_failed_step_and_of_the_graphics_row_name_the_identifier_and_the_renderer"),
    case("renderer-block-not-in-evidence", '("memory", KEEP_MEMORY), ("renderer", KEEP_RENDERER)):', '("memory", KEEP_MEMORY)):',
         "test_evidence_keeps_the_error_identifier_and_the_renderer_record_and_nothing_else_of_them", "test_the_evidence_file_of_a_failed_step_and_of_the_graphics_row_name_the_identifier_and_the_renderer"),
    case("colour-minimum-lowered", '"gfx_distinct_colors": 64.0', '"gfx_distinct_colors": 1.0',
         "test_the_host_minimums_of_the_new_atleast_checks_come_from_its_own_table", "test_gfx_colour_minimum_is_sixty_four"),
    case("rows-lost-from-the-table", '    "gfx_render": ("gfx_render.m", 1200,', '    "gfx_render_gone": ("gfx_render.m", 1200,',
         "test_the_rows_are_in_the_table_in_order_after_the_first_eight", "test_every_row_opens_with_its_falsifier_and_every_step_reports_a_status_or_is_the_whole_row"),
    case("mex-source-not-injected", '"kf_dl_predict.m", "kf_mex.cu")', '"kf_dl_predict.m")',
         "test_every_row_has_its_script_and_no_stray_files", "test_the_cuda_source_the_mex_row_compiles_is_listed_for_injection_and_copied_by_the_script"),
]


def run(tree):
    proc = subprocess.run([sys.executable, "-I", "-B", str(tree / "scripts/ci/test_matlab_rows.py")], text=True, capture_output=True, timeout=900)
    out = proc.stdout + proc.stderr
    return proc.returncode, sorted(set(re.findall(r"^(?:FAIL|ERROR): (\S+)", out, re.M))), out


def main():
    argparse.ArgumentParser(description=__doc__).parse_args()
    test_src = ROOT / "scripts/ci/test_matlab_rows.py"
    known = {n.name for n in ast.walk(ast.parse(test_src.read_text())) if isinstance(n, ast.FunctionDef)}
    missing = {t for c in CASES for t in c[2]} - known
    assert not missing, "unknown test selector: %s" % missing
    weak = 0
    with tempfile.TemporaryDirectory(prefix="matlab-rows-mutants-") as tmp:
        tree = Path(tmp)
        for rel in COPIES:
            (tree / rel).parent.mkdir(parents=True, exist_ok=True)
            shutil.copy(ROOT / rel, tree / rel)
        shutil.copytree(ROOT / "scripts/bench/linux/matlab", tree / "scripts/bench/linux/matlab")
        (tree / "scripts/ci").mkdir(parents=True, exist_ok=True)
        shutil.copy(test_src, tree / "scripts/ci/test_matlab_rows.py")
        target = tree / REL
        target.write_text(SRC)
        code, _, out = run(tree)
        if code:
            print("FAIL unmutated baseline\n" + out[-4000:])
            return 2
        print("PASS baseline: " + re.search(r"Ran (\d+) tests", out).group(1), flush=True)
        for name, mutated, tests in CASES:
            target.write_text(mutated)
            try:
                code, killers, out = run(tree)
            finally:
                target.write_text(SRC)
            names = [k.split(".")[-1] for k in killers]
            ok = code != 0 and len(set(names)) >= 2 and bool(set(tests) & set(names))
            print("%s %s: %d killers %s" % ("PASS" if ok else "WEAK", name, len(set(names)), names[:4]), flush=True)
            weak += not ok
    print("%d/%d mutants killed by at least two tests" % (len(CASES) - weak, len(CASES)))
    return 1 if weak else 0


if __name__ == "__main__":
    sys.exit(main())
