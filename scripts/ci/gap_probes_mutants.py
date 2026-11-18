#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later
"""Mutation checks of the GPU-PV gap probes and the differential runner core (2026-11-18): every mutant needs two named killers (stdlib, GPU-free).

STATUS: RESEARCH, 2026-11-18 (docs/design/V3_GPUPV_GAP_MATRIX.md section 3). Same shape as open_rows_mutants.py: each mutant is one exact-once text replacement in a private
copy of one file; the named tests of scripts/ci/test_gap_probes.py run against the copy, and a mutant is killed only when at least TWO of them fail.

    python3 scripts/ci/gap_probes_mutants.py [--jobs N] [substring-of-mutant-name]
"""
import argparse
import concurrent.futures
import json
from pathlib import Path
import shutil
import subprocess
import sys
import tempfile

ROOT = Path(__file__).resolve().parents[2]
FILES = {"diff": "scripts/apps/gap_probes/diffrun.py", "lib": "scripts/apps/gap_probes/gap_lib.py", "run": "scripts/apps/gap_probes/gap_probes.py",
         "c": "scripts/apps/gap_probes/gap_probe.c", "scripts": "scripts/apps/gap_probes/gap_scripts.py", "cts": "scripts/apps/gap_probes/cts_groups.py", "table": "scripts/apps/gap_probes/gap_cases.tsv", "runapps": "scripts/apps/run_apps.sh", "hook": "scripts/apps/apps_hook.sh"}
SUITE = "test_gap_probes."
COPY_DIRS = ["scripts/apps/gap_probes", "scripts/apps/open_rows"]
COPY_FILES = ["scripts/ci/test_gap_probes.py", "scripts/apps/run_apps.sh", "scripts/apps/apps_hook.sh", "scripts/apps/apps_matrix.sh", "scripts/apps/build_bundle.sh",
              "scripts/apps/image/recipe.json", "scripts/bench/windows/kf3_census.py"]
COLLECT = """import io, json, sys, unittest
suite = unittest.defaultTestLoader.loadTestsFromNames(sys.argv[1:])
r = unittest.TextTestRunner(stream=io.StringIO()).run(suite)
print('KF_MUTATION_RESULT=' + json.dumps({'tests': r.testsRun, 'killers': [t.id() for t, _ in r.failures + r.errors]}))
"""


def K(*names):
    return [SUITE + n.split(".")[0] + ".test_" + n.split(".")[1] for n in names]


def M(name, key, old, new, *killers):
    return (name, key, old, new, [t for g in killers for t in g])


