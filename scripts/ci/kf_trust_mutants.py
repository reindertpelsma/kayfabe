#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later
"""Mutation checks of scripts/ci/kf_trust.py (STATUS: LIVE, 2026-11-17): every mutant is ONE exact-once text replacement in a private
copy of the tree; its two named killers (tests of scripts/ci/test_kf_trust.py) must both fail on the copy. The unmutated copy must pass
every named killer first. GPU-free, stdlib only. `trust_lint_mutants.py` shares `run_mutants` from here."""
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
TARGET = "scripts/ci/kf_trust.py"
COPY = ["scripts/ci/kf_trust.py", "scripts/ci/kf_fetch.py", "scripts/ci/trust_lint.py", "scripts/ci/trust_manifest.json",
        "scripts/ci/test_kf_trust.py", "scripts/ci/test_trust_lint.py"]
COLLECT = '''import io, json, sys, unittest
suite = unittest.TestSuite(unittest.defaultTestLoader.loadTestsFromName(n) for n in sys.argv[1:])
r = unittest.TextTestRunner(stream=io.StringIO()).run(suite)
print(json.dumps({"tests": r.testsRun, "failed": sorted(set(t.id() for t, _ in r.failures + r.errors))}))
'''


def run_tests(tree, names):
    p = subprocess.run([sys.executable, "-B", "-c", COLLECT, *names], cwd=tree / "scripts/ci", capture_output=True, text=True, timeout=900,
                       env={**os.environ, "PYTHONDONTWRITEBYTECODE": "1"})
    if p.returncode:
        raise RuntimeError("mutation harness failed: " + p.stderr[-500:])
    return json.loads(p.stdout.splitlines()[-1])


def prepare(parent, extra=()):
    root = Path(tempfile.mkdtemp(prefix="trustm-", dir=parent))
    for rel in [*COPY, *extra]:
        dest = root / rel
        dest.parent.mkdir(parents=True, exist_ok=True)
        shutil.copyfile(ROOT / rel, dest)
    return root


def run_mutants(mutants, prefix, jobs, only="", extra=()):
    """mutants: (name, target path, old, new, [two killer test ids]). Returns a process exit status."""
    selected = [m for m in mutants if only in m[0]]
    if not selected:
        print("no mutant matches %r" % only)
        return 2
    sources = {}
    for _, target, old, _, _ in selected:
        sources.setdefault(target, (ROOT / target).read_text())
        if sources[target].count(old) != 1:
            print("BAD mutant anchor (found %d times, need exactly 1): %s" % (sources[target].count(old), old[:70].replace("\n", "\\n")))
            return 2
    for name, _, _, _, killers in selected:
        if len(set(killers)) < 2:
            print("BAD mutant %s: two distinct named killers are required" % name)
            return 2
    with tempfile.TemporaryDirectory(prefix="trustm-parent-") as parent:
        base = prepare(parent, extra)
        names = sorted({k for m in mutants for k in m[4]})
        baseline = run_tests(base, names)
        if baseline["failed"]:
            print("FAIL unmutated baseline: %s" % baseline["failed"][:5])
            return 2
        print("PASS baseline (%d named tests)" % baseline["tests"], flush=True)

        def one(m):
            name, target, old, new, killers = m
            tree = prepare(parent, extra)
            (tree / target).write_text(sources[target].replace(old, new))
            got = run_tests(tree, killers)
            shutil.rmtree(tree, ignore_errors=True)
            return name, killers, got["failed"]

        weak = 0
        with concurrent.futures.ThreadPoolExecutor(max_workers=jobs) as pool:
            for name, killers, failed in pool.map(one, selected):
                killed = [k for k in killers if k in failed]
                ok = len(set(killed)) >= 2
                print("%s %s: %s" % ("PASS" if ok else "WEAK", name, [k.rsplit(".", 1)[-1] for k in killed] if ok else
                                     "only %d of 2 named killers fail: %s" % (len(killed), [k.rsplit(".", 1)[-1] for k in killed])), flush=True)
                weak += not ok
    print("%d/%d %s mutants killed by both named tests" % (len(selected) - weak, len(selected), prefix))
    return 1 if weak else 0


