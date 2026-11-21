#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later
"""GPU-free tests of scripts/ci/comment_hygiene.py (STATUS: LIVE, 2026-11-20) on fixture trees in a scratch directory.
The names are the killers named by scripts/ci/comment_hygiene_mutants.py: renaming a test is a change of that file."""
import json
from pathlib import Path
import subprocess
import sys
import tempfile
import unittest

HERE = Path(__file__).resolve().parent
sys.path.insert(0, str(HERE))
import comment_hygiene as H  # noqa: E402


def c_like(text, lang="c"):
    return H.comments_c_like(text, lang)


class Extract(unittest.TestCase):
    def test_line_doc_and_inner_doc_comments_are_comments(self):
        got = c_like("// a\n/// b\n//! c\nfn f() {}\n", "rust")
        self.assertEqual(got, [(1, " a"), (2, "/ b"), (3, "! c")])

    def test_a_block_comment_gives_one_entry_per_line_with_its_line_number(self):
        got = c_like("int x;\n/* one\n   two\n   three */\nint y; // after\n")
        self.assertEqual(got, [(2, " one"), (3, "   two"), (4, "   three "), (5, " after")])

    def test_a_slash_slash_inside_a_string_is_not_a_comment(self):
        self.assertEqual(c_like('const char *u = "http://x"; // real\n'), [(1, " real")])

    def test_a_block_start_inside_a_string_is_not_a_comment(self):
        self.assertEqual(c_like('char *s = "a /* b"; int y; // real\n'), [(1, " real")])

    def test_a_double_quote_char_literal_does_not_open_a_string(self):
        self.assertEqual(c_like("let q = '\"'; // real\n", "rust"), [(1, " real")])

    def test_a_rust_lifetime_does_not_swallow_a_comment(self):
        self.assertEqual(c_like("fn f<'a>(x: &'a str, y: &'b u8) { // it's real\n}\n", "rust"), [(1, " it's real")])

    def test_a_rust_string_may_span_lines_and_its_slashes_are_not_comments(self):
        self.assertEqual(c_like('let s = "line one\n// not a comment\nend"; // real\n', "rust"), [(3, " real")])

    def test_a_c_string_ends_at_a_newline(self):
        self.assertEqual(c_like('char *s = "unterminated\nint y; // real\n'), [(2, " real")])

    def test_a_backslash_continued_string_keeps_the_line_count(self):
        self.assertEqual(c_like('let s = "a \\\n b"; // real\n', "rust"), [(2, " real")])

    def test_a_multiline_rust_string_keeps_the_line_count(self):
        self.assertEqual(c_like('let s = "a\nb\nc"; // real\n', "rust"), [(3, " real")])

    def test_rust_block_comments_nest(self):
        self.assertEqual(c_like("/* a /* b */ still comment */ code // real\n", "rust"),
                         [(1, " a /* b */ still comment "), (1, " real")])

    def test_a_c_block_comment_does_not_nest(self):
        self.assertEqual(c_like("/* a /* b */ code // real\n"), [(1, " a /* b "), (1, " real")])

    def test_an_unterminated_block_comment_runs_to_the_end(self):
        self.assertEqual(c_like("/* a\nb"), [(1, " a"), (2, "b")])

    def test_an_empty_block_comment_is_one_empty_entry(self):
        self.assertEqual(c_like("/**/ int x; // real\n"), [(1, ""), (1, " real")])

    def test_a_js_single_quoted_string_with_slashes_is_not_a_comment(self):
        self.assertEqual(c_like("const u = 'http://x'; // real\n", "js"), [(1, " real")])

    def test_a_js_template_literal_may_span_lines(self):
        self.assertEqual(c_like("const t = `a\n// inside\nb`; // real\n", "js"), [(3, " real")])

    def test_a_lone_single_quote_in_c_does_not_start_a_string(self):
        self.assertEqual(c_like("int x = 1; // don't\nint y; // real\n"), [(1, " don't"), (2, " real")])

    def test_a_char_literal_with_an_escape_is_skipped_whole(self):
        self.assertEqual(c_like("char c = '\\''; // real\n"), [(1, " real")])

    def test_a_hash_comment_is_found_in_python(self):
        self.assertEqual(H.comments_python("x = 1  # one\n# two\n"), [(1, " one"), (2, " two")])

    def test_a_hash_in_a_python_string_is_not_a_comment(self):
        self.assertEqual(H.comments_python('x = "a # b"\n'), [])

    def test_a_python_docstring_is_not_a_comment(self):
        self.assertEqual(H.comments_python('"""owner ruling # here"""\nx = 1\n'), [])

    def test_a_python_file_that_does_not_tokenize_falls_back_to_leading_hash_lines(self):
        got = H.comments_python('x = ( # trailing\n# only this\n  "unterminated\n')
        self.assertEqual(got, [(2, " only this")])

    def test_a_full_line_shell_comment(self):
        self.assertEqual(H.comments_shell("# a\necho x\n"), [(1, " a")])

    def test_a_trailing_shell_comment_after_whitespace(self):
        self.assertEqual(H.comments_shell("echo x  # b\n"), [(1, " b")])

    def test_a_hash_in_shell_quotes_is_not_a_comment(self):
        self.assertEqual(H.comments_shell("echo \"a # b\" 'c # d'\n"), [])

    def test_dollar_hash_and_length_expansion_are_not_comments(self):
        self.assertEqual(H.comments_shell('n=$#; m=${#x}\n'), [])

    def test_a_shebang_is_not_a_comment_but_a_later_hash_bang_line_is(self):
        self.assertEqual(H.comments_shell("#!/bin/sh\n#!not a shebang\n"), [(2, "!not a shebang")])

    def test_a_hash_inside_a_word_is_not_a_comment(self):
        self.assertEqual(H.comments_shell("echo a#b\n"), [])

    def test_a_hash_after_an_ampersand_is_a_comment(self):
        self.assertEqual(H.comments_shell("ls &# c\n"), [(1, " c")])

    def test_a_hash_after_a_semicolon_is_a_comment(self):
        self.assertEqual(H.comments_shell("true;# c\n"), [(1, " c")])

    def test_an_escaped_quote_in_shell_does_not_end_the_string(self):
        self.assertEqual(H.comments_shell('echo "a \\" # b" # real\n'), [(1, " real")])

    def test_a_powershell_block_comment_spans_lines(self):
        self.assertEqual(H.comments_shell("<# one\ntwo #>\nGet-Item # three\n", powershell=True), [(1, " one"), (2, "two "), (3, " three")])

    def test_a_one_line_powershell_block_comment(self):
        self.assertEqual(H.comments_shell("$x = 1 <# a #> ; $y # b\n", powershell=True), [(1, " a "), (1, " b")])

    def test_a_backtick_escaped_quote_in_powershell_does_not_end_the_string(self):
        self.assertEqual(H.comments_shell('Write-Host "a `" # b" # real\n', powershell=True), [(1, " real")])

    def test_the_language_follows_the_suffix(self):
        self.assertEqual(H.comments_of("a.py", "# p\n"), [(1, " p")])
        self.assertEqual(H.comments_of("a.sh", "# s\n"), [(1, " s")])
        self.assertEqual(H.comments_of("a.ps1", "<# q #>\n"), [(1, " q ")])
        self.assertEqual(H.comments_of("a.rs", "/* r /* nested */ */\n"), [(1, " r /* nested */ ")])
        self.assertEqual(H.comments_of("a.js", "x = 'a//b'; // j\n"), [(1, " j")])
        self.assertEqual(H.comments_of("a.cu", "int x; // k\n"), [(1, " k")])

    def test_a_comment_after_a_multiline_block_comment_has_the_right_line(self):
        self.assertEqual(c_like("/* a\nb */\n// real\n"), [(1, " a"), (2, "b "), (3, " real")])

    def test_a_js_block_comment_does_not_nest(self):
        self.assertEqual(c_like("/* a /* b */ x // real\n", "js"), [(1, " a /* b "), (1, " real")])

    def test_a_double_quote_char_literal_in_c_does_not_swallow_a_comment(self):
        self.assertEqual(c_like("char c = '\"'; // real \"x\"\n"), [(1, ' real "x"')])

    def test_an_unterminated_js_string_ends_at_the_line(self):
        self.assertEqual(c_like('const s = "oops\n// real\n', "js"), [(2, " real")])

    def test_a_continued_js_string_keeps_the_line_count(self):
        self.assertEqual(c_like("const s = 'a \\\n b'; // real\n", "js"), [(2, " real")])

    def test_two_rust_lifetimes_on_a_line_do_not_swallow_a_comment(self):
        self.assertEqual(c_like("fn f<'a>(x: &'a u8, y: &'b u8) { // real\n}\n", "rust"), [(1, " real")])

    def test_an_unterminated_rust_block_comment_with_a_nested_open_runs_to_the_end(self):
        self.assertEqual(c_like("/* a /* b\nc", "rust"), [(1, " a /* b"), (2, "c")])

    def test_a_python_fallback_reads_only_leading_hash_lines(self):
        self.assertEqual(H.comments_python('x = ( # trailing\n  "unterminated\n# b\n'), [(3, " b")])

    def test_two_escaped_quotes_in_a_row_in_shell(self):
        self.assertEqual(H.comments_shell('echo "a \\"" # real\n'), [(1, " real")])

    def test_a_shebang_line_alone_is_no_comment(self):
        self.assertEqual(H.comments_shell("#!/bin/sh\n"), [])

    def test_a_three_line_powershell_block_comment(self):
        self.assertEqual(H.comments_shell("<# a\nb\nc #>\n$x\n", powershell=True), [(1, " a"), (2, "b"), (3, "c ")])

    def test_two_backtick_escapes_in_powershell(self):
        self.assertEqual(H.comments_shell('Write-Host "a `"" # real\n', powershell=True), [(1, " real")])

    def test_a_python_comment_keeps_its_text_without_the_hash(self):
        self.assertEqual(H.comments_python("#x\n"), [(1, "x")])


