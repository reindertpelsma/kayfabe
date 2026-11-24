#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later
"""Mutation checks of scripts/ci/env_allowlist.py (STATUS: LIVE, 2026-11-20): every mutant is ONE exact-once text replacement in a
private copy; its two named killers (tests of scripts/ci/test_env_allowlist.py) must both fail on the copy. The unmutated copy must
pass every named killer first. The runner is the one of kf_trust_mutants.py. GPU-free, stdlib only.
Run: python3 scripts/ci/env_allowlist_mutants.py [--jobs N] [name-substring]"""
import argparse
from pathlib import Path
import re
import sys

HERE = Path(__file__).resolve().parent
sys.path.insert(0, str(HERE))
import kf_trust_mutants as KM  # noqa: E402

TARGET = "scripts/ci/env_allowlist.py"
TESTS = (HERE / "test_env_allowlist.py").read_text()


def k(name):
    """The id of a test of test_env_allowlist.py by its method name (the class it sits in is looked up)."""
    cls = None
    for line in TESTS.splitlines():
        m = re.match(r"class (\w+)\(", line)
        if m:
            cls = m.group(1)
        if line.strip().startswith("def %s(" % name):
            return "test_env_allowlist.%s.%s" % (cls, name)
    raise SystemExit("no test named %s in test_env_allowlist.py" % name)


def c(name, old, new, k1, k2):
    return name, TARGET, old, new, [k(k1), k(k2)]


