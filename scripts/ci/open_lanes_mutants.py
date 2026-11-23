#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later
"""Mutation checks of the nvbandwidth row and the llama.cpp Vulkan row (2026-11-23): every mutant needs two named killers (stdlib, GPU-free).

STATUS: LIVE, 2026-11-23 (docs/design/V3_OPEN_LANES_NVBW_LLAMAVK_LHM.md section 7). The Windows LibreHardwareMonitor row has its own mutants in
scripts/bench/windows/open_rows/open_rows_mutants.py.

Each mutant is one exact-once text replacement in a private copy of one file of scripts/apps (or of the GPU-use judge); the named tests of
scripts/ci/test_open_lanes.py run against the copy, and a mutant is killed only when at least TWO of its named tests fail. The unmutated copy must be
green first. The mutants are the ways the assertion of these rows could be quietly weakened: a waived case that passes, a case with no figure that passes,
a hidden GPU that is accepted, a software Vulkan driver that is accepted, a negative control that does not hide the GPU.

    python3 scripts/ci/open_lanes_mutants.py [--jobs N] [substring-of-mutant-name]
"""
import argparse
import concurrent.futures
import json
from pathlib import Path
import shutil
import subprocess
import sys
import tempfile

HERE = Path(__file__).resolve().parent
ROOT = HERE.parents[1]
FILES = {
    "nv": "scripts/apps/open_rows/nvbandwidth_row.py",
    "lv": "scripts/apps/open_rows/llama_vulkan_row.py",
    "gu": "scripts/bench/gpu_use.py",
    "rows": "scripts/apps/gpu_use_rows.py",
    "run_apps": "scripts/apps/run_apps.sh",
    "prov": "scripts/apps/open_rows/provision_lanes.sh",
    "pins": "scripts/apps/open_rows/pins.env",
    "cases": "scripts/apps/open_rows/nvbandwidth_cases.tsv",
    "hook": "scripts/apps/apps_hook.sh",
    "bundle": "scripts/apps/build_bundle.sh",
    "compare": "scripts/apps/open_rows/compare_logs.py",
}
SUITE = "test_open_lanes."
COPY_DIRS = ["scripts/apps", "scripts/ci"]
COPY_FILES = ["scripts/bench/gpu_use.py", "scripts/bench/windows/gpu_assert.py", "scripts/bench/windows/kf3_census.py"]
COLLECT = """import io, json, sys, unittest
suite = unittest.defaultTestLoader.loadTestsFromNames(sys.argv[1:])
r = unittest.TextTestRunner(stream=io.StringIO()).run(suite)
print('KF_MUTATION_RESULT=' + json.dumps({'tests': r.testsRun, 'killers': [t.id() for t, _ in r.failures + r.errors]}))
"""


def M(name, key, old, new, *tests):
    return (name, key, old, new, [SUITE + n.split(".")[0] + ".test_" + n.split(".")[1] for n in tests])


