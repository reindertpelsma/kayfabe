#!/usr/bin/env python3
"""Mutation checks of the llm_parity stage (2026-11-15): every mutant needs two named killers (stdlib, GPU-free).

Each mutant is one exact-once text replacement in a private copy of llm_parity.py, llm_parity_arm.py, vast_payload.py or
vast_report.py (whole scripts/ci and scripts/kfci are copied, as box_downloader_mutants.py does: the payload tests import each
other); the named tests of test_llm_parity.py run against the copy and the failing ones are the killers. The unmutated copy must be
green first. No mutant weakens a check whose failure would make a test write outside its own directory (the tar path checks).

    python3 scripts/ci/llm_parity_mutants.py [--jobs N] [substring-of-mutant-name]
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
    "lp": "scripts/ci/llm_parity.py",
    "arm": "scripts/ci/llm_parity_arm.py",
    "payload": "scripts/ci/vast_payload.py",
    "report": "scripts/ci/vast_report.py",
}
SOURCES = {key: (ROOT / path).read_text() for key, path in FILES.items()}
SUITE = "test_llm_parity."
COPY_DIRS = ["scripts/ci", "scripts/kfci"]
COPY_FILES = [
    "scripts/bench/native_xid_guard.py",
    "scripts/bench/native_fault_names.json",
    "scripts/bench/specviewperf/lanes.json",
    "ci/vast-broker/planner/schemas/instruction.schema.json",
    "ci/vast-broker/release-matrix.json",
    "ci/vast-broker/defaults.json",
]
# profile_resources.validate (the Budget tests) checks that each budgeted stage's `source` file exists
COPY_FILES += sorted({v["source"] for v in json.loads((ROOT / "ci/vast-broker/defaults.json").read_text())["payloadStageResources"].values()
                      if "source" in v})
COLLECT = """import io, json, sys, unittest
suite = unittest.defaultTestLoader.loadTestsFromNames(sys.argv[1:])
r = unittest.TextTestRunner(stream=io.StringIO()).run(suite)
print('KF_MUTATION_RESULT=' + json.dumps({'tests': r.testsRun, 'killers': [t.id() for t, _ in r.failures + r.errors]}))
"""

PI, SH, TR, MA, AO, VE, RE, AE, OR, GS, FS, PL, BU = (SUITE + n + "." for n in (
    "Pins", "ShareArguments", "Transport", "Maths", "ArmOutput", "Verdict", "Record", "ArmEndToEnd", "Orchestration", "GuestSide",
    "FetchStep", "PayloadIntegration", "Budget"))


def M(name, key, old, new, *tests):
    return (name, key, old, new, list(tests))


MUTANTS = [
    # --- pins: an unpinned item is never fetched or run
    M("pins-unset-counts-as-pinned", "lp", 'out = [n for n in names if pins["items"][n]["sha256"] == UNSET]', 'out = [n for n in names if pins["items"][n]["sha256"] == "NEVER"]',
      PI + "test_the_repository_pins_file_is_valid_and_every_hash_is_unset", PI + "test_the_unpinned_repository_manifest_is_refused_by_name",
      OR + "test_unpinned_items_refuse_before_anything_is_fetched"),
    M("pins-unset-version-accepted", "lp", 'if "ollama" in names and pins["ollama_version"] == UNSET:', "if False:",
      PI + "test_an_unset_ollama_version_refuses_even_with_every_hash_pinned", PI + "test_the_repository_pins_file_is_valid_and_every_hash_is_unset"),
    M("pins-half-pinned-allowed", "lp", "if (sha == UNSET) != (size == UNSET):", "if False:",
      PI + "test_half_pinned_item_is_refused", PI + "test_the_loader_refuses_a_half_pinned_file"),
    M("pins-bad-hash-allowed", "lp", 'if sha != UNSET and not (type(sha) is str and HEX64.fullmatch(sha)):', "if False:",
      PI + "test_bad_hash_size_and_shape_are_refused", PI + "test_a_non_hex_or_uppercase_hash_in_a_file_is_refused_by_the_loader"),
    M("pins-manifest-not-validated-by-the-downloader", "lp", "        box_downloader.from_document(doc)\n", "        pass\n",
      PI + "test_the_downloader_refuses_a_pin_with_no_source", PI + "test_a_dest_that_leaves_the_directory_is_refused_by_the_downloader"),
    M("pins-unpinned-names-hidden", "lp", 'raise StageError("unpinned", ",".join(missing))\n    doc = {', 'raise StageError("unpinned")\n    doc = {',
      PI + "test_the_unpinned_repository_manifest_is_refused_by_name", PI + "test_one_unpinned_item_is_enough_to_refuse"),
    # --- the share arguments
    M("share-comma-allowed", "lp", 'PATH_RE = re.compile(r"/[A-Za-z0-9._@+/-]{1,4000}")', 'PATH_RE = re.compile(r"/[A-Za-z0-9._@+/,-]{1,4000}")',
      SH + "test_hostile_paths_never_reach_an_argument", SH + "test_a_comma_cannot_inject_a_qemu_property"),
    M("share-equals-allowed", "lp", 'PATH_RE = re.compile(r"/[A-Za-z0-9._@+/-]{1,4000}")', 'PATH_RE = re.compile(r"/[A-Za-z0-9._@+/=-]{1,4000}")',
      SH + "test_hostile_paths_never_reach_an_argument", SH + "test_an_equals_sign_alone_is_refused_in_a_socket_path"),
    M("share-dotdot-allowed", "lp", 'if any(seg in (".", "..") for seg in value.split("/")[1:]):', "if False:",
      SH + "test_dotdot_and_dot_segments_are_refused_without_a_filesystem_check", SH + "test_hostile_paths_never_reach_an_argument"),
    M("share-symlink-allowed", "lp", "        if os.path.realpath(value) != value:\n", "        if False:\n",
      SH + "test_a_symlinked_or_missing_or_writable_share_is_refused", SH + "test_a_symlink_in_the_middle_of_the_path_is_refused"),
    M("share-writable-directory-allowed", "lp", "os.stat(value).st_mode & 0o022", "os.stat(value).st_mode & 0",
      SH + "test_a_symlinked_or_missing_or_writable_share_is_refused", SH + "test_a_group_writable_share_is_refused_by_the_daemon_argument_builder"),
    M("share-file-accepted-as-directory", "lp", "        if directory and not os.path.isdir(value):\n", "        if False:\n",
      SH + "test_a_file_is_not_a_share", SH + "test_a_symlinked_or_missing_or_writable_share_is_refused"),
    M("share-socket-length-unchecked", "lp",
      ['    if len(socket_path) > SOCKET_PATH_MAX:\n        raise StageError("share_path_unsafe")\n    safe_path(shared_dir, directory=True)',
       '        if len(socket_path) > SOCKET_PATH_MAX:\n            raise StageError("share_path_unsafe")\n        return ["-chardev"'],
      ['    safe_path(shared_dir, directory=True)', '        return ["-chardev"'],
      SH + "test_a_socket_path_longer_than_sun_path_is_refused", SH + "test_a_socket_path_of_exactly_the_limit_is_accepted_and_one_more_is_not"),
    M("daemon-not-readonly", "lp", '"--readonly", "--sandbox", "none"', '"--sandbox", "none"',
      SH + "test_the_daemon_runs_unprivileged_with_no_capability_and_exports_one_readonly_directory", SH + "test_the_export_is_readonly_on_the_daemon_and_on_9p"),
    M("daemon-keeps-bounding-set", "lp", '"--bounding-set=-all", "--no-new-privs",', '"--no-new-privs",',
      SH + "test_the_daemon_runs_unprivileged_with_no_capability_and_exports_one_readonly_directory", SH + "test_the_daemon_drops_every_capability_set"),
    M("daemon-can-gain-privileges", "lp", '"--bounding-set=-all", "--no-new-privs",', '"--bounding-set=-all",',
      SH + "test_the_daemon_runs_unprivileged_with_no_capability_and_exports_one_readonly_directory", SH + "test_the_daemon_cannot_gain_privileges"),
    M("daemon-namespace-sandbox", "lp", '"--readonly", "--sandbox", "none"', '"--readonly", "--sandbox", "namespace"',
      SH + "test_the_daemon_runs_unprivileged_with_no_capability_and_exports_one_readonly_directory", SH + "test_the_sandbox_mode_is_none_because_it_is_the_one_that_starts_without_root"),
    M("daemon-uid-root-allowed", "lp", "not 1 <= n <= 2**31 - 2", "not 0 <= n <= 2**31 - 2",
      SH + "test_the_daemon_uid_may_not_be_root_negative_or_a_bool", SH + "test_root_is_never_the_daemons_uid"),
    M("daemon-gid-unchecked", "lp", "uid = safe_uid(uid, gid)\n    gid = uid if gid is None else gid", "gid = uid if gid is None else gid",
      SH + "test_root_is_never_the_daemons_uid", SH + "test_the_daemon_uid_may_not_be_root_negative_or_a_bool"),
    M("virtiofs-dax-window", "lp", 'tag={tag}"]\n    if transport == "9p":', 'tag={tag},cache-size=1G"]\n    if transport == "9p":',
      SH + "test_nothing_enables_dax_or_a_writable_export", SH + "test_virtiofs_arguments"),
    M("9p-writable", "lp", ",security_model=none,readonly=on", ",security_model=none",
      SH + "test_9p_fallback_arguments_are_readonly_and_unmapped", SH + "test_the_export_is_readonly_on_the_daemon_and_on_9p"),
    M("9p-mapped-security", "lp", ",security_model=none,readonly=on", ",security_model=passthrough,readonly=on",
      SH + "test_9p_fallback_arguments_are_readonly_and_unmapped", SH + "test_nothing_enables_dax_or_a_writable_export"),
    M("guest-mounts-read-write", "lp", '"virtiofs": "-t virtiofs -o ro"', '"virtiofs": "-t virtiofs -o rw"',
      SH + "test_the_guest_mounts_readonly_at_a_fixed_point", GS + "test_the_hook_mounts_proves_readonly_and_returns_the_arms_output"),
    M("tag-unvalidated", "lp", "    if type(tag) is not str or not TAG_RE.fullmatch(tag):", "    if False:",
      SH + "test_the_tag_is_validated", SH + "test_a_tag_with_a_comma_cannot_reach_the_guest_mount_line_or_the_device_line"),
    M("traversable-never-false", "lp", "        if not os.stat(os.path.join(*parts[:n])).st_mode & 0o001:\n", "        if False:\n",
      SH + "test_traversable_needs_search_permission_on_every_ancestor", OR + "test_a_work_directory_nobody_else_can_search_is_refused"),
    # --- transport
    M("transport-auto-prefers-9p", "lp", 'for t in ("virtiofs", "9p"):', 'for t in ("9p", "virtiofs"):',
      TR + "test_auto_prefers_virtiofs", TR + "test_auto_with_both_devices_and_noise_never_picks_9p"),
    M("transport-named-replaced", "lp", "    if not has[requested]:", "    if False:",
      TR + "test_a_named_transport_is_never_silently_replaced", TR + "test_9p_requested_without_its_device_is_refused"),
    M("transport-neither-device-accepted", "lp", '        raise StageError("no_share_transport")', "        return 'virtiofs'",
      TR + "test_auto_refuses_when_neither_device_exists", TR + "test_an_empty_device_list_offers_no_transport"),
    # --- maths
    M("tok-s-zero-count-or-duration-accepted", "lp", "if type(v) is not int or v <= 0:", "if type(v) is not int or v < 0:",
      MA + "test_zero_negative_nan_and_wrong_types_are_not_measurements", VE + "test_unusable_timing_fails_with_a_named_reason_and_no_ratio"),
    M("ratio-accepts-nan-and-inf-timings", "lp",
      ["or not math.isfinite(r) or r <= 0 for r in rates", "    if value is not None and not (math.isfinite(value) and value > 0):\n        value = None\n"],
      ["or r <= 0 for r in rates", ""],
      MA + "test_median_refuses_any_unusable_member", MA + "test_an_invalid_timing_gives_no_ratio_at_all"),
    M("median-accepts-zero", "lp", "or not math.isfinite(r) or r <= 0 for r in rates", "or not math.isfinite(r) or r < 0 for r in rates",
      MA + "test_median_refuses_any_unusable_member", MA + "test_an_invalid_timing_gives_no_ratio_at_all"),
    M("ratio-inverted", "lp", "value = None if h is None or g is None else g / h", "value = None if h is None or g is None else h / g",
      MA + "test_ratio_is_guest_over_host_from_medians", OR + "test_both_arms_agree_and_the_ratio_is_recorded"),
    M("ratio-from-the-mean", "lp", "return statistics.median(rates)", "return statistics.mean(rates)",
      MA + "test_ratio_is_guest_over_host_from_medians", MA + "test_one_slow_run_does_not_move_the_median"),
    M("ratio-block-gate-true", "lp", '        "gate": False,\n    }', '        "gate": True,\n    }',
      MA + "test_the_ratio_block_never_carries_a_gate", RE + "test_a_passing_record_validates_in_both_validators"),
    M("ratio-targets-inverted", "lp", '"meets_min": None if value is None else value >= RATIO_MIN,', '"meets_min": None if value is None else value < RATIO_MIN,',
      MA + "test_the_targets_are_facts_about_the_value", MA + "test_ratio_is_guest_over_host_from_medians"),
    # --- arm output (the guest's is hostile data)
    M("arm-two-result-lines-accepted", "lp", "    if len(found) != 1 or len(found[0]) > MAX_ARM_OUTPUT:", "    if len(found) < 1 or len(found[0]) > MAX_ARM_OUTPUT:",
      AO + "test_zero_or_two_result_lines_are_refused", OR + "test_a_guest_that_prints_two_result_lines_is_refused"),
    M("arm-label-unchecked", "lp", 'or doc["label"] != label:', "or False:",
      AO + "test_the_label_must_match_the_arm_that_was_asked", AO + "test_a_guest_cannot_pass_off_the_hosts_result"),
    M("arm-unknown-fields-accepted", "lp", 'if type(doc) is not dict or set(doc) != keys or doc["schema"] != 1', 'if type(doc) is not dict or not keys <= set(doc) or doc["schema"] != 1',
      AO + "test_an_unknown_top_level_field_is_refused", AO + "test_hostile_guest_documents_are_refused"),
    M("arm-duplicate-keys-accepted", "lp", 'doc = json.loads(found[0], parse_constant=lambda _c: (_ for _ in ()).throw(ValueError("constant")), object_pairs_hook=_no_duplicates)',
      'doc = json.loads(found[0])',
      AO + "test_a_duplicated_key_cannot_hide_a_value", AO + "test_nan_and_duplicate_keys_in_the_json_are_refused"),
    M("arm-count-accepts-bool", "lp", "return type(v) is int and lo <= v <= hi", "return isinstance(v, int) and lo <= v <= hi",
      AO + "test_hostile_guest_documents_are_refused", RE + "test_the_two_validators_agree_on_a_corpus"),
    M("arm-count-has-no-upper-bound", "lp", "return type(v) is int and lo <= v <= hi", "return type(v) is int and lo <= v",
      AO + "test_hostile_guest_documents_are_refused", RE + "test_the_two_validators_agree_on_a_corpus"),
    # --- the verdict
    M("verdict-reads-the-ratio", "lp", '    if reason is None and ratio["state"] != "OK":', '    if reason is None and (ratio["state"] != "OK" or ratio["value"] < RATIO_MIN):',
      VE + "test_the_ratio_value_never_changes_the_verdict", VE + "test_a_ratio_below_the_minimum_is_recorded_and_still_passes",
      OR + "test_a_slow_guest_is_a_pass_with_the_ratio_recorded"),
    M("verdict-ignores-invalid-timing", "lp", '    if reason is None and ratio["state"] != "OK":', "    if False:",
      VE + "test_unusable_timing_fails_with_a_named_reason_and_no_ratio", OR + "test_a_guest_with_a_zero_duration_has_no_ratio_and_fails_on_timing"),
    M("verdict-partial-gpu-residency-ok", "lp", 'a["ps"]["size"] <= 0 or a["ps"]["size_vram"] != a["ps"]["size"]', 'a["ps"]["size"] <= 0 or a["ps"]["size_vram"] <= 0',
      VE + "test_part_of_the_model_in_system_memory_fails", OR + "test_a_guest_with_part_of_the_model_in_system_memory_fails"),
    M("verdict-cpu-only-ok", "lp", 'a["ps"]["size"] <= 0 or a["ps"]["size_vram"] != a["ps"]["size"]', "False",
      VE + "test_a_model_with_no_gpu_residency_fails_on_either_arm", OR + "test_a_guest_off_the_gpu_fails"),
    M("verdict-text-disagreement-ignored", "lp", "    equal = len(texts) == 1 and len(counts) == 1 and (not compared or distinct_ids == 1)", "    equal = True",
      VE + "test_different_text_fails", OR + "test_a_guest_with_different_text_fails_with_the_ratio_still_recorded", VE + "test_a_different_token_count_fails"),
    M("verdict-token-ids-ignored", "lp", "(not compared or distinct_ids == 1)", "True",
      VE + "test_different_token_ids_fail_when_every_run_has_them", OR + "test_different_token_ids_fail_even_with_the_same_text"),
    M("verdict-token-count-ignored", "lp", "len(texts) == 1 and len(counts) == 1 and", "len(texts) == 1 and",
      VE + "test_a_different_token_count_fails", OR + "test_a_guest_that_generated_another_number_of_tokens_fails"),
    M("verdict-input-hash-ignored", "lp", 'if arm["inputs"][name]["sha256"] != pins["items"][name]["sha256"] or', "if False and",
      VE + "test_an_input_that_does_not_match_the_pin_fails_on_either_arm", OR + "test_pins_that_do_not_match_what_the_arms_read_fail"),
    M("verdict-input-size-ignored", "lp", 'or arm["inputs"][name]["bytes"] != pins["items"][name]["size"]:', "or False:",
      VE + "test_an_input_that_does_not_match_the_pin_fails_on_either_arm", OR + "test_a_model_of_another_size_than_pinned_fails"),
    M("verdict-version-ignored", "lp", "    if reason is None and not (host[\"ollama_version\"] == guest[\"ollama_version\"] == pins[\"ollama_version\"]):", "    if False:",
      VE + "test_ollama_must_be_the_pinned_version_on_both_arms", OR + "test_a_guest_running_another_ollama_version_fails"),
    M("verdict-params-ignored", "lp", '    if reason is None and host["params"] != guest["params"]:', "    if False:",
      VE + "test_the_two_arms_must_have_run_the_same_parameters", OR + "test_a_guest_that_reports_other_parameters_fails"),
    # --- the record
    M("record-gate-allowed", "lp", 'r["gate"] is False and r["min"] == RATIO_MIN', 'r["min"] == RATIO_MIN',
      RE + "test_the_gate_flag_must_be_false_in_both_validators", RE + "test_the_two_validators_agree_on_a_corpus"),
    M("record-pass-with-disagreement-allowed", "lp", "rec.get(\"ratio\", {}).get(\"state\") == \"OK\" and (\n            rec.get(\"agreement\", {}).get(\"equal\") is True)", "True",
      RE + "test_a_pass_record_with_a_failed_agreement_or_invalid_ratio_is_refused", RE + "test_the_two_validators_agree_on_a_corpus"),
    M("record-fail-without-reason-allowed", "lp", '    if ok and rec["verdict"] == "FAIL":\n        ok = "reason" in rec', "    pass",
      RE + "test_the_two_validators_agree_on_a_corpus", RE + "test_a_failed_record_must_name_its_reason_in_both_validators"),
    M("record-unknown-key-allowed", "lp", 'and {"schema", "sha", "verdict", "transport", "ollama_version", "items", "arms"} <= set(rec) <= keys', 'and {"schema", "sha", "verdict", "transport", "ollama_version", "items", "arms"} <= set(rec)',
      RE + "test_the_two_validators_agree_on_a_corpus", RE + "test_a_record_with_an_unknown_field_is_refused_by_both_validators"),
    M("reporter-gate-allowed", "report", 'r["gate"] is False and r["min"] == 0.9', 'r["min"] == 0.9',
      RE + "test_the_gate_flag_must_be_false_in_both_validators", RE + "test_the_two_validators_agree_on_a_corpus"),
    M("reporter-pass-with-disagreement-allowed", "report", "        ok = set(value[\"arms\"]) == {\"host\", \"guest\"} and \"reason\" not in value and value.get(\"ratio\", {}).get(\"state\") == \"OK\" and (\n            value.get(\"agreement\", {}).get(\"equal\") is True)", "        ok = True",
      RE + "test_a_pass_record_with_a_failed_agreement_or_invalid_ratio_is_refused", RE + "test_the_two_validators_agree_on_a_corpus"),
    M("reporter-unknown-key-allowed", "report", '{"schema", "sha", "verdict", "transport", "ollama_version", "items", "arms"} <= set(value) <= keys', '{"schema", "sha", "verdict", "transport", "ollama_version", "items", "arms"} <= set(value)',
      RE + "test_the_two_validators_agree_on_a_corpus", PL + "test_the_reporter_refuses_a_malformed_record_in_a_result_file"),
    M("reporter-does-not-read-the-record", "report", '        if "llm_parity" in doc:\n            validate_llm_parity(doc["llm_parity"])', "        pass",
      PL + "test_the_reporter_reads_a_record_the_stage_built", PL + "test_the_reporter_refuses_a_malformed_record_in_a_result_file", PL + "test_the_reporter_refuses_a_gate_flag_in_a_result_file"),
    # --- the arm itself
    M("arm-hash-dropped", "arm", "            h.update(block)\n", "            pass\n",
      AE + "test_the_arm_prints_one_valid_result_and_the_share_throughput_lines", OR + "test_both_arms_agree_and_the_ratio_is_recorded"),
    M("arm-port-busy-ignored", "arm", "        if answers(self.port):\n            raise ArmFail(\"port_busy\")", "        pass",
      AE + "test_it_refuses_to_measure_somebody_elses_server", AE + "test_a_listener_that_speaks_http_is_not_mistaken_for_our_server"),
    M("arm-server-left-running", "arm", "    finally:\n        server.stop()\n", "    finally:\n        pass\n",
      AE + "test_the_server_is_gone_afterwards_and_the_port_is_free", AE + "test_the_server_is_stopped_even_when_a_later_step_fails"),
    M("arm-environment-passed-through", "arm", 'keep = {k: os.environ[k] for k in ("PATH", "LANG", "LD_LIBRARY_PATH", "CUDA_VISIBLE_DEVICES") if k in os.environ}', "keep = dict(os.environ)",
      AE + "test_the_server_environment_is_an_allowlist_and_loopback_only", AE + "test_no_secret_in_the_callers_environment_reaches_the_server"),
    M("arm-server-listens-everywhere", "arm", 'OLLAMA_HOST=f"127.0.0.1:{port}"', 'OLLAMA_HOST=f"0.0.0.0:{port}"',
      AE + "test_the_server_environment_is_an_allowlist_and_loopback_only", AE + "test_no_secret_in_the_callers_environment_reaches_the_server"),
    M("arm-reuses-a-work-directory", "arm", "    except FileExistsError:\n        raise ArmFail(\"work_exists\") from None", "    except FileExistsError:\n        pass",
      AE + "test_an_arm_work_directory_that_already_exists_is_refused_not_reused", AE + "test_hostile_tar_members_are_refused_before_anything_is_written", AE + "test_a_stale_binary_from_an_earlier_run_is_never_executed"),
    M("arm-hard-links-extracted", "arm", "        return not os.path.isabs(link) and (resolved == dest or resolved.startswith(dest + os.sep))\n    return False", "        return not os.path.isabs(link) and (resolved == dest or resolved.startswith(dest + os.sep))\n    return True",
      AE + "test_a_hard_link_is_refused", AE + "test_a_device_node_is_refused", AE + "test_hostile_tar_members_are_refused_before_anything_is_written"),
    M("arm-symlink-escape-extracted", "arm", "        return not os.path.isabs(link) and (resolved == dest or resolved.startswith(dest + os.sep))", "        return True",
      AE + "test_a_symlink_that_leaves_the_directory_is_refused", AE + "test_hostile_tar_members_are_refused_before_anything_is_written"),
    # --- the guest side
    M("hook-mount-type-unchecked", "lp", 'or first[0] != want or', "or False or",
      GS + "test_a_wrong_filesystem_type_is_refused_on_its_own", GS + "test_a_mount_that_is_the_wrong_type_writable_or_not_readonly_is_refused"),
    M("hook-ro-option-unchecked", "lp", 'or "ro" not in (first[1].split(",") if len(first) > 1 else []) or', "or False or",
      GS + "test_a_mount_without_the_ro_option_is_refused_on_its_own", GS + "test_a_mount_that_is_the_wrong_type_writable_or_not_readonly_is_refused"),
    M("hook-write-probe-ignored", "lp", 'or "WRITABLE" in text:\n        out.write_text', "or False:\n        out.write_text",
      GS + "test_a_share_the_guest_can_write_to_is_refused_on_its_own", GS + "test_a_mount_that_is_the_wrong_type_writable_or_not_readonly_is_refused"),
    M("hook-disk-unchecked", "lp", "    if free < MIN_FREE_BYTES:", "    if False:",
      GS + "test_too_little_guest_disk_is_named", GS + "test_a_guest_whose_df_prints_nothing_counts_as_having_no_space"),
    M("hook-unreachable-guest-ignored", "lp", '    if guest_ssh(gssh, "true", timeout=60).returncode != 0:', "    if False:",
      GS + "test_an_unreachable_guest_is_named", GS + "test_the_hook_mounts_proves_readonly_and_returns_the_arms_output", GS + "test_an_unreachable_guest_is_asked_once_and_nothing_else"),
    M("daemon-left-running", "lp", "    finally:\n        stop_process(proc)\n", "    finally:\n        pass\n",
      GS + "test_virtiofsd_is_started_before_the_boot_and_stopped_after_it", GS + "test_virtiofsd_is_stopped_when_the_boot_fails"),
    M("daemon-socket-directory-public", "lp", "    sock_dir.mkdir(mode=0o700)", "    sock_dir.mkdir(mode=0o777)",
      GS + "test_the_socket_directory_is_private", GS + "test_virtiofsd_is_started_before_the_boot_and_stopped_after_it"),
    M("daemon-death-ignored", "lp", "        if proc.poll() is not None:\n            raise StageError(\"virtiofsd_failed\", \"exited rc=%s\" % proc.returncode)", "        if False:\n            pass",
      GS + "test_a_daemon_that_dies_stops_the_stage_before_any_boot", GS + "test_virtiofsd_is_started_before_the_boot_and_stopped_after_it", GS + "test_a_dead_daemon_is_noticed_at_once_not_after_the_timeout"),
    M("boot-share-arguments-before-the-separator", "lp", 'tag, "--", *args]', "tag, *args]",
      GS + "test_the_boot_gets_the_share_arguments_after_the_double_dash_and_the_hook_in_the_environment", GS + "test_virtiofsd_is_started_before_the_boot_and_stopped_after_it"),
    M("boot-failure-without-result-ignored", "lp", '        if done.returncode != 0 and not out_path.exists():\n            raise StageError("boot_failed", "rc=%d" % done.returncode)\n', "",
      GS + "test_a_boot_that_fails_without_a_result_is_named", GS + "test_virtiofsd_is_stopped_when_the_boot_fails"),
    M("stale-scratch-kept", "lp", "        clean_scratch(work)\n", "        pass\n",
      OR + "test_a_rerun_over_a_previous_share_works", OR + "test_a_previous_share_is_replaced_not_reused"),
    M("share-files-writable", "lp", "            os.chmod(dst, 0o444)\n", "            os.chmod(dst, 0o644)\n",
      OR + "test_the_share_holds_only_the_model_and_the_tarball_readonly", OR + "test_the_share_directory_is_not_writable_by_anyone", OR + "test_no_file_in_the_share_is_writable_by_anyone"),
    M("share-directory-writable", "lp", "        os.chmod(share, 0o555)\n", "        os.chmod(share, 0o777)\n",
      OR + "test_the_share_directory_is_not_writable_by_anyone", OR + "test_the_share_holds_only_the_model_and_the_tarball_readonly"),
    M("work-not-traversable-accepted", "lp", "        if not traversable(work):\n", "        if False:\n",
      OR + "test_a_work_directory_nobody_else_can_search_is_refused", OR + "test_both_arms_agree_and_the_ratio_is_recorded", OR + "test_a_work_directory_that_is_not_searchable_refuses_before_any_download"),
    M("downloader-run-with-the-callers-interpreter-flags", "lp", 'cmd = [sys.executable, "-I", str(HERE / "box_downloader.py"), "fetch"', 'cmd = [sys.executable, str(HERE / "box_downloader.py"), "fetch"',
      FS + "test_the_downloader_command_line_carries_no_url_or_credential", FS + "test_a_local_pin_that_matches_is_used_and_nothing_is_fetched_from_the_network", FS + "test_the_downloader_runs_in_isolated_mode"),
    M("download-failure-ignored", "lp", '    if done.returncode != 0:\n        raise StageError("download_failed"', '    if False:\n        raise StageError("download_failed"',
      FS + "test_a_download_that_cannot_complete_is_named_and_leaves_no_verified_file", FS + "test_the_downloader_command_line_carries_no_url_or_credential", FS + "test_a_nonzero_downloader_exit_is_a_download_failure"),
    # --- the payload
    M("payload-flag-true", "payload", "LLM_PARITY_IN_LARGE = False", "LLM_PARITY_IN_LARGE = True",
      PL + "test_the_stage_is_not_in_any_default_plan", BU + "test_the_live_payload_and_the_live_table_agree", PL + "test_dry_run_prints_the_plan_and_touches_nothing"),
    M("payload-only-ignored", "payload", 'or "llm_parity" in only)', "or False)",
      PL + "test_only_selects_it_in_the_large_tier_and_nowhere_else", PL + "test_select_stages_refuses_it_when_it_is_not_planned",
      PL + "test_dry_run_prints_the_plan_and_touches_nothing", PL + "test_it_runs_after_the_linux_guests_and_before_the_windows_gate"),
    M("payload-any-tier", "payload", 'if i.tier == "large" and (LLM_PARITY_IN_LARGE', "if (LLM_PARITY_IN_LARGE",
      PL + "test_only_selects_it_in_the_large_tier_and_nowhere_else", PL + "test_the_flag_puts_it_in_the_large_plan_only"),
    M("payload-worker-slot-opened", "payload", '"llm_parity": "DESIGN_ONLY"}', '"llm_parity": "PLUMBING"}',
      PL + "test_the_workers_stage_slot_is_still_design_only", PL + "test_kf_stages_still_refuses_the_stage"),
    M("payload-manual-only-dropped", "payload", 'MANUAL_ONLY_STAGES = {"llm_parity": "LLM_PARITY_IN_LARGE"}', "MANUAL_ONLY_STAGES = {}",
      BU + "test_a_stage_that_is_neither_budgeted_nor_manual_only_is_refused", BU + "test_the_live_payload_and_the_live_table_agree", BU + "test_the_manual_only_table_names_the_flag_and_the_flag_is_false"),
    M("payload-rows-ignore-the-verdict", "payload", 'or doc.get("verdict") != "PASS" or doc.get("sha") != sha:\n        raise EvidenceError("llm_parity")', 'or doc.get("sha") != sha:\n        raise EvidenceError("llm_parity")',
      PL + "test_rows_refuse_a_red_verdict", PL + "test_rows_refuse_every_bad_shape_in_one_sweep", PL + "test_rows_keep_a_red_record_in_the_result"),
    M("payload-rows-ignore-the-exit-code", "payload", 'if rc or type(doc) is not dict or doc.get("verdict") != "PASS" or doc.get("sha") != sha:\n        raise EvidenceError("llm_parity")', 'if type(doc) is not dict or doc.get("verdict") != "PASS" or doc.get("sha") != sha:\n        raise EvidenceError("llm_parity")',
      PL + "test_rows_refuse_a_nonzero_exit_with_a_green_record", PL + "test_rows_refuse_every_bad_shape_in_one_sweep", PL + "test_rows_keep_a_red_record_in_the_result"),
    M("payload-rows-ignore-the-commit", "payload", 'or doc.get("verdict") != "PASS" or doc.get("sha") != sha:\n        raise EvidenceError("llm_parity")', 'or doc.get("verdict") != "PASS":\n        raise EvidenceError("llm_parity")',
      PL + "test_rows_refuse_a_record_of_another_commit", PL + "test_rows_refuse_every_bad_shape_in_one_sweep"),
    M("payload-rows-accept-two-lines", "payload", '    found = re.findall(r"^KF_LLM_PARITY_RESULT (\\{.*\\})$", text, re.M)\n    if len(found) != 1:', '    found = re.findall(r"^KF_LLM_PARITY_RESULT (\\{.*\\})$", text, re.M)[:1]\n    if len(found) != 1:',
      PL + "test_rows_refuse_two_result_lines_even_when_both_are_green", PL + "test_rows_refuse_a_red_record_then_a_green_one", PL + "test_rows_refuse_every_bad_shape_in_one_sweep"),
    M("payload-rows-drop-the-record", "payload", '    result["llm_parity"] = doc\n', "",
      PL + "test_rows_accept_a_green_record_and_keep_it", PL + "test_rows_keep_a_red_record_in_the_result"),
    M("payload-rows-read-the-ratio", "payload", '    if rc or type(doc) is not dict or doc.get("verdict") != "PASS"', '    if rc or type(doc) is not dict or doc.get("ratio", {}).get("value", 1) < 0.9 or doc.get("verdict") != "PASS"',
      PL + "test_rows_never_read_the_ratio_value", PL + "test_rows_accept_a_record_whose_ratio_is_far_below_the_target"),
    M("payload-stage-runner-dropped", "payload", 'str(root / "scripts/ci/llm_parity.py"), "run"', 'str(root / "scripts/ci/llm_parity_arm.py"), "run"',
      PL + "test_the_stage_is_built_by_main_with_the_runner_and_its_arguments", PL + "test_the_runner_the_payload_names_accepts_the_arguments_the_payload_passes"),
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
    with tempfile.TemporaryDirectory(prefix="kf-llm-parity-mutants-") as parent:
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