MUTANTS = [
    c("line-comments-not-masked", '        if text.startswith("//", i):', "        if False:",
      "test_a_name_in_a_line_comment_is_not_a_read", "test_a_name_in_a_doc_comment_is_not_a_read"),
    c("block-comments-not-masked", '        elif text.startswith("/*", i):', "        elif False:",
      "test_a_name_in_a_block_comment_is_not_a_read_and_blocks_nest", "test_getenv_in_a_c_comment_or_string_is_not_a_read"),
    c("rust-block-comments-do-not-nest", '                if lang == "rust" and text.startswith("/*", j):', "                if False:",
      "test_a_name_in_a_block_comment_is_not_a_read_and_blocks_nest", "test_a_rust_block_comment_can_hold_a_second_one_and_code_after_it_is_scanned"),
    c("string-contents-not-masked", "            blank(skel, i + 1, min(j, n))", "            pass",
      "test_a_name_inside_a_string_is_not_a_read", "test_getenv_in_a_c_comment_or_string_is_not_a_read"),
    c("string-escapes-ignored", '                j += 2 if text[j] == "\\\\" else 1\n            blank(skel, i + 1, min(j, n))',
      '                j += 1\n            blank(skel, i + 1, min(j, n))',
      "test_an_escaped_quote_can_hide_a_read_inside_a_string", "test_an_escaped_quote_does_not_end_a_string"),
    c("raw-strings-not-handled", '        elif lang == "rust" and c in "rb" and re.match', '        elif False and re.match',
      "test_a_raw_string_with_an_odd_number_of_quotes_hides_a_read", "test_a_raw_string_with_a_quote_inside_does_not_derail_the_scan"),
    c("char-literals-not-masked", '            elif text[i + 2:i + 3] == "\'":', "            elif False:",
      "test_a_double_quote_char_literal_does_not_open_a_string", "test_a_brace_char_literal_does_not_unbalance_a_skipped_module"),
    c("cfg-test-item-ends-at-its-head", '        end = match_brace(skel, stop.start()) if stop.group() == "{" else stop.end()', "        end = stop.end()",
      "test_an_inline_cfg_test_module_is_not_product_code", "test_code_after_a_cfg_test_module_is_product_code_again"),
    c("further-attributes-not-skipped", "        while True:  # further attributes of the same item", "        while False:",
      "test_a_declared_test_file_behind_another_attribute_is_skipped", "test_a_declared_test_directory_behind_another_attribute_is_skipped"),
    c("cfg-all-test-not-recognised", r"(?:test|all\s*\(\s*test\b[^\]]*)", "(?:test)",
      "test_a_cfg_all_test_module_is_skipped", "test_a_cfg_all_test_with_a_second_condition_is_skipped"),
    c("every-cfg-counts-as-test", r"(?:test|all\s*\(\s*test", r"(?:\w+|all\s*\(\s*test",
      "test_a_cfg_debug_assertions_module_is_product_code", "test_a_cfg_unix_item_is_product_code"),
    c("declared-test-files-scanned", "        if p in test_files:", "        if False:",
      "test_a_file_declared_as_a_cfg_test_module_is_skipped", "test_a_directory_declared_as_a_cfg_test_module_is_skipped"),
    c("test-module-directory-not-walked", "            if (base / name).is_dir():", "            if False:",
      "test_a_directory_declared_as_a_cfg_test_module_is_skipped", "test_a_nested_directory_of_a_test_module_is_skipped"),
    c("tests-directories-scanned", 'SKIP_DIRS = {"tests", "benches"', 'SKIP_DIRS = {"benches"',
      "test_a_tests_directory_is_skipped", "test_a_nested_tests_directory_is_skipped"),
    c("benches-scanned", '"benches", "examples", "target"', '"examples", "target"',
      "test_a_benches_directory_is_skipped", "test_benches_and_examples_are_skipped"),
    c("examples-scanned", '"examples", "target"', '"target"',
      "test_an_examples_directory_is_skipped", "test_benches_and_examples_are_skipped"),
    c("test-named-files-scanned", "and not TEST_STEM.search(p.stem)", "and True",
      "test_a_tests_rs_file_is_skipped", "test_test_named_files_are_skipped"),
    c("test-stem-matches-inside-words", r'TEST_STEM = re.compile(r"(^|[_-])tests?($|[_-])")', r'TEST_STEM = re.compile(r"tests?")',
      "test_contest_and_attest_names_are_scanned", "test_a_file_whose_name_only_contains_test_inside_a_word_is_scanned"),
    c("every-crates-directory-scanned", 'sorted(d for d in dirnames if d.startswith("kf-"))', "sorted(dirnames)",
      "test_a_directory_not_named_kf_is_not_scanned", "test_the_frozen_crates_and_the_archive_are_not_scanned"),
    c("archive-scanned", 'SCAN = ("qemu/hw/misc/kf3", "broker", "cuda/walk", "firmware", "crates")',
      'SCAN = ("qemu/hw/misc/kf3", "broker", "cuda/walk", "firmware", "crates", "archive/crates")',
      "test_the_archive_is_not_scanned", "test_the_frozen_crates_and_the_archive_are_not_scanned"),
    c("consts-not-resolved", '        if how == "ident" and arg in consts or (how == "ident" and arg.split("::")[-1] in consts):', "        if False:",
      "test_a_const_imported_by_name_from_another_file_is_resolved", "test_a_const_in_another_file_is_resolved_through_its_path"),
    c("own-consts-not-preferred", '        if how == "ident" and arg in own:', "        if False:",
      "test_a_files_own_const_wins_over_a_later_file_of_the_same_name", "test_a_const_name_defined_in_two_files_resolves_to_each_files_own_value"),
    c("name-lists-not-collected", "        if names:\n", "        if False:\n",
      "test_a_list_of_names_in_the_function_names_each", "test_a_name_list_in_an_array_of_one_is_found"),
    c("injected-lookups-not-followed", "        elif lookups(code):", "        elif False:",
      "test_an_injected_lookup_names_the_get_calls", "test_an_injected_lookup_with_a_kayfabe_name_is_found"),
    c("dynamic-reads-dropped", '            reads.append((f"<dynamic:{rel}:{fn}>", line, "dynamic"))', "            pass",
      "test_a_computed_name_is_a_dynamic_read_that_names_file_and_fn", "test_a_computed_getenv_is_dynamic"),
    c("enumeration-ignored", '            if kind in ("vars", "vars_os"):', "            if False:",
      "test_vars_os_is_an_enumeration", "test_vars_is_an_enumeration_too"),
    c("use-without-a-call-counted", "            if m.group(1) and not m.group(2):", "            if False:",
      "test_a_use_without_a_call_is_not_a_read", "test_a_use_of_var_os_without_a_call_is_not_a_read"),
    c("plain-env-macro-missed", r"|(?<![\w])(option_env|env)!\s*\(", r"|(?<![\w])(option_env)!\s*\(",
      "test_the_env_macro_alone_is_a_read", "test_compile_time_env_macros_are_reads"),
    c("line-numbers-off-by-one", '        line = skel.count("\\n", 0, m.start()) + 1', '        line = skel.count("\\n", 0, m.start())',
      "test_the_line_of_a_read_is_reported", "test_an_unregistered_read_names_its_file_and_line"),
    c("secure-getenv-missed", r"(?<![\w])(?:secure_getenv|getenv)\s*\(", r"(?<![\w])(?:getenv)\s*\(",
      "test_secure_getenv_is_found", "test_secure_getenv_in_the_device_is_found"),
    c("getenv-suffix-of-a-longer-name-counts", r'C_CALL = re.compile(r"(?<![\w])(?:secure_getenv', r'C_CALL = re.compile(r"(?:secure_getenv',
      "test_a_longer_identifier_ending_in_getenv_is_not_a_read", "test_a_prefixed_getenv_wrapper_is_not_a_read"),
    c("unregistered-not-reported", "        if name not in entries and name not in invalid:\n            f, l, _ = sites[0]",
      "        if False:\n            f, l, _ = sites[0]",
      "test_a_read_that_is_not_in_the_registry_is_a_finding", "test_a_dynamic_read_must_be_registered_under_its_pseudo_name"),
    c("stale-entry-not-reported", '        if name not in reads:\n            out.append(("STALE_ENTRY"', '        if False:\n            out.append(("STALE_ENTRY"',
      "test_an_entry_that_nothing_reads_is_stale", "test_an_entry_whose_name_is_read_only_in_a_test_is_stale"),
    c("file-mismatch-not-reported", '        elif e["file"] not in {f for f, _, _ in reads[name]}:', "        elif False:",
      "test_an_entry_whose_file_does_not_read_the_name_is_a_mismatch", "test_an_entry_for_a_name_read_in_another_crate_is_a_mismatch"),
    c("only-the-first-read-file-is-accepted", '        elif e["file"] not in {f for f, _, _ in reads[name]}:', '        elif e["file"] != reads[name][0][0]:',
      "test_an_entry_may_name_any_file_that_reads_the_name", "test_an_entry_may_name_the_file_in_another_crate"),
    c("class-unchecked", '    if e.get("class") not in CLASSES:', "    if False:",
      "test_an_unknown_class_is_a_bad_entry", "test_a_missing_class_is_a_bad_entry"),
    c("guest-flag-type-unchecked", '    if not isinstance(e.get("changes_guest_served"), bool):', "    if False:",
      "test_a_guest_flag_that_is_not_a_boolean_is_a_bad_entry", "test_a_missing_guest_flag_is_a_bad_entry"),
    c("why-newlines-allowed", '    if not isinstance(why, str) or "\\n" in why or len(why.split()) < 3:', "    if not isinstance(why, str) or len(why.split()) < 3:",
      "test_a_why_with_a_newline_is_a_bad_entry", "test_a_why_that_spans_a_blank_line_is_a_bad_entry"),
    c("why-length-unchecked", "len(why.split()) < 3", "len(why.split()) < 1",
      "test_a_why_of_one_or_two_words_is_a_bad_entry", "test_a_one_word_why_is_a_bad_entry"),
    c("proposal-not-required", '    if is_offender(e) and e.get("proposal") not in PROPOSALS:', "    if False:",
      "test_an_offender_without_a_proposal_is_a_bad_entry", "test_an_unknown_proposal_is_a_bad_entry"),
    c("duplicate-names-allowed", '        if e["name"] in entries:', "        if False:",
      "test_a_name_listed_twice_is_a_bad_entry", "test_a_name_listed_twice_with_different_classes_is_a_bad_entry"),
    c("bad-entries-also-reported-as-unregistered", '                invalid.add(e["name"])', "                pass",
      "test_an_unknown_class_is_a_bad_entry", "test_a_why_with_a_newline_is_a_bad_entry"),
    c("new-offenders-not-reported", "    for name in sorted(offenders - base):", "    for name in []:",
      "test_a_guest_affecting_entry_that_is_not_on_the_baseline_is_a_new_offender", "test_a_product_class_entry_that_is_not_on_the_baseline_is_a_new_offender"),
    c("baseline-does-not-tolerate", "    for name in sorted(offenders - base):", "    for name in sorted(offenders):",
      "test_an_offender_on_the_baseline_is_tolerated", "test_an_offender_on_the_baseline_exits_zero"),
    c("two-new-offenders-reduced-to-one", "    for name in sorted(offenders - base):\n        e = entries[name]", "    for name in sorted(offenders - base)[:1]:\n        e = entries[name]",
      "test_two_new_offenders_are_both_reported", "test_only_the_names_missing_from_the_baseline_are_new"),
    c("product-class-is-not-an-offender", '    return entry.get("class") == "product" or entry.get("changes_guest_served") is True',
      '    return entry.get("changes_guest_served") is True',
      "test_a_product_class_entry_that_is_not_on_the_baseline_is_a_new_offender", "test_recording_includes_a_product_class_entry"),
    c("guest-effect-is-not-an-offence", 'or entry.get("changes_guest_served") is True', "or False",
      "test_a_guest_affecting_entry_that_is_not_on_the_baseline_is_a_new_offender", "test_recording_creates_the_baseline_from_the_offenders"),
    c("allowlist-ignored", "if is_offender(e) and n not in allow}\n    base_known", "if is_offender(e)}\n    base_known",
      "test_an_offender_on_the_allowlist_needs_no_baseline_entry", "test_a_baseline_name_that_is_now_allowlisted_is_stale"),
    c("allowlist-ignored-by-the-recorder", 'offenders = {e["name"] for e in reg["entries"] if is_offender(e) and e["name"] not in allow}',
      'offenders = {e["name"] for e in reg["entries"] if is_offender(e)}',
      "test_recording_leaves_allowlisted_names_out", "test_recording_leaves_every_allowlisted_name_out"),
    c("stale-baseline-not-reported", "    for name in sorted(base_known - offenders):", "    for name in []:",
      "test_a_baseline_name_that_is_no_longer_an_offender_is_stale", "test_a_baseline_name_that_is_gone_from_the_registry_is_stale"),
    c("unknown-allowlist-names-not-reported", '        if name not in entries and name not in invalid:\n            out.append(("BAD_ALLOWLIST"',
      '        if False:\n            out.append(("BAD_ALLOWLIST"',
      "test_an_allowlisted_name_the_registry_lacks_is_a_finding", "test_an_allowlist_may_name_several_unknown_names"),
    c("missing-registry-ignored", '        return [("NO_REGISTRY", REGISTRY, "the registry is missing")]', "        return []",
      "test_a_missing_registry_is_a_finding", "test_a_missing_registry_exits_one"),
    c("recorder-adds-names", "    if path.exists():\n        offenders &=", "    if False:\n        offenders &=",
      "test_recording_never_adds_a_name_to_an_existing_baseline", "test_recording_never_adds_even_when_the_baseline_is_empty"),
    c("recorder-keeps-resolved-names", "        offenders &= set(", "        offenders |= set(",
      "test_recording_removes_a_name_that_is_no_longer_an_offender", "test_recording_never_adds_a_name_to_an_existing_baseline"),
    c("report-guest-counts-lost", '"| %s | %d | %d |" % (cls, len(rows), sum(1 for e in rows if e["changes_guest_served"]))',
      '"| %s | %d | %d |" % (cls, len(rows), 0)',
      "test_the_report_counts_names_and_guest_effects_per_class", "test_the_report_counts_guest_effects_in_every_class"),
    c("report-lists-every-name", "for e in sorted((e for e in entries if is_offender(e))", "for e in sorted((e for e in entries if True)",
      "test_the_report_leaves_out_names_that_do_not_change_the_guest", "test_the_offender_table_has_one_row_per_offender"),
    c("report-drops-empty-classes", 'for cls in CLASSES:\n        rows', "for cls in ('debug', 'measurement'):\n        rows",
      "test_a_class_with_no_names_is_listed_with_zero", "test_the_report_lists_all_six_classes"),
    c("findings-exit-zero", "    return 1 if found else 0", "    return 0",
      "test_exit_one_with_a_named_line_when_not", "test_exit_one_for_a_stale_entry"),
]


def main():
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument("--jobs", type=int, default=4)
    ap.add_argument("only", nargs="?", default="")
    a = ap.parse_args()
    if not 1 <= a.jobs <= 8:
        ap.error("--jobs is 1 through 8")
    return KM.run_mutants(MUTANTS, "env_allowlist", a.jobs, a.only,
                          extra=["scripts/ci/env_allowlist.py", "scripts/ci/test_env_allowlist.py"])


if __name__ == "__main__":
    sys.exit(main())