MUTANTS = [
    # ---- nvbandwidth: the judge of one case
    M("nvb-unexpected-waive-passes", "nv", '        if ngpus >= needs:\n            return "FAIL", "unexpected_waive", status, []\n', "",
      "Judge.a_case_that_needs_one_gpu_and_was_waived_is_a_failure", "Judge.a_peer_case_waived_on_two_gpus_is_a_failure", "RunnerEndToEnd.an_unexpected_waive_fails_the_row"),
    M("nvb-peer-case-passes-without-peers", "nv", '    if ngpus < needs:\n        return "MISMATCH", "passed_without_%d_gpus" % needs, status, []\n', "",
      "Judge.a_peer_case_that_passed_without_peers_is_a_mismatch", "Judge.the_hidden_gpu_form_fails_every_case"),
    M("nvb-no-figure-passes", "nv", '    if not vals:\n        return "FAIL", "no_positive_figure", status, []', '    if False:\n        return "FAIL", "no_positive_figure", status, []',
      "Judge.a_passed_case_without_a_positive_figure_fails", "RunnerEndToEnd.cases_that_pass_with_no_figure_fail_the_row"),
    M("nvb-floor-ignored", "nv", '    if min(vals) < MIN_GBPS and case["class"] != "latency":', "    if False:",
      "Judge.a_figure_below_the_floor_fails_but_a_small_latency_does_not", "RunnerEndToEnd.a_figure_below_the_floor_fails_the_row"),
    M("nvb-floor-applies-to-latency", "nv", '    if min(vals) < MIN_GBPS and case["class"] != "latency":', "    if min(vals) < MIN_GBPS:",
      "Judge.a_figure_below_the_floor_fails_but_a_small_latency_does_not", "Judge.a_latency_in_nanoseconds_is_not_judged_against_the_bandwidth_floor"),
    M("nvb-timeout-not-a-failure", "nv", '    if timed_out:\n        return "FAIL", "timeout", "Timeout", []', '    if False:\n        return "FAIL", "timeout", "Timeout", []',
      "Judge.timeout_crash_missing_binary_and_missing_json", "RunnerEndToEnd.a_hung_case_is_killed_booked_as_a_timeout_and_fails_the_row"),
    M("nvb-crash-with-no-json-is-ok", "nv", '    if rc != 0 and body is None:\n        return "FAIL", "crash_rc%s" % rc, "Crash", []', '    if False:\n        return "FAIL", "crash_rc%s" % rc, "Crash", []',
      "Judge.timeout_crash_missing_binary_and_missing_json", "RunnerEndToEnd.a_crash_is_booked_on_its_case_and_the_row_fails"),
    M("nvb-error-status-passes", "nv", '    if status != "Passed":\n        return "FAIL", "status_"', '    if False:\n        return "FAIL", "status_"',
      "Judge.an_error_status_is_a_failure", "Judge.a_status_that_never_finished_is_a_failure"),
    M("nvb-missing-case-in-json-passes", "nv", '    if not cases:\n        return "FAIL", "case_not_reported", "NoCase", []', '    if False:\n        return "FAIL", "case_not_reported", "NoCase", []',
      "Judge.a_document_that_does_not_report_the_case_fails", "RunnerEndToEnd.a_binary_that_reports_another_case_fails_the_row"),
    M("nvb-json-error-key-ignored", "nv", '    if body.get("error"):\n        return "FAIL", "binary_error", "Error", []', '    if False:\n        return "FAIL", "binary_error", "Error", []',
      "Judge.a_document_with_an_error_key_fails", "RunnerEndToEnd.a_binary_that_reports_an_error_fails_the_row"),
    M("nvb-zero-and-na-cells-count", "nv", '            if NUMBER_RX.match(s) and float(s) > 0:', "            if NUMBER_RX.match(s):",
      "Parsing.matrix_values_skip_not_available_and_zero", "Judge.a_passed_case_without_a_positive_figure_fails"),
    # ---- nvbandwidth: the row
    M("nvb-failures-do-not-fail-the-row", "nv", '    if counts["FAIL"] or counts["MISMATCH"]:\n        return "FAIL", counts, "unexpected_results"', '    if False:\n        return "FAIL", counts, "unexpected_results"',
      "Summary.one_failure_or_mismatch_fails_the_row", "RunnerEndToEnd.a_crash_is_booked_on_its_case_and_the_row_fails", "RunnerEndToEnd.an_unexpected_waive_fails_the_row"),
    M("nvb-missing-class-passes", "nv", '    if missing:\n        return "FAIL", counts, "class_not_passed:" + "+".join(missing)', '    if False:\n        return "FAIL", counts, "class_not_passed:" + "+".join(missing)',
      "Summary.every_class_must_have_a_passed_case", "RunnerEndToEnd.only_selects_cases"),
    M("nvb-nothing-ran-is-a-pass", "nv", '    if ran == 0:\n        return "NOT_RUN", counts, "nothing_ran"', '    if False:\n        return "NOT_RUN", counts, "nothing_ran"',
      "Summary.a_row_where_nothing_ran_is_not_run_never_a_pass", "RunnerEndToEnd.a_missing_binary_is_not_run_with_an_appnotrun_line_and_exit_zero"),
    M("nvb-not-run-line-dropped", "nv", '        print("APPNOTRUN nvbandwidth:%s" % (header_note or reason))\n', "",
      "RunnerEndToEnd.a_missing_binary_is_not_run_with_an_appnotrun_line_and_exit_zero", "RunAppsShell.a_row_whose_tool_is_missing_is_notrun_with_the_reason"),
    M("nvb-exit-status-ignores-failure", "nv", '    return 0 if status == "PASS" else 1\n\n\n# -----', '    return 0\n\n\n# -----',
      "RunnerEndToEnd.a_crash_is_booked_on_its_case_and_the_row_fails", "RunnerEndToEnd.the_hidden_gpu_negative_control_fails_and_the_judge_agrees"),
    M("nvb-data-verification-skipped", "nv", '    return [binary, "-j", "-t", case_id, "-b", str(int(buffer_mib)), "-i", str(int(samples))]', '    return [binary, "-j", "-t", case_id, "-b", str(int(buffer_mib)), "-i", str(int(samples)), "-s"]',
      "RunnerEndToEnd.case_commands_never_go_through_a_shell", "RunnerEndToEnd.nvbandwidth_is_never_told_to_skip_its_data_verification"),
    M("nvb-device-count-from-nowhere", "nv", '                ngpus, device = len(names), (names[0] if names else "")', '                ngpus, device = max(1, len(names)), (names[0] if names else "")',
      "RunnerEndToEnd.the_hidden_gpu_negative_control_fails_and_the_judge_agrees", "RunnerEndToEnd.the_hidden_gpu_proof_line_says_no_device"),
    M("nvb-table-peer-case-needs-one", "cases", "device_to_device_memcpy_read_ce\tce\t2", "device_to_device_memcpy_read_ce\tce\t1",
      "Table.peer_pair_cases_need_two_gpus_and_the_others_one", "RunnerEndToEnd.a_single_gpu_run_passes_with_the_peer_cases_waived"),
    M("nvb-table-drops-the-latency-case", "cases", "host_device_latency_sm\tlatency\t1\n", "",
      "Table.the_committed_table_is_the_35_testcases_of_the_pinned_source", "Table.the_latency_and_local_cases_are_what_their_names_say"),
    # ---- the GPU-use judge: nvbandwidth
    M("gu-nvb-no-gpu-accepted", "gu", '    if int(gpus) < 1:\n        why.append("nvbandwidth listed no GPU device")', '    if False:\n        why.append("nvbandwidth listed no GPU device")',
      "ProofJudge.no_gpu_device_fails", "ProofJudge.every_way_to_fail", "Summary.the_proof_line_of_a_hidden_gpu_run_is_judged_a_failure"),
    M("gu-nvb-device-name-ignored", "gu", '    elif not _nvidia(name):\n        why.append("device name is not an NVIDIA GPU: " + name[:60])', '    elif False:\n        why.append("device name is not an NVIDIA GPU: " + name[:60])',
      "ProofJudge.a_non_nvidia_device_name_fails", "ProofJudge.every_way_to_fail"),
    M("gu-nvb-no-copy-engine-accepted", "gu", '    if int(ce) < 1:\n        why.append("no copy-engine case passed")', '    if False:\n        why.append("no copy-engine case passed")',
      "ProofJudge.no_copy_engine_case_fails", "ProofJudge.every_way_to_fail"),
    M("gu-nvb-no-sm-copy-accepted", "gu", '    if int(sm) < 1:\n        why.append("no SM-copy case passed")', '    if False:\n        why.append("no SM-copy case passed")',
      "ProofJudge.no_sm_copy_case_fails", "ProofJudge.every_way_to_fail"),
    M("gu-nvb-floor-ignored", "gu", '    if int(passed) >= 1 and float(floor) < NVB_MIN_GBPS:', "    if False:",
      "ProofJudge.a_figure_below_any_real_link_fails", "ProofJudge.every_way_to_fail"),
    M("gu-nvb-missing-proof-is-a-pass", "gu", '        return _proof(None, "", ["no NVBANDWIDTH_PROOF line (the row dropped its device proof)"])', '        return _proof(True, "", [])',
      "ProofJudge.a_dropped_proof_line_is_none_so_the_row_fails", "ProofJudge.a_log_without_the_proof_line_says_so_in_its_reason"),
    M("rows-nvbandwidth-not-wired", "rows", '"nvbandwidth": ("a+proof", "nvbandwidth",', '"nvbandwidth": ("a", "nvbandwidth",',
      "ProofJudge.the_row_is_classified_and_asserts_the_gpu_engines", "ProofJudge.a_dropped_proof_line_is_none_so_the_row_fails"),
    M("rows-nvbandwidth-missing-proof-is-not-run", "rows", '("gpu", "mem"), dict(LONG), "FAIL"),\n}', '("gpu", "mem"), dict(LONG), "NOT_RUN"),\n}',
      "ProofJudge.the_row_is_classified_and_asserts_the_gpu_engines", "ProofJudge.a_dropped_proof_line_is_none_so_the_row_fails"),
    # ---- the GPU-use judge: llama.cpp on Vulkan
    M("gu-lv-software-device-accepted", "gu", '    gpu = [d for d in devs if _nvidia(d[1]) and not SOFT_GPU_NAMES.search(d[2])]', '    gpu = devs',
      "LlamaVulkanParsing.the_gpu_use_module_names_the_reason", "LlamaVulkanParsing.the_judge_of_the_row_and_the_judge_of_the_gpu_use_module_agree_on_every_fixture"),
    M("gu-lv-backend-unchecked", "gu", '    bad = [r for r in rows if "vulkan" not in r[0].lower() or "cpu" in r[0].lower() or int(r[1]) < 1 or float(r[2]) <= 0]', "    bad = []",
      "LlamaVulkanParsing.the_gpu_use_module_names_the_reason", "LlamaVulkanParsing.the_judge_of_the_row_and_the_judge_of_the_gpu_use_module_agree_on_every_fixture",
      "LlamaVulkanParsing.the_judge_of_the_rows_module_fails_every_fallback_and_passes_the_real_log"),
    M("gu-lv-ngl-unchecked", "gu", ' or int(r[1]) < 1 or float(r[2]) <= 0]', ' or float(r[2]) <= 0]',
      "LlamaVulkanParsing.the_gpu_use_module_names_the_reason", "LlamaVulkanParsing.the_judge_of_the_rows_module_fails_every_fallback_and_passes_the_real_log"),
    M("gu-lv-no-table-is-a-pass", "gu", '        return _proof(None, gpu[0][1], ["no llama-bench table row"])', '        return _proof(True, gpu[0][1], [])',
      "LlamaVulkanParsing.a_device_line_without_a_table_is_not_a_proof", "LlamaVulkanParsing.the_judge_of_the_row_and_the_judge_of_the_gpu_use_module_agree_on_every_fixture"),
    M("rows-llama-vulkan-cannot-fall-back", "rows", '"llama_vulkan": ("b", "llama_vulkan",', '"llama_vulkan": ("a", "llama_vulkan",',
      "LlamaVulkanParsing.the_row_is_classified_as_able_to_fall_back", "LlamaVulkanParsing.the_judge_of_the_rows_module_fails_every_fallback_and_passes_the_real_log"),
    M("rows-llama-vulkan-missing-proof-is-not-run", "rows", '"llama_vulkan": ("b", "llama_vulkan", {}, ("gpu", "mem"), dict(LONG), "FAIL")', '"llama_vulkan": ("b", "llama_vulkan", {}, ("gpu", "mem"), dict(LONG), "NOT_RUN")',
      "LlamaVulkanParsing.the_row_is_classified_as_able_to_fall_back", "LlamaVulkanParsing.a_log_with_no_vulkan_proof_at_all_is_a_failure_not_a_not_run"),
    # ---- llama.cpp on Vulkan: the row's own judge and runner
    M("lv-software-device-accepted", "lv", '    if not gpu:\n        return "FAIL", "device_is_not_nvidia:"', '    if False:\n        return "FAIL", "device_is_not_nvidia:"',
      "LlamaVulkanParsing.a_cpu_vulkan_driver_fails", "LlamaVulkanRunner.a_software_vulkan_driver_fails"),
    M("lv-no-device-accepted", "lv", '    if not devices:\n        return "FAIL", "no_vulkan_device", facts', '    if False:\n        return "FAIL", "no_vulkan_device", facts',
      "LlamaVulkanParsing.no_device_fails", "LlamaVulkanRunner.the_negative_control_forms_fail_and_the_judge_agrees"),
    M("lv-backend-unchecked", "lv", '        if "vulkan" not in backend.lower() or "cpu" in backend.lower():', "        if False:",
      "LlamaVulkanParsing.a_cpu_backend_row_next_to_an_nvidia_device_fails", "LlamaVulkanRunner.a_cpu_backend_row_next_to_an_nvidia_device_fails"),
    M("lv-ngl-unchecked", "lv", "        if ngl < 1:", "        if False:",
      "LlamaVulkanParsing.ngl_zero_fails", "LlamaVulkanRunner.ngl_zero_alone_is_a_failure_too"),
    M("lv-throughput-unchecked", "lv", "        if tps <= 0:", "        if False:",
      "LlamaVulkanParsing.a_zero_throughput_row_fails", "LlamaVulkanRunner.a_zero_throughput_fails"),
    M("lv-generation-row-optional", "lv", '    if not (facts["pp512"] > 0 and facts["tg64"] > 0):', "    if False:",
      "LlamaVulkanParsing.a_missing_generation_row_fails", "LlamaVulkanRunner.a_table_without_the_generation_row_fails"),
    M("lv-no-table-passes", "lv", '    if not rows:\n        return "FAIL", "no_table_row", facts', '    if False:\n        return "FAIL", "no_table_row", facts',
      "LlamaVulkanParsing.no_table_fails", "LlamaVulkanRunner.a_log_without_a_table_says_no_table_row"),
    M("lv-exit-status-ignored", "lv", '    elif rc != 0:\n        status, reason, facts = "FAIL", "rc_%s" % rc, judge(text)[2]', '    elif False:\n        status, reason, facts = "FAIL", "rc_%s" % rc, judge(text)[2]',
      "LlamaVulkanRunner.a_nonzero_exit_status_fails_even_with_a_good_table", "RunAppsShell.a_row_that_prints_a_pass_summary_but_fails_its_exit_status_is_not_a_pass"),
    M("lv-not-run-line-dropped", "lv", '        print("APPNOTRUN llama_vulkan:%s" % reason)\n', "",
      "LlamaVulkanRunner.a_missing_binary_or_model_is_not_run_never_a_failure_or_a_pass", "RunAppsShell.the_vulkan_row_without_its_binary_is_notrun_with_the_reason"),
    M("lv-matrix-cores-hardcoded", "lv", '    return m.group(1) if m else "unknown"', '    return "KHR_coopmat"',
      "LlamaVulkanParsing.the_real_nvidia_log_passes_and_records_the_matrix_cores", "LlamaVulkanParsing.a_die_without_cooperative_matrix_still_passes_with_coopmat_none"),
    M("lv-bench-line-differs-from-the-cuda-row", "lv", 'BENCH_ARGS = ("-p", "512", "-n", "64", "-r", "2")', 'BENCH_ARGS = ("-p", "256", "-n", "64", "-r", "2")',
      "LlamaVulkanRunner.the_command_line_is_the_bench_the_cuda_row_runs", "LlamaVulkanRunner.the_bench_arguments_equal_those_of_the_cuda_llama_bench_row"),
    # ---- wiring: the negative controls, the rows, the pins
    M("negctl-keeps-the-nvidia-icd", "run_apps", '  export KF_VK_ENV="VK_DRIVER_FILES=/nonexistent/kayfabe-negctl.json VK_ICD_FILENAMES=/nonexistent/kayfabe-negctl.json"\n', "",
      "Wiring.the_negative_control_hides_the_gpu_in_the_run_apps_block", "RunAppsShell.llama_vulkan_passes_by_default_and_its_negative_control_fails_by_itself"),
    M("negctl-keeps-cuda-visible", "run_apps", 'KF_GPU_ENV="CUDA_VISIBLE_DEVICES="', 'KF_GPU_ENV=""',
      "Wiring.the_negative_control_hides_the_gpu_in_the_run_apps_block", "RunAppsShell.nvbandwidth_passes_by_default_and_its_negative_control_fails_by_itself"),
    M("nvbandwidth-row-ignores-the-gpu-switch", "run_apps", "${KF_GPU_ENV:+env $KF_GPU_ENV }python3 -B -I $B/share/open_rows/nvbandwidth_row.py", "python3 -B -I $B/share/open_rows/nvbandwidth_row.py",
      "Wiring.the_negative_control_hides_the_gpu_in_the_run_apps_block", "RunAppsShell.nvbandwidth_passes_by_default_and_its_negative_control_fails_by_itself"),
    M("llama-vulkan-row-ignores-the-icd-switch", "run_apps", "${KF_VK_ENV:+env $KF_VK_ENV }python3 -B -I $B/share/open_rows/llama_vulkan_row.py", "python3 -B -I $B/share/open_rows/llama_vulkan_row.py",
      "Wiring.the_negative_control_hides_the_gpu_in_the_run_apps_block", "RunAppsShell.llama_vulkan_passes_by_default_and_its_negative_control_fails_by_itself"),
    M("llama-vulkan-row-ignores-ngl-switch", "run_apps", "--data $D --ngl ${KF_NGL:-99}", "--data $D --ngl 99",
      "RunAppsShell.llama_vulkan_passes_by_default_and_its_negative_control_fails_by_itself", "Wiring.the_negative_control_hides_the_gpu_in_the_run_apps_block"),
    M("nvbandwidth-summary-regex-too-weak", "run_apps", "nvbandwidth|1800|NVBANDWIDTH_SUMMARY status=PASS|", "nvbandwidth|1800|NVBANDWIDTH_SUMMARY|",
      "RunAppsShell.the_summary_regexes_demand_status_pass", "Wiring.each_row_regex_names_the_summary_the_row_prints_with_its_pass_status"),
    M("llama-vulkan-summary-regex-too-weak", "run_apps", "llama_vulkan|1200|LLAMA_VULKAN_SUMMARY status=PASS|", "llama_vulkan|1200|LLAMA_VULKAN_SUMMARY|",
      "RunAppsShell.the_summary_regexes_demand_status_pass", "Wiring.each_row_regex_names_the_summary_the_row_prints_with_its_pass_status"),
    M("prov-nvbandwidth-commit-unpinned", "prov", '"$OPENROWS_NVB_TAG" "$OPENROWS_NVB_COMMIT" || return 1', '"$OPENROWS_NVB_TAG" "" || return 1',
      "Wiring.provision_checks_every_source_out_at_its_pinned_commit", "Wiring.every_pin_variable_the_script_uses_is_defined_and_the_commit_pins_are_used"),
    M("prov-llama-has-its-own-pin", "prov", '"$PIN_LLAMA_URL" "" "$PIN_LLAMA_COMMIT" || return 1', '"$PIN_LLAMA_URL" "" "0000000000000000000000000000000000000000" || return 1',
      "Wiring.provision_checks_every_source_out_at_its_pinned_commit", "Wiring.every_pin_variable_the_script_uses_is_defined_and_the_commit_pins_are_used"),
    M("prov-llama-keeps-cuda-on", "prov", "-DGGML_CUDA=OFF", "-DGGML_CUDA=ON",
      "Wiring.provision_builds_both_tools_from_pins", "Wiring.the_vulkan_build_is_vulkan_only_and_builds_the_bench"),
    M("prov-checkout-failure-ignored", "prov", '"$OPENROWS_NVB_COMMIT" || return 1', '"$OPENROWS_NVB_COMMIT"',
      "Wiring.every_command_of_a_step_ends_in_a_return_because_set_e_is_off_inside_if", "Wiring.provision_checks_every_source_out_at_its_pinned_commit"),
    M("prov-model-is-a-default-step", "prov", "STEPS=${2:-nvbandwidth llama_vulkan}", "STEPS=${2:-nvbandwidth llama_vulkan llama_model}",
      "Wiring.the_model_step_is_not_a_default_step_and_run_standalone_asks_for_it", "Wiring.a_gate_bundle_never_downloads_the_model"),
    M("pins-nvbandwidth-commit-is-a-tag", "pins", "OPENROWS_NVB_COMMIT=4a49bdae82d69e69330a88746cb12b449796ee8b", "OPENROWS_NVB_COMMIT=v0.9",
      "Wiring.the_nvbandwidth_commit_is_one_pin_git_checkout_accepts", "Wiring.the_pins_are_commits_and_there_is_no_second_llama_pin"),
    M("pins-a-binary-digest-appears", "pins", "OPENROWS_NVB_ARCHS=", "OPENROWS_NVB_SHA256=0000000000000000000000000000000000000000000000000000000000000000\nOPENROWS_NVB_ARCHS=",
      "Wiring.the_pins_are_commits_and_there_is_no_second_llama_pin", "Wiring.no_file_of_the_lanes_pins_a_digest"),
    M("bundle-lane-failure-can-fail-the-gate", "bundle", 'bash "$HERE/open_rows/provision_lanes.sh" "$B" ||', 'bash "$HERE/open_rows/provision_lanes.sh" "$B" &&',
      "Wiring.the_provision_script_is_glue_and_the_bundle_calls_it_without_being_able_to_fail", "Wiring.a_failed_lane_step_prints_a_line_in_the_bundle_build_and_nothing_else"),
    M("hook-does-not-push-the-vulkan-build", "hook", "tar -C /workspace/apps/bundle -cf - llama_vk |", "tar -C /workspace/apps/bundle -cf - bin |",
      "Wiring.the_hook_pushes_the_vulkan_build_of_llama_bench_when_the_host_built_one", "Wiring.the_hook_unpacks_into_the_bundle_of_the_guest"),
    M("compare-logs-forgets-nvbandwidth", "compare", '"nvml_dcgm": dcgm_nvml.cmd_compare, "nvbandwidth": nvbandwidth_row.cmd_compare}', '"nvml_dcgm": dcgm_nvml.cmd_compare}',
      "Wiring.compare_logs_knows_the_nvbandwidth_row", "RunnerEndToEnd.compare_logs_front_end_compares_two_runs"),
]


