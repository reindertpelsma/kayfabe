#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later
"""Mutation checks of the open Vulkan CTS row (2026-11-17): every mutant is one exact-once text replacement in a private copy of the
tree; scripts/ci/test_vkcts.py runs against the copy (the test classes named for the mutant) and the failing tests are the killers.
The unmutated copy must be green first and each mutant needs at least two killers, two of them the ones named here (stdlib, GPU-free).

Falsifier: a mutant survives, has fewer than two failing tests, an anchor occurs zero or several times, or the baseline is red."""
import re
import shutil
import subprocess
import sys
import tempfile
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
COPY = [
    "scripts/ci/test_vkcts.py",
    "scripts/apps/vkcts/vkcts.py", "scripts/apps/vkcts/vkcts_lib.py", "scripts/apps/vkcts/gen_caselist.py",
    "scripts/apps/vkcts/pins.json", "scripts/apps/vkcts/caselist.tsv", "scripts/apps/vkcts/caselist_spec.json",
    "scripts/apps/run_apps.sh", "scripts/apps/apps_matrix.sh", "scripts/apps/apps_hook.sh", "scripts/apps/matrix_table.py",
    "scripts/apps/known_unsupported.json", "scripts/apps/gate_list.txt", "scripts/apps/gate_list_middle.txt", "scripts/apps/expected_rows.txt",
    "scripts/bench/windows/gpu_assert.py",
    # read at import time by test_vkcts.RealOutputTests: without them the unmutated baseline dies with FileNotFoundError
    "scripts/ci/fixtures/vkcts_real_qpa.txt", "scripts/ci/fixtures/vkcts_real_console.txt",
]
LIB, TOOL = "scripts/apps/vkcts/vkcts_lib.py", "scripts/apps/vkcts/vkcts.py"
RUNAPPS, TABLE = "scripts/apps/run_apps.sh", "scripts/apps/matrix_table.py"
FAST = ["ClassifyTests", "QpaTests", "DeviceTests", "BlockTests", "DiffTests", "ExecuteTests", "PinsTests", "CaselistTests"]
# (name, file, old, new, killer one, killer two, test classes to run)
CASES = [
    ("unknown-status-is-a-pass", LIB, 'return STATUS_CLASS.get(status, "fail")', 'return STATUS_CLASS.get(status, "pass")',
     "test_an_unknown_status_is_a_failure_never_a_pass", "test_an_unknown_status_counts_as_fail_in_the_summary", ["ClassifyTests"]),
    ("not-supported-counted-as-fail", LIB, '"NotSupported": "not_supported",', '"NotSupported": "fail",',
     "test_every_cts_status_has_its_class", "test_not_supported_is_not_a_failure_and_a_pass_is_not_a_not_supported", ["ClassifyTests"]),
    ("quality-warning-counted-as-fail", LIB, '"QualityWarning": "warn",', '"QualityWarning": "fail",',
     "test_every_cts_status_has_its_class", "test_the_committed_table_is_the_cts_status_list", ["ClassifyTests"]),
    ("crash-counted-as-pass", LIB, '"Crash": "crash",', '"Crash": "pass",',
     "test_every_cts_status_has_its_class", "test_not_supported_is_not_a_failure_and_a_pass_is_not_a_not_supported", ["ClassifyTests"]),
    ("unclosed-case-is-a-pass", LIB, 'QpaResult(cur, "Crash", "incomplete: log ends inside the case", False, body)', 'QpaResult(cur, "Pass", "incomplete: log ends inside the case", False, body)',
     "test_a_case_that_is_never_closed_is_a_crash_and_is_the_open_case", "test_a_crash_and_a_hang_are_attributed_and_the_rest_still_runs", ["QpaTests", "RunIntegrationTests"]),
    ("terminate-word-ignored", LIB, 'word if word in ("Crash", "Timeout") else "Crash"', '"Crash"',
     "test_a_terminate_marker_carries_its_status", "test_a_terminated_timeout_a_block_without_result_and_an_unknown_case_are_classified", ["QpaTests", "RunIntegrationTests"]),
    ("block-without-result-is-a-pass", LIB, 'QpaResult(cur, "Crash", "no Result element", False, body)', 'QpaResult(cur, "Pass", "no Result element", False, body)',
     "test_a_block_without_a_result_element_is_a_crash_not_a_pass", "test_a_terminated_timeout_a_block_without_result_and_an_unknown_case_are_classified", ["QpaTests", "RunIntegrationTests"]),
    ("next-case-does-not-close-the-open-one", LIB, 'results.append(QpaResult(cur, "Crash", "incomplete: next case began", False, body))', 'pass',
     "test_a_new_case_while_one_is_open_closes_the_open_one_as_a_crash", "test_a_forged_result_in_the_body_of_another_case_is_only_data", ["QpaTests"]),
    ("lost-cases-after-a-crash", LIB, '        remaining = [c for c in remaining if c not in results]\n        if got:', '        remaining = remaining[len(batch):]\n        if got:',
     "test_a_crash_is_attributed_to_the_running_case_and_the_run_continues_after_it", "test_every_crash_is_counted_and_many_crashes_do_not_stop_a_run_that_progresses", ["ExecuteTests"]),
    ("hang-recorded-as-crash", LIB, 'status, detail = "Timeout", "killed by the harness"', 'status, detail = "Crash", "killed by the harness"',
     "test_a_hang_killed_by_the_harness_is_a_timeout_not_a_crash", "test_a_crash_and_a_hang_are_attributed_and_the_rest_still_runs", ["ExecuteTests", "RunIntegrationTests"]),
    ("open-case-hang-fallback-recorded-as-crash", LIB, 'results[out.open_case] = ("Timeout", "killed by the harness") if out.killed else', 'results[out.open_case] = ("Crash", "killed by the harness") if out.killed else',
     "test_an_open_case_without_a_parsed_result_is_still_attributed", "test_an_open_case_without_a_parsed_result_that_was_killed_is_a_timeout", ["ExecuteTests"]),
    ("budget-never-checked", LIB, "        if clock() - start >= budget_s:", "        if False:",
     "test_the_budget_is_checked_before_every_batch_and_the_rest_is_not_run", "test_a_spent_budget_runs_nothing_and_marks_every_case_not_run", ["ExecuteTests"]),
    ("idle-limit-lost", LIB, "            if idle > max_idle_restarts:", "            if idle > max_idle_restarts + 1000:",
     "test_a_wedged_device_that_gives_no_result_stops_after_the_idle_limit", "test_the_idle_limit_is_the_argument_given", ["ExecuteTests"]),
    ("idle-counter-never-reset", LIB, "        if got:\n            idle = 0\n", "        if got:\n            pass\n",
     "test_progress_resets_the_idle_counter_so_a_flaky_start_does_not_end_the_run", "test_two_wedged_starts_then_progress_is_still_within_a_limit_of_two", ["ExecuteTests"]),
    ("clean-exit-gap-not-marked", LIB, "        if left and out.rc == 0 and not out.killed:", "        if False:",
     "test_a_clean_exit_that_leaves_a_case_without_a_result_marks_it_not_run_and_ends", "test_a_terminated_timeout_a_block_without_result_and_an_unknown_case_are_classified", ["ExecuteTests", "RunIntegrationTests"]),
    ("digest-includes-details", LIB, 'h.update(("%s\\t%s\\n" % (case, cls)).encode())', 'h.update(("%s\\t%s\\n" % (case, _detail)).encode())',
     "test_the_digest_ignores_details_and_order_but_not_classes", "test_the_digest_does_not_depend_on_areas_or_details", ["BlockTests"]),
    ("block-count-unchecked", LIB, "    if ended[0] != len(rows) or ended[1] != digest(rows):", "    if ended[1] != digest(rows):",
     "test_a_block_with_a_wrong_count_is_refused", "test_every_tampering_of_a_block_is_refused", ["BlockTests"]),
    ("block-digest-unchecked", LIB, "    if ended[0] != len(rows) or ended[1] != digest(rows):", "    if ended[0] != len(rows):",
     "test_a_block_with_a_wrong_digest_is_refused", "test_every_tampering_of_a_block_is_refused", ["BlockTests"]),
    ("sanitize-keeps-markup-and-shell-characters", LIB, '    text = re.sub(r"[^A-Za-z0-9 _.,:;()/=+\\-]", "?", str(text))\n', "    text = str(text)\n",
     "test_sanitize_removes_everything_that_could_forge_a_line", "test_a_forged_detail_never_reaches_the_block_unsanitised", ["ClassifyTests", "RunIntegrationTests"]),
    ("cpu-type-accepted", LIB, '    if info.get("type") not in GPU_TYPES:\n        return False\n', "",
     "test_a_software_rasteriser_or_a_cpu_device_is_not_a_gpu", "test_a_cpu_device_type_fails_even_with_a_gpu_like_name", ["DeviceTests", "RunIntegrationTests"]),
    ("software-name-accepted", LIB, '    return not re.search(r"llvmpipe|lavapipe|swiftshader|softpipe", info.get("name", ""), re.I)', "    return True",
     "test_a_software_rasteriser_or_a_cpu_device_is_not_a_gpu", "test_a_software_name_fails_even_with_a_gpu_type", ["DeviceTests", "RunIntegrationTests"]),
    ("guest-fail-and-lost-support-swapped", LIB, 'return "GUEST_ONLY_FAIL" if g in BAD_CLASSES else "GUEST_LOSES_SUPPORT"', 'return "GUEST_LOSES_SUPPORT" if g in BAD_CLASSES else "GUEST_ONLY_FAIL"',
     "test_the_comparison_table", "test_diff_rows_counts_candidates_and_areas", ["DiffTests"]),
    ("both-fail-is-a-candidate", LIB, 'return "BOTH_FAIL" if h == g else "BOTH_FAIL_DIFFERENT"', 'return "GUEST_ONLY_FAIL"',
     "test_a_case_that_fails_on_both_sides_is_never_a_candidate", "test_the_comparison_table", ["DiffTests"]),
    ("incomplete-counts-as-same", LIB, '    if h == "not_run" or g == "not_run":\n        return "INCOMPLETE"', '    if h == "not_run" or g == "not_run":\n        return "SAME_OK"',
     "test_the_comparison_table", "test_a_case_on_one_side_only_is_incomplete_never_a_pass", ["DiffTests"]),
    ("missing-guest-case-counts-as-pass", LIB, 'hs, gs = (hr[1] if hr else "NotRun"), (gr[1] if gr else "NotRun")', 'hs, gs = (hr[1] if hr else "NotRun"), (gr[1] if gr else "Pass")',
     "test_a_case_on_one_side_only_is_incomplete_never_a_pass", "test_a_case_missing_in_the_guest_is_incomplete_not_same_and_not_a_candidate", ["DiffTests"]),
    ("candidates-not-counted", LIB, "    cand = sum(count[c] for c in CANDIDATES)", "    cand = 0",
     "test_diff_rows_counts_candidates_and_areas", "test_the_diff_text_lists_only_the_differences_and_has_one_summary_line", ["DiffTests"]),
    ("strict-never-fails", TOOL, 'return 1 if (a.strict and summary["candidates"]) else 0', "return 0",
     "test_strict_exits_one_when_there_is_a_candidate", "test_the_results_directory_form_writes_the_diff_and_the_summary", ["DiffCommandTests"]),
    ("short-commit-accepted", LIB, 'COMMIT_RE = re.compile(r"^[0-9a-f]{40}$")', 'COMMIT_RE = re.compile(r"^[0-9a-f]{4,40}$")',
     "test_a_cts_pin_that_is_a_tag_or_short_or_empty_is_refused", "test_a_short_pin_is_refused_even_when_the_head_equals_it", ["PinsTests"]),
    ("fetch-script-origin-unchecked", LIB, '        if url != row["url"]:', "        if False:",
     "test_a_changed_origin_in_the_fetch_script_is_refused", "test_a_moved_revision_url_or_digest_in_the_fetch_script_is_refused", ["PinsTests"]),
    ("fetch-script-file-digest-unchecked", LIB, '        if url != row["url"] or digest != row["sha256"]:', '        if url != row["url"]:',
     "test_a_changed_digest_of_a_file_external_is_refused", "test_a_moved_revision_url_or_digest_in_the_fetch_script_is_refused", ["PinsTests"]),
    ("unpinned-external-accepted", LIB, '    unpinned = sorted(set(git) - set(pins["externals_git"]) - {"vulkan-validationlayers"})', "    unpinned = []",
     "test_a_git_external_this_repository_does_not_pin_is_refused", "test_an_unpinned_external_with_a_valid_commit_is_still_refused", ["PinsTests"]),
    ("head-never-compared", LIB, "    if have != want:\n        raise PinRefused", "    if False:\n        raise PinRefused",
     "test_head_must_equal_the_pinned_commit", "test_externals_are_verified_by_head_and_by_file_digest", ["PinsTests", "GitFetchTests"]),
    ("key-ignores-the-build-configuration", LIB, '"fetch_scripts": pins.get("fetch_scripts", []), "build": pins["build"]}', '"fetch_scripts": pins.get("fetch_scripts", []), "build": {}}',
     "test_a_changed_cmake_argument_changes_the_key", "test_the_content_key_follows_the_pins_and_the_build_configuration_only", ["PinsTests"]),
    ("key-ignores-external-commits", LIB, '"externals_git": pins["externals_git"], "externals_files"', '"externals_git": {}, "externals_files"',
     "test_a_changed_external_commit_changes_the_key", "test_the_content_key_follows_the_pins_and_the_build_configuration_only", ["PinsTests"]),
    ("external-file-digest-unchecked", TOOL, '        if hashlib.sha256(found[0].read_bytes()).hexdigest() != row["sha256"]:', "        if False:",
     "test_a_fetched_file_external_with_another_digest_is_refused_on_its_own", "test_externals_are_verified_by_head_and_by_file_digest", ["GitFetchTests"]),
    ("fetched-cts-head-unchecked", TOOL, '    L.head_matches(git_out(["rev-parse", "HEAD"], src), want)\n\n\ndef verify_externals', "    pass\n\n\ndef verify_externals",
     "test_a_tag_moved_to_another_commit_is_refused_when_the_pinned_commit_cannot_be_fetched", "test_a_second_moved_tag_is_refused_the_same_way", ["GitFetchTests"]),
    ("committed-list-digest-unchecked", TOOL, "    if not trusted and sha != want:", "    if False:",
     "test_a_changed_list_is_refused_unless_it_is_the_users_own", "test_a_user_caselist_is_trusted_and_the_committed_one_is_checked", ["CaselistTests", "RunIntegrationTests"]),
    ("gpu-use-not-enforced", TOOL, '    if rec and rec["verdict"] != "PASS":', "    if False:",
     "test_an_idle_gpu_fails_the_row_and_names_the_reason", "test_a_host_run_without_nvidia_smi_cannot_claim_gpu_use", ["RunIntegrationTests"]),
    ("detail-not-sanitised-in-the-block", LIB, '        out.append("%s\\t%s\\t%s\\t%s\\t%s" % (case, status, cls, area, sanitize(detail)))', '        out.append("%s\\t%s\\t%s\\t%s\\t%s" % (case, status, cls, area, detail))',
     "test_details_cannot_forge_markers_or_columns", "test_render_block_never_emits_a_tab_or_newline_or_markup_inside_a_detail", ["BlockTests"]),
    ("discovery-row-gates", TABLE, '    if app in DISCOVERY:\n        return "DISCOVERY"', '    if False:\n        return "DISCOVERY"',
     "test_matrix_table_calls_it_discovery", "test_the_discovery_row_stays_out_of_the_generated_gate_list_even_when_it_passes_everywhere", ["MatrixRowTests"]),
    ("discovery-row-in-the-all-list", RUNAPPS, """[ "${1:-}" = listall ] && { echo "$APPS" | cut""", """[ "${1:-}" = listall ] && { { echo "$APPS"; echo "$DISCOVERY"; } | cut""",
     "test_the_row_is_listed_but_not_in_all_and_not_in_any_gate_list", "test_apps_matrix_expands_all_from_listall_which_is_list_minus_the_discovery_row", ["MatrixRowTests"]),
    ("row-note-lost", RUNAPPS, """appnote=$(grep -a -m1 '^APPNOTE ' "$log" | cut -c9-160 | tr '|' '/'); [ -n "$appnote" ] && note=$appnote""", "appnote=",
     "test_the_row_runs_on_the_committed_list_and_reports_a_pass_row_with_digest_and_note", "test_a_row_whose_device_is_not_a_gpu_is_a_fail_row", ["MatrixRowTests"]),
]