class Tree(unittest.TestCase):
    def setUp(self):
        self._t = tempfile.TemporaryDirectory()
        self.addCleanup(self._t.cleanup)
        self.root = Path(self._t.name)

    def write(self, rel, text):
        p = self.root / rel
        p.parent.mkdir(parents=True, exist_ok=True)
        p.write_text(text)

    def hits(self, comment, rel="crates/kf-x/src/lib.rs"):
        """The pattern names that flag `comment` written as a `//` comment (or a `#` comment for a script)."""
        marker = "#" if rel.endswith((".py", ".sh", ".ps1")) else "//"
        self.write(rel, "%s %s\n" % (marker, comment))
        return sorted({v["pattern"] for v in H.sweep(self.root, [rel]).values()})

    def git_init(self):
        for cmd in (["git", "init", "-q"], ["git", "add", "-A"]):
            subprocess.run(cmd, cwd=self.root, check=True, capture_output=True)


class Patterns(Tree):
    def test_owner_ruling_is_found(self):
        self.assertIn("owner-ruling", self.hits("Owner ruling, 2026-08-08: the answer is no"))

    def test_owner_ruled_and_decided_are_found(self):
        self.assertIn("owner-ruling", self.hits("the owner ruled that it stays"))
        self.assertIn("owner-ruling", self.hits("owner decided to drop it"))

    def test_the_owners_statement_is_found(self):
        self.assertIn("owner-ruling", self.hits("from the owner's statement of 3 May"))

    def test_the_owner_asked_is_found(self):
        self.assertIn("the-owner", self.hits("the owner asked for this behaviour"))

    def test_pending_the_owner_is_found(self):
        self.assertIn("the-owner", self.hits("parked, pending the owner"))

    def test_the_owner_of_a_resource_is_not_a_note(self):
        self.assertEqual(self.hits("borrows it until the owner of the buffer drops it"), [])

    def test_the_owner_thread_and_lock_are_not_notes(self):
        self.assertEqual(self.hits("pending the owner thread, which holds the lock"), [])
        self.assertEqual(self.hits("until the owner lock is released"), [])

    def test_a_selection_owner_that_wants_data_is_not_a_note(self):
        self.assertEqual(self.hits("INCR means the owner wants to stream it"), [])

    def test_the_selection_owner_wants_the_data_is_not_a_note(self):
        self.assertEqual(self.hits("the selection owner wants the data converted first"), [])

    def test_a_handoff_without_a_tick_is_not_a_note(self):
        self.assertEqual(self.hits("a handoff, no tick yet: the free run continues"), [])

    def test_a_capitalised_lane_word_without_a_label_is_not_a_note(self):
        self.assertEqual(self.hits("Lane count is a power of two"), [])

    def test_owner_with_a_date_is_found(self):
        self.assertIn("owner-dated", self.hits("sized for the box (owner, 2026-11-12)"))

    def test_owner_colon_date_is_found(self):
        self.assertIn("owner-dated", self.hits("Owner: 2026-09-08 said so"))

    def test_the_rulings_document_is_found(self):
        self.assertIn("owner-rulings-doc", self.hits("see OWNER_RULINGS section AQ"))

    def test_the_rulings_document_with_a_section_sign_is_found(self):
        self.assertIn("owner-rulings-doc", self.hits("(`OWNER_RULINGS.md` §V)"))

    def test_a_todo_for_the_owner_is_found(self):
        self.assertIn("owner-todo", self.hits("TODO(owner): decide"))

    def test_a_fixme_for_the_owner_is_found(self):
        self.assertIn("owner-todo", self.hits("FIXME (owner) later"))

    def test_a_model_name_is_found(self):
        self.assertIn("agent-or-model", self.hits("other Claude sessions run sweeps"))

    def test_a_codex_or_opus_name_is_found(self):
        self.assertIn("agent-or-model", self.hits("Opus review of the change"))

    def test_a_sibling_agent_is_found(self):
        self.assertIn("agent-or-model", self.hits("a sibling agent holds the lock"))

    def test_another_agent_is_found(self):
        self.assertIn("agent-or-model", self.hits("a net that another agent started"))

    def test_the_guest_agent_and_user_agent_are_not_notes(self):
        self.assertEqual(self.hits("the guest agent answered; send a User-Agent header"), [])

    def test_the_agent_instruction_files_are_found(self):
        self.assertIn("agent-instructions-file", self.hits("(CLAUDE.md, shell is glue)"))

    def test_agents_md_is_found(self):
        self.assertIn("agent-instructions-file", self.hits("recorded in AGENTS.md"))

    def test_an_agent_session_is_found(self):
        self.assertIn("session-or-handoff-note", self.hits("the agent session that found this"))

    def test_a_chat_session_is_found(self):
        self.assertIn("session-or-handoff-note", self.hits("lost with the chat session"))

    def test_the_handoff_document_with_its_markdown_name_is_found(self):
        self.assertIn("session-or-handoff-note", self.hits("`STATUS_AND_HANDOFF.md` records it"))

    def test_the_handoff_document_is_found(self):
        self.assertIn("session-or-handoff-note", self.hits("see STATUS_AND_HANDOFF.md section 4"))

    def test_an_rm_session_is_not_a_note(self):
        self.assertEqual(self.hits("an object this session allocated; one UVM session client per process"), [])

    def test_a_handoff_between_threads_is_not_a_note(self):
        self.assertEqual(self.hits("the handoff back happens ONCE"), [])

    def test_a_coordinator_ruling_is_found(self):
        self.assertIn("coordinator-note", self.hits("per the coordinator ruling that the default is ring"))

    def test_the_coordinators_decision_is_found(self):
        self.assertIn("coordinator-note", self.hits("the coordinator's decision for the frontend"))

    def test_a_parenthesised_coordinator_without_a_comma_is_found(self):
        self.assertIn("coordinator-note", self.hits("shutdown leaves the journal dirty (coordinator 2026-09-30: use ring)"))

    def test_coordinator_colon_is_found(self):
        self.assertIn("coordinator-note", self.hits("investigation; coordinator: iterate until Linux passes"))

    def test_a_second_coordinator_colon_is_found(self):
        self.assertIn("coordinator-note", self.hits("coordinator: skip this step"))

    def test_a_parenthesised_coordinator_note_is_found(self):
        self.assertIn("coordinator-note", self.hits("the version is cosmetic (coordinator, 2026-09-26)"))

    def test_a_coordinator_thread_is_not_a_note(self):
        self.assertEqual(self.hits("unbounded on a ThreadClass::Coordinator by design; a coordinator thread may park"), [])

    def test_as_discussed_is_found(self):
        self.assertIn("as-discussed", self.hits("as discussed, keep it"))

    def test_as_agreed_is_found(self):
        self.assertIn("as-discussed", self.hits("as agreed, keep it"))

    def test_we_decided_is_found(self):
        self.assertIn("as-discussed", self.hits("we decided to drop it"))

    def test_we_agreed_is_found(self):
        self.assertIn("as-discussed", self.hits("we agreed to keep it"))

    def test_added_with_a_date_is_found(self):
        self.assertIn("added-dated", self.hits("Added 2026-07-31 for the second architecture"))

    def test_corrected_with_a_date_is_found(self):
        self.assertIn("added-dated", self.hits("Corrected 2026-10-10 (later the same day) above the text"))

    def test_a_verb_without_a_date_is_not_added_dated(self):
        self.assertEqual(self.hits("Added for the second architecture"), [])

    def test_since_a_date_is_found(self):
        self.assertIn("since-dated", self.hits("the table assembled for (since 2026-09-26 the pin moved)"))

    def test_as_of_a_date_is_found(self):
        self.assertIn("since-dated", self.hits("kept (as of 2026-09-26)"))

    def test_a_star_marker_with_a_date_is_found(self):
        self.assertIn("dated-annotation", self.hits("★ 2026-11-14: `try_lock` (measurement only)"))

    def test_a_comment_that_opens_with_a_date_is_found(self):
        self.assertIn("dated-annotation", self.hits("2026-11-12: `rehearsal` is exactly true"))

    def test_a_marker_a_word_and_a_date_is_found(self):
        self.assertIn("dated-annotation", self.hits("⊘ CORRECTED 2026-10-08 (V3_DISPLAY.md): the launcher"))

    def test_a_date_in_the_middle_of_a_sentence_is_not_an_annotation(self):
        self.assertEqual(self.hits("the fuzz target found it on 2026-08-01 and reached the parser"), [])

    def test_a_status_line_with_a_date_is_not_an_annotation(self):
        self.assertEqual(self.hits("STATUS: LIVE, 2026-11-17 (docs/CI_V3.md)"), [])

    def test_a_marker_without_a_date_is_not_an_annotation(self):
        self.assertEqual(self.hits("★ the strongest form of the rule"), [])

    def test_a_lane_label_is_found(self):
        self.assertIn("lane-label", self.hits("Lane C: the host rows"))

    def test_a_lane_label_with_a_digit_is_found(self):
        self.assertIn("lane-label", self.hits("Lane 2 gate"))

    def test_a_lane_word_without_a_label_is_not_a_note(self):
        self.assertEqual(self.hits("the slow lane and a lane count"), [])

    def test_a_script_comment_is_scanned_too(self):
        self.assertIn("owner-ruling", self.hits("owner ruling 2026-10-24 item 4", "scripts/x.sh"))
        self.assertIn("owner-ruling", self.hits("owner ruling 2026-10-24 item 4", "scripts/x.py"))
        self.assertIn("owner-ruling", self.hits("owner ruling 2026-10-24 item 4", "scripts/x.ps1"))

    def test_code_that_is_not_a_comment_is_not_scanned(self):
        self.write("crates/kf-x/src/lib.rs", 'const S: &str = "owner ruling 2026-10-24";\nfn owner_ruling() {}\n')
        self.assertEqual(H.sweep(self.root, ["crates/kf-x/src/lib.rs"]), {})

    def test_the_owners_earlier_formula_is_found(self):
        self.assertIn("the-owner", self.hits("the owner's earlier formula was hash(id)"))

    def test_awaiting_the_owner_is_found(self):
        self.assertIn("the-owner", self.hits("this awaits the owner."))

    def test_the_ssh_agent_and_the_qemu_guest_agent_are_not_notes(self):
        self.assertEqual(self.hits("start the ssh agent, then ask the QEMU guest agent"), [])

    def test_a_new_session_of_the_broker_is_not_a_note(self):
        self.assertEqual(self.hits("a new session, whose window holds no frame"), [])

    def test_a_coordinator_that_never_parked_is_not_a_note(self):
        self.assertEqual(self.hits("a coordinator that was never idle"), [])

    def test_changed_without_a_date_is_not_added_dated(self):
        self.assertEqual(self.hits("Changed the order of the two steps"), [])

    def test_a_fuzz_target_citation_with_a_date_is_not_an_annotation(self):
        self.assertEqual(self.hits("Found by the `gsp_msgq` fuzz target (2026-08-01), which reaches it"), [])

    def test_two_capitalised_words_before_a_date_are_found(self):
        self.assertIn("dated-annotation", self.hits("Security Review 2026-11-14: the bound is checked first"))

    def test_a_powershell_file_block_comment_is_scanned_by_the_sweep(self):
        self.write("a.ps1", "<# owner ruling 2026-10-24: keep it #>\n")
        self.assertEqual({v["pattern"] for v in H.sweep(self.root, ["a.ps1"]).values()}, {"owner-ruling"})


