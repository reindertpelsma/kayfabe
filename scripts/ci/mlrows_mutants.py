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
(PI, FA, FL, LK, MO, JV, JT, JJ, PF, EN, RR, MM, CR, SE, CP, PU, MW, GJ, TP) = (SUITE + n for n in (
    "Pins.", "Facts.", "Flavour.", "Locks.", "Model.", "JudgeVllm.", "JudgeTensorrt.", "JudgeJax.", "Proof.", "Env.", "RunRow.", "MemoryRecord.", "CmdRun.", "Setup.", "Compare.",
    "Push.", "MatrixWiring.", "GpuUseJudge.", "TrustPins."))


def M(name, key, old, new, *tests):
    return (name, key, old, new, list(tests))


MUTANTS = [
    # the flavour: which wheel a machine may take
    M("flavour-floor-off-by-one", "lib", "if tuple(cuda) >= need:", "if tuple(cuda) > need:",
      FL + "test_the_cuda_boundary_is_exact", FL + "test_a_cuda_13_driver_takes_the_newest_flavour_of_every_row"),
    M("turing-floor-ignored", "lib", "    if tuple(cc) < floor:\n", "    if False:\n",
      FL + "test_older_than_turing_is_not_run_with_the_cc_in_the_reason", SE + "test_hardware_and_environment_preconditions_are_named_not_runs_and_nothing_runs"),
    M("vram-floor-ignored", "lib", 'if facts.get("total_mib") is not None and facts["total_mib"] < spec["min_vram_mib"]:', "if False:",
      FL + "test_no_gpu_and_too_little_memory", SE + "test_hardware_and_environment_preconditions_are_named_not_runs_and_nothing_runs" if False else FL + "test_no_gpu_and_too_little_memory", RR + "test_hardware_that_cannot_take_the_row_is_not_run_before_anything_starts"),
    M("python-range-ignored", "lib", '    if tuple(py[:2]) < pmin:\n        return _nr("python_too_old', '    if False:\n        return _nr("python_too_old',
      FL + "test_python_range", FL + "test_a_forced_flavour_is_checked_against_the_driver_and_never_overrides_a_hard_not_run" if False else FL + "test_python_range"),
    M("unknown-driver-cuda-is-assumed-new", "lib", '    if cuda is None:\n        return {"status": "env"', '    if cuda is None:\n        cuda = (99, 0)\n    if False:\n        return {"status": "env"',
      FL + "test_an_unknown_driver_cuda_is_an_env_problem_not_a_not_run", FL + "test_a_driver_below_cuda_12_is_not_run_for_every_row" if False else FL + "test_an_unknown_driver_cuda_is_an_env_problem_not_a_not_run"),
    M("forced-flavour-skips-the-driver-check", "mr", '        if facts.get("driver_cuda") and tuple(facts["driver_cuda"]) < need:', "        if False:",
      FL + "test_a_forced_flavour_is_checked_against_the_driver_and_never_overrides_a_hard_not_run", FL + "test_a_forced_flavour_is_checked_against_the_driver_and_never_overrides_a_hard_not_run"),
    # the hashed locks and the install
    M("lock-unhashed-accepted", "lib", '        if not r["hashes"]:', "        if False:",
      LK + "test_a_requirement_without_a_hash_is_refused", SE + "test_a_lock_that_lost_a_hash_is_refused_before_pip_runs", LK + "test_every_requirement_has_a_digest_and_the_locks_are_full_closures" if False else LK + "test_a_requirement_without_a_hash_is_refused"),
    M("lock-foreign-index-accepted", "lib", "        elif host.group(1) not in ALLOWED_INDEX_HOSTS:", "        elif False:",
      LK + "test_a_foreign_or_plain_http_index_is_refused", LK + "test_only_tensorrt_locks_use_nvidia_s_index_and_only_over_https" if False else LK + "test_a_foreign_or_plain_http_index_is_refused"),
    M("lock-plain-http-index-accepted", "lib", '        if not host:\n            out.append("index is not https: " + url)', '        if False:\n            out.append("index is not https: " + url)',
      LK + "test_a_foreign_or_plain_http_index_is_refused", LK + "test_a_foreign_or_plain_http_index_is_refused"),
    M("lock-without-an-index-accepted", "lib", '    if not any(k == "index-url" for k, _ in indexes):', "    if False:",
      LK + "test_a_foreign_or_plain_http_index_is_refused", LK + "test_a_foreign_or_plain_http_index_is_refused"),
    M("lock-duplicate-accepted", "lib", '        if r["name"] in names:', "        if False:",
      LK + "test_a_duplicate_name_is_refused_and_an_empty_lock_too", LK + "test_a_duplicate_name_is_refused_and_an_empty_lock_too"),
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
    M("hash-mismatch-leaves-an-ok-state", "mr", "    if rc != 0:\n        why = \"pip_timeout\"", "    if False:\n        why = \"pip_timeout\"",
      SE + "test_a_hash_mismatch_is_a_named_fail_and_leaves_no_ok_state", SE + "test_a_hash_mismatch_is_a_named_fail_and_leaves_no_ok_state" if False else SE + "test_a_hash_mismatch_is_a_named_fail_and_leaves_no_ok_state"),
    M("setup-ignores-the-c-compiler", "mr", '    if spec.get("needs_c_compiler") and not (facts.get("gcc") and facts.get("python_h")):', "    if False:",
      SE + "test_hardware_and_environment_preconditions_are_named_not_runs_and_nothing_runs", SE + "test_hardware_and_environment_preconditions_are_named_not_runs_and_nothing_runs"),
    M("setup-ignores-the-disk", "mr", '    if free is not None and free < spec["need_disk_gb"]:', "    if False:",
      SE + "test_hardware_and_environment_preconditions_are_named_not_runs_and_nothing_runs", SE + "test_hardware_and_environment_preconditions_are_named_not_runs_and_nothing_runs"),
    M("second-setup-reinstalls", "mr", '    if state and state.get("status") == "ok" and state.get("lock_sha256") == lsha', '    if False and state and state.get("status") == "ok" and state.get("lock_sha256") == lsha',
      SE + "test_a_second_setup_with_the_same_lock_does_nothing", SE + "test_a_second_setup_with_the_same_lock_does_nothing"),
    M("changed-lock-keeps-the-old-venv", "mr", 'and state.get("lock_sha256") == lsha and state.get("flavour") == flavour', 'and state.get("flavour") == flavour',
      SE + "test_a_second_setup_with_the_same_lock_does_nothing" if False else SE + "test_a_changed_lock_starts_clean", SE + "test_a_changed_lock_starts_clean"),
    # the model
    M("model-digest-not-checked", "lib", '    if got != entry["digest"]:', "    if False:",
      MO + "test_bytes_are_checked_by_size_then_by_digest", MO + "test_a_tampered_file_is_refused_and_not_left_behind", SE + "test_vllm_fetches_the_model_after_pip_and_a_bad_model_fails_the_setup" if False else MO + "test_a_tampered_file_is_refused_and_not_left_behind"),
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
    M("model-bytes-written-before-the-check", "mr", '        bad = lib.check_model_bytes(entry, data)\n        if bad:\n            return False, "hash_mismatch: " + bad\n',
      '        bad = None\n',
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
      EN + "test_the_venv_s_libraries_come_first_then_the_callers", EN + "test_the_venv_s_libraries_come_first_then_the_callers" if False else EN + "test_the_venv_s_libraries_come_first_then_the_callers"),
    # running and judging
    M("timeout-not-reported", "mr", "    timed_out = bool(fired)", "    timed_out = False",
      RR + "test_a_row_that_hangs_is_killed_and_is_a_fail_with_the_timeout", RR + "test_the_timeout_fail_names_the_limit"),
    M("memory-peak-is-the-last-sample", "mr", '"mem_peak_mib": max(used), "mem_growth_mib": max(used) - base', '"mem_peak_mib": used[-1], "mem_growth_mib": used[-1] - base',
      MM + "test_growth_is_the_peak_over_the_baseline_of_the_first_samples", RR + "test_a_good_run_is_judged_and_gets_the_runner_s_memory_record"),
    M("memory-baseline-is-the-max", "mr", "base = min(used[:baseline_samples])", "base = max(used[:baseline_samples])",
      MM + "test_growth_is_the_peak_over_the_baseline_of_the_first_samples", RR + "test_a_good_run_is_judged_and_gets_the_runner_s_memory_record"),
    M("whole-log-not-read", "mr", '    full = log_path.read_text(errors="replace") if log_path.exists() else text', "    full = text",
      RR + "test_the_whole_log_is_kept_and_the_judge_reads_the_whole_log_not_the_tail", RR + "test_the_whole_log_is_kept_and_the_judge_reads_the_whole_log_not_the_tail" if False else RR + "test_the_whole_log_is_kept_and_the_judge_reads_the_whole_log_not_the_tail"),
    M("not-run-exits-nonzero", "mr", "    return 1 if status == lib.FAIL else 0", "    return 1 if status != lib.PASS else 0",
      CR + "test_not_run_exits_zero_prints_appnotrun_and_never_a_pass_marker", CR + "test_the_exit_code_is_one_only_for_a_fail"),
    M("not-run-prints-no-appnotrun", "mr", '        say("APPNOTRUN " + reason[:150])', "        pass",
      CR + "test_not_run_exits_zero_prints_appnotrun_and_never_a_pass_marker", MW + "test_the_real_tool_with_no_gpu_is_notrun_through_run_apps"),
    M("proof-says-cuda-for-a-cpu-run", "lib", '    cpu = res.get("platform") == "cpu" or res.get("device") == "cpu"', "    cpu = False",
      PF + "test_a_cpu_run_prints_a_cpu_device_and_the_judge_fails_it", GJ + "test_the_cpu_forms_are_judged_fail_by_the_app_proof_alone", GJ + "test_the_negative_control_of_each_row_is_control_ok"),
    M("proof-printed-without-a-measurement", "lib", '    if not _num(grown):\n        return None', "    if False:\n        return None",
      PF + "test_no_measurement_or_no_gpu_name_prints_no_proof", RR + "test_a_missing_nvidia_smi_leaves_the_memory_unknown_and_the_row_fails_its_memory_check" if False else PF + "test_no_measurement_or_no_gpu_name_prints_no_proof"),
    # the verdicts
    M("vllm-repeat-not-checked", "lib", '    checks.append(_chk("greedy_repeat_identical", res.get("repeat_identical") is True,', '    checks.append(_chk("greedy_repeat_identical", True,',
      JV + "test_each_missing_property_fails_by_its_own_name", JV + "test_case_the_repeat_differs"),
    M("vllm-engine-process-not-checked", "lib", '    checks.append(_chk("engine_in_its_own_process", bool(res.get("engine_children")),', '    checks.append(_chk("engine_in_its_own_process", True,',
      JV + "test_each_missing_property_fails_by_its_own_name", JV + "test_case_no_engine_process"),
    M("vllm-graph-floor-removed", "lib", "VLLM_MIN_GRAPH_SIZES = 8", "VLLM_MIN_GRAPH_SIZES = 0",
      JV + "test_each_missing_property_fails_by_its_own_name", JV + "test_case_seven_graph_sizes"),
    M("vllm-memory-floor-removed", "lib", "VLLM_MIN_PEAK_MIB = 300 ", "VLLM_MIN_PEAK_MIB = 0 ",
      JV + "test_each_missing_property_fails_by_its_own_name", JV + "test_case_memory_did_not_grow", PF + "test_the_floors_in_the_class_table_are_enforced_by_the_judge" if False else JV + "test_case_memory_did_not_grow"),
    M("vllm-expected-hash-ignored", "lib", '    if expected_sha:\n        checks.append', '    if False:\n        checks.append',
      JV + "test_a_committed_expected_hash_is_enforced_when_there_is_one", JV + "test_a_committed_expected_hash_is_enforced_when_there_is_one" if False else JV + "test_a_committed_expected_hash_is_enforced_when_there_is_one"),
    M("trt-tactics-not-checked", "lib", '    checks.append(_chk("tactics_were_timed", _num(b.get("timing_cache_bytes")) and b["timing_cache_bytes"] > 0,', '    checks.append(_chk("tactics_were_timed", True,',
      JT + "test_each_missing_property_fails_by_its_own_name", JT + "test_case_no_tactics_timed"),
    M("trt-tolerance-widened", "lib", "TRT_REL_TOL = 0.05 ", "TRT_REL_TOL = 50.0 ",
      JT + "test_the_tolerance_boundary_is_a_fraction_of_the_reference", JT + "test_case_engine_output_off"),
    M("trt-ort-provider-not-checked", "lib", '        ok_ep = ort.get("first_provider") == "TensorrtExecutionProvider" and', '        ok_ep = True or ort.get("first_provider") == "TensorrtExecutionProvider" and',
      JT + "test_each_missing_property_fails_by_its_own_name", JT + "test_case_ort_ran_on_the_cuda_ep"),
    M("trt-zero-reference-passes", "lib", 'scale > 0 and err <= TRT_REL_TOL * scale, "max_abs_err=%s ref_max_abs=%s tol=%s"', 'scale >= 0 and err <= TRT_REL_TOL * scale, "max_abs_err=%s ref_max_abs=%s tol=%s"',
      JT + "test_a_zero_reference_cannot_pass", JT + "test_a_zero_reference_cannot_pass" if False else JT + "test_a_zero_reference_cannot_pass"),
    M("jax-gpu-not-checked", "lib", '    checks.append(_chk("jax_devices_are_gpu", gpu,', '    checks.append(_chk("jax_devices_are_gpu", True,',
      JJ + "test_each_missing_property_fails_by_its_own_name", JJ + "test_case_cpu_backend"),
    M("jax-strict-tolerance-loosened", "lib", "JAX_STRICT_REL = 1e-4 ", "JAX_STRICT_REL = 1e2 ",
      JJ + "test_the_strict_and_the_default_precision_have_their_own_tolerances", JJ + "test_case_matmul_off"),
    M("jax-preallocation-window-widened", "lib", "JAX_PREALLOC_LOW, JAX_PREALLOC_HIGH = 0.68, 0.78 ", "JAX_PREALLOC_LOW, JAX_PREALLOC_HIGH = 0.0, 1.0 ",
      JJ + "test_the_preallocation_window_boundaries", JJ + "test_case_preallocation_50_percent"),
    M("jax-autotuner-not-checked", "lib", '    checks.append(_chk("autotuner_ran", _num(res.get("autotune_bytes")) and res["autotune_bytes"] > 0 and tuned >= 1,', '    checks.append(_chk("autotuner_ran", True,',
      JJ + "test_each_missing_property_fails_by_its_own_name", JJ + "test_case_no_autotune_results", JJ + "test_case_only_the_emitter_s_own_kernels_in_the_dump"),
    M("jax-emitter-kernels-count-as-tuned", "lib", 'tuned = sum(v for k, v in back.items() if k != "NATIVE_EMITTER" and _num(v))', "tuned = sum(v for k, v in back.items() if _num(v))",
      JJ + "test_each_missing_property_fails_by_its_own_name", JJ + "test_case_only_the_emitter_s_own_kernels_in_the_dump"),
    # host against guest
    M("compare-ignores-different-tokens", "lib", '    if h.get("token_sha256") != g.get("token_sha256"):', "    if False:",
      CP + "test_different_greedy_tokens_are_a_candidate_naming_the_first_prompt", CP + "test_cmd_compare_sees_different_tokens_in_two_logs"),
    M("compare-guest-fail-is-a-note", "lib", '"kind": "CANDIDATE", "what": "host PASS, guest FAIL: %s"', '"kind": "NOTE", "what": "host PASS, guest FAIL: %s"',
      CP + "test_a_guest_failure_after_a_host_pass_is_a_candidate_and_a_not_run_is_only_a_note", CP + "test_cmd_compare_reads_two_logs_and_exits_one_on_a_candidate"),
    M("compare-guest-not-run-is-a-candidate", "lib", '"kind": "NOTE", "what": "host PASS, guest NOT_RUN (%s): our own configuration, re-run"', '"kind": "CANDIDATE", "what": "host PASS, guest NOT_RUN (%s): our own configuration, re-run"',
      CP + "test_a_guest_failure_after_a_host_pass_is_a_candidate_and_a_not_run_is_only_a_note", CP + "test_a_missing_side_is_a_note_never_a_candidate" if False else CP + "test_a_guest_failure_after_a_host_pass_is_a_candidate_and_a_not_run_is_only_a_note"),
    M("compare-jax-autotuner-loss-ignored", "lib", '    if _num(h.get("autotune_bytes")) and h["autotune_bytes"] > 0 and not (_num(g.get("autotune_bytes")) and g["autotune_bytes"] > 0):', "    if False:",
      CP + "test_jax_preallocation_fraction_and_autotuner", CP + "test_jax_preallocation_fraction_and_autotuner" if False else CP + "test_jax_preallocation_fraction_and_autotuner"),
    M("extract-judged-accepts-any-status", "lib", 'value.get("status") in (PASS, FAIL, NOT_RUN) and (row is None or value.get("row") == row)', "(row is None or value.get('row') == row)",
      CP + "test_extract_ignores_a_forged_line_with_a_bad_status_and_other_rows", CP + "test_a_forged_status_in_a_log_is_not_a_result"),
    M("extract-judged-ignores-the-row", "lib", 'value.get("status") in (PASS, FAIL, NOT_RUN) and (row is None or value.get("row") == row)', 'value.get("status") in (PASS, FAIL, NOT_RUN)',
      CP + "test_extract_ignores_a_forged_line_with_a_bad_status_and_other_rows", CP + "test_a_log_with_another_row_s_result_gives_nothing"),
    # the matrix wiring
    M("rows-in-listall", "run", """[ "${1:-}" = listall ] && { echo "$APPS" | cut""", """[ "${1:-}" = listall ] && { { echo "$APPS"; echo "$DISCOVERY"; } | cut""",
      MW + "test_the_rows_are_listed_but_not_in_listall_and_in_no_gate_list", MW + "test_all_and_listall_leave_the_discovery_rows_out"),
    M("rows-in-all", "run", """[ "${1:-}" = all ] && set -- $(echo "$APPS" | cut""", """[ "${1:-}" = all ] && set -- $({ echo "$APPS"; echo "$DISCOVERY"; } | cut""",
      MW + "test_all_and_listall_leave_the_discovery_rows_out", MW + "test_the_rows_are_listed_but_not_in_listall_and_in_no_gate_list" if False else MW + "test_all_and_listall_leave_the_discovery_rows_out"),
    M("rows-not-listed", "run", """[ "${1:-}" = list ] && { { echo "$APPS"; echo "$DISCOVERY"; } | cut""", """[ "${1:-}" = list ] && { { echo "$APPS"; } | cut""",
      MW + "test_the_rows_are_listed_but_not_in_listall_and_in_no_gate_list", MW + "test_each_row_runs_mlrows_with_its_own_name_root_and_out_and_a_timeout_above_the_row_s" if False else MW + "test_the_rows_are_listed_but_not_in_listall_and_in_no_gate_list"),
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
      GJ + "test_the_cpu_forms_are_judged_fail_by_the_host_alone_when_the_guest_lies", GJ + "test_the_cpu_forms_are_judged_fail_by_the_host_alone_when_the_guest_lies" if False else GJ + "test_a_jax_run_the_host_saw_idle_is_a_fail"),
    M("hook-does-not-push-the-rows", "hook", '[ -n "$MLR" ] && python3 -B -I "$HERE/mlrows/mlrows.py" push', '[ -n "$MLR" ] && : python3 -B -I "$HERE/mlrows/mlrows.py" push',
      MW + "test_the_shell_glue_pushes_sets_up_and_compares_the_rows", MW + "test_the_hook_pushes_the_rows_it_was_asked_for_before_the_guest_runs_anything"),
    M("hook-pushes-after-the-guest-output-reset", "hook", "$G 'sudo rm -rf /opt/apps/out/guest'\n", "",
      MW + "test_the_hook_pushes_the_rows_it_was_asked_for_before_the_guest_runs_anything", MW + "test_the_hook_pushes_the_rows_it_was_asked_for_before_the_guest_runs_anything" if False else MW + "test_the_hook_pushes_the_rows_it_was_asked_for_before_the_guest_runs_anything"),
    M("matrix-does-not-set-up-before-the-gpu-lock", "matrix", "    python3 -B -I \"$HERE/mlrows/mlrows.py\" setup --root \"$MLROWS\" --rows \"$MLR\" | tee -a \"$R/mlrows_setup.log\"\n", "    :\n",
      MW + "test_the_shell_glue_pushes_sets_up_and_compares_the_rows", MW + "test_the_host_run_sets_the_rows_up_before_it_takes_the_gpu_lock"),
    M("matrix-does-not-compare", "matrix", '''python3 -B -I "$HERE/mlrows/mlrows.py" compare "$row" "$R/$row.host.log" "$R/$row.guest.log" > "$R/$row.compare.txt" 2>&1;''', ''': "$R/$row.host.log" "$R/$row.guest.log" > "$R/$row.compare.txt" 2>&1;''',
      MW + "test_the_shell_glue_pushes_sets_up_and_compares_the_rows", MW + "test_the_matrix_compares_host_and_guest_for_each_row_after_the_guest_runs"),
]
# The `... if False else ...` tails above name the same killer twice only where a second distinct killer does not exist yet; the check in main() needs two DISTINCT names, so a mutant
# that lists one killer twice is reported WEAK, never passed. (They are listed that way while the tests are being written; none may remain.)


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
    killers = sorted(set(k.split(" (")[0] for k in run_tests(root)["killers"]))     # a failing subTest is the test
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
