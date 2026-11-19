#!/usr/bin/env python3
"""Mutation checks of the image B `gpu` step diagnostics (2026-11-17): every mutant needs two named killers (stdlib, GPU-free).

Same machinery as propimage_apps_mutants.py (a private copy of scripts/ci and the harness files, one exact-once text replacement, the named tests of
test_gpu_diag.py run against the copy). The unmutated copy must be green first.

    python3 scripts/ci/propimage_gpu_diag_mutants.py [--jobs N] [substring-of-mutant-name]
"""
import argparse
import concurrent.futures
import json
import sys
import tempfile
from pathlib import Path

HERE = Path(__file__).resolve().parent
sys.path.insert(0, str(HERE))
import propimage_apps_mutants as base  # noqa: E402

base.FILES.update(check="scripts/bench/windows/propimage_check.py", bstage="scripts/ci/propimage_apps_b_stage.py")
base.SOURCES.update({k: (base.ROOT / base.FILES[k]).read_text() for k in ("check", "bstage")})
base.COPY_DIRS.append("scripts/bench/windows")  # the stage imports the image B rows and their helpers
base.COPY_FILES[:] = [f for f in base.COPY_FILES if not f.startswith("scripts/bench/windows/")]
SUITE = "test_gpu_diag."
C, L, D, Q, W, S = (SUITE + n + "." for n in ("Classify", "Loop", "Detail", "QemuTail", "Wiring", "StageGpu"))
M = base.M