class GateBase(Tree):
    NOTE = "// owner ruling 2026-10-24: keep it\nfn f() {}\n"

    def baseline(self, doc):
        self.write(H.BASELINE, json.dumps(doc))

    def recorded(self):
        return json.loads((self.root / H.BASELINE).read_text())

    def kinds(self, files=None):
        return sorted(k for k, _, _ in H.findings(self.root, files))


class Gate(GateBase):
    def test_a_new_flagged_comment_is_a_finding(self):
        self.write("crates/kf-x/src/lib.rs", self.NOTE)
        self.assertEqual(self.kinds(["crates/kf-x/src/lib.rs"]), ["NEW"])

    def test_a_finding_names_file_line_and_pattern(self):
        self.write("crates/kf-x/src/lib.rs", "fn g() {}\n" + self.NOTE)
        detail = H.findings(self.root, ["crates/kf-x/src/lib.rs"])[0][2]
        self.assertIn("crates/kf-x/src/lib.rs:2 [owner-ruling]", detail)

    def test_a_recorded_comment_is_tolerated(self):
        self.write("crates/kf-x/src/lib.rs", self.NOTE)
        self.baseline({k: "x" for k in H.sweep(self.root, ["crates/kf-x/src/lib.rs"])})
        self.assertEqual(self.kinds(["crates/kf-x/src/lib.rs"]), [])

    def test_a_recorded_comment_that_moved_to_another_line_is_still_tolerated(self):
        self.write("crates/kf-x/src/lib.rs", self.NOTE)
        self.baseline({k: "x" for k in H.sweep(self.root, ["crates/kf-x/src/lib.rs"])})
        self.write("crates/kf-x/src/lib.rs", "fn pad() {}\n\n\n" + self.NOTE)
        self.assertEqual(self.kinds(["crates/kf-x/src/lib.rs"]), [])

    def test_whitespace_changes_do_not_make_a_recorded_comment_new(self):
        self.write("crates/kf-x/src/lib.rs", "//   owner ruling 2026-10-24:   keep it\n")
        self.baseline({k: "x" for k in H.sweep(self.root, ["crates/kf-x/src/lib.rs"])})
        self.write("crates/kf-x/src/lib.rs", "// owner ruling 2026-10-24: keep it\n")
        self.assertEqual(self.kinds(["crates/kf-x/src/lib.rs"]), [])

    def test_a_recorded_comment_with_changed_words_is_new_and_the_old_one_stale(self):
        self.write("crates/kf-x/src/lib.rs", self.NOTE)
        self.baseline({k: "x" for k in H.sweep(self.root, ["crates/kf-x/src/lib.rs"])})
        self.write("crates/kf-x/src/lib.rs", "// owner ruling 2026-10-24: keep it, but louder\n")
        self.assertEqual(self.kinds(["crates/kf-x/src/lib.rs"]), ["NEW", "STALE_BASELINE"])

    def test_the_same_note_in_another_file_is_new(self):
        self.write("crates/kf-x/src/lib.rs", self.NOTE)
        self.baseline({k: "x" for k in H.sweep(self.root, ["crates/kf-x/src/lib.rs"])})
        self.write("crates/kf-x/src/other.rs", self.NOTE)
        self.assertEqual(self.kinds(["crates/kf-x/src/lib.rs", "crates/kf-x/src/other.rs"]), ["NEW"])

    def test_a_second_identical_note_in_the_same_file_is_new(self):
        self.write("crates/kf-x/src/lib.rs", self.NOTE)
        self.baseline({k: "x" for k in H.sweep(self.root, ["crates/kf-x/src/lib.rs"])})
        self.write("crates/kf-x/src/lib.rs", self.NOTE + self.NOTE)
        self.assertEqual(self.kinds(["crates/kf-x/src/lib.rs"]), ["NEW"])

    def test_removing_one_of_two_identical_notes_leaves_a_stale_entry(self):
        self.write("crates/kf-x/src/lib.rs", self.NOTE + self.NOTE)
        self.baseline({k: "x" for k in H.sweep(self.root, ["crates/kf-x/src/lib.rs"])})
        self.write("crates/kf-x/src/lib.rs", self.NOTE)
        self.assertEqual(self.kinds(["crates/kf-x/src/lib.rs"]), ["STALE_BASELINE"])

    def test_a_rewritten_comment_leaves_a_stale_entry_that_must_be_deleted(self):
        self.write("crates/kf-x/src/lib.rs", self.NOTE)
        self.baseline({k: "x" for k in H.sweep(self.root, ["crates/kf-x/src/lib.rs"])})
        self.write("crates/kf-x/src/lib.rs", "// keep it: the guest depends on it\n")
        self.assertEqual(self.kinds(["crates/kf-x/src/lib.rs"]), ["STALE_BASELINE"])

    def test_two_patterns_on_one_line_are_two_findings(self):
        self.write("crates/kf-x/src/lib.rs", "// owner ruling (OWNER_RULINGS section AQ)\n")
        self.assertEqual(self.kinds(["crates/kf-x/src/lib.rs"]), ["NEW", "NEW"])

    def test_the_scope_leaves_out_documents_evidence_and_frozen_code(self):
        for rel in ("docs/design/a.rs", "archive/crates/kayfabe-core/src/a.rs", "third_party/x/a.rs", "traces/run/a.py",
                    "site/a.js", "crates/kayfabe-doorbell/src/a.rs"):
            self.write(rel, self.NOTE if rel.endswith(".rs") else "// owner ruling 2026-10-24: x\n" if rel.endswith(".js") else "# owner ruling 2026-10-24: x\n")
        self.write("crates/kf-x/src/lib.rs", self.NOTE)
        self.git_init()
        self.assertEqual(H.tracked(self.root), ["crates/kf-x/src/lib.rs"])

    def scope(self, rel):
        self.write(rel, "x\n")
        self.write("kept.rs", "x\n")
        self.git_init()
        return H.tracked(self.root)

    def test_docs_are_not_scanned(self):
        self.assertEqual(self.scope("docs/design/a.rs"), ["kept.rs"])

    def test_traces_are_not_scanned(self):
        self.assertEqual(self.scope("traces/run/a.py"), ["kept.rs"])

    def test_the_frozen_crates_are_not_scanned(self):
        self.assertEqual(self.scope("crates/kayfabe-doorbell/src/a.rs"), ["kept.rs"])

    def test_a_kf_crate_is_scanned(self):
        self.assertEqual(self.scope("crates/kf-core/src/a.rs"), ["crates/kf-core/src/a.rs", "kept.rs"])

    def test_the_public_site_is_not_scanned(self):
        self.assertEqual(self.scope("site/a.js"), ["kept.rs"])

    def test_the_archive_and_third_party_are_not_scanned(self):
        self.assertEqual(self.scope("archive/a.rs"), ["kept.rs"])
        self.assertEqual(self.scope("third_party/a.rs"), ["kept.rs"])

    def test_tests_and_scripts_are_in_scope(self):
        for rel in ("crates/kf-x/tests/a.rs", "scripts/a.sh", "scripts/ci/a.py", "ci/vast-broker/src/a.js", "scripts/a.ps1", "broker/a.c", "broker/a.h",
                    "cuda/walk/a.cu", "ci/a.mjs"):
            self.write(rel, "x\n")
        self.git_init()
        self.assertEqual(len(H.tracked(self.root)), 9)

    def test_files_of_other_kinds_are_not_scanned(self):
        for rel in ("a.md", "a.json", "a.yml", "a.txt", "Makefile"):
            self.write(rel, "owner ruling\n")
        self.git_init()
        self.assertEqual(H.tracked(self.root), [])

    def test_an_untracked_file_is_not_scanned(self):
        self.write("a.rs", "x\n")
        self.git_init()
        self.write("b.rs", self.NOTE)
        self.assertEqual(H.tracked(self.root), ["a.rs"])

    def test_a_symlink_is_not_followed(self):
        self.write("real.rs", self.NOTE)
        (self.root / "link.rs").symlink_to(self.root / "real.rs")
        self.assertEqual(sorted({v["path"] for v in H.sweep(self.root, ["real.rs", "link.rs"]).values()}), ["real.rs"])

    def test_a_missing_file_is_skipped(self):
        self.assertEqual(H.sweep(self.root, ["nope.rs"]), {})

    def test_no_baseline_file_means_an_empty_baseline(self):
        self.write("crates/kf-x/src/lib.rs", self.NOTE)
        self.git_init()
        self.assertEqual(H.load_baseline(self.root), {})
        self.assertEqual([k for k, _, _ in H.findings(self.root)], ["NEW"])


