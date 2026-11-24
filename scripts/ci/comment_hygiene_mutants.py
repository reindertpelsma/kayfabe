#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later
"""Mutation checks of scripts/ci/comment_hygiene.py (STATUS: LIVE, 2026-11-20): every mutant is ONE exact-once text replacement in a
private copy; its two named killers (tests of scripts/ci/test_comment_hygiene.py) must both fail on the copy. The unmutated copy must
pass every named killer first. The runner is the one of kf_trust_mutants.py. GPU-free, stdlib only.
Run: python3 scripts/ci/comment_hygiene_mutants.py [--jobs N] [name-substring]"""
import argparse
from pathlib import Path
import re
import sys

HERE = Path(__file__).resolve().parent
sys.path.insert(0, str(HERE))
import kf_trust_mutants as KM  # noqa: E402

TARGET = "scripts/ci/comment_hygiene.py"
TESTS = (HERE / "test_comment_hygiene.py").read_text()


def k(name):
    """The id of a test of test_comment_hygiene.py by its method name (the class it sits in is looked up)."""
    cls = None
    for line in TESTS.splitlines():
        m = re.match(r"class (\w+)\(", line)
        if m:
            cls = m.group(1)
        if line.strip().startswith("def %s(" % name):
            return "test_comment_hygiene.%s.%s" % (cls, name)
    raise SystemExit("no test named %s in test_comment_hygiene.py" % name)


def c(name, old, new, k1, k2):
    return name, TARGET, old, new, [k(k1), k(k2)]


def disabled(name, head, k1, k2):
    """A pattern that can no longer match: `(?!)` is placed where its regex begins."""
    assert head.endswith('r"'), head
    return c(name, head, head + "(?!)", k1, k2)


SCOPE = "test_the_scope_leaves_out_documents_evidence_and_frozen_code"
Q = r'''    quotes = "\"`'" if lang == "js" else "\""'''

