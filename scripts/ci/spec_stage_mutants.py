#!/usr/bin/env python3
"""Mutation checks of the SPECviewperf gate stage: every mutant needs two killers (stdlib only, GPU-free).

Each mutant is one exact-once text replacement (a few use two) in a private copy of lane.py, tarfilter.py,
specviewperf_stage.sh, vast_stage_specviewperf.sh or spec_stage_fakes.py; the tests of test_vast_spec_lane.py and test_vast_spec_stage.py
that the mutant names run against the copy, and the failing test names are the killers. The first run is the
baseline over every test of both files and must be green. The payload's own rules (grading, KF_SPEC, the time
check) are in payload_mutants.py; the report's rows in run_report_mutants.py.

    python3 scripts/ci/spec_stage_mutants.py [--jobs N] [substring-of-mutant-name]
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
    "lane": "scripts/bench/specviewperf/lane.py",
    "tarfilter": "scripts/bench/specviewperf/tarfilter.py",
    "stage": "scripts/bench/box/specviewperf_stage.sh",
    "wrapper": "scripts/ci/vast_stage_specviewperf.sh",
    "fakes": "scripts/ci/spec_stage_fakes.py",
    "licence": "scripts/bench/specviewperf/licence.py",
    "copybudget": "scripts/bench/specviewperf/copy_budget.py",
}
LANE = "test_vast_spec_lane"
STG = "test_vast_spec_stage"
NET = "test_vast_spec_online_ltsc"
CBT = "test_spec_copy_budget"
COPY = [
    "scripts/ci/test_vast_spec_lane.py",
    "scripts/ci/test_vast_spec_stage.py",
    "scripts/ci/test_vast_spec_online_ltsc.py",
    "scripts/ci/test_spec_copy_budget.py",
    "scripts/bench/windows/clockrule.py",
    "scripts/bench/windows/spec/licence_probe.ps1",
    "scripts/ci/vast_payload.py",
    "scripts/ci/payload_instruction.py",
    "scripts/ci/box_downloader.py",
    "scripts/ci/propimage_apps.py",
    "scripts/ci/kf_fetch.py",
    "scripts/ci/vast_report.py",
    "scripts/bench/native_xid_guard.py",
    "scripts/bench/native_fault_names.json",
    "scripts/bench/specviewperf/lanes.json",
    "scripts/bench/specviewperf/stream_url.py",   # the stage's user-URL-file path (the sandbox copies it, 2026-11-17)
    "scripts/ci/kf_trust.py",                     # and records the user's archive with it
    "scripts/bench/windows/spec/spec_guest.ps1",
    "scripts/bench/windows/spec/spec_run.ps1",
    *FILES.values(),
]
SOURCES = {key: (ROOT / path).read_text() for key, path in FILES.items()}


def M(name, key, old, new, *tests):
    """One mutant: replace `old` (exactly once) by `new` in file `key`; run the named test classes."""
    return (name, key, [(old, new)], list(tests))


def M2(name, key, edits, *tests):
    return (name, key, list(edits), list(tests))


G, D, T, S, TF = (LANE + ".GradeViewset", LANE + ".Decide", LANE + ".TimeoutFor", LANE + ".Scrub", LANE + ".TarFilter")
def t(cls, name):
    """The id of one test of test_vast_spec_stage.py (a lane test is killed with one stage test when a stage test sees it)."""
    return "%s.%s.test_%s" % (STG, cls, name)


HP, IN, PF, CP, TD, VD, GA, SE, TG, SC, WR, IR = (
    STG + ".HappyPath", STG + ".Inputs", STG + ".Preflight", STG + ".Copy", STG + ".Teardown", STG + ".Verdicts",
    STG + ".Gather", STG + ".Settle", STG + ".TimeGuard", STG + ".Scores", STG + ".Wrapper", STG + ".Interrupted",
)

MUTANTS = [
    M("fake-clock-uses-wall-time", "fakes", "1794000000 + $(cat",
      "$(' + REAL_DATE + ' +%s) + $(cat", STG + ".FakeClock"),
    # --- lane.py: one viewset's verdict ---------------------------------------------------------------
    M("grade-nonzero-exit-passes", "lane", "and total >= 1 and done == total and rc == 0 and resets == 0:",
      "and total >= 1 and done == total and resets == 0:", G),
    M("grade-reset-ignored", "lane", '    if resets is not None and resets > 0:\n        row["reason"] = "RESET"',
      '    if False:\n        row["reason"] = "RESET"', G, t("Verdicts", "a_guest_reset_under_a_completed_viewset_is_a_fail")),
    M("grade-incomplete-listing-passes", "lane", "and total >= 1 and done == total and rc == 0",
      "and total >= 1 and rc == 0", G),
    M("grade-failed-subtest-passes", "lane",
      '    if cli["subtests"] and any(s != "Completed" for _, s in cli["subtests"]):', "    if False:", G,
      t("Verdicts", "a_failed_subtest_is_a_fail_and_the_others_still_run")),
    M("grade-empty-viewset-passes", "lane", "and total >= 1 and done == total and rc == 0",
      "and done == total and rc == 0", G),
    M("grade-workload-status-ignored", "lane",
      'if cli["workload_status"] == "Completed" and total >= 1 and done == total and rc == 0 and resets == 0:',
      "if total >= 1 and done == total and rc == 0 and resets == 0:", G),
    M("grade-cut-is-a-fail", "lane", '        row.update(result="NOT_RUN", reason="TIME")\n        return row',
      '        row["reason"] = "TIME"\n        return row', G,
      t("TimeGuard", "a_viewset_cut_by_the_clock_that_does_not_end_is_a_retry_time_not_a_fail")),
    M("grade-full-timeout-passes", "lane",
      '    if timeout == "full":\n        row["reason"] = "TIMEOUT"\n        return row\n',
      '    if timeout == "full":\n        pass\n', G,
      t("Verdicts", "a_viewset_that_hangs_past_its_budget_is_a_fail_with_the_cli_killed"),
      t("TimeGuard", "a_viewset_with_its_full_budget_that_hangs_is_a_fail")),
    M("grade-dead-guest-is-not-a-failure", "lane",
      '    if timeout in ("dead", "silent"):\n        row["reason"] = "GUEST"\n        return row\n', "", G,
      t("Verdicts", "a_guest_that_dies_during_a_viewset_is_a_fail_and_the_next_viewset_gets_a_fresh_boot")),
    M("grade-gather-is-a-failure", "lane", '        row.update(result="NOT_RUN", reason="GATHER")\n        return row',
      '        row["reason"] = "NOTDONE"\n        return row', G,
      t("Gather", "a_second_gather_failure_is_not_run_and_a_retry_never_a_pass")),
    M("grade-refusal-undetected", "lane", '    if cli["refused"] and not cli["subtests"]:', "    if False:", G,
      t("Verdicts", "a_viewset_refused_by_spec_validation_is_a_fail_never_excluded")),
    M("grade-missing-counters-are-not-a-retry", "lane",
      '    if resets is None:\n        row.update(result="NOT_RUN", reason="NO_TDR")', '    if False:\n        row.update(result="NOT_RUN", reason="NO_TDR")', G,
      t("Verdicts", "missing_reset_counters_are_a_retry_not_a_pass")),
    # --- lane.py: the verdict of the stage ----------------------------------------------------------------
    M("decide-red-host-reference-ignored", "lane",
      '        if hostref_before != "green" or hostref_after != "green":\n            return "RETRY", "HOSTREF"\n        return "FAIL", "KAYFABE"\n    if len(passed)',
      '        return "FAIL", "KAYFABE"\n    if len(passed)', D,
      t("Verdicts", "a_red_host_reference_after_a_failure_makes_it_a_retry")),
    M("decide-bar1-ignored", "lane", '        if bar1:\n            return "RETRY", "BAR1_EXCEPTION"\n', "", D,
      t("Verdicts", "the_vast_bar1_refusal_is_the_documented_exception_not_a_fail")),
    M("decide-pass-on-a-red-reference-after", "lane",
      '        if hostref_after != "green":\n            return "RETRY", "HOSTREF"\n        if xid_delta:', "        if xid_delta:", D,
      t("Verdicts", "a_red_host_reference_after_a_pass_is_not_a_pass")),
    M("decide-xid-ignored", "lane", '        if xid_delta:\n            return "FAIL", "KAYFABE"\n', "", D,
      t("Verdicts", "a_new_host_xid_over_a_run_that_passed_is_a_fail")),
    M("decide-abort-hidden-by-passed-rows", "lane", "    if len(passed) == len(expected) and not abort:",
      "    if len(passed) == len(expected):", D, LANE + ".Cli"),
    # --- lane.py: the time guard ------------------------------------------------------------------------------
    M("timeout-never-skips", "lane", '    if usable < need_s(workload, tbl):\n        return "skip", 0',
      '    if False:\n        return "skip", 0', T,
      t("TimeGuard", "a_viewset_without_time_is_not_started_and_the_lane_is_a_retry_time")),
    M("timeout-spends-the-tail", "lane", '    usable = int(remaining_s) - tbl["tail_reserve_s"]',
      "    usable = int(remaining_s)", T, t("TimeGuard", "the_stage_reserves_the_tail_for_the_graceful_stop")),
    M("timeout-never-cut", "lane", '    if usable < budget:\n        return "cut", usable\n', "", T,
      t("TimeGuard", "a_viewset_cut_by_the_clock_that_does_not_end_is_a_retry_time_not_a_fail")),
    M("scrub-keeps-everything", "lane",
      "        if len(line) <= 240 and (STATUS_LINE.match(line) or LISTING_LINE.match(line) or NOTE_LINE.match(line)):",
      "        if True:", S, t("Scores", "the_status_extract_of_the_cli_log_holds_only_status_lines")),
    M("lane-both-is-one-lane", "lane", '        return list(tbl["lanes"]["lanea"]) + list(tbl["lanes"]["laneb"])',
      '        return list(tbl["lanes"]["laneb"])', LANE + ".Lanes", t("HappyPath", "both_runs_the_nine_viewsets_lane_a_then_lane_b")),
    # --- tarfilter.py -----------------------------------------------------------------------------------------------
    M("filter-keeps-other-lanes", "tarfilter", "    return vid is None or vid in wanted", "    return True", TF,
      t("Copy", "only_the_lanes_viewsets_are_streamed_into_the_guest")),
    M("filter-drops-the-tool", "tarfilter", "    return vid is None or vid in wanted", "    return vid in wanted", TF),
    M("filter-accepts-links", "tarfilter", "            if not (member.isfile() or member.isdir()):", "            if False:", TF,
      t("Copy", "an_archive_with_a_link_member_is_refused_as_media")),
    M("filter-accepts-dotdot", "tarfilter", ' or ".." in parts', "", TF,
      t("Copy", "an_archive_with_a_dotdot_member_is_refused_as_media")),
    M("filter-accepts-absolute-paths", "tarfilter", 'name.startswith("/") or ', "", TF,
      t("Copy", "an_archive_with_an_absolute_member_is_refused_as_media")),
    M("filter-accepts-a-truncated-stream", "tarfilter", '    if source.tail != b"\\0" * 1024:', "    if False:", TF),
    M("filter-manifest-bytes-wrong", "tarfilter", "stats[dirname] = (f + 1, b + member.size)",
      "stats[dirname] = (f + 1, b)", TF, t("HappyPath", "lane_a_passes_in_order_with_one_row_per_viewset")),
    # --- specviewperf_stage.sh -------------------------------------------------------------------------------------------
    M("stage-no-pin-check", "stage", '[ -n "$PIN" ] || abort MEDIA', "true || abort MEDIA", IN),
    M("stage-sha-mismatch-is-provider", "stage", "    3 | 4) abort MEDIA", "    3 | 4) abort PROVIDER", IN),
    M("stage-fetch-failure-is-operator", "stage", '    *) abort PROVIDER "the download failed', '    *) abort OPERATOR "the download failed', IN),
    M("stage-fetch-timeout-is-provider", "stage", "    124) abort TIME", "    124) abort PROVIDER", IN),
    M("stage-no-settle", "stage", "    settle\n    return 0", "    return 0", SE,
      t("Verdicts", "a_fresh_boot_waits_for_the_settle_time_again")),
    M("stage-no-time-guard", "stage", '        if [ "$mode" = skip ]; then\n            note "time guard: $w not started ($(left)s left)"',
      '        if false; then\n            note "time guard: $w not started ($(left)s left)"', TG),
    M("stage-no-time-guard-after-the-boot", "stage",
      '        if [ "$mode" = skip ]; then\n            note "time guard: $w not started after the boot ($(left)s left)"',
      '        if false; then\n            note "time guard: $w not started after the boot ($(left)s left)"',
      t("TimeGuard", "a_viewset_that_no_longer_fits_after_the_boot_is_not_started"),
      t("TimeGuard", "enscape_that_no_longer_fits_after_the_fresh_boot_of_a_dead_blender_is_not_started")),
    M("stage-archive-kept-after-the-copy", "stage", 'rm -f -- "$ZST"\nsay "SPEC_COPY_DELETED removed=archive"\n', "", TD),
    M("stage-overlay-kept", "stage",
      '            rm -f -- "$VM/disk/$OVERLAY.qcow2"; gone+="${gone:+,}overlay"\n', "            true\n", TD),
    M("stage-no-final-delete", "stage", "    delete_copy\n    for n in", "    for n in", TD),
    M("stage-guest-left-running", "stage", '    teardown_guest\n    if [ "$GUEST_STARTED" = 1 ] && [ "$HOSTREF_BEFORE" = green ]',
      '    if [ "$GUEST_STARTED" = 1 ] && [ "$HOSTREF_BEFORE" = green ]', TD),
    M("stage-copy-unverified", "stage",
      '    grep -qxF "$l" "$WORK/guest_view.txt" || abort TRANSPORT "the guest holds a different tree than the one streamed ($l)"',
      "    true", CP),
    M("stage-host-reference-before-ignored", "stage", '[ "$HOSTREF_BEFORE" = green ] || abort HOSTREF', "true || abort HOSTREF", PF),
    M("stage-rows-not-streamed", "stage", '            emit_row "$GRADE_LINE"\n        fi\n        licprobe "after_$w"', '            echo "$GRADE_LINE" >> "$ROWS"\n        fi\n        licprobe "after_$w"',
      t("HappyPath", "lane_a_passes_in_order_with_one_row_per_viewset"), t("Wrapper", "the_streamed_log_holds_only_status_lines_and_the_verdict")),
    M("stage-small-card-keeps-the-8192-default", "stage", "        export WINVM_KF3_FB_MB=$(((vram_mb - 2048) / 1024 * 1024))", "        true",
      t("Copy", "the_kf3_boot_leaves_2_gib_of_a_small_card_as_the_gate_does"), t("Copy", "a_10_gib_card_is_still_a_small_card")),
    M("stage-large-card-gets-an-fb-too", "stage", '[ "$vram_mb" -le 10240 ]', '[ "$vram_mb" -le 99999 ]',
      t("Copy", "a_12_gib_card_keeps_the_default_fb"), t("Copy", "a_24_gib_card_keeps_the_default_fb")),
    M("stage-stale-logs-of-the-same-tag-stay", "stage", '    if mkdir -p "$old" && mv -- "${stale[@]}" "$old/"; then', '    if true; then',
      t("Copy", "logs_of_an_earlier_run_with_the_same_tag_are_moved_aside"), t("Copy", "the_new_boot_log_holds_nothing_of_the_old_run")),
    M("stage-narrowed-verdict-unmarked", "stage", '[ -z "${KF_SPEC_TABLE:-}" ] || NARROWED=" NARROWED"', "true",
      t("Copy", "a_narrowed_table_marks_every_verdict_line_and_the_notes"), t("Copy", "a_narrowed_failure_and_retry_are_marked_too")),
    M("wrapper-drops-the-narrowed-verdict", "wrapper", "( NARROWED)?$' \"$log\"", "$' \"$log\"",
      t("Copy", "a_narrowed_table_marks_every_verdict_line_and_the_notes"), t("Copy", "a_narrowed_failure_and_retry_are_marked_too")),
    M("stage-narrowed-run-without-a-note", "stage", '[ -z "$NARROWED" ] || note "narrowed table=', 'true || note "narrowed table=',
      t("Copy", "a_narrowed_table_marks_every_verdict_line_and_the_notes"), t("Copy", "a_narrowed_failure_and_retry_are_marked_too")),
    M("lane-decide-passes-an-empty-lane", "lane", '    if not expected:\n', '    if False:\n',
      LANE + ".Cli.test_decide_on_an_empty_lane_is_not_a_pass_through_the_command_line_either",
      LANE + ".Cli.test_decide_refuses_an_empty_lane_as_a_pass"),
    M("stage-log-ownership-by-tag-prefix", "stage", "^${TAG}_(copy|run)[0-9]+_(run\\.(log|out)|b[0-9]+_.+)$", "^${TAG}_.*$",
      t("Copy", "logs_of_a_tag_that_extends_this_tag_are_not_moved"), t("Copy", "the_aside_directory_holds_only_the_logs_of_this_tag")),
    M("stage-no-guest-ram", "stage", "    if ! WINVM_NET=$SPEC_NET WINVM_RAM_MB=$GUEST_RAM_MB bash", "    if ! WINVM_NET=$SPEC_NET bash", CP),
    M("stage-evidence-unscrubbed", "stage", '--scrub-out "$EVD/$w.cli.txt" ${abn:+--timeout "$abn"})',
      '${abn:+--timeout "$abn"}); cp "$d/cli.log" "$EVD/$w.cli.txt"', SC),
    M("stage-raw-output-in-the-evidence-directory", "stage", "WORK=${KF_SPEC_PRIVATE:-$EVD/work}", "WORK=$EVD/work", SC, WR),
    M("stage-gather-never-retried", "stage", 'if [ "$attempt" = 1 ] && [[ "$GRADE_LINE" == *"reason=GATHER"* ]]; then',
      "if false; then", GA),
    M2("stage-gather-retried-forever", "stage",
       [("for attempt in 1 2; do", "for attempt in 1 2 3 4 5; do"), ('if [ "$attempt" = 1 ] && [[', "if [[")], GA),
    M("stage-dead-guest-reused", "stage", '    if [ "$FRESH" = 0 ] && qemu_alive && guest_answers; then return 0; fi',
      '    return 0', t("Verdicts", "a_guest_that_dies_during_a_viewset_is_a_fail_and_the_next_viewset_gets_a_fresh_boot"),
      t("Verdicts", "the_boots_of_a_dying_guest_are_limited"), t("Verdicts", "a_guest_that_dies_every_time_is_boot_limited_and_every_row_is_a_fail_or_not_run")),
    M("stage-unlimited-boots", "stage", '    [ "$RUN_BOOTS" -lt "$MAX_BOOTS" ] || { note "max_guest_boots reached"; return 1; }', "    true",
      t("Verdicts", "the_boots_of_a_dying_guest_are_limited"), t("Verdicts", "a_guest_that_dies_every_time_is_boot_limited_and_every_row_is_a_fail_or_not_run")),
    M("stage-term-ignored", "stage", "trap 'exit 143' TERM INT HUP\n", "", IR),
    M("stage-retry-prints-pass", "stage", '        *) VERDICT=RETRY; rc_final=3; say "SPECVIEWPERF_STAGE_RETRY class=${cls:-OPERATOR}$NARROWED" ;;',
      '        *) VERDICT=RETRY; rc_final=3; say "SPECVIEWPERF_STAGE_PASS" ;;',
      t("Inputs", "a_sha256_mismatch_is_retry_media_and_no_viewset_runs"), t("Inputs", "an_object_the_worker_does_not_know_is_retry_media")),
    M("stage-fail-exits-zero", "stage", "        FAIL) VERDICT=FAIL; rc_final=1; say", "        FAIL) VERDICT=FAIL; rc_final=0; say",
      t("Verdicts", "the_exit_code_of_a_fail_is_1_and_of_a_retry_is_3"), t("Wrapper", "the_wrapper_exit_code_is_the_stages")),
    M("stage-no-gate-pass-check", "stage",
      "grep -q '^WINDOWS_GATE_PASS ' \"$PROV/${T}_windows.log\" 2> /dev/null || abort OPERATOR", "true || abort OPERATOR", PF),
    M("stage-no-disk-check", "stage", '[ "${avail_gb:-0}" -ge "$need_gb" ] || abort PROVIDER', "true || abort PROVIDER", PF),
    M("stage-no-ram-check", "stage", '[ "${mem_gb:-0}" -ge 24 ] || abort PROVIDER', "true || abort PROVIDER", PF),
    M("stage-run-boot-without-the-gate-flags", "stage", '[ -z "$FLAGS" ] || args+=(--env-file "$FLAGS")', "true", CP),
    M("stage-copy-boot-with-kf3", "stage", "        args+=(--no-kf3)", "        args+=()", CP),
    M("stage-gate-overlay-unsealed", "stage", 'chmod 0400 "$VM/disk/$GATE_TAG.qcow2"', "true", CP),
    M("stage-cut-viewset-is-a-fail", "stage", 'if [ "$mode" = cut ]; then abn="cut"; else abn="full"; fi', 'abn="full"', TG),
    M("stage-hung-cli-not-killed", "stage", "            gspec 120 kill > /dev/null\n", "",
      t("Verdicts", "a_viewset_that_hangs_past_its_budget_is_a_fail_with_the_cli_killed"),
      t("TimeGuard", "a_viewset_cut_by_the_clock_that_does_not_end_is_a_retry_time_not_a_fail")),
    M("stage-no-fresh-boot-after-a-reset", "stage",
      '        case "$GRADE_LINE" in *"reason=RESET"* | *"reason=TIMEOUT"* | *"reason=GUEST"*) FRESH=1 ;; esac\n', "",
      t("Verdicts", "a_viewset_that_reset_the_guest_is_followed_by_a_fresh_boot"),
      t("Verdicts", "a_viewset_killed_for_its_budget_is_followed_by_a_fresh_boot")),
    M("stage-budget-not-recomputed-after-the-boot", "stage",
      '        read -r mode secs < <(lane_py timeout "$w" "$(left)")\n        if [ "$mode" = skip ]; then\n            note "time guard: $w not started after the boot ($(left)s left)"\n            synthetic_row "$w" NOT_RUN TIME\n            break\n        fi\n', "",
      t("TimeGuard", "the_budget_of_a_viewset_after_a_fresh_boot_comes_from_the_time_left_after_the_boot"),
      t("TimeGuard", "a_viewset_that_no_longer_fits_after_the_boot_is_not_started")),
    M("stage-refused-launch-is-a-guest-failure", "stage", "        launch) abn=launch ;;", "        launch) abn=silent ;;",
      t("Verdicts", "a_launch_the_guest_script_refuses_is_a_retry_operator_never_a_guest_failure"),
      t("Verdicts", "a_refused_launch_in_lane_b_is_a_retry_with_exit_3")),
    M("stage-boot-failure-is-not-a-guest-failure", "stage",
      '        if [ "$GUEST_DEAD_ROW" = 0 ]; then synthetic_row "$w" FAIL GUEST; GUEST_DEAD_ROW=1; else synthetic_row "$w" NOT_RUN BOOT; fi\n        continue',
      '        synthetic_row "$w" NOT_RUN BOOT\n        continue',
      t("Verdicts", "a_kf3_boot_that_fails_after_the_gate_booted_the_same_image_is_a_guest_failure"),
      t("Verdicts", "the_vast_bar1_refusal_is_the_documented_exception_not_a_fail")),
    # --- 2026-11-17: the guest's network and the licence probe (test_vast_spec_online_ltsc.py) ----------------------------------
    M("stage-net-default-offline", "stage", "SPEC_NET=${KF_SPEC_NET:-online}", "SPEC_NET=${KF_SPEC_NET:-offline}",
      NET + ".GuestNetwork.test_every_boot_is_online_on_the_real_clock_by_default", NET + ".GuestNetwork.test_an_empty_setting_is_the_default",
      NET + ".GuestNetwork.test_the_stage_setting_wins_over_a_wrongly_inherited_winvm_net"),
    M("stage-net-not-handed-to-the-launcher", "stage", "    if ! WINVM_NET=$SPEC_NET WINVM_RAM_MB=$GUEST_RAM_MB bash", "    if ! WINVM_RAM_MB=$GUEST_RAM_MB bash",
      NET + ".GuestNetwork.test_every_boot_is_online_on_the_real_clock_by_default", NET + ".GuestNetwork.test_the_stage_setting_wins_over_a_wrongly_inherited_winvm_net"),
    M("stage-net-offline-only-for-the-run-boot", "stage", "    if ! WINVM_NET=$SPEC_NET WINVM_RAM_MB=$GUEST_RAM_MB bash",
      '    if ! WINVM_NET=$([ "$kind" = run ] && echo "$SPEC_NET" || echo offline) WINVM_RAM_MB=$GUEST_RAM_MB bash',
      NET + ".GuestNetwork.test_every_boot_is_online_on_the_real_clock_by_default", NET + ".GuestNetwork.test_the_stage_setting_wins_over_a_wrongly_inherited_winvm_net"),
    M("stage-net-any-value-accepted", "stage", 'case "$SPEC_NET" in online | offline) ;; *) echo "$USAGE_LINE"; exit 2 ;; esac', 'case "$SPEC_NET" in *) ;; esac',
      NET + ".GuestNetwork.test_any_other_value_is_a_usage_error_and_boots_nothing",
      NET + ".GuestNetwork.test_the_stage_setting_wins_over_a_wrongly_inherited_winvm_net"),
    M("stage-no-network-note", "stage", 'note "guest network=$SPEC_NET (', 'true "guest network=$SPEC_NET (',
      NET + ".GuestNetwork.test_the_stage_says_which_network_the_guest_has", NET + ".GuestNetwork.test_the_network_is_recorded_in_the_stage_log_of_the_evidence"),
    M("stage-licence-probe-off-by-default", "stage", "LIC_PROBE=${KF_SPEC_LICENCE_PROBE-1}", "LIC_PROBE=${KF_SPEC_LICENCE_PROBE-}",
      NET + ".LicenceProbe.test_the_probe_is_on_by_default_at_boot_after_each_viewset_and_at_the_end", NET + ".LicenceProbe.test_every_state_line_says_state_and_network"),
    M("stage-licence-probe-cannot-be-switched-off", "stage", 'case "$LIC_PROBE" in 0 | off | no) LIC_PROBE="" ;; esac', "true",
      NET + ".LicenceProbe.test_zero_and_off_switch_the_probe_off", NET + ".ShutdownClassification.test_without_the_probe_the_row_is_emitted_at_once_as_a_bare_guest"),
    M("stage-licence-no-periodic-probe", "stage", '        if [ -n "$LIC_PROBE" ] && [ $(($(date +%s) - LAST_LIC)) -ge 900 ]; then', '        if false; then',
      NET + ".LicenceProbe.test_a_long_viewset_is_probed_at_least_every_15_minutes", NET + ".LicenceProbe.test_the_periodic_probes_are_state_lines_like_the_others"),
    M("stage-licence-probe-every-45-minutes", "stage", "-ge 900 ]; then licprobe", "-ge 2700 ]; then licprobe",
      NET + ".LicenceProbe.test_a_long_viewset_is_probed_at_least_every_15_minutes", NET + ".LicenceProbe.test_the_periodic_probes_are_state_lines_like_the_others"),
    M("stage-licence-no-end-probe", "stage", '        licprobe "end"', "        true",
      NET + ".LicenceProbe.test_the_probe_is_on_by_default_at_boot_after_each_viewset_and_at_the_end",
      NET + ".LicenceProbe.test_one_probe_per_moment_and_no_more"),
    M("stage-licence-states-not-kept", "stage", '    echo "$line" >> "$LIC_STATES"\n', "",
      NET + ".LicenceProbe.test_the_states_are_kept_in_the_evidence_as_printed", NET + ".LicenceProbe.test_the_raw_readout_stays_in_the_private_directory"),
    M("stage-licence-raw-readout-in-the-evidence", "stage", '    raw="$WORK/licence_$lbl.txt"', '    raw="$EVD/licence_$lbl.txt"',
      NET + ".LicenceProbe.test_the_raw_readout_stays_in_the_private_directory", NET + ".LicenceProbe.test_no_key_id_computer_or_account_leaves_the_readout"),
    M("stage-licence-shutdown-never-classified", "stage", "        row=${row/ reason=GUEST / reason=GUEST_SHUTDOWN_LICENCE }\n", "",
      NET + ".ShutdownClassification.test_a_licence_1074_makes_the_row_guest_shutdown_licence_with_the_text",
      NET + ".ShutdownClassification.test_the_last_viewset_that_dies_gets_one_more_boot_to_be_read"),
    M("stage-licence-dead-row-not-held-for-the-probe", "stage", '[[ "$GRADE_LINE" == *" reason=GUEST "* ]] && ! qemu_alive; then', "false; then",
      NET + ".ShutdownClassification.test_a_licence_1074_makes_the_row_guest_shutdown_licence_with_the_text",
      NET + ".ShutdownClassification.test_the_last_viewset_that_dies_gets_one_more_boot_to_be_read"),
    M("stage-licence-held-row-lost-at-the-end", "stage", "    final_licence_read\n    flush_pending\n", "    final_licence_read\n",
      NET + ".ShutdownClassification.test_the_last_viewset_that_dies_gets_one_more_boot_to_be_read",
      NET + ".ShutdownClassification.test_a_row_without_a_boot_left_to_read_it_stays_a_bare_guest_row_once",
      NET + ".ShutdownClassification.test_the_forensic_boot_is_probed_like_any_other_and_says_so"),
    M("stage-licence-no-forensic-boot-for-the-last-viewset", "stage", '            boot_guest run && licprobe "forensic_boot${RUN_BOOTS}"', "            true",
      NET + ".ShutdownClassification.test_the_last_viewset_that_dies_gets_one_more_boot_to_be_read",
      NET + ".ShutdownClassification.test_the_forensic_boot_is_probed_like_any_other_and_says_so"),
    M("stage-licence-forensic-boot-ignores-the-boot-limit", "stage", '        if [ "$RUN_BOOTS" -lt "$MAX_BOOTS" ] && [ "$(left)" -gt 1200 ]; then', '        if [ "$(left)" -gt 1200 ]; then',
      NET + ".ShutdownClassification.test_no_forensic_boot_beyond_the_boot_limit",
      NET + ".ShutdownClassification.test_a_row_without_a_boot_left_to_read_it_stays_a_bare_guest_row_once"),
    M("stage-licence-shutdown-line-before-the-row-lost", "stage", "        say \"$LIC_SHUTDOWN_LINE\"\n        row=", "        row=",
      NET + ".ShutdownClassification.test_the_shutdown_line_comes_before_its_row_and_the_order_of_the_rows_is_the_lanes",
      NET + ".ShutdownClassification.test_a_licence_1074_makes_the_row_guest_shutdown_licence_with_the_text"),
    M("stage-licence-shutdown-without-the-previous-boot-floor", "stage", '${PREV_LASTBOOT:+--since "$PREV_LASTBOOT"}', "",
      NET + ".ShutdownClassification.test_a_licence_1074_older_than_the_boot_that_went_down_is_not_the_cause",
      NET + ".ShutdownClassification.test_an_old_1074_is_not_the_cause_of_the_last_viewset_either"),
    M("licence-any-1074-is-a-licence-shutdown", "licence", "        if not (re.search(r\"wlms\", ev[\"process\"], re.I) or LICENCE_WORDS.search(text)):\n            continue\n", "",
      NET + ".ShutdownClassification.test_a_guest_that_went_down_for_another_reason_stays_a_bare_guest_row", NET + ".LicenceModule.test_other_events_do_not"),
    M("licence-since-ignored", "licence", "        if floor is not None and ev[\"time\"] < floor:\n            continue\n", "",
      NET + ".ShutdownClassification.test_a_licence_1074_older_than_the_boot_that_went_down_is_not_the_cause", NET + ".LicenceModule.test_since_is_a_floor_on_the_guest_clock"),
    M("licence-process-keeps-the-computer-name", "licence", '.rsplit("/", 1)[-1].split(" ")[0]', '.rsplit("/", 1)[-1]',
      NET + ".LicenceModule.test_a_licence_event_is_found_and_the_process_is_its_basename", NET + ".ShutdownClassification.test_the_classification_passes_the_wrapper_and_the_payload_pattern"),
    M("licence-key-shaped-groups-kept", "licence", 'ACCOUNTLIKE.sub(" ", KEYLIKE.sub(" ", GUIDLIKE.sub(" ", str(value))))', 'ACCOUNTLIKE.sub(" ", GUIDLIKE.sub(" ", str(value)))',
      NET + ".LicenceModule.test_nothing_secret_survives_the_alphabet", NET + ".LicenceModule.test_a_product_key_in_the_expiry_text_is_dropped"),
    M("licence-account-names-kept", "licence", 'ACCOUNTLIKE.sub(" ", KEYLIKE.sub(" ", GUIDLIKE.sub(" ", str(value))))', 'KEYLIKE.sub(" ", GUIDLIKE.sub(" ", str(value)))',
      NET + ".LicenceModule.test_nothing_secret_survives_the_alphabet", NET + ".ShutdownClassification.test_the_account_and_the_computer_of_the_event_text_are_not_printed"),
    M("licence-newest-event-not-preferred", "licence", 'if best is None or ev["time"] >= best["time"]:', 'if best is None:',
      NET + ".LicenceModule.test_the_newest_licence_event_is_reported_whatever_the_order_of_the_readout",
      NET + ".LicenceModule.test_the_newest_of_three_licence_events_is_reported"),
    # --- 2026-11-20: the copy budget and the copy progress (copy_budget.py; test_spec_copy_budget.py, test_vast_spec_stage.CopyBudget) ---------
    M("copy-budget-ignores-the-knob", "copybudget", '    if knob is not None:\n        unclamped, source = knob, "knob"', '    if False:\n        unclamped, source = knob, "knob"',
      CBT + ".Budget", t("CopyBudget", "the_knob_sets_the_budget"), t("CopyBudget", "a_larger_knob_than_the_default_is_honoured_too")),
    M("copy-budget-derived-from-the-wrong-size", "copybudget", "for w in lane_mod.lane_ids(lane, tbl))", 'for w in tbl["viewsets"])',
      CBT + ".Derivation", t("CopyBudget", "the_default_budget_is_derived_from_the_lane_and_no_fixed_70_minutes"), t("CopyBudget", "the_log_says_where_the_budget_comes_from")),
    M("copy-budget-not-clamped-to-the-time-left", "copybudget", "    return min(unclamped, max(int(left_s), 1)), source, nbytes, unclamped", "    return unclamped, source, nbytes, unclamped",
      CBT + ".Budget", t("CopyBudget", "the_budget_never_exceeds_the_time_left")),
    M("copy-budget-floor-rate-ten-times-faster", "copybudget", "FLOOR_RATE_BYTES_S = 5_000_000", "FLOOR_RATE_BYTES_S = 50_000_000",
      CBT + ".Derivation", t("CopyBudget", "the_default_budget_is_derived_from_the_lane_and_no_fixed_70_minutes")),
    M("copy-budget-without-the-margin", "copybudget", "MARGIN_PCT = 20 ", "MARGIN_PCT = 0 ",
      CBT + ".Derivation", t("CopyBudget", "the_default_budget_is_derived_from_the_lane_and_no_fixed_70_minutes")),
    M("copy-bad-knob-accepted", "copybudget", "re.ASCII) is None or not lo <= int(text) <= hi:", "re.ASCII) is None:",
      CBT + ".Knob", t("CopyBudget", "a_bad_budget_knob_is_refused_by_name_before_anything_starts"), t("CopyBudget", "a_bad_progress_interval_knob_is_refused_by_name_before_anything_starts")),
    M("copy-progress-line-never-written", "copybudget", '                os.write(fd, line.encode("ascii"))', "                pass",
      CBT + ".Pipe", t("CopyBudget", "a_copy_leaves_a_progress_line_with_every_byte_it_moved"), t("CopyBudget", "progress_lines_come_while_the_copy_runs")),
    M("copy-progress-only-at-the-end", "copybudget", "            if self.elapsed() >= due:", "            if False:",
      CBT + ".Pipe", t("CopyBudget", "progress_lines_come_while_the_copy_runs")),
    M2("copy-abort-line-without-bytes", "copybudget",
       [("did not finish inside its budget: bytes=%d expected_bytes=%d rate_mb_s", "did not finish inside its budget: expected_bytes=%d rate_mb_s"),
        ("        % (nbytes, expected, rate_mb_s(nbytes, elapsed_s), int(elapsed_s)", "        % (expected, rate_mb_s(nbytes, elapsed_s), int(elapsed_s)")],
       CBT + ".Lines", CBT + ".Pipe", t("CopyBudget", "the_abort_line_names_the_bytes_the_rate_and_the_budget_that_would_have_sufficed")),
    M("copy-abort-line-estimates-at-the-floor-rate", "copybudget", "    return math.ceil(round(max(expected, nbytes) * elapsed_s / nbytes, 6))", "    return math.ceil(round(expected / FLOOR_RATE_BYTES_S, 6))",
      CBT + ".Rates", CBT + ".Lines", CBT + ".Pipe"),
    M("copy-abort-line-divides-by-zero-bytes", "copybudget", "    if nbytes <= 0 or elapsed_s <= 0:\n        return None", "    if elapsed_s <= 0:\n        return None",
      CBT + ".Rates", CBT + ".Lines", t("CopyBudget", "a_copy_that_never_moved_a_byte_says_so")),
    M("copy-pipe-drops-a-byte", "copybudget", "            view = memoryview(chunk)\n", "            view = memoryview(chunk[:-1] if len(chunk) > 1 else chunk)\n",
      CBT + ".Pipe", t("HappyPath", "lane_a_passes_in_order_with_one_row_per_viewset"), t("CopyBudget", "the_pipeline_status_of_each_stage_is_named_in_the_log")),
    M("stage-copy-timeout-is-a-fixed-number", "stage", '    | timeout "$(cap "$COPY_BUDGET")" bash "$WV" ssh', '    | timeout "$(cap 4200)" bash "$WV" ssh',
      t("CopyBudget", "the_default_budget_is_derived_from_the_lane_and_no_fixed_70_minutes"), t("CopyBudget", "the_knob_sets_the_budget"), t("CopyBudget", "a_larger_knob_than_the_default_is_honoured_too")),
    M("stage-copy-without-the-byte-counter", "stage",
      '    | copy_py pipe --status "$COPY_PROGRESS" --log "$LOG" --budget "$COPY_BUDGET" --expected "$COPY_EXPECTED" --note-s "${KF_SPEC_COPY_NOTE_S:-}" 2> "$WORK/copy_progress.err" \\\n', "",
      t("CopyBudget", "a_copy_leaves_a_progress_line_with_every_byte_it_moved"), t("CopyBudget", "the_pipeline_status_of_each_stage_is_named_in_the_log"), t("CopyBudget", "the_status_of_the_guests_tar_is_the_last_of_five")),
    M("stage-copy-guest-tar-status-index-stale", "stage", '[ "${cps[4]}" != 124 ] || abort TIME', '[ "${cps[3]}" != 124 ] || abort TIME',
      t("CopyBudget", "a_copy_that_runs_out_of_time_is_retry_time_with_every_row_not_run_setup"), t("CopyBudget", "the_abort_line_names_the_bytes_the_rate_and_the_budget_that_would_have_sufficed")),
    M("stage-bad-copy-knob-not-refused-up-front", "stage", 'cb_msg=$(copy_py check --budget "${KF_SPEC_COPY_BUDGET_S:-}" --note-s "${KF_SPEC_COPY_NOTE_S:-}") || abort OPERATOR "${cb_msg:-the copy knobs could not be checked}"\n', "",
      t("CopyBudget", "a_bad_budget_knob_is_refused_by_name_before_anything_starts"), t("CopyBudget", "a_bad_progress_interval_knob_is_refused_by_name_before_anything_starts")),
    M("stage-copy-knob-not-handed-to-the-helper", "stage", '--knob "${KF_SPEC_COPY_BUDGET_S:-}" --left', '--knob "" --left',
      t("CopyBudget", "the_knob_sets_the_budget"), t("CopyBudget", "a_larger_knob_than_the_default_is_honoured_too")),
    M("stage-copy-time-abort-says-nothing-of-the-bytes", "stage", '[ "${cps[4]}" != 124 ] || abort TIME "$(copy_py abort-text --status "$COPY_PROGRESS" --budget "$COPY_BUDGET" --expected "$COPY_EXPECTED")"',
      '[ "${cps[4]}" != 124 ] || abort TIME "the copy into the guest did not finish inside its budget"',
      t("CopyBudget", "the_abort_line_names_the_bytes_the_rate_and_the_budget_that_would_have_sufficed"), t("CopyBudget", "the_budget_that_would_have_sufficed_is_more_than_the_time_already_spent"),
      t("CopyBudget", "a_copy_that_never_moved_a_byte_says_so")),
    M("stage-copy-progress-notes-outside-the-stage-log", "stage", '--log "$LOG" --budget "$COPY_BUDGET"', '--log "$WORK/copy_progress.log" --budget "$COPY_BUDGET"',
      t("CopyBudget", "a_copy_leaves_a_progress_line_with_every_byte_it_moved"), t("CopyBudget", "progress_lines_come_while_the_copy_runs")),
    # --- vast_stage_specviewperf.sh -------------------------------------------------------------------------------------------
    M("wrapper-no-start-marker", "wrapper", 'echo "SPEC_STAGE_START lane=$startlane"\n', "", WR),
    M("wrapper-start-marker-echoes-the-environment", "wrapper", "*) startlane=none ;; esac", "*) startlane=${KF_SPEC_LANE:-none} ;; esac", WR),
    M("wrapper-filter-removed", "wrapper", '| grep --line-buffered -E "$status"', "| cat", WR),
    M("wrapper-row-pattern-unanchored", "wrapper",
      "failed=([A-Za-z0-9_,]{1,120}|-)$'\nstatus+='|^[a-z][a-z_]*_[0-9]{2} --- FAILED$'",
      "failed=([A-Za-z0-9_,]{1,120}|-)'\nstatus+='|^[a-z][a-z_]*_[0-9]{2} --- FAILED$'", WR),
    M("wrapper-counts-pattern-unanchored", "wrapper", "of=[0-9]{1,2}$'\nstatus+='|^SPEC_COPY_DELETED", "of=[0-9]{1,2}'\nstatus+='|^SPEC_COPY_DELETED", WR),
    M("wrapper-verdict-class-open", "wrapper", "(NOT_BUILT|MEDIA|PROVIDER|TRANSPORT|TIME|HOSTREF|BAR1_EXCEPTION|OPERATOR|USAGE)", "[A-Z_]{1,32}", WR),
    M("wrapper-private-log-readable", "wrapper", "umask 077\n", "", WR),
    M("wrapper-exit-code-lost", "wrapper", 'rc=$(cat "$rcfile" 2>/dev/null || echo 1)', "rc=0", WR),
    M("wrapper-licence-state-lines-dropped", "wrapper", "status+='|^SPEC_LICENCE_STATE at=", "status+='|^SPEC_LICENCE_STATE_X at=",
      NET + ".LicenceProbe.test_no_key_id_computer_or_account_leaves_the_readout", NET + ".LicenceProbe.test_the_wrapper_passes_every_state_line_the_stage_keeps"),
    M("wrapper-shutdown-line-dropped", "wrapper", "status+='|^SPEC_GUEST_SHUTDOWN_LICENCE viewset=", "status+='|^SPEC_GUEST_SHUTDOWN_LICENCE_X viewset=",
      NET + ".ShutdownClassification.test_the_classification_passes_the_wrapper_and_the_payload_pattern",
      NET + ".ShutdownClassification.test_the_wrapper_keeps_the_licence_row_and_puts_the_shutdown_line_before_it"),
    M("wrapper-reason-limited-to-16", "wrapper", "reason=([A-Z_]{1,24}|-) failed=", "reason=([A-Z_]{1,16}|-) failed=",
      NET + ".ShutdownClassification.test_the_classification_passes_the_wrapper_and_the_payload_pattern",
      NET + ".ShutdownClassification.test_the_wrapper_keeps_the_licence_row_and_puts_the_shutdown_line_before_it"),
    M("wrapper-private-dir-not-handed-down", "wrapper", "export KF_SPEC_PRIVATE=$private/spec_$tag", ":", WR, SC),
]

COLLECT = """import io, json, sys, unittest
names = sys.argv[1:]
suite = unittest.defaultTestLoader.loadTestsFromNames(names)
r = unittest.TextTestRunner(stream=io.StringIO()).run(suite)
print('KF_MUTATION_RESULT=' + json.dumps({'tests': r.testsRun, 'killers': [t.id() for t, _ in r.failures + r.errors]}))
"""


def tree(parent, name, overrides):
    root = Path(tempfile.mkdtemp(prefix=name + "-", dir=parent))
    for rel in COPY:
        (root / rel).parent.mkdir(parents=True, exist_ok=True)
        shutil.copyfile(ROOT / rel, root / rel)
    shutil.copytree(ROOT / "scripts/bench/specviewperf/fixtures", root / "scripts/bench/specviewperf/fixtures")
    for f in (ROOT / "traces/specviewperf_vast_20261105/spec").glob("*.txt"):
        (root / "traces/specviewperf_vast_20261105/spec").mkdir(parents=True, exist_ok=True)
        shutil.copyfile(f, root / "traces/specviewperf_vast_20261105/spec" / f.name)
    for key, text in overrides.items():
        (root / FILES[key]).write_text(text)
    return root


def run_tests(root, names):
    done = subprocess.run(
        [sys.executable, "-c", COLLECT, *names],
        cwd=root / "scripts/ci",
        capture_output=True,
        text=True,
        timeout=3000,
        env={"PATH": os.environ["PATH"], "HOME": os.environ.get("HOME", "/tmp")},
    )
    line = next((s for s in done.stdout.splitlines() if s.startswith("KF_MUTATION_RESULT=")), None)
    if line is None:
        raise SystemExit("harness failure: " + done.stderr[-600:] + done.stdout[-300:])
    return json.loads(line.split("=", 1)[1])


def one(parent, mutant):
    name, key, edits, tests = mutant
    text = SOURCES[key]
    for old, new in edits:
        if text.count(old) != 1:
            return {"mutant": name, "count": 0, "killers": [], "error": "anchor x%d: %r" % (text.count(old), old[:60])}
        text = text.replace(old, new)
    if text == SOURCES[key]:
        return {"mutant": name, "count": 0, "killers": [], "error": "no change"}
    root = tree(parent, "m", {key: text})
    result = run_tests(root, tests)
    shutil.rmtree(root, ignore_errors=True)
    killers = sorted(set(result["killers"]))
    print("%s: %d killers" % (name, len(killers)), file=sys.stderr, flush=True)
    return {"mutant": name, "count": len(killers), "killers": killers}


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--jobs", type=int, default=4)
    parser.add_argument("--no-baseline", action="store_true")
    parser.add_argument("only", nargs="?", default="")
    args = parser.parse_args()
    chosen = [m for m in MUTANTS if args.only in m[0]]
    with tempfile.TemporaryDirectory(prefix="kf-spec-stage-mutants-") as parent:
        if not args.no_baseline:
            baseline = run_tests(tree(parent, "base", {}), [LANE, STG, NET, CBT])
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