def run(tree, classes):
    p = subprocess.run([sys.executable, "-I", "-B", "scripts/ci/test_vkcts.py"] + list(classes), cwd=tree, text=True, capture_output=True, timeout=1500)
    out = p.stdout + p.stderr
    return p.returncode, sorted(set(re.findall(r"^(?:FAIL|ERROR): (\S+)", out, re.M))), out


def main():
    weak = 0
    only = sys.argv[1:]
    with tempfile.TemporaryDirectory(prefix="vkcts-mutants-") as tmp:
        tree = Path(tmp)
        for rel in COPY:
            (tree / rel).parent.mkdir(parents=True, exist_ok=True)
            shutil.copy(ROOT / rel, tree / rel)
        for d in ("scripts", "scripts/ci", "scripts/apps", "scripts/bench"):
            (tree / d / "__init__.py").write_text("")
        texts = {}
        for name, rel, old, new, *_ in CASES:
            texts.setdefault(rel, (tree / rel).read_text())
            if texts[rel].count(old) != 1:
                print("BAD mutant %s: anchor found %d times in %s" % (name, texts[rel].count(old), rel))
                return 2
        code, _, out = run(tree, [])
        if code:
            print("FAIL unmutated baseline\n" + out[-3000:])
            return 2
        print("PASS baseline", flush=True)
        for name, rel, old, new, k1, k2, classes in CASES:
            if only and not any(o in name for o in only):
                continue
            target = tree / rel
            target.write_text(texts[rel].replace(old, new))
            try:
                code, killers, _ = run(tree, classes)
            finally:
                target.write_text(texts[rel])
            names = [k.split(".")[-1].split("(")[0] for k in killers]
            ok = code != 0 and len(killers) >= 2 and k1 in names and k2 in names and (k1 != k2 or len(killers) >= 2)
            print("%s %s: %d killers %s" % ("PASS" if ok else "WEAK", name, len(killers), names[:4]), flush=True)
            weak += not ok
    print("%d/%d mutants killed by at least two tests" % (len(CASES) - weak, len(CASES)))
    return 1 if weak else 0


if __name__ == "__main__":
    sys.exit(main())
