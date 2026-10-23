#!/usr/bin/env python3
"""Behavioural mutations for the repo-side Vast box scripts (stdlib only).

Run alone, never beside other test runs:  python3 scripts/ci/vast_box_mutants.py [--jobs N]

Each case applies one textual mutation (exactly one occurrence of `old`) to a private copy of the
scripts and runs only its named killer tests against that copy (KF_CI_SCRIPTS_DIR). A case is
killed when EVERY named killer fails. The baseline (whole suite on the unmutated copy) must be
green first. Exit status is 1 on a red baseline, a survivor, a named killer that did not fail,
or a behaviour with fewer than two mutants.
"""

import argparse
import concurrent.futures
import os
import re
import shutil
import subprocess
import sys
import tempfile
from pathlib import Path

HERE = Path(__file__).resolve().parent
FILES = [
    "vast_onstart.sh",
    "vast_report.py",
    "kf-box-kill",
    "kf-box-hold",
    "kf-box-blob-url",
    "kf-ci-hold.sh",
]
R, E, K, D = "vast_report.py", "EnvValidation.", "KillAndHold.", "Daemon."
RP, FR, ON = "Reporter.", "FinalResult.", "Onstart."
V = "vast_onstart.sh"

CASES = [
    # (behaviour, name, file, old, new, [killer tests "Class.test"])
    (
        "env-contract",
        "accept-old-protocol",
        R,
        '{"4", "5"}',
        '{"3", "4", "5"}',
        [E + "test_malformed_values_are_rejected"],
    ),
    (
        "env-contract",
        "optional-runner-nonce",
        R,
        '("KF_RUNNER_NONCE", True,',
        '("KF_RUNNER_NONCE", False,',
        [E + "test_each_required_variable_missing_is_named"],
    ),
    (
        "env-contract",
        "allow-http",
        R,
        'r"https://[A-Za-z0-9.-]',
        'r"https?://[A-Za-z0-9.-]',
        [
            E + "test_malformed_values_are_rejected",
            E + "test_cli_never_sends_the_token_over_plain_http",
        ],
    ),
    (
        "env-contract",
        "values-in-errors",
        R,
        'problems.append("%s: malformed" % name)',
        'problems.append("%s: malformed (%s)" % (name, value))',
        [E + "test_problems_never_contain_values"],
    ),
    (
        "env-contract",
        "no-token-run-match",
        R,
        '.replace(".", "-") != run:',
        '.replace(".", "-") != run and False:',
        [E + "test_token_must_match_run_id"],
    ),
    (
        "env-file",
        "file-world-readable",
        R,
        "os.O_TRUNC, 0o600)",
        "os.O_TRUNC, 0o644)",
        [E + "test_cli_write_makes_a_private_env_file_of_contract_names_only"],
    ),
    (
        "env-file",
        "dir-world-readable",
        R,
        "os.chmod(directory, 0o700)",
        "os.chmod(directory, 0o755)",
        [E + "test_cli_write_makes_a_private_env_file_of_contract_names_only"],
    ),
    (
        "env-file",
        "onstart-helper-mode",
        V,
        'install -m 700 "$here/kf-box-kill"',
        'install -m 755 "$here/kf-box-kill"',
        [ON + "test_env_file_is_root_only_and_helpers_are_installed"],
    ),
    (
        "env-file",
        "hold-hook-not-installed",
        V,
        'install -m 644 "$here/kf-ci-hold.sh" "$profile_dir/kf-ci-hold.sh"',
        "true",
        [ON + "test_env_file_is_root_only_and_helpers_are_installed"],
    ),
    (
        "sequence-idempotency",
        "retry-renumbers",
        R,
        "        self.next_try = now + delay\n",
        "        self.next_try = now + delay\n        b = json.loads(message['body'])\n"
        "        b = json.loads(message['body'])\n        self.pending = None\n"
        "        self._message(b['kind'], b.get('text'), b.get('result'))\n",
        [
            RP + "test_lost_response_retry_reuses_payload_and_sequence",
            RP + "test_failed_request_is_retried_with_same_sequence",
        ],
    ),
    (
        "sequence-idempotency",
        "sequence-gap",
        R,
        "        self.seq += 1\n",
        "        self.seq += 2\n",
        [
            RP + "test_progress_logs_result_are_numbered_and_authorised",
            RP + "test_lost_response_retry_reuses_payload_and_sequence",
        ],
    ),
    (
        "sequence-idempotency",
        "not-ready-dropped",
        R,
        "if status in (400, 413):",
        "if status in (400, 409, 413):",
        [RP + "test_not_ready_is_retried_not_dropped"],
    ),
    (
        "sequence-idempotency",
        "unauthorised-retried",
        R,
        "if status in (401, 403, 404):",
        "if status in (403, 404):",
        [RP + "test_unauthorised_stops_sending_but_never_hangs"],
    ),
    (
        "token-scope",
        "follow-redirects",
        R,
        "            _NoRedirect(),\n",
        "",
        [RP + "test_redirects_are_never_followed_and_token_does_not_travel"],
    ),
    (
        "token-scope",
        "kill-follows-redirects",
        "kf-box-kill",
        "--max-time 20 --max-redirs 0",
        "--location --max-time 20 --max-redirs 5",
        [
            K + "test_kill_does_not_follow_redirects_and_fails",
            K + "test_kill_keeps_the_token_off_the_command_line",
        ],
    ),
    (
        "token-scope",
        "hold-follows-redirects",
        "kf-box-hold",
        "--max-time 20 --max-redirs 0",
        "--location --max-time 20 --max-redirs 5",
        [K + "test_hold_does_not_follow_redirects"],
    ),
    (
        "token-scope",
        "kill-token-on-argv",
        "kf-box-kill",
        "curl --config - --silent",
        'curl --header "Authorization: Bearer $KF_RUN_TOKEN" --silent',
        [K + "test_kill_keeps_the_token_off_the_command_line"],
    ),
    (
        "kill-command",
        "kill-allows-http",
        "kf-box-kill",
        " --proto '=https'",
        "",
        [K + "test_kill_refuses_plain_http", K + "test_kill_keeps_the_token_off_the_command_line"],
    ),
    (
        "kill-command",
        "kill-any-status-ok",
        "kf-box-kill",
        '[ "$code" != 200 ]',
        '[ "$code" = 0 ]',
        [
            K + "test_kill_does_not_follow_redirects_and_fails",
            K + "test_kill_failure_status_is_an_error",
        ],
    ),
    (
        "kill-command",
        "kill-wrong-path",
        "kf-box-kill",
        "/kill/${KF_RUN_ID",
        "/stop/${KF_RUN_ID",
        [K + "test_kill_request_shape"],
    ),
    (
        "kill-command",
        "kill-reason-unsanitised",
        "kf-box-kill",
        " | tr -cd 'A-Za-z0-9 ._:,-'",
        "",
        [K + "test_kill_default_reason_and_bounded_sanitised_reason"],
    ),
    (
        "kill-command",
        "kill-reason-unbounded",
        "kf-box-kill",
        "'%.200s'",
        "'%.2000s'",
        [K + "test_kill_default_reason_and_bounded_sanitised_reason"],
    ),
    (
        "kill-command",
        "hold-wrong-method-body",
        "kf-box-hold",
        "--data-binary ''",
        "--data-binary 'x'",
        [K + "test_hold_request_shape"],
    ),
    (
        "heartbeat",
        "heartbeat-disabled",
        R,
        "self.pending is None and self.clock() - self.last_ok >= self.heartbeat",
        "False",
        [RP + "test_heartbeat_is_sent_with_zero_new_lines"],
    ),
    (
        "heartbeat",
        "heartbeat-ignores-interval",
        R,
        "self.clock() - self.last_ok >= self.heartbeat",
        "self.clock() - self.last_ok >= 0",
        [
            RP + "test_heartbeat_waits_for_the_interval",
            RP + "test_heartbeat_is_sent_with_zero_new_lines",
        ],
    ),
    (
        "caps",
        "post-cap-off",
        R,
        "self.posts >= limits.max_posts",
        "False",
        [RP + "test_post_count_cap_omits_excess_and_still_sends_result"],
    ),
    (
        "caps",
        "byte-cap-off",
        R,
        "or self.bytes + body_size > limits.max_bytes",
        "or False",
        [RP + "test_byte_cap_bounds_the_total_log_body_bytes"],
    ),
    (
        "caps",
        "line-cap-off",
        R,
        "if len(piece) <= room:",
        "if True:",
        [RP + "test_long_lines_are_capped_and_the_rest_survives"],
    ),
    (
        "caps",
        "chunk-unbounded",
        R,
        "self.chunk = 8192",
        "self.chunk = 60000",
        [RP + "test_chunks_are_bounded"],
    ),
    (
        "caps",
        "omitted-not-reported",
        R,
        'text += " Sender omitted %d raw bytes beyond its bounds." % self.omitted',
        "pass",
        [
            RP + "test_post_count_cap_omits_excess_and_still_sends_result",
            RP + "test_byte_cap_bounds_the_total_log_body_bytes",
        ],
    ),
    (
        "result-schema",
        "exit-code-ignored",
        R,
        'elif rc != 0 and verdict == "PASS":',
        "elif False:",
        [
            FR + "test_exit_code_demotes_pass",
            ON + "test_failing_payload_exit_code_demotes_and_is_propagated",
        ],
    ),
    (
        "result-schema",
        "skips-allowed",
        R,
        'r["fail"] == 0 and r["skip"] == 0 and',
        'r["fail"] == 0 and',
        [FR + "test_skips_or_failures_or_zero_pass_demote_pass"],
    ),
    (
        "result-schema",
        "old-gpu-allowed",
        R,
        'hardware["compute_cap"] >= 750',
        "True",
        [FR + "test_unknown_or_old_hardware_demotes_pass"],
    ),
    (
        "result-schema",
        "payload-binding-trusted",
        R,
        "if any(key in doc and doc[key] != value for key, value in bound.items()):",
        "if False:",
        [
            RP + "test_payload_cannot_choose_the_binding_fields",
            FR + "test_bad_result_files_fail_closed",
        ],
    ),
    (
        "result-schema",
        "duplicate-keys-allowed",
        R,
        'raise ValueError("duplicate field")',
        "pass",
        [FR + "test_bad_result_files_fail_closed"],
    ),
    (
        "result-schema",
        "fail-without-failing-row",
        R,
        'if verdict != "PASS" and not any(row["fail"] > 0 for row in rows):',
        "if False:",
        [
            FR + "test_payload_fail_without_failing_row_gets_one",
            FR + "test_unavailable_is_preserved_with_a_failing_row",
        ],
    ),
    (
        "result-schema",
        "unknown-hardware-invented",
        R,
        'hardware = dict(doc.get("hardware") or UNKNOWN_HW)',
        'hardware = dict(doc.get("hardware") or {"gpu_name": "NVIDIA GeForce RTX 3060", '
        '"compute_cap": 860, "driver_version": "580.159.04"})',
        [FR + "test_missing_hardware_reports_unknown_not_invented"],
    ),
    (
        "timeout",
        "timeout-not-forced-fail",
        R,
        'if timed_out:\n        verdict = "FAIL"',
        "if timed_out:\n        verdict = verdict",
        [FR + "test_timeout_cannot_be_hidden_by_a_payload_pass"],
    ),
    (
        "timeout",
        "timeout-row-missing",
        R,
        '_ensure_fail(rows, "timeout")',
        "pass",
        [
            FR + "test_timeout_forces_fail_and_keeps_the_finished_suites",
            ON + "test_soft_deadline_kills_the_payload_and_reports_the_finished_prefix",
        ],
    ),
    (
        "timeout",
        "deadline-not-enforced",
        V,
        '"${remaining}s"',
        '"$((remaining + 9999))s"',
        [
            ON + "test_soft_deadline_kills_the_payload_and_reports_the_finished_prefix",
            ON + "test_soft_deadline_keeps_partial_suites_in_the_result",
        ],
    ),
    (
        "timeout",
        "timeout-never-detected",
        V,
        '[ "$(date +%s)" -ge $((KF_SOFT_DEADLINE - 2)) ]',
        "false",
        [
            ON + "test_soft_deadline_kills_the_payload_and_reports_the_finished_prefix",
            ON + "test_soft_deadline_keeps_partial_suites_in_the_result",
        ],
    ),
    (
        "timeout",
        "expired-deadline-still-runs",
        V,
        'elif [ "$remaining" -le 0 ]; then',
        "elif false; then",
        [ON + "test_soft_deadline_already_passed_does_not_run_the_payload"],
    ),
    (
        "payload-absent",
        "reason-dropped",
        V,
        "reason=payload_missing",
        "reason=",
        [ON + "test_payload_absent_is_a_structured_fail"],
    ),
    (
        "payload-absent",
        "absent-check-removed",
        V,
        'if [ ! -f "$repo/$payload" ]; then',
        "if false; then",
        [ON + "test_payload_absent_is_a_structured_fail"],
    ),
    (
        "payload-absent",
        "reasons-ignored",
        R,
        'REASONS = {"payload_missing"}',
        "REASONS = set()",
        [
            FR + "test_payload_missing_reason_overrides_any_file",
            RP + "test_payload_missing_reason_is_a_structured_fail",
        ],
    ),
    (
        "payload-env",
        "token-leaks",
        V,
        " -u KF_RUN_TOKEN",
        "",
        [ON + "test_payload_never_sees_the_run_token"],
    ),
    (
        "payload-env",
        "endpoint-leaks",
        V,
        " -u KF_BROKER_URL",
        "",
        [ON + "test_payload_never_sees_the_broker_endpoint_or_run_id"],
    ),
    (
        "payload-env",
        "run-id-leaks",
        V,
        " -u KF_RUN_ID",
        "",
        [ON + "test_payload_never_sees_the_broker_endpoint_or_run_id"],
    ),
    (
        "payload-env",
        "inputs-lost",
        V,
        '"KF_SOFT_DEADLINE_MS=$((KF_SOFT_DEADLINE * 1000))"',
        "KF_UNUSED=1",
        [ON + "test_payload_still_gets_its_own_inputs"],
    ),
    (
        "tool-install",
        "always-install",
        V,
        'if [ -n "$(missing_tools)" ]; then\n    export',
        "if true; then\n    export",
        [ON + "test_missing_tools_are_installed_and_present_tools_are_not"],
    ),
    (
        "tool-install",
        "never-install",
        V,
        "apt-get update -qq && apt-get install -y -qq curl python3 git ca-certificates",
        "true",
        [ON + "test_missing_tools_are_installed_and_present_tools_are_not"],
    ),
    (
        "tool-install",
        "install-failure-ignored",
        V,
        "exit 2\nfi\n\ninstall -d",
        "true\nfi\n\ninstall -d",
        [ON + "test_tool_install_failure_is_a_clear_error"],
    ),
    (
        "contract-failure",
        "payload-runs-despite-bad-env",
        V,
        'report check-env --write "$run_dir/env" || exit 2',
        'report check-env --write "$run_dir/env" || true',
        [
            ON + "test_bad_contract_cannot_run_with_an_existing_run_directory",
            ON + "test_bad_contract_cannot_reuse_a_stale_valid_environment",
        ],
    ),
    (
        "contract-failure",
        "failure-not-posted",
        R,
        '        reporter.finish(result, "Environment contract violated.")\n',
        "        pass\n",
        [
            E + "test_cli_failure_posts_structured_fail_and_hides_token",
            ON + "test_malformed_variable_posts_a_fail_result_and_skips_the_payload",
        ],
    ),
    (
        "contract-failure",
        "stale-result-trusted",
        V,
        'rm -f "$result" ',
        "true ",
        [ON + "test_stale_result_file_is_not_trusted"],
    ),
    (
        "hold-hook",
        "no-once-guard",
        "kf-ci-hold.sh",
        '[ -z "${KF_CI_HOLD_DONE:-}" ]',
        "true",
        [K + "test_login_hook_calls_hold_once_per_login"],
    ),
    (
        "hold-hook",
        "any-login",
        "kf-ci-hold.sh",
        '[ -n "${SSH_CONNECTION:-}" ] && ',
        "",
        [K + "test_login_hook_ignores_non_ssh_and_non_interactive_shells"],
    ),
    (
        "hold-hook",
        "non-interactive",
        "kf-ci-hold.sh",
        "*i*)\n",
        "*)\n",
        [K + "test_login_hook_ignores_non_ssh_and_non_interactive_shells"],
    ),
    (
        "hold-hook",
        "blocking-call",
        "kf-ci-hold.sh",
        "2>&1 &) \\\n",
        "2>&1) \\\n",
        [K + "test_login_hook_never_blocks_on_a_slow_broker"],
    ),
    (
        "hold-hook",
        "failure-aborts-login",
        "kf-ci-hold.sh",
        "2>&1 &) \\\n                >/dev/null 2>&1 || true",
        "2>&1) \\\n                >/dev/null 2>&1 || exit 1",
        [
            K + "test_login_hook_tolerates_every_failure",
            K + "test_login_hook_never_blocks_on_a_slow_broker",
        ],
    ),
]


