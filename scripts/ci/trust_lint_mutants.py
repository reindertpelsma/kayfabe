#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later
"""Mutation checks of scripts/ci/trust_lint.py (STATUS: LIVE, 2026-11-17): every mutant is ONE exact-once text replacement in a private copy;
its two named killers (tests of scripts/ci/test_trust_lint.py) must both fail on the copy. The unmutated copy must pass every named killer
first. The runner is the one of kf_trust_mutants.py. GPU-free, stdlib only."""
import argparse
from pathlib import Path
import sys

sys.path.insert(0, str(Path(__file__).resolve().parent))
import kf_trust_mutants as KM  # noqa: E402

TARGET = "scripts/ci/trust_lint.py"


def c(name, old, new, k1, k2):
    return name, TARGET, old, new, ["test_trust_lint." + k1, "test_trust_lint." + k2]


A, R, S, D, K = "Anchors.", "Routing.", "Scan.", "Dockerfiles.", "Cli."

MUTANTS = [
    c("forbidden-anchor-not-reported", "            if isinstance(a, dict) and a.get(\"class\") == kf_trust.CLASS_FORBIDDEN:", "            if False:",
      A + "test_a_forbidden_anchor_is_a_finding", A + "test_a_forbidden_anchor_beside_a_valid_pin_is_still_a_finding"),
    c("empty-anchor-list-passes-as-no-anchor", '        if not anchors:\n            out.append(("NO_ANCHOR", name, "the artifact has no anchor"))',
      '        if False:\n            out.append(("NO_ANCHOR", name, "the artifact has no anchor"))',
      A + "test_an_artifact_with_an_empty_anchor_list_is_a_finding", A + "test_an_artifact_with_no_anchor_key_is_a_finding"),
    c("manifest-problems-dropped", "    for problem in kf_trust.validate_manifest(doc):", "    for problem in []:",
      A + "test_a_checksum_url_on_a_host_we_run_is_a_manifest_finding", A + "test_a_pin_whose_source_is_ours_is_a_manifest_finding"),
    c("missing-consumer-ignored", '                out.append(("CONSUMER_MISSING", name, c))', "                pass",
      R + "test_a_missing_consumer_is_a_finding", R + "test_a_missing_consumer_of_a_native_entry_is_a_finding_too"),
    c("routing-not-checked", "                if c not in routed and not beside:", "                if False:",
      R + "test_a_helper_routed_consumer_that_does_not_mention_kf_trust_is_a_finding", R + "test_a_second_consumer_that_names_no_routed_file_is_a_finding_too"),
    c("any-consumer-counts-as-beside-a-routed-one", "                beside = any(Path(r).name in t for r in routed if r != c)", "                beside = True",
      R + "test_a_helper_routed_consumer_that_does_not_mention_kf_trust_is_a_finding", R + "test_a_second_consumer_that_names_no_routed_file_is_a_finding_too"),
    c("raw-origin-not-checked", '                if origin and origin in "\\n".join(code_lines(Path(c), t)) and c not in EXEMPT:', "                if False:",
      R + "test_a_consumer_that_still_spells_the_origin_url_is_a_finding", R + "test_a_python_consumer_that_still_spells_the_origin_url_is_a_finding"),
    c("origin-in-a-comment-counts", '                if origin and origin in "\\n".join(code_lines(Path(c), t)) and c not in EXEMPT:',
      "                if origin and origin in t and c not in EXEMPT:",
      R + "test_a_comment_that_names_the_origin_is_not_a_raw_fetch", R + "test_a_python_docstring_that_names_the_origin_is_not_a_raw_fetch"),
    c("pin-drift-not-checked", '                if s not in t:\n                    out.append(("PIN_DRIFT"', '                if False:\n                    out.append(("PIN_DRIFT"',
      R + "test_a_pin_that_left_its_file_is_drift", R + "test_a_string_missing_among_several_is_drift"),
    c("missing-pin-file-ignored", '                out.append(("PIN_DRIFT", name, "%s is missing" % pf.get("file")))', "                pass",
      R + "test_a_pin_file_that_is_gone_is_drift", R + "test_every_pin_file_of_an_entry_is_checked_not_only_the_first"),
    c("image-digest-not-checked-in-the-from-line", "                    if d and not any(d in l and e.get(\"origin\", \"\").split(\"/\", 1)[-1] in l for l in froms):", "                    if False:",
      R + "test_an_image_entry_needs_its_digest_in_the_dockerfile_from_line", R + "test_an_image_digest_on_another_images_from_line_is_drift"),
    c("new-download-not-flagged", '        if u is None:\n            out.append(("UNROUTED_DOWNLOAD", path, "%d download primitive line(s), not routed',
      '        if False:\n            out.append(("UNROUTED_DOWNLOAD", path, "%d download primitive line(s), not routed',
      S + "test_a_new_script_with_a_curl_download_is_a_finding", S + "test_a_new_python_urlopen_is_a_finding"),
    c("more-downloads-than-listed-allowed", '        elif isinstance(u, dict) and type(u.get("count")) is int and n > u["count"]:', "        elif False:",
      S + "test_a_listed_file_that_gained_a_download_is_a_finding", S + "test_a_listed_python_file_that_gained_a_download_is_a_finding"),
    c("fewer-downloads-than-listed-allowed", '        elif isinstance(u, dict) and type(u.get("count")) is int and n < u["count"]:', "        elif False:",
      S + "test_a_listed_file_that_lost_a_download_is_stale", S + "test_a_listed_python_file_that_lost_a_download_is_stale"),
    c("stale-entry-not-reported", "        if path not in found and path not in dirs:", "        if False:",
      S + "test_a_listed_file_without_any_download_left_is_stale", S + "test_a_listed_file_that_is_gone_is_stale"),
    c("bad-entry-accepted", '            out.append(("BAD_UNROUTED", path, "needs count >= 1, one of %s and a reason" % (CLASSES,)))', "            pass",
      S + "test_an_entry_needs_a_class_a_count_and_a_reason", S + "test_a_short_reason_is_refused"),
    c("shell-comments-counted", '    return [l for l in lines if not l.lstrip().startswith("#")]', "    return lines",
      S + "test_a_comment_that_mentions_curl_is_not_a_download", R + "test_a_comment_that_names_the_origin_is_not_a_raw_fetch"),
    c("python-comments-counted", "                if tok.type == tokenize.COMMENT:", "                if False:",
      S + "test_a_comment_that_mentions_curl_is_not_a_download", R + "test_a_python_comment_that_names_the_origin_is_not_a_raw_fetch"),
    c("docstrings-counted", "                        blank.update(range(first.lineno - 1, first.end_lineno))", "                        pass",
      S + "test_a_python_docstring_that_mentions_urlopen_is_not_a_download", R + "test_a_python_docstring_that_names_the_origin_is_not_a_raw_fetch"),
    c("package-names-counted", '_ARG = r"\\s+(?:-|\\"|\'|\\$|https?://)"', '_ARG = r"\\b"',
      S + "test_a_package_name_in_an_apt_line_is_not_a_download", S + "test_a_dependency_table_entry_is_not_a_download"),
    c("test-files-scanned", '    if name.startswith("test_") or name.endswith("_mutants.py") or name.endswith(".md"):\n        return False',
      '    if False:\n        return False',
      S + "test_test_files_and_test_directories_are_not_scanned", S + "test_mutant_files_and_markdown_are_not_scanned"),
    c("kf-trust-not-exempt", 'EXEMPT = {"scripts/ci/kf_trust.py"}', "EXEMPT = set()",
      S + "test_kf_trust_itself_is_the_one_exempt_downloader", S + "test_the_exempt_downloader_may_also_shell_out"),
    c("any-file-named-kf-trust-is-exempt", "    if parts & SKIP_PARTS or str(rel) in EXEMPT:", "    if parts & SKIP_PARTS or rel.name == \"kf_trust.py\":",
      S + "test_a_file_named_like_the_exempt_one_elsewhere_is_scanned", S + "test_a_new_python_urlopen_is_a_finding_in_a_file_called_kf_trust"),
    c("extensionless-scripts-not-scanned", '        try:\n            return path.read_bytes()[:2] == b"#!"', "        try:\n            return False",
      S + "test_an_extensionless_script_with_a_shebang_is_scanned", S + "test_an_extensionless_python_script_is_scanned_too"),
    c("directory-total-ignored", "            totals[owner] += n", "            pass",
      S + "test_a_directory_entry_counts_the_total_of_its_files", S + "test_a_directory_entry_at_its_total_is_clean"),
    c("unpinned-from-allowed", '                if ref != "scratch" and ref not in stages and not re.search(r"@sha256:[0-9a-f]{64}$", ref):', "                if False:",
      D + "test_a_from_without_a_digest_is_a_finding", D + "test_a_later_unpinned_stage_is_a_finding"),
    c("any-anchor-makes-an-artifact-not-pending", '        if verdicts and all(v == "pending-owner-confirmation" for v in verdicts):',
      '        if verdicts and any(v == "pending-owner-confirmation" for v in verdicts):',
      A + "test_an_artifact_with_one_confirmed_anchor_is_not_pending", A + "test_the_pending_list_names_only_artifacts_resting_on_a_measured_binary"),
    c("findings-exit-zero", "    return 1 if findings else 0", "    return 0",
      K + "test_exit_zero_when_clean_and_one_with_a_named_line_when_not", K + "test_a_forbidden_anchor_exits_one_with_its_name"),
]


def main():
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument("--jobs", type=int, default=4)
    ap.add_argument("only", nargs="?", default="")
    a = ap.parse_args()
    if not 1 <= a.jobs <= 8:
        ap.error("--jobs is 1 through 8")
    return KM.run_mutants(MUTANTS, "trust_lint", a.jobs, a.only)


if __name__ == "__main__":
    sys.exit(main())