MUTANTS = [
    M("guest-only-fail-reads-as-lost-support", "diff", 'return "GUEST_ONLY_FAIL" if g in BAD else "GUEST_LOSES_SUPPORT"', 'return "GUEST_LOSES_SUPPORT" if g in BAD else "GUEST_ONLY_FAIL"',
      K("Categories.the_comparison_table", "Compare.only_differences_are_reported")),
    M("timeout-reads-as-fail", "diff", '''    if cls == "fail" and re.search(r"time ?out", str(reason), re.I):
        return "timeout"''', '''    if False:
        return "timeout"''', K("Categories.statuses_are_normalised_and_timeouts_and_crashes_stay_apart_from_fail", "AdaptersMore.the_gap_results_and_kv_files_round_trip")),
    M("unknown-status-is-a-pass", "diff", 'cls = RAW.get(key, "fail")', 'cls = RAW.get(key, "pass")',
      K("Categories.statuses_are_normalised_and_timeouts_and_crashes_stay_apart_from_fail", "Categories.an_unknown_raw_status_is_a_failure_never_a_pass")),
    M("host-control-failure-is-a-candidate", "diff", 'bucket, why = "app_or_harness_bug", "the host control fails the same case"', 'bucket, why = "kayfabe_gap_candidate", "unclassified"',
      K("Compare.a_case_that_fails_on_the_host_control_too_is_not_kayfabes", "Compare.the_summary_counts_candidates_and_buckets")),
    M("a-limit-hides-unrelated-keys", "diff", 'if _glob(l["case"], case) and _glob(l["key"], key) and', 'if _glob(l["case"], case) and',
      K("Compare.a_listed_limit_explains_exactly_the_difference_it_names", "Compare.a_limit_for_one_key_does_not_hide_a_difference_in_another")),
    M("a-limit-needs-no-reason", "diff", 'if not c[5].strip() or c[5].strip() == "-":', 'if False:',
      K("Compare.a_limit_needs_a_known_bucket_and_a_reason", "Compare.a_limit_with_a_dash_or_a_blank_reason_is_refused")),
    M("a-limit-needs-no-known-bucket", "diff", 'if c[4] not in BUCKETS:', 'if False:',
      K("Compare.a_limit_needs_a_known_bucket_and_a_reason", "Compare.the_cli_refuses_a_limits_file_with_an_unknown_bucket")),
    M("xid-window-unbounded", "diff", 'r["t0"] - 1 <= t <= r["t1"] + slack', 'r["t0"] - 1e9 <= t <= r["t1"] + slack',
      K("Compare.a_host_xid_inside_a_case_window_is_attached_and_one_outside_is_not", "Compare.an_xid_before_the_window_is_not_attached_and_one_just_after_it_is")),
    M("value-diff-ignored", "diff", '        if kd:\n            cat = "VALUE_DIFF"', '        if False:\n            cat = "VALUE_DIFF"',
      K("Compare.a_value_difference_between_two_passes_is_reported", "Compare.a_listed_limit_explains_exactly_the_difference_it_names")),
    M("guest-only-runs-on-host", "lib", 'if "guest_only" in c["tags"] and side != "guest":', 'if False:',
      K("Selection.a_guest_only_case_never_runs_on_the_host_side", "RunnerEndToEnd.a_guest_only_case_is_not_run_on_the_host_and_says_why")),
    M("destructive-without-flag", "lib", 'elif "destructive" in c["tags"] and not destructive:', 'elif False:',
      K("Selection.destructive_and_terminal_cases_need_their_flag", "SelectionOrder.the_default_selection_has_neither_destructive_nor_terminal_cases")),
    M("terminal-not-last", "lib", 'run.sort(key=lambda c: ("terminal" in c["tags"], "destructive" in c["tags"]))', 'pass',
      K("SelectionOrder.a_terminal_case_listed_first_still_runs_last", "SelectionOrder.a_destructive_case_listed_before_a_safe_one_runs_after_it")),
    M("timeout-is-not-run", "lib", 'if timed_out:\n        return "FAIL", "timeout"', 'if timed_out:\n        return "NOT_RUN", "timeout"',
      K("ProbeOutput.a_timeout_a_crash_and_a_missing_line_are_failures_with_a_named_reason", "RunnerEndToEnd.a_hang_is_a_timeout_failure_not_a_pass_and_the_run_goes_on")),
    M("crash-is-no-result-line", "lib", 'if rc is not None and rc < 0:\n        return "FAIL", "crash_signal_%d" % -rc', 'if False:\n        return "FAIL", "crash_signal_%d" % -rc',
      K("ProbeOutput.a_timeout_a_crash_and_a_missing_line_are_failures_with_a_named_reason", "RunnerEndToEnd.pass_refused_fail_and_crash_are_each_told_apart")),
    M("attr-lines-not-split", "lib", 'if key == "attr" and ":" in val:', 'if False:',
      K("ProbeOutput.attribute_lines_become_per_attribute_keys", "RunnerEndToEnd.pass_refused_fail_and_crash_are_each_told_apart")),
    M("other-case-lines-accepted", "lib", 'if m and m[1] == case:\n            key, val', 'if m:\n            key, val',
      K("ProbeOutput.lines_of_another_case_are_ignored", "RunnerKv.a_case_keeps_only_its_own_kv_lines")),
    M("canary-never-asked", "lib", 'if status in ("FAIL", "MISMATCH") and reason != "-":\n        return True, dead_count >= 2', 'if False:\n        return True, dead_count >= 2',
      K("ProbeOutput.a_failure_asks_for_a_canary_and_three_dead_canaries_stop_the_run", "RunnerEndToEnd.a_failure_runs_a_canary_and_records_it")),
    M("dead-device-never-stops", "lib", 'return True, dead_count >= 2', 'return True, False',
      K("ProbeOutput.a_failure_asks_for_a_canary_and_three_dead_canaries_stop_the_run", "RunnerEndToEnd.a_dead_device_stops_the_run_and_the_rest_are_not_run_with_the_reason")),
    M("permission-denied-is-an-error", "lib", 'if "insufficient permissions" in low or', 'if "xx never xx" in low or',
      K("Setters.classes_of_the_answers_a_geforce_gives", "ScriptCases.an_insufficient_permissions_answer_is_not_an_error")),
    M("setters-ignore-a-dead-device", "lib", 'if not canary_ok:\n        return "FAIL", "device_unusable_after_setters"', 'if False:\n        return "FAIL", "device_unusable_after_setters"',
      K("Setters.a_dead_device_after_the_setters_fails_the_probe", "ScriptCases.smi_setters_fails_when_the_device_is_dead_afterwards")),
    M("host-state-leak-needs-two-changes", "lib", 'return len(distinct) > 1, distinct', 'return len(distinct) > 2, distinct',
      K("Setters.a_host_state_that_changes_while_the_guest_runs_setters_is_a_leak", "Setters.the_hoststate_verdict_command_reads_the_csv")),
    M("uuids-kept", "lib", 'NOISE = re.compile(r"(deviceUUID|driverUUID|', 'NOISE = re.compile(r"(xdeviceUUID|xdriverUUID|',
      K("Vulkan.uuids_and_luids_are_dropped_and_the_rest_is_kept_sorted", "ScriptCases.vk_dump_writes_leaves_without_uuids_and_digests_per_group")),
    M("judge-pass-against-attribute", "c", 'if (rc == 0 && verified) return attr_ok ? PASS : (REASON = "worked_against_attribute", MISMATCH);',
      'if (rc == 0 && verified) return PASS;', K("CJudge.worked_and_the_attribute_says_yes_is_pass_and_worked_against_it_is_a_mismatch", "CJudgeTable.the_whole_judge_table_is_exactly_this")),
    M("judge-refusal-against-attribute-is-not-supported", "c", 'return attr_ok ? FAIL : NOT_SUPPORTED;', 'return NOT_SUPPORTED;',
      K("CJudge.refused_and_the_attribute_says_no_is_not_supported_and_refused_against_it_is_fail", "CJudgeTable.the_whole_judge_table_is_exactly_this")),
    M("permission-errors-are-errors", "lib", 'if "err_nvgpuctrperm" in low or "permission" in low and "counter" in low:', 'if False:',
      K("Tools.classes_of_the_answers_the_nvidia_tools_give", "Tools.a_counter_permission_answer_from_ncu_is_a_declined_case_not_a_failure")),
    M("a-broken-tool-is-not-supported", "lib", 'if classes[name] in ("TIMEOUT", "CRASH", "ERROR"):', 'if classes[name] in ("TIMEOUT", "CRASH"):',
      K("Tools.a_tool_that_breaks_fails_the_case_and_one_that_declines_is_not_supported", "Tools.a_sanitizer_that_errors_fails_the_case_with_the_tool_named")),
    M("tools-root-not-searched", "scripts", 'TOOL_ROOTS = ("", str(Path(__file__).resolve().parent / "tools"))', 'TOOL_ROOTS = ("",)',
      K("Tools.a_tool_pushed_beside_the_script_is_found_with_the_default_roots", "Tools.the_default_tool_roots_are_the_machine_and_the_directory_beside_the_script")),
    M("sample-not-seeded", "cts", 'random.Random("%s:%s:%s" % (seed, group, key)).shuffle(lst)', 'random.Random("fixed").shuffle(lst)',
      K("CtsGroups.the_same_seed_gives_the_same_list_and_another_seed_another_one", "CtsGroups.the_pick_inside_one_split_depends_on_the_seed")),
    M("sample-depth-first", "cts", 'k = order[i % len(order)]', 'k = order[0] if splits[order[0]] else order[i % len(order)]',
      K("CtsGroups.the_sample_is_breadth_first_one_case_per_split_in_turn", "CtsGroups.a_sample_as_small_as_the_number_of_splits_touches_each_split_once")),
    M("directory-prefix-too-loose", "cts", 'p.startswith("vk-default/%s/" % stem)', 'p.startswith("vk-default/%s" % stem)',
      K("CtsGroups.a_group_picks_up_its_files_under_a_directory", "CtsGroups.a_directory_whose_name_only_starts_like_the_group_is_not_part_of_it")),
    M("falsifier-not-required", "lib", 'if len(r["falsifier"].strip()) < 20:', 'if False:',
      K("CaseTable.a_case_without_a_falsifier_is_refused", "TableMore.a_falsifier_of_a_few_words_is_refused")),
    M("duplicate-ids-allowed", "lib", 'if r["id"] in seen:', 'if False:', K("CaseTable.ids_are_unique_and_a_duplicate_is_refused", "TableMore.an_id_repeated_across_kinds_is_refused")),
    M("row-in-the-gate-list", "runapps", "gap_probes|3600|GAP_PROBES_SUMMARY side=|python3 $B/share/gap_probes/gap_probes.py run --side $SIDE --out $O/gap_probes --egl $B/bin/egl_offscreen --emit\nvkcts|2700|", "vkcts|2700|",
      K("Wiring.the_row_is_a_discovery_row_with_its_own_summary_line", "WiringMore.run_apps_lists_the_row_but_all_does_not_expand_to_it")),
    M("census-forgets-the-row", "hook", "gap_probes) echo info;; esac;", "esac;",
      K("Wiring.the_host_census_asks_for_the_row_and_the_compare_step_runs_it", "WiringMore.the_census_function_of_the_hook_answers_info_for_the_row")),
]