def apply(text, old, new):
    if text.count(old) != 1:
        raise SystemExit("mutant target occurs %d times: %r" % (text.count(old), old[:80]))
    return text.replace(old, new, 1)


def run_one(mutant):
    name, key, old, new, tests = mutant
    tests = sorted(set(tests))
    with tempfile.TemporaryDirectory() as tmp:
        tmp = Path(tmp)
        for d in COPY_DIRS:
            shutil.copytree(ROOT / d, tmp / d, ignore=shutil.ignore_patterns("__pycache__"))
        for f in COPY_FILES:
            (tmp / f).parent.mkdir(parents=True, exist_ok=True)
            shutil.copyfile(ROOT / f, tmp / f)
        target = tmp / FILES[key]
        target.write_text(apply(target.read_text(), old, new))
        p = subprocess.run([sys.executable, "-B", "-c", COLLECT] + tests, cwd=tmp / "scripts/ci", capture_output=True, text=True, timeout=900)
        line = next((l for l in p.stdout.splitlines() if l.startswith("KF_MUTATION_RESULT=")), None)
        if line is None:
            return name, None, tests, p.stderr[-400:]
        return name, json.loads(line.split("=", 1)[1]), tests, ""


def main(argv):
    ap = argparse.ArgumentParser(description=__doc__.split("\n\n")[0])
    ap.add_argument("--jobs", type=int, default=4)
    ap.add_argument("only", nargs="?", default="")
    a = ap.parse_args(argv)
    names = [m[0] for m in MUTANTS]
    if len(names) != len(set(names)):
        print("duplicate mutant names")
        return 2
    base = run_one(("baseline", "nv", "import fnmatch", "import fnmatch", [t for m in MUTANTS for t in m[4]]))
    if base[1] is None or base[1]["killers"]:
        print("BASELINE not green:", base[1] and base[1]["killers"][:5], base[3])
        return 2
    selected = [m for m in MUTANTS if a.only in m[0]]
    bad = 0
    with concurrent.futures.ThreadPoolExecutor(a.jobs) as pool:
        for name, res, tests, err in pool.map(run_one, selected):
            if res is None:
                print("ERROR    %s: %s" % (name, err))
                bad += 1
                continue
            killers = sorted(res["killers"])
            status = "killed" if len(killers) >= 2 else "SURVIVED" if not killers else "WEAK"
            if status != "killed":
                bad += 1
            print("%-8s %s  (%d of %d named tests failed: %s)" % (status, name, len(killers), len(tests), ", ".join(k.split(".")[-1][5:45] for k in killers)))
    print("MUTANTS total=%d not_killed_twice=%d" % (len(selected), bad))
    return 1 if bad else 0


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