class Record(GateBase):
    def test_recording_creates_the_baseline_from_the_findings(self):
        self.write("crates/kf-x/src/lib.rs", self.NOTE)
        self.git_init()
        H.record_baseline(self.root)
        self.assertEqual(len(self.recorded()), 1)
        self.assertEqual(H.findings(self.root), [])

    def test_recording_never_adds_a_finding_to_an_existing_baseline(self):
        self.write("crates/kf-x/src/lib.rs", self.NOTE)
        self.git_init()
        self.baseline({})
        H.record_baseline(self.root)
        self.assertEqual(self.recorded(), {})

    def test_recording_removes_a_resolved_entry(self):
        self.write("crates/kf-x/src/lib.rs", self.NOTE)
        self.git_init()
        keys = list(H.sweep(self.root, ["crates/kf-x/src/lib.rs"]))
        self.baseline({keys[0]: "x", "crates/kf-x/src/gone.rs|owner-ruling|0123456789abcdef": "y"})
        H.record_baseline(self.root)
        self.assertEqual(list(self.recorded()), keys)

    def test_recording_twice_does_not_add_a_new_note(self):
        self.write("crates/kf-x/src/lib.rs", self.NOTE)
        self.git_init()
        H.record_baseline(self.root)
        self.write("crates/kf-x/src/other.rs", self.NOTE)
        subprocess.run(["git", "add", "-A"], cwd=self.root, check=True, capture_output=True)
        H.record_baseline(self.root)
        self.assertEqual(len(self.recorded()), 1)

    def test_the_recorded_value_names_file_pattern_and_text(self):
        self.write("crates/kf-x/src/lib.rs", self.NOTE)
        self.git_init()
        H.record_baseline(self.root)
        self.assertEqual(list(self.recorded().values()), ["crates/kf-x/src/lib.rs [owner-ruling] owner ruling 2026-10-24: keep it"])