MUTANTS = [
    # ---- extraction: C, Rust, CUDA, JavaScript ----
    c("line-comments-not-extracted", '        elif text.startswith("//", i):', "        elif False:",
      "test_line_doc_and_inner_doc_comments_are_comments", "test_a_slash_slash_inside_a_string_is_not_a_comment"),
    c("line-comment-text-keeps-the-slashes", "            out.append((line, text[i + 2:j]))", "            out.append((line, text[i:j]))",
      "test_line_doc_and_inner_doc_comments_are_comments", "test_a_slash_slash_inside_a_string_is_not_a_comment"),
    c("block-comments-not-extracted", '        elif text.startswith("/*", i):', "        elif False:",
      "test_a_block_comment_gives_one_entry_per_line_with_its_line_number", "test_an_empty_block_comment_is_one_empty_entry"),
    c("block-comment-line-numbers-lost", "                out.append((line + k, part))", "                out.append((line, part))",
      "test_a_block_comment_gives_one_entry_per_line_with_its_line_number", "test_an_unterminated_block_comment_runs_to_the_end"),
    c("lines-after-a-block-comment-miscounted", '            line += text.count("\\n", i, j)\n', "",
      "test_a_block_comment_gives_one_entry_per_line_with_its_line_number", "test_a_comment_after_a_multiline_block_comment_has_the_right_line"),
    c("rust-block-comments-do-not-nest", '                if lang == "rust" and text.startswith("/*", j):', "                if False:",
      "test_rust_block_comments_nest", "test_the_language_follows_the_suffix"),
    c("c-block-comments-nest", '                if lang == "rust" and text.startswith("/*", j):', '                if text.startswith("/*", j):',
      "test_a_c_block_comment_does_not_nest", "test_a_js_block_comment_does_not_nest"),
    c("unterminated-block-comment-loses-its-tail", "            end = max(j - 2, i + 2) if depth == 0 else n", "            end = max(j - 2, i + 2)",
      "test_an_unterminated_block_comment_runs_to_the_end", "test_an_unterminated_rust_block_comment_with_a_nested_open_runs_to_the_end"),
    c("strings-not-skipped", "        elif c in quotes:", "        elif False:",
      "test_a_slash_slash_inside_a_string_is_not_a_comment", "test_a_block_start_inside_a_string_is_not_a_comment"),
    c("char-literals-not-skipped", "            i = m.end() if m else i + 1", "            i += 1",
      "test_a_double_quote_char_literal_does_not_open_a_string", "test_a_double_quote_char_literal_in_c_does_not_swallow_a_comment"),
    c("rust-strings-end-at-a-newline", '                    if c != "`" and lang != "rust":', '                    if c != "`":',
      "test_a_rust_string_may_span_lines_and_its_slashes_are_not_comments", "test_a_multiline_rust_string_keeps_the_line_count"),
    c("c-strings-span-lines", '                    if c != "`" and lang != "rust":', "                    if False:",
      "test_a_c_string_ends_at_a_newline", "test_an_unterminated_js_string_ends_at_the_line"),
    c("string-newlines-not-counted", "                    line += 1\n                elif text[j] ==", "                    pass\n                elif text[j] ==",
      "test_a_multiline_rust_string_keeps_the_line_count", "test_a_rust_string_may_span_lines_and_its_slashes_are_not_comments"),
    c("continued-string-lines-not-counted", 'text[j + 1:j + 2] == "\\n":\n                    line += 1', 'text[j + 1:j + 2] == "\\n":\n                    pass',
      "test_a_backslash_continued_string_keeps_the_line_count", "test_a_continued_js_string_keeps_the_line_count"),
    c("js-strings-use-double-quotes-only", Q, '    quotes = "\\""',
      "test_a_js_single_quoted_string_with_slashes_is_not_a_comment", "test_a_js_template_literal_may_span_lines"),
    c("single-quotes-start-strings-everywhere", Q, '    quotes = "\\"`\'"',
      "test_a_rust_lifetime_does_not_swallow_a_comment", "test_two_rust_lifetimes_on_a_line_do_not_swallow_a_comment"),
    # ---- extraction: Python, shell, PowerShell ----
    c("python-comments-not-found", "if t.type == tokenize.COMMENT]", "if False]",
      "test_a_hash_comment_is_found_in_python", "test_the_language_follows_the_suffix"),
    c("python-comment-keeps-the-hash", "(t.start[0], t.string[1:])", "(t.start[0], t.string)",
      "test_a_python_comment_keeps_its_text_without_the_hash", "test_a_hash_comment_is_found_in_python"),
    c("python-fallback-dropped", "SyntaxError):\n        return [(i + 1, l.split", "SyntaxError):\n        return [] and [(i + 1, l.split",
      "test_a_python_file_that_does_not_tokenize_falls_back_to_leading_hash_lines", "test_a_python_fallback_reads_only_leading_hash_lines"),
    c("python-fallback-reads-trailing-hashes", 'if "#" in l and l.lstrip().startswith("#")]', 'if "#" in l]',
      "test_a_python_fallback_reads_only_leading_hash_lines", "test_a_python_file_that_does_not_tokenize_falls_back_to_leading_hash_lines"),
    c("shell-comments-not-found", '            elif c == "#" and (k == 0', '            elif False and (k == 0',
      "test_a_full_line_shell_comment", "test_a_trailing_shell_comment_after_whitespace"),
    c("shell-quotes-ignored", '            elif c in "\\"\'":\n                quote = c', "            elif False:\n                quote = c",
      "test_a_hash_in_shell_quotes_is_not_a_comment", "test_an_escaped_quote_in_shell_does_not_end_the_string"),
    c("shell-escapes-ignored", "                if c == \"\\\\\" and quote == '\"' and not powershell:", "                if False:",
      "test_an_escaped_quote_in_shell_does_not_end_the_string", "test_two_escaped_quotes_in_a_row_in_shell"),
    c("hash-after-any-character-is-a-comment", '(k == 0 or raw[k - 1] in " \\t;&|(" )', "True",
      "test_a_hash_inside_a_word_is_not_a_comment", "test_dollar_hash_and_length_expansion_are_not_comments"),
    c("hash-needs-whitespace-before-it", 'in " \\t;&|(" )', 'in " \\t" )',
      "test_a_hash_after_a_semicolon_is_a_comment", "test_a_hash_after_an_ampersand_is_a_comment"),
    c("shebang-is-a-comment", ' and not (no == 1 and raw.startswith("#!"))', "",
      "test_a_shebang_is_not_a_comment_but_a_later_hash_bang_line_is", "test_a_shebang_line_alone_is_no_comment"),
    c("powershell-block-comments-not-found", '            elif powershell and raw.startswith("<#", k):', "            elif False:",
      "test_a_powershell_block_comment_spans_lines", "test_a_one_line_powershell_block_comment"),
    c("powershell-block-state-lost", "                    in_block = True", "                    in_block = False",
      "test_a_powershell_block_comment_spans_lines", "test_a_three_line_powershell_block_comment"),
    c("powershell-backtick-escape-ignored", "                elif c == \"`\" and powershell and quote == '\"':", "                elif False:",
      "test_a_backtick_escaped_quote_in_powershell_does_not_end_the_string", "test_two_backtick_escapes_in_powershell"),
    c("powershell-files-read-as-shell", 'powershell=path.endswith(".ps1")', "powershell=False",
      "test_a_powershell_file_block_comment_is_scanned_by_the_sweep", "test_the_language_follows_the_suffix"),
    # ---- scope ----
    c("docs-scanned", 'SKIP_TOP = ("docs/", "archive/"', 'SKIP_TOP = ("archive/"', "test_docs_are_not_scanned", SCOPE),
    c("archive-scanned", '"archive/", "third_party/", "traces/"', '"third_party/", "traces/"',
      "test_the_archive_and_third_party_are_not_scanned", SCOPE),
    c("third-party-scanned", '"third_party/", "traces/"', '"traces/"', "test_the_archive_and_third_party_are_not_scanned", SCOPE),
    c("traces-scanned", '"traces/", "site/"', '"site/"', "test_traces_are_not_scanned", SCOPE),
    c("site-scanned", '"site/", "crates/kayfabe-")', '"crates/kayfabe-")', "test_the_public_site_is_not_scanned", SCOPE),
    c("frozen-crates-scanned", '"site/", "crates/kayfabe-")', '"site/")', "test_the_frozen_crates_are_not_scanned", SCOPE),
    c("every-crates-directory-skipped", '"site/", "crates/kayfabe-")', '"site/", "crates/")',
      "test_a_kf_crate_is_scanned", "test_tests_and_scripts_are_in_scope"),
    c("powershell-files-not-scanned", '".sh": "shell", ".ps1": "powershell"}', '".sh": "shell"}',
      "test_tests_and_scripts_are_in_scope", "test_a_script_comment_is_scanned_too"),
    c("untracked-files-scanned", '["git", "ls-files", "-z"]', '["git", "ls-files", "-z", "-o"]',
      "test_an_untracked_file_is_not_scanned", "test_tests_and_scripts_are_in_scope"),
    # ---- patterns: each one disabled in turn (two positive tests each), then its false-positive guard ----
    disabled("owner-ruling-disabled", '("owner-ruling", re.compile(r"',
             "test_owner_ruling_is_found", "test_owner_ruled_and_decided_are_found"),
    disabled("the-owner-first-form-disabled", '("the-owner", re.compile(\n        r"',
             "test_the_owner_asked_is_found", "test_the_owners_earlier_formula_is_found"),
    c("the-owner-second-form-disabled", '        r"|\\b(?:awaits?|asks?|asked|pending|until|per|unless) the owner', '        r"|(?!)\\b(?:awaits?|asks?|asked|pending|until|per|unless) the owner',
      "test_pending_the_owner_is_found", "test_awaiting_the_owner_is_found"),
    c("the-owner-guard-dropped", "(?!\\s+(?:of|must|thread|lock|guard|handle|holds|that|is|has|may|can))", "",
      "test_the_owner_of_a_resource_is_not_a_note", "test_the_owner_thread_and_lock_are_not_notes"),
    c("owner-wants-is-a-note", "|said|asked|approved|go\\b|intent|statement)", "|said|asked|wants?|approved|go\\b|intent|statement)",
      "test_a_selection_owner_that_wants_data_is_not_a_note", "test_the_selection_owner_wants_the_data_is_not_a_note"),
    disabled("owner-dated-disabled", '("owner-dated", re.compile(r"',
             "test_owner_with_a_date_is_found", "test_owner_colon_date_is_found"),
    disabled("owner-rulings-doc-disabled", '("owner-rulings-doc", re.compile(r"',
             "test_the_rulings_document_is_found", "test_the_rulings_document_with_a_section_sign_is_found"),
    disabled("owner-todo-disabled", '("owner-todo", re.compile(r"',
             "test_a_todo_for_the_owner_is_found", "test_a_fixme_for_the_owner_is_found"),
    c("agent-model-names-disabled", 'r"\\b(?:Claude|Codex', 'r"(?!)\\b(?:Claude|Codex',
      "test_a_model_name_is_found", "test_a_codex_or_opus_name_is_found"),
    c("agent-process-agents-disabled", "|\\b(?:another|other|parallel", "|(?!)\\b(?:another|other|parallel",
      "test_another_agent_is_found", "test_a_sibling_agent_is_found"),
    c("every-agent-is-a-note", "|\\b(?:another|other|parallel", "|\\b(?:\\w+|other|parallel",
      "test_the_guest_agent_and_user_agent_are_not_notes", "test_the_ssh_agent_and_the_qemu_guest_agent_are_not_notes"),
    disabled("agent-instructions-file-disabled", '("agent-instructions-file", re.compile(r"',
             "test_the_agent_instruction_files_are_found", "test_agents_md_is_found"),
    c("session-phrases-disabled", 'r"\\b(?:chat|agent|Claude|AI|coding) sessions?', 'r"(?!)\\b(?:chat|agent|Claude|AI|coding) sessions?',
      "test_an_agent_session_is_found", "test_a_chat_session_is_found"),
    c("handoff-document-disabled", "|\\bSTATUS_AND_HANDOFF\\b", "|(?!)\\bSTATUS_AND_HANDOFF\\b",
      "test_the_handoff_document_is_found", "test_the_handoff_document_with_its_markdown_name_is_found"),
    c("every-session-is-a-note", "\\b(?:chat|agent|Claude|AI|coding) sessions?", "\\b(?:\\w+) sessions?",
      "test_an_rm_session_is_not_a_note", "test_a_new_session_of_the_broker_is_not_a_note"),
    c("every-handoff-is-a-note", "|\\bSTATUS_AND_HANDOFF\\b", "|\\bhandoff\\b",
      "test_a_handoff_between_threads_is_not_a_note", "test_a_handoff_without_a_tick_is_not_a_note"),
    c("coordinator-ruling-disabled", 're.compile(r"\\bcoordinator(?:\'s)?\\s+(?:ruling', 're.compile(r"(?!)\\bcoordinator(?:\'s)?\\s+(?:ruling',
      "test_a_coordinator_ruling_is_found", "test_the_coordinators_decision_is_found"),
    c("parenthesised-coordinator-disabled", "|\\(coordinator\\b|", "|(?!)\\(coordinator\\b|",
      "test_a_parenthesised_coordinator_note_is_found", "test_a_parenthesised_coordinator_without_a_comma_is_found"),
    c("coordinator-colon-disabled", "|\\bcoordinator:\"", "|(?!)\\bcoordinator:\"",
      "test_coordinator_colon_is_found", "test_a_second_coordinator_colon_is_found"),
    c("every-coordinator-is-a-note", 'r"\\bcoordinator(?:\'s)?\\s+(?:ruling|decision|first step|rule\\b|asked|said|wants)|\\(coordinator\\b|\\bcoordinator:"',
      'r"\\bcoordinator\\b"',
      "test_a_coordinator_thread_is_not_a_note", "test_a_coordinator_that_never_parked_is_not_a_note"),
    c("as-discussed-form-disabled", 'r"\\bas (?:discussed|agreed)', 'r"(?!)\\bas (?:discussed|agreed)',
      "test_as_discussed_is_found", "test_as_agreed_is_found"),
    c("we-agreed-form-disabled", "|\\bwe (?:agreed|discussed|decided)", "|(?!)\\bwe (?:agreed|discussed|decided)",
      "test_we_agreed_is_found", "test_we_decided_is_found"),
    disabled("added-dated-disabled", '("added-dated", re.compile(r"',
             "test_added_with_a_date_is_found", "test_corrected_with_a_date_is_found"),
    c("added-dated-needs-no-date", "(?:\\w+\\s+)?20\\d\\d-\\d\\d-\\d\\d\")),\n    (\"since-dated\"", "(?:\\w+\\s+)?(?:20\\d\\d-\\d\\d-\\d\\d)?\")),\n    (\"since-dated\"",
      "test_a_verb_without_a_date_is_not_added_dated", "test_changed_without_a_date_is_not_added_dated"),
    disabled("since-dated-disabled", '("since-dated", re.compile(r"',
             "test_since_a_date_is_found", "test_as_of_a_date_is_found"),
    disabled("dated-annotation-disabled", '("dated-annotation", re.compile(r"',
             "test_a_star_marker_with_a_date_is_found", "test_a_comment_that_opens_with_a_date_is_found"),
    c("dated-annotation-anchor-dropped", '("dated-annotation", re.compile(r"^[', '("dated-annotation", re.compile(r"[',
      "test_a_date_in_the_middle_of_a_sentence_is_not_an_annotation", "test_a_fuzz_target_citation_with_a_date_is_not_an_annotation"),
    c("dated-annotation-markers-dropped", "\\u2605\\u2298\\u26a0]*", "]*",
      "test_a_star_marker_with_a_date_is_found", "test_a_marker_a_word_and_a_date_is_found"),
    c("dated-annotation-words-dropped", "(?:[A-Z][\\w-]*\\s+){0,2}20", "(?:[A-Z][\\w-]*\\s+){0,0}20",
      "test_a_marker_a_word_and_a_date_is_found", "test_two_capitalised_words_before_a_date_are_found"),
    disabled("lane-label-disabled", '("lane-label", re.compile(r"',
             "test_a_lane_label_is_found", "test_a_lane_label_with_a_digit_is_found"),
    c("every-lane-is-a-note", 'r"\\bLane [A-Z0-9]\\b"', 'r"\\b[Ll]ane\\b"',
      "test_a_lane_word_without_a_label_is_not_a_note", "test_a_capitalised_lane_word_without_a_label_is_not_a_note"),
]


def main():
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument("--jobs", type=int, default=4)
    ap.add_argument("only", nargs="?", default="")
    a = ap.parse_args()
    if not 1 <= a.jobs <= 8:
        ap.error("--jobs is 1 through 8")
    return KM.run_mutants(MUTANTS, "comment_hygiene", a.jobs, a.only,
                          extra=["scripts/ci/comment_hygiene.py", "scripts/ci/test_comment_hygiene.py"])


if __name__ == "__main__":
    sys.exit(main())