MUTANTS = [
    # (a) the case is named
    M("no-uuid-reported-as-wrong-value", "check", 'elif not out and rc == 0:\n        res["kind"] = "no_uuid"', 'elif not out and rc == 0:\n        res["kind"] = "wrong_value"',
      C + "test_each_case_has_its_own_name", D + "test_the_detail_names_the_case_the_counts_and_the_problem_code", L + "test_a_driver_that_never_answers_is_reported_absent_inside_a_bounded_budget"),
    M("error-text-reported-as-no-uuid", "check", 'else:\n        res["kind"] = "error_text"', 'else:\n        res["kind"] = "no_uuid"',
      C + "test_each_case_has_its_own_name", C + "test_a_failing_return_code_is_never_an_answer_even_with_the_right_line"),
    M("two-gpus-not-multi", "check", 'if len(gpu) > 1:\n        res["kind"] = "multi"', 'if len(gpu) > 5:\n        res["kind"] = "multi"',
      C + "test_each_case_has_its_own_name", L + "test_two_gpus_and_a_wrong_value_stop_at_once_and_are_named"),
    M("return-code-ignored", "check", 'elif len(gpu) == 1 and rc == 0 and not other:', 'elif len(gpu) == 1 and not other:',
      C + "test_a_failing_return_code_is_never_an_answer_even_with_the_right_line", L + "test_a_right_looking_line_with_a_failing_return_code_is_not_a_pass"),
    M("host-flag-lost", "check", 'res["observed_is_host"] = (gpu[0] in host_uuids) if host_uuids else None', 'res["observed_is_host"] = None',
      C + "test_wrong_value_says_whether_it_is_the_hosts", D + "test_the_detail_names_the_case_the_counts_and_the_problem_code"),
    M("timeout-reported-as-no-uuid", "check", 'return dict(rec, kind="timeout", devices=[])', 'return dict(rec, kind="no_uuid", devices=[])',
      L + "test_a_timeout_and_a_transport_error_are_their_own_cases_and_keep_polling", L + "test_a_guest_that_only_times_out_is_reported_as_a_timeout_not_as_absent"),
    M("problem-codes-dropped", "check", 'return [d["problem"] for d in devices if d["present"]]', "return []",
      L + "test_code_43_gets_one_disable_enable_at_half_the_budget_with_the_state_before_it", D + "test_the_detail_names_the_case_the_counts_and_the_problem_code"),
    M("events-not-collected-on-failure", "check", '    if not diag["ok"]:\n        diag["events"] = gpu_events(g)', '    if False:\n        diag["events"] = gpu_events(g)',
      L + "test_failure_collects_the_guest_events_and_no_uuid_survives", L + "test_two_gpus_and_a_wrong_value_stop_at_once_and_are_named", D + "test_the_evidence_files_hold_the_case_and_the_events_without_a_uuid"),
    M("uuid-not-redacted", "check", 'return BARE_UUID.sub("<uuid>", GPU_ID.sub("GPU-<redacted>", text))[:limit]', "return text[:limit]",
      L + "test_failure_collects_the_guest_events_and_no_uuid_survives", Q + "test_the_tail_is_bounded_and_redacted", D + "test_the_evidence_files_hold_the_case_and_the_events_without_a_uuid"),
    M("repair-failure-text-unredacted", "check", 'repair_text=redact(rep.stdout + rep.stderr, 300)', 'repair_text=rep.stdout + rep.stderr',
      L + "test_a_failed_repair_is_named_and_the_text_is_kept", D + "test_a_repair_failure_is_named_in_the_detail_and_the_diag_file_hides_the_value"),
    M("no-wait-for-settled", "check", "        if settled or clock() - t0 >= wait_s:\n            break", "        if True:\n            break",
      L + "test_waits_for_the_device_to_settle_before_the_restart", L + "test_gives_up_waiting_when_the_device_never_starts_and_says_so"),
    M("code-43-not-settled", "check", 'and shown[0]["problem"] in (0, 43)', 'and shown[0]["problem"] in (0,)',
      C + "test_code_43_counts_as_settled_because_a_new_die_sits_in_it_until_the_repair", L + "test_a_new_die_in_code_43_is_restarted_at_once_not_after_the_whole_wait"),
    M("install-state-counts-as-settled", "check", 'and shown[0]["problem"] in (0, 43)', 'and True',
      C + "test_settled_means_one_present_nvidia_service_device_out_of_the_install_states", L + "test_waits_for_the_device_to_settle_before_the_restart"),
    M("basic-display-counts-as-settled", "check", 'shown[0]["service"].lower() == "nvlddmkm" and', "True and",
      C + "test_settled_means_one_present_nvidia_service_device_out_of_the_install_states", L + "test_waits_for_the_device_to_settle_before_the_restart"),
    M("pre-restart-text-dropped", "check", 'for k in ("t", "dur_s", "smi_s", "kind", "devices", "text") if k in q', 'for k in ("t", "dur_s", "smi_s", "kind", "devices") if k in q',
      L + "test_a_new_die_in_code_43_is_restarted_at_once_not_after_the_whole_wait", L + "test_gives_up_waiting_when_the_device_never_starts_and_says_so"),
    M("query-timeout-too-short", "check", "GPU_QUERY_TIMEOUT_S = 150", "GPU_QUERY_TIMEOUT_S = 90",
      L + "test_the_query_call_limit_covers_the_measured_first_call_after_a_restart", L + "test_a_query_may_block_as_long_as_the_first_nvidia_smi_after_a_restart_does"),
    M("query-duration-not-recorded", "check", '    rec["dur_s"] = int(clock() - t)\n', "",
      L + "test_every_query_records_how_long_it_took_and_how_long_nvidia_smi_blocked", L + "test_a_driver_that_never_answers_is_reported_absent_inside_a_bounded_budget"),
    M("wait-unbounded-by-the-budget", "check", "        if settled or clock() - t0 >= wait_s:", "        if settled or clock() - t0 >= wait_s * 1000:",
      L + "test_gives_up_waiting_when_the_device_never_starts_and_says_so", L + "test_the_whole_step_is_bounded_when_nothing_ever_works"),
    M("poll-once-after-the-restart", "check", "        if q[\"kind\"] in GPU_TERMINAL or elapsed >= poll_s:\n            break", "        if True:\n            break",
      L + "test_repolls_after_the_restart_until_the_guest_answers", L + "test_a_device_that_comes_back_after_the_disable_enable_passes", L + "test_code_43_gets_one_disable_enable_at_half_the_budget_with_the_state_before_it"),
    M("poll-budget-not-enforced", "check", "        if q[\"kind\"] in GPU_TERMINAL or elapsed >= poll_s:", "        if q[\"kind\"] in GPU_TERMINAL or elapsed >= poll_s * 1000:",
      L + "test_a_driver_that_never_answers_is_reported_absent_inside_a_bounded_budget", L + "test_the_whole_step_is_bounded_when_nothing_ever_works"),
    M("no-disable-enable-retry", "check", "if not reenabled and elapsed >= poll_s / 2:", "if False:",
      L + "test_code_43_gets_one_disable_enable_at_half_the_budget_with_the_state_before_it", L + "test_a_device_that_comes_back_after_the_disable_enable_passes"),
    M("wrong-value-keeps-polling", "check", 'GPU_TERMINAL = ("expected", "wrong_value", "multi")', 'GPU_TERMINAL = ("expected",)',
      L + "test_two_gpus_and_a_wrong_value_stop_at_once_and_are_named", L + "test_a_wrong_value_is_final_and_does_not_burn_the_poll_budget"),
    M("restart-failure-ignored", "check", "    if rep.returncode:\n        diag.update(kind=\"repair_failed\"", "    if False:\n        diag.update(kind=\"repair_failed\"",
      L + "test_a_failed_repair_is_named_and_the_text_is_kept", L + "test_a_failed_repair_is_not_followed_by_polling"),
    M("log-tail-not-saved-on-failure", "check", '    (Path(out) / "qemu_kf3_tail.txt").write_text(tail)\n', "",
      Q + "test_save_keeps_the_tail_in_out_and_prints_the_last_lines", W + "test_a_failed_gpu_step_names_the_case_and_keeps_the_diagnostics_and_the_log_tail"),
    M("log-tail-not-saved-by-main", "check", '                    save_qemu_tail(out, r / "qemu.log")\n', "                    pass\n",
      W + "test_a_failed_gpu_step_names_the_case_and_keeps_the_diagnostics_and_the_log_tail", W + "test_a_failed_gpu_step_prints_the_last_lines_of_the_kf3_log"),
    M("log-tail-not-printed", "check", 'print("".join(tail.splitlines(True)[-30:]), end="", flush=True)', "pass",
      Q + "test_save_keeps_the_tail_in_out_and_prints_the_last_lines", W + "test_a_failed_gpu_step_prints_the_last_lines_of_the_kf3_log"),
    M("stage-deletes-the-kf3-log", "bstage", "        Path(dst).write_text(pi_check.qemu_log_tail(src, limit=400))", "        pass",
      S + "test_the_kf3_log_is_kept_after_the_cleanup_and_redacted", S + "test_a_failed_gpu_step_prints_the_case_and_the_tail"),
    M("stage-keeps-the-log-unredacted", "bstage", "Path(dst).write_text(pi_check.qemu_log_tail(src, limit=400))", "Path(dst).write_text(Path(src).read_text())",
      S + "test_the_kf3_log_is_kept_after_the_cleanup_and_redacted", S + "test_a_failed_gpu_step_prints_the_case_and_the_tail"),
    M("stage-does-not-report-the-failure", "bstage", "            report_gpu_step(rows_json, label, out, logs / (label + \"_qemu_kf3.txt\"), evidence_dir)\n", "",
      S + "test_a_failed_gpu_step_prints_the_case_and_the_tail", S + "test_a_failed_gpu_step_copies_the_diagnostic_files_and_the_kept_log"),
    M("stage-reports-a-passing-gpu-step", "bstage", 'if step is None or step.get("verdict") == "PASS":\n        return', "if step is None:\n        return",
      S + "test_a_passing_gpu_step_prints_nothing", S + "test_a_passing_gpu_step_copies_no_failure_files"),
    M("stub-reported-as-exact-match", "check", "        return redact(detail, 300).strip() or (\"PASS\" if ok else \"FAIL\")  # a stub or the VFIO control says what it is",
      "        return \"post-fix seed UUID exact match\" if ok else \"repair or UUID check failed\"",
      D + "test_a_stub_or_the_vfio_control_reports_its_own_text_never_exact_match", W + "test_a_stubbed_repair_is_recorded_as_what_it_is"),
    M("main-records-a-fixed-string", "check", 'record("gpu", "PASS" if fixed else "FAIL", gpu_step_detail(fixed, detail))',
      'record("gpu", "PASS" if fixed else "FAIL", "post-fix seed UUID exact match" if fixed else "repair or UUID check failed")',
      W + "test_a_failed_gpu_step_names_the_case_and_keeps_the_diagnostics_and_the_log_tail", W + "test_a_passing_gpu_step_records_its_own_result_and_leaves_no_failure_files"),
    M("main-writes-no-diagnostic-files", "check", "                write_gpu_evidence(out, detail)\n", "                (out / \"gpu_fix.txt\").write_text(detail)\n",
      W + "test_a_failed_gpu_step_names_the_case_and_keeps_the_diagnostics_and_the_log_tail", W + "test_a_passing_gpu_step_records_its_own_result_and_leaves_no_failure_files"),
]


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--jobs", type=int, default=2)
    parser.add_argument("only", nargs="?", default="")
    args = parser.parse_args()
    chosen = [m for m in MUTANTS if args.only in m[0]]
    with tempfile.TemporaryDirectory(prefix="kf-gpu-diag-mutants-") as parent:
        baseline = base.run_tests(base.tree(parent, "base", {}), [SUITE.rstrip(".")])
        if baseline["killers"]:
            print("BASELINE RED: %s" % baseline["killers"], file=sys.stderr)
            return 1
        print("baseline green: %d tests" % baseline["tests"], file=sys.stderr, flush=True)
        with concurrent.futures.ThreadPoolExecutor(args.jobs) as pool:
            rows = list(pool.map(lambda m: base.one(parent, m), chosen))
    bad = [r for r in rows if r["count"] < 2]
    print(json.dumps({"cases": rows}, indent=2))
    for row in bad:
        print("TOO FEW KILLERS: %(mutant)s (%(count)d) %(error)s" % dict(row, error=row.get("error", "")), file=sys.stderr)
    return 1 if bad else 0


if __name__ == "__main__":
    sys.exit(main())