class Report(GateBase):
    def test_the_report_counts_per_pattern_and_names_the_worst_files(self):
        self.write("crates/kf-x/src/zz_big.rs", "// owner ruling 2026-10-24: a\n// owner ruling 2026-10-25: b\n// Lane C: c\n")
        self.write("crates/kf-x/src/aa_small.rs", "// Lane D: d\n")
        self.git_init()
        out = H.report(self.root)
        self.assertIn("owner-ruling               2", out)
        self.assertIn("lane-label                 2", out)
        self.assertLess(out.index("crates/kf-x/src/zz_big.rs"), out.index("crates/kf-x/src/aa_small.rs"))

    def test_a_pattern_without_hits_is_listed_with_zero(self):
        self.write("a.rs", "// fine\n")
        self.git_init()
        self.assertIn("owner-todo                 0", H.report(self.root))

    def test_the_worst_files_are_limited_to_the_top_n(self):
        self.write("a.rs", "// Lane C: a\n")
        self.write("b.rs", "// Lane D: b\n// Lane E: c\n")
        self.git_init()
        out = H.report(self.root, top=1)
        self.assertIn("  b.rs", out.split("worst files:")[1])
        self.assertNotIn("  a.rs", out.split("worst files:")[1])

    def test_the_report_counts_per_file_type(self):
        self.write("a.rs", "// Lane C: a\n")
        self.write("b.sh", "# Lane C: b\n# Lane D: c\n")
        self.git_init()
        out = H.report(self.root)
        self.assertIn("c-like:.rs     1", out)
        self.assertIn("shell:.sh      2", out)