def killers_of(case):
    return case[5]


def failed_names(text):
    """Python 3.10 uses Class; newer unittest also includes Class.method."""
    pattern = (
        r"^(?:FAIL|ERROR): (?P<method>\w+) \(test_vast_box\.(?P<cls>\w+)"
        r"(?:\.(?P<inner>\w+))?\)"
        r"(?: (?:\([^\r\n]*\)|\[[^\r\n]*\]))*$"
    )
    return {
        match["cls"] + "." + match["method"]
        for match in re.finditer(pattern, text, re.M)
        if match["inner"] is None or match["inner"] == match["method"]
    }


def run_tests(directory, tests, timeout=600):
    env = dict(os.environ, KF_CI_SCRIPTS_DIR=str(directory))
    names = ["test_vast_box." + t for t in tests] or ["test_vast_box"]
    proc = subprocess.run(
        [sys.executable, "-m", "unittest", "-v", *names],
        cwd=HERE,
        env=env,
        capture_output=True,
        text=True,
        timeout=timeout,
    )
    text = proc.stdout + proc.stderr
    return proc.returncode, failed_names(text), text


def run_case(case, root):
    behaviour, name, filename, old, new, killers = case
    work = Path(tempfile.mkdtemp(prefix="mut-", dir=root))
    for f in FILES:
        shutil.copy(HERE / f, work / f)
    path = work / filename
    source = path.read_text()
    if source.count(old) != 1:
        return case, "BAD-ANCHOR (%d occurrences)" % source.count(old)
    path.write_text(source.replace(old, new))
    code, failed, text = run_tests(work, killers)
    if code == 0:
        return case, "SURVIVED"
    missing = [k for k in killers if k not in failed]
    if missing:
        return case, "KILLER-DID-NOT-FAIL " + ", ".join(missing)
    return case, "killed"


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--jobs", type=int, default=1)
    parser.add_argument("--only", help="substring of a case name")
    args = parser.parse_args()
    root = Path(tempfile.mkdtemp(prefix="kf-mutants-"))
    try:
        base = root / "base"
        base.mkdir()
        for f in FILES:
            shutil.copy(HERE / f, base / f)
        code, failed, text = run_tests(base, [])
        if code != 0:
            print("BASELINE RED: %s" % sorted(failed))
            print(text[-3000:])
            return 1
        print("baseline green")
        cases = [c for c in CASES if not args.only or args.only in c[1]]
        with concurrent.futures.ThreadPoolExecutor(args.jobs) as pool:
            results = list(pool.map(lambda c: run_case(c, root), cases))
    finally:
        shutil.rmtree(root, ignore_errors=True)
    bad, table = 0, {}
    for case, verdict in results:
        print("%-22s %-30s %s" % (case[0], case[1], verdict))
        table.setdefault(case[0], []).append((case, verdict))
        bad += verdict != "killed"
    for behaviour, rows in table.items():
        killers = {k for case, _ in rows for k in case[5]}
        if len(rows) < 2 and not args.only:
            print("behaviour %s has fewer than two mutants" % behaviour)
            bad += 1
        print("%s: %d mutants, %d distinct killers" % (behaviour, len(rows), len(killers)))
    print("mutants: %d, killed: %d, problems: %d" % (len(results), len(results) - bad, bad))
    return 1 if bad else 0


if __name__ == "__main__":
    sys.exit(main())