def c(name, old, new, k1, k2, target=TARGET):
    return name, target, old, new, ["test_kf_trust." + k1, "test_kf_trust." + k2]


P = "UpstreamPinAnchor."
O = "OfficialChecksumAnchor."
A = "AttestationStub."
F = "ForbiddenAndMissingAnchors."
M = "Modes."
L = "ExplicitLift."
U = "UserInput."
E = "EvidenceRecord."
G = "GitCommitAnchor."
R = "RealTransport."
H = "Helpers."
K = "Cli."
V = "ManifestChecks."

MUTANTS = [
    c("pin-not-compared-after-the-transfer", "        if got != want or (size is not None and n != size):\n            _discard(tmp)", "        if False:\n            _discard(tmp)",
      P + "test_pin_mismatch_when_the_transport_does_not_enforce", P + "test_pin_size_is_enforced"),
    c("unverified-bytes-kept-after-a-mismatch",
      '            _discard(tmp)\n            raise TrustError(_mismatch_code(exp), name, "the download is not the anchored object (computed %s)" % got)',
      '            raise TrustError(_mismatch_code(exp), name, "the download is not the anchored object (computed %s)" % got)',
      P + "test_pin_mismatch_when_the_transport_does_not_enforce", P + "test_pin_size_is_enforced"),
    c("pin-size-dropped", "    size = next((e[2] for e in exp if e[2] is not None), None)", "    size = None",
      P + "test_pin_size_is_enforced", P + "test_pin_is_the_expected_hash_handed_to_the_transport"),
    c("cache-trusted-without-hashing", "        cache = have == want and size in (None, n)", "        cache = True",
      P + "test_existing_wrong_file_is_replaced_not_trusted", K + "test_verify_checks_a_file_on_disk_without_the_network"),
    c("sum-mismatch-code-lost", '    return "TRUST_UPSTREAM_SUM_MISMATCH" if any(e[0] == CLASS_OFFICIAL for e in exp) else "TRUST_PIN_MISMATCH"',
      '    return "TRUST_PIN_MISMATCH"',
      O + "test_wrong_official_checksum_is_refused_by_name", O + "test_wrong_official_checksum_when_the_transport_does_not_enforce"),
    c("checksum-from-our-host-accepted", '            if is_ours(url):\n                raise TrustError("TRUST_SAME_ORIGIN"',
      '            if False:\n                raise TrustError("TRUST_SAME_ORIGIN"',
      O + "test_checksum_from_a_host_we_run_is_refused_as_same_origin", O + "test_checksum_from_our_github_repository_is_same_origin"),
    c("our-hosts-not-recognised", "    if any(host == s or host.endswith(\".\" + s) for s in OUR_HOST_SUFFIXES):", "    if False:",
      H + "test_redact_url_and_ownership", O + "test_checksum_from_a_host_we_run_is_refused_as_same_origin"),
    c("anchors-may-disagree", "    if len(hashes) != 1:", "    if False:",
      O + "test_checksum_and_pin_that_disagree_refuse_before_any_download", M + "test_a_pin_in_git_and_an_operator_pin_that_disagree_are_refused"),
    c("ambiguous-checksum-document-read", "    return rows[0][0] if len(rows) == 1 else None", "    return rows[0][0] if rows else None",
      O + "test_an_ambiguous_checksum_document_is_refused", H + "test_parse_sha256sum_formats"),
    c("duplicate-member-read", "    return found[0] if len(found) == 1 else None", "    return found[0] if found else None",
      O + "test_two_entries_for_the_member_are_refused", H + "test_parse_sha256sum_formats"),
    c("attestation-passes", '    raise TrustError("TRUST_ATTEST_NOT_IMPLEMENTED", name, "the attestation anchor is not implemented in this tree")', "    return True",
      A + "test_attestation_function_refuses_and_never_returns_true", A + "test_attestation_beside_a_pin_is_skipped_by_name_not_silently"),
    c("forbidden-anchor-accepted", '        if cls == CLASS_FORBIDDEN:\n            raise TrustError("TRUST_ANCHOR_FORBIDDEN", name,',
      '        if False:\n            raise TrustError("TRUST_ANCHOR_FORBIDDEN", name,',
      F + "test_a_sha256_in_git_of_our_own_artifact_is_never_an_anchor", F + "test_forbidden_anchor_is_refused_even_when_a_valid_pin_sits_beside_it"),
    c("no-anchor-downloads-anyway", "    if not exp:\n        code = ", "    if False:\n        code = ",
      F + "test_an_artifact_without_any_anchor_is_refused_in_the_default_mode", M + "test_an_operator_pin_is_not_an_anchor_in_the_default_mode"),
    c("operator-pin-counts-in-the-default-mode", "    if mode == MODE_STRICT and operator_pin:\n        exp.append", "    if operator_pin:\n        exp.append",
      M + "test_an_operator_pin_is_not_an_anchor_in_the_default_mode", M + "test_an_operator_pin_that_disagrees_is_ignored_in_the_default_mode"),
    c("default-mode-is-strict", 'os.environ.get("KF_TRUST_MODE") or MODE_GIT_ONLY', 'os.environ.get("KF_TRUST_MODE") or MODE_STRICT',
      M + "test_default_mode_is_trust_git_only", U + "test_a_user_path_is_trusted_whatever_the_pin_says_and_is_recorded"),
    c("bad-mode-accepted", '    if mode not in MODES:\n        raise TrustError("TRUST_BAD_MODE"', '    if False:\n        raise TrustError("TRUST_BAD_MODE"',
      M + "test_mode_comes_from_the_environment_and_a_bad_value_is_refused", K + "test_the_environment_selects_the_mode_and_the_lift"),
    c("strict-mode-ignores-the-pin-of-a-user-object", "    if refusal is None and mode == MODE_STRICT:", "    if refusal is None and False:",
      U + "test_strict_mode_holds_a_user_object_to_the_pin", U + "test_strict_mode_holds_a_user_object_to_the_operator_pin"),
    c("default-mode-enforces-our-pin-on-a-user-object", "    if refusal is None and mode == MODE_STRICT:", "    if refusal is None:",
      U + "test_a_user_path_is_trusted_whatever_the_pin_says_and_is_recorded", U + "test_a_user_url_for_a_repo_defined_artifact_is_a_user_input_not_a_mirror_check"),
    c("expect-sha256-not-enforced", "    if expect and sha != expect:", "    if False:",
      U + "test_expect_sha256_is_enforced_in_every_mode", U + "test_expect_sha256_mismatch_deletes_a_downloaded_user_url"),
    c("expect-sha256-format-unchecked", "    if expect_sha256 is not None and not SHA.match(expect_sha256):", "    if False:",
      U + "test_a_malformed_expect_sha256_is_a_usage_error", K + "test_a_malformed_expect_sha256_exits_two"),
    c("unlisted-user-object-refused", "    if entry is None and user_source is not None:", "    if False:",
      U + "test_a_users_own_object_needs_no_manifest_entry", K + "test_a_user_object_of_an_unlisted_name_is_recorded"),
    c("lift-implied", "            if lift is None or e.code not in LIFTABLE or not download:", "            if e.code not in LIFTABLE or not download:",
      L + "test_a_failure_stays_a_refusal_without_a_lift", L + "test_an_empty_or_unprintable_reason_is_refused_not_treated_as_a_lift"),
    c("lift-covers-the-users-own-expectation", 'LIFTABLE = ("TRUST_NO_ANCHOR",', 'LIFTABLE = ("TRUST_EXPECT_MISMATCH", "TRUST_TRANSPORT", "TRUST_NO_ANCHOR",',
      L + "test_a_lift_never_covers_the_users_own_expect_sha256", L + "test_a_lift_cannot_hide_an_expect_sha256_that_names_a_third_object"),
    c("lifted-result-is-gating", 'detail={"lifted_error": error.code}, gating=False, lift_reason=lift)', 'detail={"lifted_error": error.code}, gating=True, lift_reason=lift)',
      L + "test_an_explicit_lift_records_unverified_lifted_with_the_computed_hash", L + "test_a_lifted_result_is_marked_non_gating_in_the_printed_line"),
    c("lift-not-announced", '    if result["anchor"] == ANCHOR_LIFTED:', "    if False:",
      L + "test_the_lift_is_printed_and_written_to_the_evidence", H + "test_record_prints_the_lift_line"),
    c("lift-reason-unbounded", "    if not (1 <= len(lift) <= LIFT_MAX) or not lift.isprintable() or lift != lift.strip():", "    if False:",
      L + "test_an_empty_or_unprintable_reason_is_refused_not_treated_as_a_lift", H + "test_normalize_lift_bounds"),
    c("evidence-gate-passes-a-lifted-line", '            if row["anchor"] == ANCHOR_LIFTED or row.get("gating") is False:', "            if False:",
      L + "test_the_check_evidence_gate_refuses_a_result_with_a_lifted_line", K + "test_check_evidence_exit_codes"),
    c("origin-url-kept-with-its-query", '    return "%s://%s%s" % (p.scheme, host, p.path), bool(p.query or p.fragment or p.username or p.password)', "    return url, False",
      E + "test_a_presigned_user_url_never_reaches_the_evidence_or_stdout", H + "test_redact_url_and_ownership"),
    c("user-url-recorded-unredacted", "        origin, redacted = redact_url(user_source)\n        kind, final = ", "        origin, redacted = user_source, False\n        kind, final = ",
      E + "test_a_presigned_user_url_never_reaches_the_evidence_or_stdout", E + "test_credentials_in_a_url_are_never_recorded"),
    c("user-path-recorded-in-full", 'origin, redacted, kind = "file:" + src.name, False, "user-path"', 'origin, redacted, kind = "file:" + str(src), False, "user-path"',
      E + "test_a_user_path_records_only_the_file_name", E + "test_the_record_of_a_user_path_has_no_directory_part"),
    c("evidence-overwritten", "os.O_WRONLY | os.O_APPEND | os.O_CREAT", "os.O_WRONLY | os.O_TRUNC | os.O_CREAT",
      E + "test_lines_append_and_never_overwrite", K + "test_a_second_record_run_appends_a_line"),
    c("manifest-digest-not-recorded", '    line["manifest_sha256"] = manifest_digest()', '    line["manifest_sha256"] = None',
      E + "test_the_record_names_the_manifest_that_judged_it", K + "test_the_record_line_carries_the_manifest_digest_and_the_mode"),
    c("record-on-stdout", "    out = out or sys.stderr", "    out = out or sys.stdout",
      K + "test_stdout_stays_empty_for_fetch", K + "test_fetch_of_a_user_path_exits_zero_and_prints_the_record"),
    c("pending-confirmation-never-flagged", '    if a.get("class") == CLASS_PIN and a.get("scope") == "upstream-artifact" and a.get("pin_origin") == "measured-from-upstream":',
      "    if False:",
      V + "test_only_an_upstream_binary_pinned_by_measurement_awaits_the_owner", V + "test_the_pending_confirmation_is_recorded_in_the_result"),
    c("plain-http-origin-accepted", '        if (p.scheme != "https" and not loop) or not p.hostname or p.username or p.password or p.fragment:',
      "        if not p.hostname:",
      R + "test_https_only_by_default", V + "test_an_http_origin_is_invalid"),
    c("checksum-document-may-change-host", '        if new.scheme != "https" or new.hostname != old.hostname:\n            return None', "        if False:\n            return None",
      R + "test_a_checksum_document_may_not_move_to_another_host", R + "test_a_checksum_document_redirected_to_plain_http_is_refused"),
    c("pin-not-handed-to-kf-fetch", '"--sha256", sha256, "--streams"', '"--streams"',
      R + "test_pinned_download_goes_through_kf_fetch_and_verifies", R + "test_end_to_end_official_checksum_then_pinned_fetch"),
    c("relative-path-handed-to-kf-fetch-as-is", "        dest = Path(os.path.abspath(dest))  # kf_fetch runs", "        dest = Path(dest)  # kf_fetch runs",
      R + "test_a_relative_output_path_means_the_callers_directory", R + "test_pinned_download_goes_through_kf_fetch_and_verifies_from_a_relative_path"),
    c("published-mode-left-as-kf-fetch-made-it", "    os.chmod(tmp, 0o666 & ~mask)\n", "",
      P + "test_the_published_file_has_the_mode_of_a_plain_download_not_kf_fetchs_0600", R + "test_a_published_download_is_readable_by_the_group_and_others"),
    c("failed-transfer-leaves-files", "                dest.unlink(missing_ok=True)\n                for leftover in dest.parent.glob(glob_escape(dest.name) + \".*\"):\n                    leftover.unlink(missing_ok=True)\n",
      "                pass\n",
      R + "test_a_failed_pinned_download_leaves_nothing_beside_the_output", R + "test_missing_object_is_a_network_failure"),
    c("unbounded-unpinned-download", "                    if n > limit:\n                        raise TransportError(\"SIZE\", \"more than %d bytes\" % limit)", "                    if False:\n                        raise TransportError(\"SIZE\", \"more than %d bytes\" % limit)",
      R + "test_unpinned_download_hashes_and_bounds_the_bytes", R + "test_a_lifted_download_is_bounded_by_twice_the_pinned_size"),
    c("git-head-not-compared", '        elif head != pin["commit"]:\n            problem = (', '        elif False:\n            problem = (',
      G + "test_a_moved_tag_is_refused_and_removes_the_checkout", G + "test_a_lift_records_the_commit_that_was_actually_checked_out"),
    c("refused-checkout-kept", "    def fail(code, detail):\n        shutil.rmtree(dest, ignore_errors=True)\n", "    def fail(code, detail):\n",
      G + "test_a_moved_tag_is_refused_and_removes_the_checkout", G + "test_strict_mode_holds_a_user_remote_to_the_pinned_commit"),
    c("user-remote-pin-enforced-in-the-default-mode", '        if mode == MODE_STRICT and state == "differs":', '        if state == "differs":',
      G + "test_a_user_remote_is_trusted_and_its_commit_recorded", G + "test_a_user_remote_that_differs_is_recorded_as_not_enforced_in_the_default_mode"),
    c("cached-checkout-head-ignored", '    if pin is None or head != pin["commit"]:', "    if pin is None:",
      G + "test_an_existing_checkout_at_another_commit_is_refused", G + "test_verify_git_with_a_lift_records_the_mismatch_as_lifted_and_not_gating"),
    c("artifact-name-unchecked-in-resolve", "    check_name(name)\n    mode, lift = normalize_mode(mode), normalize_lift(lift)\n    if expect_sha256", "    mode, lift = normalize_mode(mode), normalize_lift(lift)\n    if expect_sha256",
      U + "test_a_name_that_could_forge_a_log_line_is_refused", K + "test_fetch_with_a_forged_name_exits_two_and_writes_nothing"),
    c("artifact-name-unchecked-in-record", "            check_name(a.name)\n", "",
      K + "test_a_name_with_a_newline_cannot_forge_a_record_line", K + "test_record_refuses_a_forged_name_in_process_too"),
    c("unknown-artifact-exits-as-a-refusal", '"TRUST_UNKNOWN_ARTIFACT": 4', '"TRUST_UNKNOWN_ARTIFACT": 3',
      K + "test_unknown_artifact_and_bad_usage_exit_codes", K + "test_exit_codes_are_the_documented_ones"),
]


def main():
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument("--jobs", type=int, default=4)
    ap.add_argument("only", nargs="?", default="")
    a = ap.parse_args()
    if not 1 <= a.jobs <= 8:
        ap.error("--jobs is 1 through 8")
    return run_mutants(MUTANTS, "kf_trust", a.jobs, a.only)


if __name__ == "__main__":
    sys.exit(main())
