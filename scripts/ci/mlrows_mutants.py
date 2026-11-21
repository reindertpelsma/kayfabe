#!/usr/bin/env python3
"""Mutation checks of the open ML rows (2026-11-17): every mutant needs two named killers (stdlib, GPU-free).

Each mutant is one exact-once text replacement in a private copy of scripts/apps/mlrows/{mlrows_lib,mlrows}.py, run_apps.sh, apps_hook.sh, apps_matrix.sh, matrix_table.py or
gpu_use_rows.py; the named tests of test_mlrows.py run against the copy and the failing ones are the killers. The unmutated copy must be green first. The mutants are the ways
the protections of these rows could be quietly weakened: an unhashed install, a foreign index, a model the origin does not list, a flavour the driver cannot take, a CPU run that
passes, a stale file read as a result, a guest that differs from the host without a finding.

    python3 scripts/ci/mlrows_mutants.py [--jobs N] [substring-of-mutant-name]
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
    "lib": "scripts/apps/mlrows/mlrows_lib.py",
    "mr": "scripts/apps/mlrows/mlrows.py",
    "run": "scripts/apps/run_apps.sh",
    "hook": "scripts/apps/apps_hook.sh",
    "matrix": "scripts/apps/apps_matrix.sh",
    "mt": "scripts/apps/matrix_table.py",
    "rows": "scripts/apps/gpu_use_rows.py",
    "pp": "scripts/apps/mlrows/private_python.py",
}
SOURCES = {key: (ROOT / path).read_text() for key, path in FILES.items()}
SUITE = "test_mlrows."
COPY_DIRS = ["scripts/ci", "scripts/apps"]
COPY_FILES = ["scripts/bench/gpu_use.py", "scripts/bench/windows/gpu_assert.py"]
COLLECT = """import io, json, sys, unittest
suite = unittest.defaultTestLoader.loadTestsFromNames(sys.argv[1:])
r = unittest.TextTestRunner(stream=io.StringIO()).run(suite)
print('KF_MUTATION_RESULT=' + json.dumps({'tests': r.testsRun, 'killers': [t.id() for t, _ in r.failures + r.errors]}))
"""
(PI, FA, FL, LK, MO, JV, JT, JJ, PF, EN, RR, MM, CR, SE, CP, PU, MW, GJ, TP, HP, PP, IC, PV) = (SUITE + n for n in (
    "Pins.", "Facts.", "Flavour.", "Locks.", "Model.", "JudgeVllm.", "JudgeTensorrt.", "JudgeJax.", "Proof.", "Env.", "RunRow.", "MemoryRecord.", "CmdRun.", "Setup.", "Compare.",
    "Push.", "MatrixWiring.", "GpuUseJudge.", "TrustPins.", "HookPush.", "PrivatePython.", "Interpreter.", "PrivatePush."))


def M(name, key, old, new, *tests):
    return (name, key, old, new, list(tests))


MUTANTS = [
    # the flavour: which wheel a machine may take
    M("flavour-floor-off-by-one", "lib", "if tuple(cuda) >= need:", "if tuple(cuda) > need:",
      FL + "test_the_cuda_boundary_is_exact", FL + "test_a_cuda_13_driver_takes_the_newest_flavour_of_every_row"),
    M("turing-floor-ignored", "lib", "    if tuple(cc) < floor:\n", "    if False:\n",
      FL + "test_older_than_turing_is_not_run_with_the_cc_in_the_reason", SE + "test_hardware_and_environment_preconditions_are_named_not_runs_and_nothing_runs"),
    M("vram-floor-ignored", "lib", 'if facts.get("total_mib") is not None and facts["total_mib"] < spec["min_vram_mib"]:', "if False:",
      FL + "test_no_gpu_and_too_little_memory", SE + "test_too_little_video_memory_is_not_run"),
    M("python-range-ignored", "lib", '    if tuple(py[:2]) < pmin:\n        return _nr("python_too_old', '    if False:\n        return _nr("python_too_old',
      FL + "test_python_range", SE + "test_an_old_python_is_not_run"),
    M("unknown-driver-cuda-is-assumed-new", "lib", '    if cuda is None:\n        return {"status": "env"', '    if cuda is None:\n        cuda = (99, 0)\n    if False:\n        return {"status": "env"',
      FL + "test_an_unknown_driver_cuda_is_an_env_problem_not_a_not_run", SE + "test_an_unknown_driver_cuda_is_a_fail_with_the_reason_not_a_not_run"),
    M("forced-flavour-skips-the-driver-check", "mr", '        if facts.get("driver_cuda") and tuple(facts["driver_cuda"]) < need:', "        if False:",
      FL + "test_a_forced_flavour_is_checked_against_the_driver_and_never_overrides_a_hard_not_run", RR + "test_a_forced_flavour_the_driver_cannot_take_is_not_run"),
    # the hashed locks and the install
    M("lock-unhashed-accepted", "lib", '        if not r["hashes"]:', "        if False:",
      LK + "test_a_requirement_without_a_hash_is_refused", SE + "test_a_lock_that_lost_a_hash_is_refused_before_pip_runs"),
    M("lock-foreign-index-accepted", "lib", "        elif host.group(1) not in ALLOWED_INDEX_HOSTS:", "        elif False:",
      LK + "test_a_foreign_index_is_refused", SE + "test_a_lock_from_a_foreign_index_is_refused_before_pip_runs"),
    M("lock-plain-http-index-accepted", "lib", '        if not host:\n            out.append("index is not https: " + url)', '        if False:\n            out.append("index is not https: " + url)',
      LK + "test_a_plain_http_index_is_refused", SE + "test_a_lock_that_names_a_plain_http_index_is_refused_before_pip_runs"),
    M("lock-without-an-index-accepted", "lib", '    if not any(k == "index-url" for k, _ in indexes):', "    if False:",
      LK + "test_a_lock_without_an_index_line_is_refused", SE + "test_a_lock_without_an_index_is_refused_before_pip_runs"),
    M("tensorrt-lock-without-nvidia-index-accepted", "lib", '    if require_nvidia_index and not any("pypi.nvidia.com" in u for _, u in indexes):', "    if False:",
      LK + "test_a_tensorrt_lock_without_nvidia_s_index_is_refused", SE + "test_a_tensorrt_lock_without_nvidia_s_index_is_refused_before_pip_runs"),
    M("lock-duplicate-accepted", "lib", '        if r["name"] in names:', "        if False:",
      LK + "test_a_duplicate_name_is_refused_and_an_empty_lock_too", SE + "test_a_lock_with_a_duplicate_package_is_refused_before_pip_runs"),
    M("pins-vs-lock-ignored", "lib", "        if got != ver:", "        if False:",
      LK + "test_a_pin_that_disagrees_with_the_lock_is_named", SE + "test_a_lock_that_disagrees_with_the_pins_is_refused_before_pip_runs"),
    M("setup-skips-the-lock-check", "mr", '    if problems:\n        return lib.FAIL, "lock_refused: "', '    if False:\n        return lib.FAIL, "lock_refused: "',
      SE + "test_a_lock_that_lost_a_hash_is_refused_before_pip_runs", SE + "test_a_lock_that_disagrees_with_the_pins_is_refused_before_pip_runs"),
    M("pip-without-require-hashes", "mr", '"--only-binary=:all:", "--require-hashes", "--no-deps"', '"--only-binary=:all:", "--no-deps"',
      SE + "test_the_install_is_one_hashed_only_binary_no_deps_pip_run_of_the_lock", TP + "test_pip_is_run_in_exactly_one_place_and_always_hashed"),
    M("pip-allows-sdists", "mr", '"--only-binary=:all:", "--require-hashes"', '"--require-hashes"',
      SE + "test_the_install_is_one_hashed_only_binary_no_deps_pip_run_of_the_lock", TP + "test_pip_is_run_in_exactly_one_place_and_always_hashed"),
    M("pip-resolves-dependencies", "mr", '"--require-hashes", "--no-deps", "-r"', '"--require-hashes", "-r"',
      SE + "test_the_install_is_one_hashed_only_binary_no_deps_pip_run_of_the_lock", TP + "test_pip_is_run_in_exactly_one_place_and_always_hashed"),
    M("pip-failure-leaves-an-ok-state", "mr", "    if rc != 0:\n        why = \"pip_timeout\"", "    if False:\n        why = \"pip_timeout\"",
      SE + "test_a_hash_mismatch_is_a_named_fail_and_leaves_no_ok_state", SE + "test_a_pip_failure_other_than_hashes_is_named_and_leaves_no_ok_state"),
    M("setup-ignores-the-c-compiler", "mr", '    if spec.get("needs_c_compiler") and not (facts.get("gcc") and facts.get("python_h")):', "    if False:",
      SE + "test_no_c_compiler_is_not_run_for_vllm", SE + "test_no_python_headers_is_not_run_for_vllm"),
    M("setup-ignores-the-disk", "mr", '    if free is not None and free < spec["need_disk_gb"]:', "    if False:",
      SE + "test_too_little_disk_is_not_run_with_the_numbers", SE + "test_hardware_and_environment_preconditions_are_named_not_runs_and_nothing_runs"),
    M("second-setup-reinstalls", "mr", '    if state and state.get("status") == "ok" and state.get("lock_sha256") == lsha', '    if False and state and state.get("status") == "ok" and state.get("lock_sha256") == lsha',
      SE + "test_a_second_setup_with_the_same_lock_does_nothing", SE + "test_a_second_setup_does_not_touch_the_staged_files"),
    M("changed-lock-keeps-the-old-venv", "mr", 'and state.get("lock_sha256") == lsha and state.get("flavour") == flavour', 'and state.get("flavour") == flavour',
      SE + "test_a_changed_lock_starts_clean", SE + "test_a_changed_lock_removes_what_the_old_one_installed"),
    # the model
    M("model-digest-not-checked", "lib", '    if got != entry["digest"]:', "    if False:",
      MO + "test_bytes_are_checked_by_size_then_by_digest", MO + "test_a_tampered_file_is_refused_and_not_left_behind"),
    M("model-size-not-checked", "lib", '    if len(data) != entry["size"]:', "    if False:",
      MO + "test_bytes_are_checked_by_size_then_by_digest", MO + "test_size_is_checked_even_when_the_digest_matches"),
    M("model-fetched-at-main", "mr", '"%s/resolve/%s/%s" % (base, m["revision"], entry["path"])', '"%s/resolve/%s/%s" % (base, "main", entry["path"])',
      MO + "test_fetch_downloads_at_the_pinned_revision_and_writes_the_manifest", MO + "test_files_already_verified_are_not_downloaded_again"),
    M("model-tree-listed-at-main", "mr", '"%s/api/models/%s/tree/%s" % (m["origin"], m["repo"], m["revision"])', '"%s/api/models/%s/tree/%s" % (m["origin"], m["repo"], "main")',
      MO + "test_fetch_downloads_at_the_pinned_revision_and_writes_the_manifest", MO + "test_files_already_verified_are_not_downloaded_again"),
    M("model-size-cap-removed", "lib", '    if total > m["max_bytes"]:', "    if False:",
      MO + "test_a_model_larger_than_pinned_is_refused", MO + "test_an_oversized_listing_fails_the_fetch_before_any_download"),
    M("model-path-traversal-allowed", "lib", '        if path.startswith("/") or ".." in path.split("/"):', "        if False:",
      MO + "test_an_unsafe_path_in_the_pins_is_refused", MO + "test_a_path_traversal_in_the_pins_is_refused_by_the_fetch_too"),
    M("model-bytes-written-before-the-check", "mr", '        bad = lib.check_model_bytes(entry, data)\n        if bad:\n            return False, "hash_mismatch: " + bad\n', '        bad = None\n',
      MO + "test_a_tampered_file_is_refused_and_not_left_behind", SE + "test_vllm_fetches_the_model_after_pip_and_a_bad_model_fails_the_setup"),
    M("http-allows-plain-http", "mr", '    if not url.startswith("https://"):', "    if False:",
      MO + "test_the_http_helper_refuses_plain_http", TP + "test_the_model_origin_is_the_only_network_use_of_setup_and_it_is_bounded"),
    # the environment of a row
    M("env-passes-everything", "mr", "    env = {k: base_env[k] for k in ENV_KEEP if k in base_env}", "    env = dict(base_env)",
      EN + "test_no_secret_and_no_loader_hook_reaches_the_row", EN + "test_the_environment_is_the_allowlist_plus_ours"),
    M("telemetry-on", "mr", '"VLLM_NO_USAGE_STATS": "1"', '"VLLM_NO_USAGE_STATS": "0"',
      EN + "test_telemetry_is_off_and_the_hub_is_offline", TP + "test_the_vllm_row_turns_telemetry_off"),
    M("hub-online", "mr", '"HF_HUB_OFFLINE": "1"', '"HF_HUB_OFFLINE": "0"',
      EN + "test_telemetry_is_off_and_the_hub_is_offline", RR + "test_the_row_runs_with_the_hub_offline_and_no_secret"),
    M("negctl-does-not-hide-the-gpu", "mr", '        env["CUDA_VISIBLE_DEVICES"] = ""\n', '        env["CUDA_VISIBLE_DEVICES"] = "0"\n',
      EN + "test_the_negative_control_hides_the_gpu_and_jax_gets_its_cpu_backend", RR + "test_the_negative_control_runs_the_cpu_form"),
    M("negctl-jax-keeps-its-gpu-backend", "mr", '        if row == "jax":\n            env["JAX_PLATFORMS"] = "cpu"', '        if False:\n            env["JAX_PLATFORMS"] = "cpu"',
      EN + "test_the_negative_control_hides_the_gpu_and_jax_gets_its_cpu_backend", RR + "test_the_negative_control_runs_the_cpu_form"),
    M("caches-survive-between-runs", "mr", "    if cache.exists():\n        shutil.rmtree(cache)", "    if False:\n        shutil.rmtree(cache)",
      RR + "test_the_cache_directory_is_fresh_on_every_run", RR + "test_a_second_run_in_the_same_directory_starts_with_empty_caches"),
    M("stale-work-files-survive", "mr", "    if work.exists():\n        shutil.rmtree(work)", "    if False:\n        shutil.rmtree(work)",
      RR + "test_the_working_directory_is_empty_at_the_start_of_every_run", RR + "test_the_working_directory_of_every_row_starts_empty"),
    M("library-path-loses-the-venv", "mr", '        env["LD_LIBRARY_PATH"] = ":".join(dirs + rest)', '        env["LD_LIBRARY_PATH"] = ":".join(rest + dirs)',
      EN + "test_the_venv_s_libraries_come_first_then_the_callers", RR + "test_the_row_s_library_path_leads_with_the_venv"),
    # running and judging
    M("timeout-not-reported", "mr", "    timed_out = bool(fired)", "    timed_out = False",
      RR + "test_a_row_that_hangs_is_killed_and_is_a_fail_with_the_timeout", RR + "test_the_timeout_fail_names_the_limit"),
    M("memory-peak-is-the-last-sample", "mr", '"mem_peak_mib": max(used), "mem_growth_mib": max(used) - base', '"mem_peak_mib": used[-1], "mem_growth_mib": used[-1] - base',
      MM + "test_growth_is_the_peak_over_the_baseline_of_the_first_samples", RR + "test_a_good_run_is_judged_and_gets_the_runner_s_memory_record"),
    M("memory-baseline-is-the-max", "mr", "base = min(used[:baseline_samples])", "base = max(used[:baseline_samples])",
      MM + "test_growth_is_the_peak_over_the_baseline_of_the_first_samples", RR + "test_a_good_run_is_judged_and_gets_the_runner_s_memory_record"),
    M("whole-log-not-read", "mr", '    full = log_path.read_text(errors="replace") if log_path.exists() else text', "    full = text",
      RR + "test_the_whole_log_is_kept_and_the_judge_reads_the_whole_log_not_the_tail", RR + "test_log_facts_are_read_from_the_whole_log_not_the_tail"),
    M("not-run-exits-nonzero", "mr", "    return 1 if status == lib.FAIL else 0", "    return 1 if status != lib.PASS else 0",
      CR + "test_not_run_exits_zero_prints_appnotrun_and_never_a_pass_marker", CR + "test_the_exit_code_is_one_only_for_a_fail"),
    M("not-run-prints-no-appnotrun", "mr", '        say("APPNOTRUN " + reason[:150])', "        pass",
      CR + "test_not_run_exits_zero_prints_appnotrun_and_never_a_pass_marker", MW + "test_the_real_tool_with_no_gpu_is_notrun_through_run_apps"),
    M("proof-says-cuda-for-a-cpu-run", "lib", '    cpu = res.get("platform") == "cpu" or res.get("device") == "cpu"', "    cpu = False",
      PF + "test_a_cpu_platform_prints_a_cpu_device_whatever_the_memory_did", GJ + "test_a_cpu_run_with_plenty_of_memory_growth_is_still_judged_fail_by_the_app_proof", PF + "test_a_cpu_run_prints_a_cpu_device_and_the_judge_fails_it"),
    M("proof-printed-without-a-measurement", "lib", '    if not _num(grown):\n        return None', "    if False:\n        return None",
      PF + "test_no_measurement_or_no_gpu_name_prints_no_proof", CR + "test_a_run_without_a_memory_measurement_prints_no_proof_and_does_not_crash"),
    # the verdicts
    M("vllm-repeat-not-checked", "lib", '    checks.append(_chk("greedy_repeat_identical", res.get("repeat_identical") is True,', '    checks.append(_chk("greedy_repeat_identical", True,',
      JV + "test_each_missing_property_fails_by_its_own_name", JV + "test_case_the_repeat_differs"),
    M("vllm-engine-process-not-checked", "lib", '    checks.append(_chk("engine_in_its_own_process", bool(res.get("engine_children")),', '    checks.append(_chk("engine_in_its_own_process", True,',
      JV + "test_each_missing_property_fails_by_its_own_name", JV + "test_case_no_engine_process"),
    M("vllm-graph-floor-removed", "lib", "VLLM_MIN_GRAPH_SIZES = 8", "VLLM_MIN_GRAPH_SIZES = 0",
      JV + "test_each_missing_property_fails_by_its_own_name", JV + "test_case_seven_graph_sizes"),
    M("vllm-memory-floor-removed", "lib", "VLLM_MIN_PEAK_MIB = 300 ", "VLLM_MIN_PEAK_MIB = 0 ",
      JV + "test_each_missing_property_fails_by_its_own_name", JV + "test_case_memory_did_not_grow"),
    M("vllm-expected-hash-ignored", "lib", '    if expected_sha:\n        checks.append', '    if False:\n        checks.append',
      JV + "test_a_committed_expected_hash_is_enforced_when_there_is_one", RR + "test_a_committed_expected_token_hash_reaches_the_judge"),
    M("trt-tactics-not-checked", "lib", '    checks.append(_chk("tactics_were_timed", _num(b.get("timing_cache_bytes")) and b["timing_cache_bytes"] > 0,', '    checks.append(_chk("tactics_were_timed", True,',
      JT + "test_each_missing_property_fails_by_its_own_name", JT + "test_case_no_tactics_timed"),
    M("trt-tolerance-widened", "lib", "TRT_REL_TOL = 0.01 ", "TRT_REL_TOL = 50.0 ",
      JT + "test_the_tolerance_boundary_is_a_fraction_of_the_reference", JT + "test_case_engine_output_off"),
    M("trt-ort-provider-not-checked", "lib", '        ok_ep = ort.get("first_provider") == "TensorrtExecutionProvider" and', '        ok_ep = True or ort.get("first_provider") == "TensorrtExecutionProvider" and',
      JT + "test_each_missing_property_fails_by_its_own_name", JT + "test_case_ort_ran_on_the_cuda_ep"),
    M("trt-zero-reference-passes", "lib", 'scale > 0 and err <= TRT_REL_TOL * scale, "max_abs_err=%s ref_max_abs=%s tol=%s"', 'scale >= 0 and err <= TRT_REL_TOL * scale, "max_abs_err=%s ref_max_abs=%s tol=%s"',
      JT + "test_a_zero_reference_cannot_pass", RR + "test_a_run_whose_reference_is_all_zero_is_a_fail_with_the_check_named"),
    M("jax-gpu-not-checked", "lib", '    checks.append(_chk("jax_devices_are_gpu", gpu,', '    checks.append(_chk("jax_devices_are_gpu", True,',
      JJ + "test_each_missing_property_fails_by_its_own_name", JJ + "test_case_cpu_backend"),
    M("jax-strict-tolerance-loosened", "lib", "JAX_STRICT_REL = 1e-4 ", "JAX_STRICT_REL = 1e2 ",
      JJ + "test_the_strict_and_the_default_precision_have_their_own_tolerances", JJ + "test_case_matmul_off"),
    M("jax-preallocation-window-widened", "lib", "JAX_PREALLOC_LOW, JAX_PREALLOC_HIGH = 0.68, 0.78 ", "JAX_PREALLOC_LOW, JAX_PREALLOC_HIGH = 0.0, 1.0 ",
      JJ + "test_the_preallocation_window_boundaries", JJ + "test_case_preallocation_50_percent"),
    M("jax-autotuner-not-checked", "lib", '    checks.append(_chk("autotuner_ran", _num(res.get("autotune_bytes")) and res["autotune_bytes"] > 0 and tuned >= 1,', '    checks.append(_chk("autotuner_ran", True,',
      JJ + "test_case_no_autotune_results", JJ + "test_case_only_the_emitter_s_own_kernels_in_the_dump", JJ + "test_each_missing_property_fails_by_its_own_name"),
    M("jax-emitter-kernels-count-as-tuned", "lib", 'tuned = sum(v for k, v in back.items() if k != "NATIVE_EMITTER" and _num(v))', "tuned = sum(v for k, v in back.items() if _num(v))",
      JJ + "test_each_missing_property_fails_by_its_own_name", JJ + "test_case_only_the_emitter_s_own_kernels_in_the_dump"),
    # host against guest
    M("compare-ignores-different-tokens", "lib", '    if h.get("token_sha256") != g.get("token_sha256"):', "    if False:",
      CP + "test_different_greedy_tokens_are_a_candidate_naming_the_first_prompt", CP + "test_cmd_compare_sees_different_tokens_in_two_logs"),
    M("compare-guest-fail-is-a-note", "lib", '"kind": "CANDIDATE", "what": "host PASS, guest FAIL: %s"', '"kind": "NOTE", "what": "host PASS, guest FAIL: %s"',
      CP + "test_a_guest_failure_after_a_host_pass_is_a_candidate_and_a_not_run_is_only_a_note", CP + "test_cmd_compare_reads_two_logs_and_exits_one_on_a_candidate"),
    M("compare-guest-not-run-is-a-candidate", "lib", '"kind": "NOTE", "what": "host PASS, guest NOT_RUN (%s): our own configuration, re-run"', '"kind": "CANDIDATE", "what": "host PASS, guest NOT_RUN (%s): our own configuration, re-run"',
      CP + "test_a_guest_failure_after_a_host_pass_is_a_candidate_and_a_not_run_is_only_a_note", CP + "test_a_not_run_guest_alone_does_not_fail_the_comparison"),
    M("compare-jax-autotuner-loss-ignored", "lib", '    if _num(h.get("autotune_bytes")) and h["autotune_bytes"] > 0 and not (_num(g.get("autotune_bytes")) and g["autotune_bytes"] > 0):', "    if False:",
      CP + "test_jax_preallocation_fraction_and_autotuner", CP + "test_the_autotuner_loss_alone_is_a_candidate"),
    M("extract-judged-accepts-any-status", "lib", 'value.get("status") in (PASS, FAIL, NOT_RUN) and (row is None or value.get("row") == row)', "(row is None or value.get('row') == row)",
      CP + "test_extract_ignores_a_forged_line_with_a_bad_status_and_other_rows", CP + "test_a_forged_status_in_a_log_is_not_a_result"),
    M("extract-judged-ignores-the-row", "lib", 'value.get("status") in (PASS, FAIL, NOT_RUN) and (row is None or value.get("row") == row)', 'value.get("status") in (PASS, FAIL, NOT_RUN)',
      CP + "test_extract_ignores_a_forged_line_with_a_bad_status_and_other_rows", CP + "test_a_log_with_another_row_s_result_gives_nothing"),
    # the matrix wiring
    M("rows-in-listall", "run", """[ "${1:-}" = listall ] && { echo "$APPS" | cut""", """[ "${1:-}" = listall ] && { { echo "$APPS"; echo "$DISCOVERY"; } | cut""",
      MW + "test_the_rows_are_listed_but_not_in_listall_and_in_no_gate_list", MW + "test_list_is_listall_plus_exactly_the_three_rows"),
    M("rows-in-all", "run", """[ "${1:-}" = all ] && set -- $(echo "$APPS" | cut""", """[ "${1:-}" = all ] && set -- $({ echo "$APPS"; echo "$DISCOVERY"; } | cut""",
      MW + "test_all_and_listall_leave_the_discovery_rows_out", MW + "test_all_runs_no_discovery_row"),
    M("rows-not-listed", "run", """[ "${1:-}" = list ] && { { echo "$APPS"; echo "$DISCOVERY"; } | cut""", """[ "${1:-}" = list ] && { { echo "$APPS"; } | cut""",
      MW + "test_the_rows_are_listed_but_not_in_listall_and_in_no_gate_list", MW + "test_list_is_listall_plus_exactly_the_three_rows"),
    M("appnotrun-with-a-crash-is-notrun", "run", '[ -n "$appnr" ] && [ "$rc" -eq 0 ] &&', '[ -n "$appnr" ] &&',
      MW + "test_a_crash_that_printed_appnotrun_is_still_a_fail", MW + "test_a_nonzero_exit_that_printed_appnotrun_is_still_a_fail"),
    M("appnotrun-ignored", "run", "{ v=NOTRUN; note=\"not-run:$appnr\"; }", "{ :; }",
      MW + "test_a_not_run_is_notrun_with_its_reason_and_never_a_pass_or_a_fail", MW + "test_the_real_tool_with_no_gpu_is_notrun_through_run_apps"),
    M("matrix-table-rows-are-gate-rows", "mt", '    if app in DISCOVERY:\n        return "DISCOVERY"', '    if False:\n        return "DISCOVERY"',
      MW + "test_the_matrix_table_classes_them_discovery_and_the_gpu_judge_has_a_class_for_each", MW + "test_a_discovery_row_never_reaches_the_gate_list"),
    M("gpu-judge-forgets-vllm", "rows", '    "vllm": ("a+proof", "framework", {"framework": "vllm", "min_mib": 300}, ("gpu",), dict(LONG), "FAIL"),\n', "",
      MW + "test_the_matrix_table_classes_them_discovery_and_the_gpu_judge_has_a_class_for_each", GJ + "test_the_gpu_run_of_every_row_passes_with_the_census_and_the_engines"),
    M("gpu-judge-floor-of-vllm-zero", "rows", '"vllm": ("a+proof", "framework", {"framework": "vllm", "min_mib": 300}', '"vllm": ("a+proof", "framework", {"framework": "vllm", "min_mib": 0}',
      PF + "test_the_floors_in_the_class_table_are_enforced_by_the_judge", GJ + "test_too_little_device_memory_is_a_fail"),
    M("gpu-judge-asks-no-engines-of-jax", "rows", '"jax": ("b", "framework", {"framework": "jax", "min_mib": 1000}, ("gpu",)', '"jax": ("b", "framework", {"framework": "jax", "min_mib": 1000}, ()',
      GJ + "test_a_jax_run_the_host_saw_idle_is_a_fail", GJ + "test_an_idle_host_engine_fails_every_row_whose_census_is_fine"),
    M("hook-does-not-push-the-rows", "hook", '[ -n "$MLR" ] && python3 -B -I "$HERE/mlrows/mlrows.py" push', '[ -n "$MLR" ] && : python3 -B -I "$HERE/mlrows/mlrows.py" push',
      HP + "test_the_rows_asked_for_and_staged_are_pushed_with_the_tool_dir", HP + "test_a_row_that_was_not_staged_is_left_out_and_the_tool_dir_still_goes"),
    M("hook-forgets-a-row", "hook", "case $a in vllm|tensorrt|jax) printf '%s,' \"$a\";; esac; done); MLR=${MLR%,}\n[ -n \"$MLR\" ] && python3", "case $a in vllm|tensorrt) printf '%s,' \"$a\";; esac; done); MLR=${MLR%,}\n[ -n \"$MLR\" ] && python3",
      HP + "test_the_rows_asked_for_and_staged_are_pushed_with_the_tool_dir", MW + "test_the_hook_pushes_the_rows_it_was_asked_for_before_the_guest_runs_anything"),
    M("matrix-does-not-set-up-the-rows", "matrix", "    python3 -B -I \"$HERE/mlrows/mlrows.py\" setup --root \"$MLROWS\" --rows \"$MLR\" | tee -a \"$R/mlrows_setup.log\"\n", "    :\n",
      MW + "test_the_shell_glue_pushes_sets_up_and_compares_the_rows", MW + "test_the_host_run_sets_the_rows_up_before_it_takes_the_gpu_lock"),
    M("matrix-does-not-compare", "matrix", 'python3 -B -I "$HERE/mlrows/mlrows.py" compare "$row" "$R/$row.host.log" "$R/$row.guest.log" > "$R/$row.compare.txt" 2>&1;', ': "$R/$row.host.log" "$R/$row.guest.log" > "$R/$row.compare.txt" 2>&1;',
      MW + "test_the_shell_glue_pushes_sets_up_and_compares_the_rows", MW + "test_the_matrix_compares_host_and_guest_for_each_row_after_the_guest_runs"),

    # 2026-11-20: the private Python 3.12 of the host (private_python.py) and the interpreter choice (mlrows.py)
    M("private-python-is-shared", "pp", '["./configure", "--prefix=" + str(prefix), "--with-ensurepip=no"]', '["./configure", "--prefix=" + str(prefix), "--with-ensurepip=no", "--enable-shared"]',
      PP + "test_the_recipe_is_a_private_static_altinstall_with_the_prefix_derived_from_one_constant", PP + "test_a_build_runs_the_steps_in_order_and_writes_the_marker_last"),
    M("altinstall-becomes-install", "pp", '("install", ["make", "altinstall"], src)', '("install", ["make", "install"], src)',
      PP + "test_the_recipe_is_a_private_static_altinstall_with_the_prefix_derived_from_one_constant", PP + "test_a_build_runs_the_steps_in_order_and_writes_the_marker_last"),
    M("parallelism-dropped", "pp", '("make", ["make", "-j%d" % jobs], src)', '("make", ["make"], src)',
      PP + "test_the_recipe_is_a_private_static_altinstall_with_the_prefix_derived_from_one_constant", PP + "test_a_build_runs_the_steps_in_order_and_writes_the_marker_last"),
    M("prefix-is-not-private", "pp", "    return Path(os.path.abspath(root)) / PRIVATE_DIR\n", "    return Path(os.path.abspath(root))\n",
      PP + "test_the_recipe_is_a_private_static_altinstall_with_the_prefix_derived_from_one_constant", PP + "test_it_only_ever_writes_below_its_own_prefix_and_never_replaces_the_system_python"),
    M("built-python-not-checked", "pp", '        if not got or got["missing"] or ".".join(map(str, got["version"])) != PY_VERSION:\n            shutil.rmtree(prefix, ignore_errors=True)\n',
      '        if False:\n            shutil.rmtree(prefix, ignore_errors=True)\n',
      PP + "test_a_built_python_is_checked_for_its_version_and_for_every_module_a_missing_header_would_drop", PP + "test_a_marker_that_is_written_before_the_probe_would_bless_a_broken_build"),
    M("cached-python-with-a-missing-module-accepted", "pp", '\n    if not got or got["missing"] or ".".join(map(str, got["version"])) != PY_VERSION:\n        return None',
      '\n    if not got or ".".join(map(str, got["version"])) != PY_VERSION:\n        return None',
      PP + "test_a_cached_python_with_a_missing_module_or_the_wrong_version_is_not_usable", IC + "test_an_old_host_python_without_a_complete_private_one_stays_and_the_refusal_stays"),
    M("cached-python-of-another-version-accepted", "pp", '\n    if not got or got["missing"] or ".".join(map(str, got["version"])) != PY_VERSION:\n        return None',
      '\n    if not got or got["missing"]:\n        return None',
      PP + "test_a_cached_python_with_a_missing_module_or_the_wrong_version_is_not_usable", IC + "test_an_old_host_python_without_a_complete_private_one_stays_and_the_refusal_stays"),
    M("marker-not-required", "pp", '    if not marker or marker.get("version") != PY_VERSION or marker.get("recipe") != recipe_id(prefix_of(root)):', "    if False:",
      PP + "test_a_prefix_without_a_marker_is_never_used_and_is_replaced", PP + "test_a_marker_of_another_recipe_or_version_or_a_missing_interpreter_is_a_rebuild", IC + "test_an_old_host_python_without_a_complete_private_one_stays_and_the_refusal_stays"),
    M("a-second-ensure-rebuilds", "pp", '    if got:\n        return result("ok", "cached:" + PY_VERSION, root, False, PY_VERSION)', '    if False:\n        return result("ok", "cached:" + PY_VERSION, root, False, PY_VERSION)',
      PP + "test_a_second_ensure_builds_nothing", PP + "test_a_cached_build_makes_no_network_apt_or_policy_call"),
    M("if-needed-ignored", "pp", "    if if_needed:\n        pins =", "    if False:\n        pins =",
      PP + "test_a_host_that_already_has_python_312_builds_nothing_with_if_needed", PP + "test_main_on_a_host_that_has_python_312_builds_nothing_and_on_an_old_one_builds"),
    M("if-needed-upper-bound-dropped", "pp", 'if tuple(pins["min"]) <= sys_version[:2] < tuple(pins["max_exclusive"]):', 'if tuple(pins["min"]) <= sys_version[:2]:',
      PP + "test_a_host_python_newer_than_the_pinned_range_is_not_good_enough_either", PP + "test_main_on_a_host_that_has_python_312_builds_nothing_and_on_an_old_one_builds"),
    M("if-needed-flag-not-passed", "pp", "r = ensure(a.root, if_needed=a.if_needed, jobs=a.jobs)", "r = ensure(a.root, jobs=a.jobs)",
      PP + "test_main_passes_the_flags_through", PP + "test_main_on_a_host_that_has_python_312_builds_nothing_and_on_an_old_one_builds"),
    M("a-refused-anchor-is-accepted", "pp", "    if rc == 0 and Path(out).is_file():\n        return True, \"\"", "    if True:\n        return True, \"\"",
      PP + "test_an_artifact_the_manifest_does_not_anchor_is_refused_by_name_before_apt_runs", PP + "test_a_transfer_that_fails_without_a_trust_code_is_named_by_its_exit_code"),
    M("dpkg-status-ignored", "pp", '        if rc != 0 or "install ok installed" not in text:', "        if False:",
      PP + "test_build_dependencies_are_installed_through_bash_as_root_and_rechecked", PP + "test_build_dependencies_nobody_may_install_are_a_named_not_run_and_nothing_builds"),
    M("apt-without-the-shim-variables", "pp", "install_packages(run, absent, build_env(environ, apt=True))", "install_packages(run, absent, build_env(environ))",
      PP + "test_the_apt_call_gets_the_payload_shim_variables_and_nothing_else_does", PP + "test_build_dependencies_are_installed_through_bash_as_root_and_rechecked"),
    M("build-env-passes-everything", "pp", '    env = {k: v for k, v in source.items() if k in ENV_KEEP or (trust and k.startswith("KF_TRUST_")) or (apt and k in APT_SHIM_ENV)}\n', "    env = dict(source)\n",
      PP + "test_the_build_runs_in_an_allowlisted_environment", PP + "test_the_apt_call_gets_the_payload_shim_variables_and_nothing_else_does"),
    M("escaping-members-extracted", "pp", '        if os.path.isabs(m.name) or ".." in Path(m.name).parts or not (target == root or target.startswith(root + os.sep)):', "        if False:",
      PP + "test_a_source_tarball_that_leaves_its_directory_is_refused_and_extracts_nothing_outside", PP + "test_ensure_refuses_hostile_or_wrong_tarballs_before_building"),
    M("escaping-links-extracted", "pp", "            if os.path.isabs(m.linkname) or not (link == root or link.startswith(root + os.sep)):", "            if False:",
      PP + "test_a_source_tarball_that_leaves_its_directory_is_refused_and_extracts_nothing_outside", PP + "test_a_refused_tarball_is_a_named_not_run_and_the_build_never_starts"),
    M("device-nodes-extracted", "pp", "        elif not (m.isreg() or m.isdir()):", "        elif False:",
      PP + "test_a_source_tarball_that_leaves_its_directory_is_refused_and_extracts_nothing_outside", PP + "test_ensure_refuses_hostile_or_wrong_tarballs_before_building"),
    M("any-top-directory-accepted", "pp", '            if tops != {"Python-" + PY_VERSION}:', "            if False:",
      PP + "test_a_tarball_with_another_top_directory_or_two_is_refused_and_a_good_one_extracts", PP + "test_ensure_refuses_hostile_or_wrong_tarballs_before_building"),
    M("an-old-prefix-is-built-over", "pp", "    if prefix.exists():\n        shutil.rmtree(prefix)", "    if False:\n        shutil.rmtree(prefix)",
      PP + "test_a_prefix_without_a_marker_is_never_used_and_is_replaced", PP + "test_a_marker_of_another_recipe_or_version_or_a_missing_interpreter_is_a_rebuild"),
    M("a-failed-step-keeps-its-prefix", "pp", '                shutil.rmtree(prefix, ignore_errors=True)\n                return result("FAIL", "%s_%s:%s"', '                return result("FAIL", "%s_%s:%s"',
      PP + "test_a_failed_step_is_a_named_fail_that_leaves_no_prefix_and_no_marker", PP + "test_a_timed_out_step_says_so"),
    M("pin-entry-says-binary", "pp", '"scope": "source"', '"scope": "upstream-artifact"',
      PP + "test_the_manifest_entry_pin_entry_prints_is_valid_for_kf_trust_and_the_lint", PP + "test_pin_entry_hashes_the_file_and_cross_checks_the_release_pages_md5"),
    M("chooser-excludes-the-lower-edge", "mr", 'if not root or lo <= tuple(own["version"][:2]) < hi:', 'if not root or lo < tuple(own["version"][:2]) < hi:',
      IC + "test_a_host_python_inside_the_range_is_used_and_nothing_is_probed", IC + "test_the_edges_of_the_pinned_range_are_inside_it"),
    M("chooser-never-takes-the-private-python", "mr", '    if got:\n        return {"exe": str(pp.interpreter_of(root))', '    if False:\n        return {"exe": str(pp.interpreter_of(root))',
      IC + "test_an_old_host_python_takes_the_private_one_when_it_is_complete", IC + "test_setup_end_to_end_on_an_old_host_makes_the_venv_with_the_private_python_and_records_it"),
    M("venv-made-with-the-running-python", "mr", '[interp["exe"], "-m", "venv", str(venv)]', '[sys.executable, "-m", "venv", str(venv)]',
      IC + "test_setup_makes_the_venv_with_the_chosen_interpreter_and_records_it", IC + "test_setup_end_to_end_on_an_old_host_makes_the_venv_with_the_private_python_and_records_it"),
    M("facts-take-the-running-pythons-version", "mr", 'tuple(interp["version"][:3]),', "tuple(sys.version_info[:3]),",
      IC + "test_facts_take_the_python_version_of_the_interpreter_that_makes_the_venvs", IC + "test_setup_end_to_end_on_an_old_host_makes_the_venv_with_the_private_python_and_records_it"),
    M("facts-take-the-running-pythons-headers", "mr", '    inc = interp["include"]\n', '    inc = sysconfig.get_paths().get("include") or ""\n',
      IC + "test_facts_take_python_h_from_the_include_directory_of_that_interpreter", IC + "test_a_private_python_without_its_headers_is_not_enough_for_vllm"),
    M("setup-forgets-the-private-python", "mr", '"python_private": bool(interp["private"]),', '"python_private": False,',
      IC + "test_setup_makes_the_venv_with_the_chosen_interpreter_and_records_it", IC + "test_setup_end_to_end_on_an_old_host_makes_the_venv_with_the_private_python_and_records_it"),
    M("no-hint-beside-the-refusal", "mr", '        if status == lib.NOT_RUN and reason.startswith("python_too_old"):\n            say("MLROWS_HINT "', '        if False:\n            say("MLROWS_HINT "',
      IC + "test_a_refused_old_python_names_the_way_out_without_changing_the_setup_line", IC + "test_setup_end_to_end_on_an_old_host_without_the_private_python_refuses_by_name_and_says_how_to_get_it"),
    M("setup-hands-no-interpreter-to-the-venv-maker", "mr", "status, reason = setup_row(row, a.root, facts, pins, a.flavor, interp=interp)", "status, reason = setup_row(row, a.root, facts, pins, a.flavor)",
      IC + "test_check_run_and_setup_all_choose_the_interpreter_from_the_root", IC + "test_setup_end_to_end_on_an_old_host_makes_the_venv_with_the_private_python_and_records_it"),
    M("run-ignores-the-private-python", "mr", "    facts = collect_facts(a.root, choose_interpreter(a.root, pins))\n    negctl", "    facts = collect_facts(a.root)\n    negctl",
      IC + "test_check_and_run_each_choose_the_interpreter_from_the_root", IC + "test_check_run_and_setup_all_choose_the_interpreter_from_the_root"),
    M("check-ignores-the-private-python", "mr", 'facts = collect_facts(a.root or ".", choose_interpreter(a.root, pins))', 'facts = collect_facts(a.root or ".")',
      IC + "test_check_and_run_each_choose_the_interpreter_from_the_root", IC + "test_check_run_and_setup_all_choose_the_interpreter_from_the_root"),
    M("push-never-ships-the-private-python", "mr", "    private = needs_private_python(a.root, targets)", "    private = False",
      PV + "test_the_private_python_travels_with_a_venv_that_needs_it_and_the_guest_gets_the_hosts_path", PV + "test_a_failing_link_fails_the_push"),
    M("push-ships-it-for-every-venv", "mr", '    return bool(pp.read_marker(root)) and any((read_setup(root, t.name) or {}).get("python_private") for t in targets)', "    return bool(pp.read_marker(root))",
      PV + "test_a_venv_made_with_the_system_python_ships_no_interpreter_and_no_link", PV + "test_a_staged_row_without_a_setup_state_ships_no_interpreter"),
    M("push-links-nothing-into-the-guest", "mr", "    if private and host_root != bundle:", "    if False:",
      PV + "test_the_private_python_travels_with_a_venv_that_needs_it_and_the_guest_gets_the_hosts_path", PV + "test_a_failing_link_fails_the_push"),
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


def run_tests(root):
    done = subprocess.run([sys.executable, "-B", "-c", COLLECT, SUITE.rstrip(".")], cwd=root / "scripts/ci", capture_output=True, text=True, timeout=900)
    line = next((s for s in done.stdout.splitlines() if s.startswith("KF_MUTATION_RESULT=")), None)
    if line is None:
        raise SystemExit("harness failure: " + done.stderr[-600:] + done.stdout[-300:])
    return json.loads(line.split("=", 1)[1])


def one(parent, mutant):
    name, key, old, new, wanted = mutant
    text = SOURCES[key]
    if text.count(old) != 1:
        return name, ["anchor x%d: %r" % (text.count(old), old[:50])], wanted
    root = tree(parent, "m", {key: text.replace(old, new)})
    killers = sorted(set(k.split(" (")[0].split(" [")[0] for k in run_tests(root)["killers"]))     # a failing subTest is the test it belongs to
    shutil.rmtree(root, ignore_errors=True)
    return name, killers, wanted


def main():
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument("--jobs", type=int, default=4)
    ap.add_argument("only", nargs="?", default="")
    a = ap.parse_args()
    chosen = [m for m in MUTANTS if a.only in m[0]]
    bad = 0
    with tempfile.TemporaryDirectory(prefix="kf-mlrows-mutants-") as parent:
        base = run_tests(tree(parent, "base", {}))
        if base["killers"]:
            print("BASELINE RED: %s" % base["killers"], file=sys.stderr)
            return 2
        print("baseline green: %d tests" % base["tests"], flush=True)
        with concurrent.futures.ThreadPoolExecutor(a.jobs) as pool:
            for name, killers, wanted in pool.map(lambda m: one(parent, m), chosen):
                short = {k.split(".", 1)[-1] for k in killers}
                named = sorted({w for w in wanted if w.split(".", 1)[-1] in short or w in killers})
                ok = len(killers) >= 2 and len(named) >= 2 and not killers[0].startswith("anchor")      # two DISTINCT named killers for EVERY mutant, no exception
                print("%s %s: %d killers (%d named)" % ("PASS" if ok else "WEAK", name, len(killers), len(named)), flush=True)
                if not ok:
                    bad += 1
                    print("   killers: %s" % [k.split(".", 1)[-1] for k in killers[:12]], flush=True)
    print("%d/%d mutants killed" % (len(chosen) - bad, len(chosen)))
    return 1 if bad else 0


if __name__ == "__main__":
    sys.exit(main())
