#!/usr/bin/env python3
"""Behavioural mutations for the simple Vast runner (stdlib only).

Run alone:  python3 scripts/ci/vast_simple_mutants.py

Each case applies one textual mutation (exactly one occurrence of `old`) to a private copy of
scripts/ci/vast_simple and runs its named killer tests against that copy (KF_VAST_SIMPLE_DIR).
A case is killed when EVERY named killer fails. The baseline (the whole suite on the unmutated
copy) must be green first. Exit status is 1 on a red baseline, a survivor, a killer that did
not fail, a mutant with fewer than two killers, or a behaviour with fewer than two mutants.
"""

import os
import shutil
import subprocess
import sys
import tempfile
from pathlib import Path

HERE = Path(__file__).resolve().parent
SRC = HERE / "vast_simple"
T, V, R, O, A = "Teardown.", "Verdicts.", "Reaper.", "OfferRules.", "HappyPath."
B, S, X = "BootFailures.", "Secrets.", "SshArgv."

CASES = [
    # (behaviour, name, file, old, new, [killer tests "Class.test"])
    (
        "label-prefix-safety",
        "destroy-without-prefix-check",
        "vast_api.py",
        "not have.startswith(PREFIX) or ",
        "",
        [
            T + "test_api_destroy_refuses_other_labels",
            T + "test_api_destroy_refuses_foreign_even_when_the_caller_names_it",
        ],
    ),
    (
        "label-prefix-safety",
        "destroy-without-label-match",
        "vast_api.py",
        " or label not in (None, have)",
        "",
        [
            T + "test_api_destroy_refuses_a_different_simple_label",
            T + "test_swapped_to_another_runs_label_is_a_leak_and_not_deleted",
        ],
    ),
    (
        "label-prefix-safety",
        "reaper-without-prefix-filter",
        "reaper.py",
        "if not isinstance(label, str) or not label.startswith(PREFIX):",
        "if not isinstance(label, str):",
        [R + "test_deletes_only_old_simple_instances", R + "test_foreign_only_listing_exits_zero"],
    ),
    (
        "verified-absence",
        "verification-skipped",
        "run.py",
        "gone = not self.api.list_instances(label) and (",
        "gone = True or (",
        [
            T + "test_instance_that_never_disappears_is_a_leak_exit_three",
            T + "test_absence_is_verified_through_instances_v1",
        ],
    ),
    (
        "verified-absence",
        "leak-ignored",
        "run.py",
        "leaked = not self.purge(label, iid) or leaked",
        "leaked = leaked",
        [
            T + "test_instance_that_never_disappears_is_a_leak_exit_three",
            T + "test_listing_error_during_teardown_is_a_leak",
        ],
    ),
    (
        "boot-retry",
        "single-boot-attempt",
        "run.py",
        "MAX_BOOTS = 2",
        "MAX_BOOTS = 1",
        [
            B + "test_boot_never_answers_destroys_and_retries_the_next_machine",
            B + "test_provider_stuck_loading_is_a_boot_failure",
        ],
    ),
    (
        "boot-retry",
        "no-destroy-before-retry",
        "run.py",
        "return None, self.purge(label, iid)",
        "return None, True",
        [
            B + "test_boot_never_answers_destroys_and_retries_the_next_machine",
            B + "test_stuck_instance_is_destroyed_before_the_retry_is_rented",
        ],
    ),
    (
        "verdict-grading",
        "exit-status-ignored",
        "run.py",
        "if rc != 0:",
        "if False:",
        [
            V + "test_nonzero_exit_overrides_a_claimed_pass",
            V + "test_payload_timeout_is_bounded_by_the_hard_deadline",
        ],
    ),
    (
        "verdict-grading",
        "failures-and-skips-ignored",
        "run.py",
        " or sum(counts[1]) or sum(counts[2])",
        "",
        [V + "test_skips_are_not_a_pass", V + "test_suite_failures_are_not_a_pass"],
    ),
    (
        "verdict-grading",
        "hardware-ignored",
        "run.py",
        "if not known or not isinstance(cc, int) or isinstance(cc, bool) or cc < 750:",
        "if False:",
        [V + "test_unknown_hardware_is_not_a_pass", V + "test_low_compute_cap_is_not_a_pass"],
    ),
    (
        "listing-fails-closed",
        "success-flag-ignored",
        "vast_api.py",
        '                or r.get("success") is not True\n                or not isinstance(page',
        "                or not isinstance(page",
        [
            R + "test_unsuccessful_listing_fails_closed",
            T + "test_listing_error_during_teardown_is_a_leak",
        ],
    ),
    (
        "listing-fails-closed",
        "malformed-reported-as-clean",
        "reaper.py",
        "return MALFORMED",
        "return CLEAN",
        [R + "test_malformed_listing_shapes_fail_closed", R + "test_malformed_rows_fail_closed"],
    ),
    (
        "secrets",
        "box-output-unredacted",
        "run.py",
        "self.red.clean(chunk).splitlines()",
        "chunk.splitlines()",
        [S + "test_key_never_appears_in_logs_or_files", S + "test_box_log_file_has_no_key"],
    ),
    (
        "secrets",
        "api-base-guard-off",
        "vast_api.py",
        "if base != CONSOLE and not LOOPBACK.fullmatch(base):",
        "if False:",
        [
            S + "test_api_base_is_https_vast_or_loopback_only",
            S + "test_the_runner_refuses_a_foreign_api_base_before_any_request",
        ],
    ),
    (
        "secrets",
        "key-format-unchecked",
        "vast_api.py",
        'if not isinstance(key, str) or not re.fullmatch(r"[\\x21-\\x7e]{8,256}", key):',
        "if False:",
        [
            S + "test_missing_or_malformed_key_is_refused_before_any_request",
            R + "test_reaper_refuses_without_a_usable_key",
        ],
    ),
    (
        "ssh-hardening",
        "agent-forwarded",
        "run.py",
        '"ForwardAgent=no"',
        '"ForwardAgent=yes"',
        [X + "test_agent_forwarding_is_off", X + "test_option_set_is_exact"],
    ),
    (
        "ssh-hardening",
        "interactive-prompts",
        "run.py",
        '"BatchMode=yes"',
        '"BatchMode=no"',
        [X + "test_never_interactive_and_only_the_named_identity", X + "test_option_set_is_exact"],
    ),
    (
        "offer-rules",
        "bad-machines-ignored",
        "offers.py",
        ' or machine in cfg["bad_machines"]',
        "",
        [
            O + "test_bad_machine_file_and_defaults_are_skipped",
            A + "test_bad_machine_file_is_honoured",
        ],
    ),
    (
        "offer-rules",
        "denied-countries-ignored",
        "offers.py",
        'if country in cfg["deniedCountries"] or (',
        "if (",
        [
            O + "test_denied_and_unknown_countries_are_rejected",
            A + "test_denied_country_offer_is_never_rented",
        ],
    ),
    (
        "offer-rules",
        "verification-ignored",
        "offers.py",
        "if not verified:",
        "if False:",
        [
            O + "test_rejections_follow_the_worker_rules",
            A + "test_unverified_offer_is_never_rented",
        ],
    ),
]