class Cli(GateBase):
    def run_gate(self, *args):
        return subprocess.run([sys.executable, "-B", str(HERE / "comment_hygiene.py"), "--root", str(self.root), *args],
                              capture_output=True, text=True)

    def test_exit_zero_when_there_is_nothing_new(self):
        self.write("a.rs", "// fine\n")
        self.git_init()
        p = self.run_gate()
        self.assertEqual(p.returncode, 0, p.stdout)
        self.assertIn("new=0 stale=0", p.stdout)

    def test_exit_one_with_the_comment_when_there_is_a_new_note(self):
        self.write("a.rs", self.NOTE)
        self.git_init()
        p = self.run_gate()
        self.assertEqual(p.returncode, 1)
        self.assertIn("a.rs:1 [owner-ruling]", p.stdout)

    def test_exit_one_for_a_stale_baseline_entry(self):
        self.write("a.rs", "// fine\n")
        self.baseline({"a.rs|owner-ruling|0123456789abcdef": "x"})
        self.git_init()
        p = self.run_gate()
        self.assertEqual(p.returncode, 1)
        self.assertIn("STALE_BASELINE", p.stdout)

    def test_adding_code_above_a_recorded_note_keeps_the_gate_green(self):
        self.write("a.rs", self.NOTE)
        self.git_init()
        self.run_gate("--record-baseline")
        self.write("a.rs", "fn pad() {}\n\n" + self.NOTE)
        self.assertEqual(self.run_gate().returncode, 0)

    def test_the_report_mode_exits_zero(self):
        self.write("a.rs", self.NOTE)
        self.git_init()
        p = self.run_gate("report")
        self.assertEqual(p.returncode, 0)
        self.assertIn("owner-ruling", p.stdout)

    def test_the_record_flag_exits_zero_and_writes_the_file(self):
        self.write("a.rs", self.NOTE)
        self.git_init()
        p = self.run_gate("--record-baseline")
        self.assertEqual(p.returncode, 0)
        self.assertTrue((self.root / H.BASELINE).exists())
        self.assertEqual(self.run_gate().returncode, 0)


if __name__ == "__main__":
    sys.exit(unittest.main())
