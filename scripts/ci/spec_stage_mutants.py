#!/usr/bin/env python3
"""Mutation checks of the SPECviewperf gate stage: every mutant needs two killers (stdlib only, GPU-free).

Each mutant is one exact-once text replacement (a few use two) in a private copy of lane.py, tarfilter.py,
specviewperf_stage.sh or vast_stage_specviewperf.sh; the tests of test_vast_spec_lane.py and test_vast_spec_stage.py
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
}
LANE = "test_vast_spec_lane"
STG = "test_vast_spec_stage"
COPY = [
    "scripts/ci/test_vast_spec_lane.py",
    "scripts/ci/test_vast_spec_stage.py",
    "scripts/ci/spec_stage_fakes.py",
    "scripts/ci/vast_payload.py",
    "scripts/ci/vast_report.py",
    "scripts/bench/native_xid_guard.py",
    "scripts/bench/specviewperf/lanes.json",
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
    M("grade-workload-status-ignored", "lane", 'if cli["workload_status"] == "Completed" and total >= 1',
      "if total >= 1", G),
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
      "stats[dirname] = (f + 1, b)", TF, t("Copy", "both_lanes_stream_the_whole_tree")),
    # --- specviewperf_stage.sh -------------------------------------------------------------------------------------------
    M("stage-no-pin-check", "stage", '[ -n "$PIN" ] || abort MEDIA', "true || abort MEDIA", IN),
    M("stage-sha-mismatch-is-provider", "stage", "    3 | 4) abort MEDIA", "    3 | 4) abort PROVIDER", IN),
    M("stage-fetch-failure-is-operator", "stage", '    *) abort PROVIDER "the download failed', '    *) abort OPERATOR "the download failed', IN),
    M("stage-fetch-timeout-is-provider", "stage", "    124) abort TIME", "    124) abort PROVIDER", IN),
    M("stage-no-settle", "stage", "    settle\n    return 0", "    return 0", SE,
      t("Verdicts", "a_fresh_boot_waits_for_the_settle_time_again")),
    M("stage-no-time-guard", "stage", '        if [ "$mode" = skip ]; then', "        if false; then", TG),
    M("stage-archive-kept-after-the-copy", "stage", 'rm -f -- "$ZST"\nsay "SPEC_COPY_DELETED removed=archive"\n', "", TD),
    M("stage-overlay-kept", "stage",
      '    if [ -e "$VM/disk/$OVERLAY.qcow2" ]; then rm -f -- "$VM/disk/$OVERLAY.qcow2"; gone+="${gone:+,}overlay"; fi\n', "", TD),
    M("stage-no-final-delete", "stage", "    delete_copy\n    for n in", "    for n in", TD),
    M("stage-guest-left-running", "stage", '    teardown_guest\n    if [ "$GUEST_STARTED" = 1 ] && [ "$HOSTREF_BEFORE" = green ]',
      '    if [ "$GUEST_STARTED" = 1 ] && [ "$HOSTREF_BEFORE" = green ]', TD),
    M("stage-copy-unverified", "stage",
      '    grep -qxF "$l" "$WORK/guest_view.txt" || abort TRANSPORT "the guest holds a different tree than the one streamed ($l)"',
      "    true", CP),
    M("stage-host-reference-before-ignored", "stage", '[ "$HOSTREF_BEFORE" = green ] || abort HOSTREF', "true || abort HOSTREF", PF),
    M("stage-rows-not-streamed", "stage", '        emit_row "$GRADE_LINE"\n        break', '        echo "$GRADE_LINE" >> "$ROWS"\n        break',
      t("HappyPath", "lane_a_passes_in_order_with_one_row_per_viewset"), t("Wrapper", "the_streamed_log_holds_only_status_lines_and_the_verdict")),
    M("stage-no-guest-ram", "stage", "    if ! WINVM_RAM_MB=$GUEST_RAM_MB bash", "    if ! bash", CP),
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
    M("stage-retry-prints-pass", "stage", '        *) VERDICT=RETRY; rc_final=3; say "SPECVIEWPERF_STAGE_RETRY class=${cls:-OPERATOR}" ;;',
      '        *) VERDICT=RETRY; rc_final=3; say "SPECVIEWPERF_STAGE_PASS" ;;',
      t("Inputs", "a_sha256_mismatch_is_retry_media_and_no_viewset_runs"), t("Inputs", "an_object_the_worker_does_not_know_is_retry_media")),
    M("stage-fail-exits-zero", "stage", "        FAIL) VERDICT=FAIL; rc_final=1; say", "        FAIL) VERDICT=FAIL; rc_final=0; say",
      t("Verdicts", "the_exit_code_of_a_fail_is_1_and_of_a_retry_is_3"), t("Wrapper", "the_wrapper_exit_code_is_the_stages")),
    M("stage-no-gate-pass-check", "stage",
      "grep -q '^WINDOWS_GATE_PASS ' \"$PROV/${T}_windows.log\" 2> /dev/null || abort OPERATOR", "true || abort OPERATOR", PF),
    M("stage-no-disk-check", "stage", '[ "${avail_gb:-0}" -ge "$need_gb" ] || abort PROVIDER', "true || abort PROVIDER", PF),
    M("stage-no-ram-check", "stage", '[ "${mem_gb:-0}" -ge 24 ] || abort PROVIDER', "true || abort PROVIDER", PF),
    M("stage-run-boot-without-the-gate-flags", "stage", ' --env-file "$FLAGS")', ")", CP),
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
      t("Verdicts", "a_launch_the_guest_script_refuses_is_a_retry_operator_never_a_guest_failure")),
    M("stage-boot-failure-is-not-a-guest-failure", "stage",
      '        if [ "$GUEST_DEAD_ROW" = 0 ]; then synthetic_row "$w" FAIL GUEST; GUEST_DEAD_ROW=1; else synthetic_row "$w" NOT_RUN BOOT; fi\n        continue',
      '        synthetic_row "$w" NOT_RUN BOOT\n        continue',
      t("Verdicts", "a_kf3_boot_that_fails_after_the_gate_booted_the_same_image_is_a_guest_failure"),
      t("Verdicts", "the_vast_bar1_refusal_is_the_documented_exception_not_a_fail")),
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
            baseline = run_tests(tree(parent, "base", {}), [LANE, STG])
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