def run_test(src, test):
    env = dict(os.environ, KF_VAST_SIMPLE_DIR=str(src), PYTHONDONTWRITEBYTECODE="1")
    argv = [sys.executable, "-m", "unittest", "test_vast_simple." + test]
    done = subprocess.run(argv, cwd=HERE, env=env, capture_output=True, text=True, timeout=300)
    return done.returncode


def main():
    behaviours = {}
    for case in CASES:
        behaviours.setdefault(case[0], []).append(case[1])
    bad = [b for b, names in behaviours.items() if len(names) < 2]
    bad += [c[1] for c in CASES if len(c[5]) < 2]
    base = subprocess.run(
        [sys.executable, "-m", "unittest", "test_vast_simple"],
        cwd=HERE,
        capture_output=True,
        text=True,
        env=dict(os.environ, PYTHONDONTWRITEBYTECODE="1"),
    )
    if base.returncode:
        print("baseline is red")
        print(base.stderr[-2000:])
        return 1
    failed = list(bad)
    with tempfile.TemporaryDirectory() as tmp:
        for behaviour, name, file, old, new, killers in CASES:
            copy = Path(tmp) / name
            shutil.copytree(SRC, copy)
            text = (copy / file).read_text()
            if text.count(old) != 1:
                print("%s: mutation site not found exactly once" % name)
                failed.append(name)
                continue
            (copy / file).write_text(text.replace(old, new))
            survived = [t for t in killers if run_test(copy, t) == 0]
            print(
                "%-22s %-30s %s"
                % (behaviour, name, "SURVIVED " + str(survived) if survived else "killed")
            )
            failed += [name] if survived else []
    print("mutants=%d behaviours=%d failed=%d" % (len(CASES), len(behaviours), len(failed)))
    return 1 if failed else 0


if __name__ == "__main__":
    sys.exit(main())
