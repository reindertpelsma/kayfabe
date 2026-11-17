#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later
"""Mutation checks of the open FFmpeg matrix and DCGM + NVML rows (2026-11-17): every mutant needs two named killers (stdlib, GPU-free).

STATUS: LIVE, 2026-11-17 (docs/design/V3_OPEN_ROWS_FFMPEG_DCGM.md section 9).

Each mutant is one exact-once text replacement in a private copy of one file of scripts/apps/open_rows (or of a wiring file); the named
tests of scripts/ci/test_open_rows.py run against the copy, and a mutant is killed only when at least TWO of its named tests fail. The
unmutated copy must be green first. Same shape as clocknet_assert_mutants.py.

    python3 scripts/ci/open_rows_mutants.py [--jobs N] [substring-of-mutant-name]
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
    "lib": "scripts/apps/open_rows/nvcodec_lib.py",
    "matrix": "scripts/apps/open_rows/nvcodec_matrix.py",
    "nvml": "scripts/apps/open_rows/nvml_lib.py",
    "sweep": "scripts/apps/open_rows/nvml_sweep.py",
    "census": "scripts/apps/open_rows/census.py",
    "rowio": "scripts/apps/open_rows/rowio.py",
    "prov": "scripts/apps/open_rows/provision.sh",
    "pins": "scripts/apps/open_rows/pins.env",
    "gate": "scripts/apps/gate_list.txt",
    "hook": "scripts/apps/apps_hook.sh",
    "bundle": "scripts/apps/build_bundle.sh",
    "run_apps": "scripts/apps/run_apps.sh",
    "classes": "scripts/apps/open_rows/nvml_field_classes.tsv",
    "cases": "scripts/apps/open_rows/nvcodec_cases.tsv",
}
SOURCES = {key: (ROOT / path).read_text() for key, path in FILES.items()}
SUITE = "test_open_rows."
COPY_DIRS = ["scripts/apps/open_rows"]
COPY_FILES = [
    "scripts/ci/test_open_rows.py",
    "scripts/apps/run_apps.sh", "scripts/apps/apps_hook.sh", "scripts/apps/apps_matrix.sh", "scripts/apps/build_bundle.sh",
    "scripts/apps/gate_list.txt", "scripts/apps/gate_list_middle.txt", "scripts/apps/expected_rows.txt", "scripts/apps/pin_lib.sh",
    "scripts/apps/image/recipe.json", "scripts/bench/windows/kf3_census.py",
]
COLLECT = """import io, json, sys, unittest
suite = unittest.defaultTestLoader.loadTestsFromNames(sys.argv[1:])
r = unittest.TextTestRunner(stream=io.StringIO()).run(suite)
print('KF_MUTATION_RESULT=' + json.dumps({'tests': r.testsRun, 'killers': [t.id() for t, _ in r.failures + r.errors]}))
"""


def T(cls, *names):
    return [SUITE + cls + ".test_" + n for n in names]


def M(name, key, old, new, *tests):
    return (name, key, old, new, [t for group in tests for t in group])


def K(*names):
    """Killers as 'Class.name' strings (the test_ prefix and the suite are added)."""
    return [SUITE + n.split(".")[0] + ".test_" + n.split(".")[1] for n in names]


MUTANTS = [
    # ---- verdicts and the caps comparison (nvcodec_lib.py)
    M("supported-and-failed-passes", "lib", 'return ("PASS", "ok") if attempt_ok else ("FAIL", attempt_reason or "other")', 'return ("PASS", "ok")',
      K("Verdicts.supported_and_failed_is_fail", "RunnerEndToEnd.a_supported_case_that_fails_is_a_fail_and_the_exit_status_says_so")),
    M("unexpected-success-is-not-supported", "lib", 'return ("MISMATCH", "unexpected_success") if attempt_ok else', 'return ("NOT_SUPPORTED", "unexpected_success") if attempt_ok else',
      K("Verdicts.unsupported_and_worked_is_a_mismatch", "RunnerEndToEnd.an_unexpected_success_is_a_mismatch")),
    M("config-errors-count-as-failures", "lib", "    if attempt_reason in CONFIG_REASONS and not attempt_ok:", "    if False:",
      K("Verdicts.our_own_configuration_errors_are_not_run_not_fail", "UnavailableEngines.the_reasons_are_configuration_errors", "RunnerEndToEnd.an_unreachable_encoder_is_not_run_with_its_reason")),
    M("unknown-probe-counts-as-supported", "lib", '    if unknown and not failed:\n        return "UNKNOWN", unknown', '    if unknown and not failed:\n        return "SUPPORTED", unknown',
      K("CapsComparison.a_probe_that_could_not_ask_is_unknown_never_a_guess", "RunnerEndToEnd.a_probe_that_cannot_ask_makes_everything_not_run")),
    M("failed-atoms-ignored", "lib", '    if failed:\n        return "UNSUPPORTED", failed', '    if False:\n        return "UNSUPPORTED", failed',
      K("CapsComparison.av1_encode_follows_the_die", "CapsComparison.decode_follows_cuvid_caps")),
    M("size-limit-ignored", "lib", 'elif not (got["WIDTH_MIN"] <= w <= got["WIDTH_MAX"] and got["HEIGHT_MIN"] <= h <= got["HEIGHT_MAX"]):', "elif False:",
      K("CapsComparison.dimension_limits_come_from_the_probe", "RunnerEndToEnd.a_size_the_probe_forbids_that_encodes_anyway_is_a_mismatch")),
    M("decoder-size-limit-ignored", "lib", 'elif kind == "dec_dim" and not (d["minw"] <= w <= d["maxw"] and d["minh"] <= h <= d["maxh"]):', "elif False:",
      K("CapsComparison.decode_follows_cuvid_caps", "RunnerEndToEnd.a_decode_size_the_probe_forbids_that_decodes_anyway_is_a_mismatch")),
    M("cap-comparison-off-by-one", "lib", 'return {">=": a >= b, "<=": a <= b, "==": a == b}[op]', 'return {">=": a > b, "<=": a <= b, "==": a == b}[op]',
      K("CapsComparison.sessions_follow_the_discovered_cap", "CapsComparison.444_and_10bit_follow_their_caps")),
    M("decoder-probe-error-reads-as-unsupported", "lib", 'if d is None or d["error"]:', "if d is None:",
      K("UnavailableEngines.decoder_rows_that_are_errors_are_unknown_not_unsupported", "RunnerEndToEnd.decoder_rows_that_are_errors_make_decode_cases_not_run")),
    # ---- PSNR
    M("psnr-threshold-is-strict", "lib", "return value is not None and value >= minimum", "return value is not None and value > minimum",
      K("Psnr.threshold_boundary", "RunnerEndToEnd.psnr_exactly_at_the_threshold_passes_and_just_under_fails")),
    M("psnr-first-summary-wins", "lib", "    for m in PSNR_RE.finditer(stderr):\n        last = m[1]", "    for m in PSNR_RE.finditer(stderr):\n        last = last or m[1]",
      K("Psnr.parse_the_last_summary_wins", "RunnerEndToEnd.the_last_psnr_summary_wins_end_to_end")),
    M("psnr-inf-is-a-failure", "lib", 'return float("inf") if last == "inf" else float(last)', 'return 0.0 if last == "inf" else float(last)',
      K("Psnr.parse_inf_is_identical_frames", "RunnerEndToEnd.identical_frames_pass_end_to_end")),
    # ---- reasons, sessions, buckets
    M("session-limit-reason-lost", "lib", '("session_limit", re.compile(r"OpenEncodeSessionEx failed: out of memory|OpenEncodeSessionEx failed: .*(10)|incompatible client key", re.I)),', '("session_limit", re.compile(r"NEVERMATCHES")),',
      K("ReasonsAndCompare.reason_session_limit", "RunnerEndToEnd.sessions_above_the_cap_are_not_supported_when_exactly_the_cap_worked")),
    M("params-reason-lost", "lib", '("encoder_params_rejected", re.compile(r"InitializeEncoder failed|invalid param|Provided device doesn\'t support required NVENC", re.I)),', '("encoder_params_rejected", re.compile(r"NEVERMATCHES")),',
      K("ReasonsAndCompare.reason_params", "RunnerEndToEnd.a_supported_case_that_fails_is_a_fail_and_the_exit_status_says_so")),
    M("no-device-reason-lost", "lib", '("no_encode_device", re.compile(r"No (NVENC )?capable devices found|Cannot load libnvidia-encode|OpenEncodeSessionEx failed: (no encode device|unsupported device)", re.I)),', '("no_encode_device", re.compile(r"NEVERMATCHES")),',
      K("ReasonsAndCompare.reason_no_device_as_a_vast_container_reports_it", "RunnerEndToEnd.an_unreachable_encoder_is_not_run_with_its_reason")),
    M("cap-equal-is-not-enforced", "lib", '    return False, "" if n_ok == cap else "n_ok=%d but cap %d" % (n_ok, cap)', '    return False, ""',
      K("Verdicts.sessions_around_the_cap", "RunnerEndToEnd.sessions_above_the_cap_with_another_count_are_a_mismatch")),
    M("under-cap-failure-forgiven", "lib", '        return all_ok, "" if all_ok else "n_ok=%d of %d under cap %d" % (n_ok, n, cap)', '        return True, ""',
      K("Verdicts.sessions_around_the_cap", "RunnerEndToEnd.a_session_failing_under_the_cap_is_a_fail")),
    M("partial-sessions-not-a-mismatch", "lib", '    if result in ("PASS", "NOT_SUPPORTED") and "n_ok=" in note:', "    if False:",
      K("Verdicts.sessions_around_the_cap", "RunnerEndToEnd.sessions_above_the_cap_with_another_count_are_a_mismatch")),
    M("host-failure-blamed-on-kayfabe", "lib", '    if host["result"] in ("FAIL", "MISMATCH"):  # the control itself is bad: not a statement about kayfabe\n        return "DISAGREE", BUCKET_HARNESS', "    if False:\n        return 'DISAGREE', BUCKET_HARNESS",
      K("ReasonsAndCompare.agreement_and_buckets", "ReasonsAndCompare.a_failing_host_control_is_not_a_kayfabe_candidate")),
    M("limit-ignores-the-result", "lib", 'if fnmatch.fnmatchcase(guest["case_id"], glob) and result == guest["result"]:', 'if fnmatch.fnmatchcase(guest["case_id"], glob):',
      K("ReasonsAndCompare.a_limit_explains_only_the_result_it_names", "ReasonsAndCompare.a_limit_for_fail_does_not_explain_not_supported")),
    M("limit-matches-every-case", "lib", 'if fnmatch.fnmatchcase(guest["case_id"], glob) and result == guest["result"]:', 'if result == guest["result"]:',
      K("ReasonsAndCompare.a_limit_explains_only_the_cases_it_names", "ReasonsAndCompare.a_limit_names_its_cases_by_glob")),
    M("works-on-host-fails-in-guest-not-a-candidate", "lib", '    return "DISAGREE", BUCKET_CANDIDATE', '    return "DISAGREE", BUCKET_KNOWN',
      K("ReasonsAndCompare.agreement_and_buckets", "RunnerEndToEnd.a_guest_that_fails_where_the_host_passes_is_a_candidate")),
    # ---- the runner
    M("runner-ignores-the-probe-for-sessions", "matrix", 'cap = int(self.probe.sessions.get("opened", "0") or 0)', "cap = 99",
      K("RunnerEndToEnd.sessions_above_the_cap_are_not_supported_when_exactly_the_cap_worked", "RunnerEndToEnd.sessions_above_the_cap_with_another_count_are_a_mismatch")),
    M("runner-exit-ignores-failures", "matrix", "return 0 if ok and counts[\"PASS\"] > 0 else 1", "return 0",
      K("RunnerEndToEnd.a_supported_case_that_fails_is_a_fail_and_the_exit_status_says_so", "RunnerEndToEnd.a_probe_that_cannot_ask_makes_everything_not_run")),
    M("runner-does-not-emit-files", "matrix", "        if self.emit:\n            for name in (\"probe.tsv\", \"results.tsv\", \"summary.txt\"):", "        if False:\n            for name in (\"probe.tsv\", \"results.tsv\", \"summary.txt\"):",
      K("RunnerEndToEnd.the_summary_line_matches_the_row_pattern_of_run_apps", "RunnerEndToEnd.compare_of_a_run_with_itself_through_its_log")),
    M("compare-exit-ignores-candidates", "matrix", "    return 1 if cand else 0", "    return 0",
      K("RunnerEndToEnd.a_guest_that_fails_where_the_host_passes_is_a_candidate", "RunnerEndToEnd.a_guest_that_says_not_supported_where_the_host_passes_is_a_candidate")),
    # ---- the NVML diff (nvml_lib.py)
    M("na-in-guest-is-ok", "nvml", '            return "FINDING_NA_IN_GUEST", "host=%s guest=%s/%s" % (h["value"], g["status"], g["value"])', '            return "OK", ""',
      K("NvmlDiff.na_in_guest_where_host_has_a_number_is_a_finding", "NvmlDiff.smi_na_text_in_guest_is_a_finding", "DcgmRowEndToEnd.na_in_the_guest_where_the_host_has_a_number_fails_the_comparison")),
    M("every-na-is-expected", "nvml", '            ex = expected_ns(kind, key, table)\n            if ex:\n                return "EXPECTED_NS", "%s: %s" % ex', '            ex = True\n            if ex:\n                return "EXPECTED_NS", "any"',
      K("NvmlDiff.an_unlisted_value_is_never_hidden_by_the_table", "NvmlDiff.na_in_guest_where_host_has_a_number_is_a_finding")),
    M("expected-table-ignored", "nvml", '            ex = expected_ns(kind, key, table)\n            if ex:\n                return "EXPECTED_NS", "%s: %s" % ex', '            ex = None\n            if ex:\n                return "EXPECTED_NS", "%s: %s" % ex',
      K("NvmlDiff.a_documented_not_supported_is_expected", "NvmlDiff.the_sweep_marks_documented_not_supported_rows_expected")),
    M("static-values-not-compared", "nvml", '        if cls == "static":', "        if False:",
      K("NvmlDiff.a_static_value_must_be_equal", "NvmlDiff.a_static_smi_property_must_be_equal")),
    M("live-tolerance-ignored", "nvml", '        elif cls == "live":', "        elif False:",
      K("NvmlDiff.a_live_counter_beyond_its_tolerance_is_a_finding", "NvmlDiff.a_live_smi_value_beyond_its_tolerance_is_a_finding")),
    M("live-tolerance-is-strict", "nvml", "        return abs(h - g) <= tol\n", "        return abs(h - g) < tol\n",
      K("NvmlDiff.tolerance_boundaries", "NvmlDiff.a_live_value_exactly_at_the_tolerance_is_ok")),
    M("missing-guest-row-is-ok", "nvml", 'else ("FINDING_GUEST_MISSING", "host %s, guest has no row" % h["value"])', 'else ("OK", "")',
      K("NvmlDiff.a_missing_row_is_a_finding", "NvmlDiff.a_field_missing_from_the_guest_is_a_finding_in_the_sweep")),
    M("missing-part-is-ok", "nvml", "        if missing:", "        if False:",
      K("NvmlDiff.a_missing_part_is_a_finding", "NvmlDiff.a_missing_encoder_stat_part_is_a_finding")),
    M("na-text-parses-as-a-number", "nvml", 'return float(m[1]) if m else None', 'return float(m[1]) if m else 0.0',
      K("NvmlValues.numbers", "NvmlValues.multi_part_values")),
    M("part-with-na-is-numeric", "nvml", "        if n is None:\n            return None\n        out[", "        if n is None:\n            n = 0.0\n        out[",
      K("NvmlValues.multi_part_values", "NvmlDiff.na_in_guest_where_host_has_a_number_is_a_finding")),
    M("smi-identity-kept", "nvml", '            out[k] = "<scrubbed>"\n    return out', '            pass\n    return out',
      K("NvmlValues.smi_xml_is_flattened_and_identity_is_scrubbed", "NvmlValues.the_identity_of_a_smi_xml_never_reaches_a_row")),
    M("query-identity-kept", "nvml", '        if IDENTITY.search(n) or n in ("serial", "uuid", "pci.bus_id", "pci.bus", "pci.device", "pci.domain", "index"):', '        if False:',
      K("NvmlValues.help_query_gpu_and_the_csv_line", "NvmlValues.the_identity_of_a_query_property_never_reaches_a_row")),
    M("default-class-is-static", "nvml", 'return "numeric", "-"', 'return "static", "-"',
      K("NvmlDiff.unknown_field_ids_default_to_numeric", "NvmlDiff.a_leaf_no_table_line_names_is_only_asked_to_be_a_number")),
    M("ignored-keys-compared", "nvml", '    if cls == "ignore":\n        return "IGNORED", ""', "    if False:\n        return 'IGNORED', ''",
      K("NvmlDiff.ignored_keys", "NvmlDiff.process_ids_are_not_compared")),
    M("diag-fail-in-guest-not-a-finding", "nvml", '        if h["status"] == "PASS" and gs in ("FAIL", "MISSING"):', '        if False:',
      K("DcgmParsers.a_test_the_host_passes_that_fails_in_the_guest_is_a_finding", "DcgmRowEndToEnd.a_diag_test_that_passes_on_the_host_and_fails_in_the_guest_fails_the_comparison")),
    M("diag-only-first-category", "nvml", "            cat = node.get(\"category\", category)\n            for v in node.values():\n                walk(v, cat)", "            cat = node.get(\"category\", category)\n            for v in list(node.values())[:1]:\n                walk(v, cat)",
      K("DcgmParsers.diag_json_both_layouts", "DcgmParsers.diag_rows_and_comparison")),
    M("dmon-reads-every-gpu", "nvml", 'if not m or m[1] != "0":', "if not m:",
      K("DcgmParsers.dmon_ignores_other_gpus", "DcgmParsers.dmon_values_and_na")),
    M("dmon-na-is-zero", "nvml", "number_of(c) if c.lower() not in (\"n/a\", \"blank\", \"not\", \"found\", \"-\") and number_of(c) is not None else None", "number_of(c) or 0.0",
      K("DcgmParsers.dmon_blank_and_not_found_cells_are_none_not_zero", "DcgmParsers.a_dmon_field_with_only_na_cells_is_listed_na")),
    # ---- census (census.py)
    M("no-samples-is-proven", "census", '    if p["n"] == 0:\n        return "UNPROVEN", "no samples (nvidia-smi absent or silent)"', '    if p["n"] == 0:\n        return "PROVEN", "no samples"',
      K("Census.no_data_is_never_a_pass", "Census.cli_verdict_without_samples_is_not_proven")),
    M("na-counters-count-as-idle-failures", "census", '        return "UNPROVEN", "%s was N/A in every sample" % "/".join(keys)', '        return "FAILED", "%s was N/A in every sample" % "/".join(keys)',
      K("Census.na_counters_are_unproven_not_failed", "Census.gpu_use_is_proven_only_by_a_readable_counter")),
    M("one-busy-sample-proves-use", "census", "ENGINE_SAMPLES = 2", "ENGINE_SAMPLES = 1",
      K("Census.gpu_need_needs_two_samples", "Census.a_single_busy_sample_is_not_enough_for_the_encoder")),
    M("na-parses-as-zero", "census", "    return float(cell) if re.fullmatch(r\"\\d+(\\.\\d+)?\", cell) else None  # [N/A], N/A, Not Supported, empty", "    return float(cell) if re.fullmatch(r\"\\d+(\\.\\d+)?\", cell) else 0.0",
      K("Census.smi_na_is_not_zero", "Census.csv_round_trip")),
    M("unserviced-before-counts-the-row", "census", "    pre = mod.census(lines[:before])", "    pre = mod.census(lines)",
      K("Census.unserviced_census_counts_before_and_after_and_has_a_known_positive", "Census.cli_kf3_counts_only_the_row_in_the_row")),
    M("unserviced-in-row-counts-the-boot", "census", "    row = mod.census(lines[before:])", "    row = mod.census(lines)",
      K("Census.unserviced_census_counts_before_and_after_and_has_a_known_positive", "Census.cli_kf3_counts_only_the_row_in_the_row")),
    M("uuid-not-scrubbed", "census", '    text = UUID.sub("<uuid>", text)', "    pass",
      K("Census.scrub", "RunnerEndToEnd.logs_never_keep_a_uuid_or_an_address")),
    M("ip-not-scrubbed", "census", '    return IPV4.sub("<ip>", text) if ips else text', "    return text",
      K("Census.scrub", "RunnerEndToEnd.logs_never_keep_a_uuid_or_an_address")),
    M("session-counter-ignored", "census", '    keys = ("sess", "enc") if need == "enc" else', '    keys = ("enc",) if need == "enc" else',
      K("Census.session_counter_alone_proves_an_encode", "Census.cli_verdict_accepts_the_session_counter_alone")),
    # ---- rowio
    M("rowio-skips-the-digest", "rowio", "hashlib.sha256(body.encode()).hexdigest()[:16] == want[0]", "True",
      K("RowIo.a_same_length_edit_is_still_dropped_by_the_digest", "RowIo.load_refuses_a_log_with_an_edited_block")),
    M("rowio-skips-the-length", "rowio", "len(body.encode()) == want[1] and ", "",
      K("RowIo.a_block_with_a_wrong_declared_length_is_dropped", "RowIo.load_refuses_a_block_with_a_wrong_declared_length")),
    M("rowio-repairs-a-truncated-block", "rowio", "        cur, buf, want = m[1], [], (m[2], int(m[3]))\n            continue", "        if cur is not None:\n                out[cur] = \"\".join(l + \"\\n\" for l in buf)\n            cur, buf, want = m[1], [], (m[2], int(m[3]))\n            continue",
      K("RowIo.a_missing_end_marker_drops_the_block_even_when_the_next_block_follows", "RowIo.load_drops_a_block_whose_end_marker_is_missing")),
    # ---- the sweep (nvml_sweep.py)
    M("unknown-fields-are-listed", "sweep", "            if fv.nvmlReturn == 2:\n                unknown += 1\n                continue", "            if False:\n                continue",
      K("SweepAgainstFakeLibrary.an_id_the_library_does_not_know_is_not_listed", "SweepAgainstFakeLibrary.unknown_and_known_ids_are_counted")),
    M("value-type-misdecoded", "sweep", 'VALUE_FORMATS = {0: "d", 1: "I", 2: "Q", 3: "Q", 4: "q", 5: "i", 6: "H"}', 'VALUE_FORMATS = {0: "I", 1: "d", 2: "Q", 3: "Q", 4: "q", 5: "i", 6: "H"}',
      K("SweepAgainstFakeLibrary.an_unsigned_field_is_decoded_by_its_value_type", "SweepAgainstFakeLibrary.a_double_field_is_decoded_by_its_value_type")),
    M("missing-symbol-is-ok", "sweep", '        return N.row_line("query", key, syms[0], "NO_SYMBOL", "-")', '        return N.row_line("query", key, syms[0], "OK", "-")',
      K("SweepAgainstFakeLibrary.a_missing_symbol_is_a_row_not_a_crash", "SweepAgainstFakeLibrary.every_query_the_library_lacks_is_listed_by_name")),
    M("rc-names-lose-not-supported", "sweep", '3: "NOT_SUPPORTED", ', '3: "ERR_3", ',
      K("SweepAgainstFakeLibrary.a_field_the_driver_calls_not_supported_is_listed_so", "SweepAgainstFakeLibrary.typed_queries")),
    M("versions-scrubbed-as-addresses", "sweep", "val = census.scrub(buf.value.decode(errors=\"replace\"), ips=False)", "val = census.scrub(buf.value.decode(errors=\"replace\"))",
      K("SweepAgainstFakeLibrary.a_version_string_that_looks_like_an_address_is_kept", "SweepAgainstFakeLibrary.typed_queries")),
    # ---- the trust rules and wiring (text)
    M("sha256-pin-added", "pins", "OPENROWS_DCGM_VERSION=", "OPENROWS_DCGM_SHA256=0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef\nOPENROWS_DCGM_VERSION=",
      K("StaticContracts.no_sha256_of_a_binary_is_pinned_in_the_open_rows", "StaticContracts.pins_env_names_a_commit_and_a_version_never_a_hash")),
    M("commit-pin-becomes-a-tag", "pins", "OPENROWS_NVCH_COMMIT=c69278340ab1d5559c7d7bf0edf615dc33ddbba7", "OPENROWS_NVCH_COMMIT=n12.2.72.0",
      K("StaticContracts.no_sha256_of_a_binary_is_pinned_in_the_open_rows", "StaticContracts.the_nv_codec_headers_are_pinned_by_a_full_upstream_commit")),
    M("apt-shim-bypassed", "prov", "  apt-get -o DPkg::Lock::Timeout=300 -o Acquire::Retries=3 update -qq || return 1", "  command apt-get -o DPkg::Lock::Timeout=300 -o Acquire::Retries=3 update -qq || return 1",
      K("StaticContracts.the_apt_shim_is_not_bypassed_here", "StaticContracts.no_apt_script_but_pin_lib_bypasses_the_shim", "StaticContracts.provision_names_the_apt_shim_instead_of_bypassing_it")),
    M("unsigned-repo", "prov", 'echo "deb [signed-by=$keyring] $base/ /"', 'echo "deb [trusted=yes] $base/ /"',
      K("StaticContracts.provision_is_glue", "StaticContracts.provision_never_trusts_an_unsigned_repository")),
    M("source-fetched-without-the-pin-check", "prov", '  pin_git_checkout "$ROOT/src/nv-codec-headers" "$OPENROWS_NVCH_URL" "$OPENROWS_NVCH_TAG" "$OPENROWS_NVCH_COMMIT" || return 1', '  git clone -q "$OPENROWS_NVCH_URL" "$ROOT/src/nv-codec-headers" || return 1',
      K("StaticContracts.provision_is_glue", "StaticContracts.the_probe_headers_come_only_through_the_pin_helper")),
    M("open-row-enters-the-gate", "gate", "nvidia_smi\n", "nvidia_smi\nnvml_dcgm\n",
      K("StaticContracts.the_rows_are_discovery_rows_not_gate_rows", "StaticContracts.gate_lists_do_not_name_discovery_rows")),
    M("hook-drops-the-before-count", "hook", '--before "$q0"', '--before 0',
      K("StaticContracts.the_hook_reads_the_host_census", "StaticContracts.the_hook_takes_the_before_count_before_the_row_runs")),
    M("bundle-failure-can-fail-the-gate", "bundle", 'bash "$HERE/open_rows/provision.sh" "$B" ||', 'bash "$HERE/open_rows/provision.sh" "$B" &&',
      K("StaticContracts.the_bundle_provisions_the_rows_without_being_able_to_fail", "StaticContracts.a_failed_open_rows_step_only_prints_a_line_in_the_bundle_build")),
    M("dcgm-row-regex-too-strict", "run_apps", "NVML_DCGM_SUMMARY nvml_fields_ok=|", "NVML_DCGM_SUMMARY NEVER|",
      K("StaticContracts.the_run_apps_regex_of_the_dcgm_row_matches_its_summary", "DcgmRowEndToEnd.a_run_with_dcgm_records_everything_and_matches_the_run_apps_pattern")),
    M("matrix-row-regex-accepts-failures", "run_apps", "NVCODEC_MATRIX_SUMMARY pass=[0-9]+ not_supported=[0-9]+ fail=0 mismatch=0|", "NVCODEC_MATRIX_SUMMARY pass=[0-9]+|",
      K("StaticContracts.the_run_apps_regex_of_the_matrix_row_demands_no_failures", "RunnerEndToEnd.a_supported_case_that_fails_is_a_fail_and_the_exit_status_says_so")),
]
M_NAMES = [m[0] for m in MUTANTS]


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
        p = subprocess.run([sys.executable, "-c", COLLECT] + tests, cwd=tmp / "scripts/ci", capture_output=True, text=True, timeout=600)
        line = next((l for l in p.stdout.splitlines() if l.startswith("KF_MUTATION_RESULT=")), None)
        if line is None:
            return name, None, tests, p.stderr[-400:]
        return name, json.loads(line.split("=", 1)[1]), tests, ""


def main(argv):
    ap = argparse.ArgumentParser(description=__doc__.split("\n\n")[0])
    ap.add_argument("--jobs", type=int, default=4)
    ap.add_argument("only", nargs="?", default="")
    a = ap.parse_args(argv)
    # the unmutated copy first
    base = run_one(("baseline", "lib", "import fnmatch", "import fnmatch", sorted({t for m in MUTANTS for t in m[4]})))
    if base[1] is None or base[1]["killers"]:
        print("BASELINE not green:", base[1] and base[1]["killers"][:5], base[3])
        return 2
    selected = [m for m in MUTANTS if a.only in m[0]]
    names = [m[0] for m in MUTANTS]
    if len(names) != len(set(names)):
        print("duplicate mutant names")
        return 2
    bad = 0
    with concurrent.futures.ThreadPoolExecutor(a.jobs) as pool:
        for name, res, tests, err in pool.map(run_one, selected):
            if res is None:
                print("ERROR  %s: %s" % (name, err))
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