def apply(text, old, new):
    if text.count(old) != 1:
        raise SystemExit("mutant text must occur exactly once (%d): %r" % (text.count(old), old[:70]))
    return text.replace(old, new)


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
        p = subprocess.run([sys.executable, "-c", COLLECT] + tests, cwd=tmp / "scripts/ci", capture_output=True, text=True, timeout=900)
        line = next((l for l in p.stdout.splitlines() if l.startswith("KF_MUTATION_RESULT=")), None)
        if line is None:
            return name, None, tests, p.stderr[-400:]
        return name, json.loads(line.split("=", 1)[1]), tests, ""


def main(argv):
    ap = argparse.ArgumentParser(description=__doc__.split("\n\n")[0])
    ap.add_argument("--jobs", type=int, default=4)
    ap.add_argument("only", nargs="?", default="")
    a = ap.parse_args(argv)
    base = run_one(("baseline", "lib", "import fnmatch", "import fnmatch", sorted({t for m in MUTANTS for t in m[4]})))
    if base[1] is None or base[1]["killers"]:
        print("BASELINE not green:", base[1] and base[1]["killers"][:5], base[3])
        return 2
    names = [m[0] for m in MUTANTS]
    if len(names) != len(set(names)):
        print("duplicate mutant names")
        return 2
    selected = [m for m in MUTANTS if a.only in m[0]]
    bad = 0
    with concurrent.futures.ThreadPoolExecutor(a.jobs) as pool:
        for name, res, tests, err in pool.map(run_one, selected):
            if res is None:
                print("ERROR  %s: %s" % (name, err))
                bad += 1
                continue
            killers = sorted(set(res["killers"]))
            status = "killed" if len(killers) >= 2 else "SURVIVED" if not killers else "WEAK"
            if status != "killed":
                bad += 1
            print("%-8s %s  (%d of %d named tests failed)" % (status, name, len(killers), len(tests)))
    print("MUTANTS total=%d not_killed_twice=%d" % (len(selected), bad))
    return 1 if bad else 0


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
