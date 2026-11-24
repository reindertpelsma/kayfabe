#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later
"""GPU-free tests of scripts/ci/env_allowlist.py (STATUS: LIVE, 2026-11-20) on fixture trees in a scratch directory.
The names are the killers named by scripts/ci/env_allowlist_mutants.py: renaming a test is a change of that file."""
import json
from pathlib import Path
import subprocess
import sys
import tempfile
import unittest

HERE = Path(__file__).resolve().parent
sys.path.insert(0, str(HERE))
import env_allowlist as E  # noqa: E402


def entry(name, cls="debug", file="crates/kf-x/src/lib.rs", guest=False, **kw):
    e = {"name": name, "class": cls, "file": file, "why": "a plain one line reason", "changes_guest_served": guest}
    if guest or cls == "product":
        e["proposal"] = "keep-as-debug"
    e.update(kw)
    return e


class Tree(unittest.TestCase):
    def setUp(self):
        self._t = tempfile.TemporaryDirectory()
        self.addCleanup(self._t.cleanup)
        self.root = Path(self._t.name)

    def write(self, rel, text):
        p = self.root / rel
        p.parent.mkdir(parents=True, exist_ok=True)
        p.write_text(text)

    def rs(self, body, rel="crates/kf-x/src/lib.rs"):
        self.write(rel, body)

    def names(self):
        return sorted(E.scan(self.root))

    def registry(self, entries, allow=(), baseline=None):
        self.write(E.REGISTRY, json.dumps({"schema": 1, "product_env_allowlist": list(allow), "entries": entries}))
        if baseline is not None:
            self.write(E.BASELINE, json.dumps({"offenders": baseline}))

    def kinds(self):
        return sorted((k, n) for k, n, _ in E.findings(self.root))


class RustReads(Tree):
    def test_a_var_read_with_a_literal_name_is_found(self):
        self.rs('fn f() { let _ = std::env::var("KF3_A"); }')
        self.assertEqual(self.names(), ["KF3_A"])

    def test_a_var_os_read_is_found(self):
        self.rs('fn f() -> bool { std::env::var_os("KF3_B").is_some() }')
        self.assertEqual(self.names(), ["KF3_B"])

    def test_a_read_through_a_use_of_std_env_is_found(self):
        self.rs('use std::env;\nfn f() { let _ = env::var("KF3_C"); }')
        self.assertEqual(self.names(), ["KF3_C"])

    def test_a_path_that_is_not_the_std_env_module_is_not_a_read(self):
        self.rs('fn f() { let _ = other::env::var("KF3_NOT"); let _ = myenv::var("KF3_NOT2"); }')
        self.assertEqual(self.names(), [])

    def test_compile_time_env_macros_are_reads(self):
        self.rs('const A: &str = env!("KF3_D"); const B: Option<&str> = option_env!("KF3_E");')
        self.assertEqual(self.names(), ["KF3_D", "KF3_E"])

    def test_a_name_in_a_line_comment_is_not_a_read(self):
        self.rs('// std::env::var("KF3_F")\nfn f() {}')
        self.assertEqual(self.names(), [])

    def test_a_name_in_a_doc_comment_is_not_a_read(self):
        self.rs('/// reads `std::env::var("KF3_G")`\n//! and std::env::var_os("KF3_G2")\nfn f() {}')
        self.assertEqual(self.names(), [])

    def test_a_name_in_a_block_comment_is_not_a_read_and_blocks_nest(self):
        self.rs('/* a /* nested */ std::env::var("KF3_H") */\nfn f() {}')
        self.assertEqual(self.names(), [])

    def test_a_name_inside_a_string_is_not_a_read(self):
        self.rs('fn f() { let s = "call std::env::var(\\"KF3_I\\") first"; }')
        self.assertEqual(self.names(), [])

    def test_a_raw_string_hides_a_read(self):
        self.rs('fn f() { let s = r#"std::env::var("KF3_J")"#; let _ = std::env::var("KF3_J2"); }')
        self.assertEqual(self.names(), ["KF3_J2"])

    def test_a_slash_slash_inside_a_string_does_not_start_a_comment(self):
        self.rs('fn f() { let u = "http://x"; let _ = std::env::var("KF3_K"); }')
        self.assertEqual(self.names(), ["KF3_K"])

    def test_a_double_quote_char_literal_does_not_open_a_string(self):
        self.rs("fn f() { let q = '\"'; let _ = std::env::var(\"KF3_L\"); }")
        self.assertEqual(self.names(), ["KF3_L"])

    def test_a_lifetime_is_not_a_char_literal(self):
        self.rs("fn f<'a>(x: &'a str) -> &'a str { let _ = std::env::var(\"KF3_M\"); x }")
        self.assertEqual(self.names(), ["KF3_M"])

    def test_a_const_name_is_resolved(self):
        self.rs('const FLAG: &str = "KF3_N";\nfn f() { let _ = std::env::var(FLAG); }')
        self.assertEqual(self.names(), ["KF3_N"])

    def test_a_const_in_another_file_is_resolved_through_its_path(self):
        self.rs('pub const NESTED: &str = "KF3_O";', "crates/kf-x/src/support.rs")
        self.rs('fn f() { let _ = std::env::var_os(support::NESTED); }', "crates/kf-x/src/lib.rs")
        self.assertEqual(self.names(), ["KF3_O"])

    def test_a_list_of_names_in_the_function_names_each(self):
        self.rs('fn f() -> bool { ["KF3_P", "KF3_Q"].iter().any(|k| std::env::var_os(k).is_some()) }')
        self.assertEqual(self.names(), ["KF3_P", "KF3_Q"])

    def test_an_injected_lookup_names_the_get_calls(self):
        self.rs('fn parse(get: impl Fn(&str) -> Option<String>) { get("KF3_R"); get("KF_S"); }\n'
                'fn from_env() { parse(|k| std::env::var(k).ok()) }')
        self.assertEqual(self.names(), ["KF3_R", "KF_S"])

    def test_a_computed_name_is_a_dynamic_read_that_names_file_and_fn(self):
        self.rs('fn lookup(k: &str) -> Option<String> { std::env::var(k).ok() }')
        self.assertEqual(self.names(), ["<dynamic:crates/kf-x/src/lib.rs:lookup>"])

    def test_vars_os_is_an_enumeration(self):
        self.rs('fn f() { for (k, _) in std::env::vars_os() { drop(k); } }')
        self.assertEqual(self.names(), ["<env:vars>"])

    def test_a_use_without_a_call_is_not_a_read(self):
        self.rs('use std::env::var;\nfn f() {}')
        self.assertEqual(self.names(), [])

    def test_the_line_of_a_read_is_reported(self):
        self.rs('fn f() {}\n\nfn g() { let _ = std::env::var("KF3_T"); }\n')
        self.assertEqual(E.scan(self.root)["KF3_T"], [("crates/kf-x/src/lib.rs", 3, "literal")])

    def test_a_name_read_in_two_files_lists_both_sites(self):
        self.rs('fn f() { let _ = std::env::var("KF3_U"); }', "crates/kf-x/src/a.rs")
        self.rs('fn g() { let _ = std::env::var("KF3_U"); }', "crates/kf-y/src/b.rs")
        self.assertEqual(sorted(f for f, _, _ in E.scan(self.root)["KF3_U"]), ["crates/kf-x/src/a.rs", "crates/kf-y/src/b.rs"])

    def test_a_rust_block_comment_can_hold_a_second_one_and_code_after_it_is_scanned(self):
        self.rs('/* a /* b */ " */ fn f() { let _ = std::env::var("KF3_AO"); }')
        self.assertEqual(self.names(), ["KF3_AO"])

    def test_an_escaped_quote_can_hide_a_read_inside_a_string(self):
        self.rs('fn f() { let s = "x\\" std::env::var(\\"KF3_BT\\") y"; }')
        self.assertEqual(self.names(), [])

    def test_a_raw_string_with_an_odd_number_of_quotes_hides_a_read(self):
        self.rs('fn f() { let s = r#"a " std::env::var("KF3_BU")"#; }')
        self.assertEqual(self.names(), [])

    def test_a_cfg_debug_assertions_module_is_product_code(self):
        self.rs('#[cfg(debug_assertions)]\nmod m { pub fn f() { let _ = std::env::var("KF3_BV"); } }\n')
        self.assertEqual(self.names(), ["KF3_BV"])

    def test_a_declared_test_file_behind_another_attribute_is_skipped(self):
        self.rs('#[cfg(test)]\n#[allow(dead_code)]\nmod checks;\nfn f() {}\n', "crates/kf-x/src/lib.rs")
        self.rs('fn t() { let _ = std::env::var("KF3_BW"); }', "crates/kf-x/src/checks.rs")
        self.assertEqual(self.names(), [])

    def test_a_declared_test_directory_behind_another_attribute_is_skipped(self):
        self.rs('#[cfg(test)]\n#[allow(dead_code)]\npub mod checks;\nfn f() {}\n', "crates/kf-x/src/lib.rs")
        self.rs('fn t() { let _ = std::env::var("KF3_BX"); }', "crates/kf-x/src/checks/inner.rs")
        self.assertEqual(self.names(), [])

    def test_an_escaped_quote_does_not_end_a_string(self):
        self.rs('fn f() { let s = "a\\"b"; let _ = std::env::var("KF3_AP"); }')
        self.assertEqual(self.names(), ["KF3_AP"])

    def test_a_raw_string_with_a_quote_inside_does_not_derail_the_scan(self):
        self.rs('fn f() { let s = r#"a " b"#; let _ = std::env::var("KF3_AQ"); }')
        self.assertEqual(self.names(), ["KF3_AQ"])

    def test_a_brace_char_literal_does_not_unbalance_a_skipped_module(self):
        self.rs("#[cfg(test)]\nmod t { const C: char = '}'; fn x() { let _ = std::env::var(\"KF3_AR\"); } }\nfn f() { let _ = std::env::var(\"KF3_AR2\"); }\n")
        self.assertEqual(self.names(), ["KF3_AR2"])

    def test_a_name_list_in_an_array_of_one_is_found(self):
        self.rs('fn f() { for k in ["KF3_AS"] { let _ = std::env::var_os(k); } }')
        self.assertEqual(self.names(), ["KF3_AS"])

    def test_an_injected_lookup_with_a_kayfabe_name_is_found(self):
        self.rs('fn parse(get: impl Fn(&str) -> Option<String>) { get("KAYFABE_AT"); }\nfn e() { parse(|k| std::env::var(k).ok()) }')
        self.assertEqual(self.names(), ["KAYFABE_AT"])

    def test_vars_is_an_enumeration_too(self):
        self.rs('fn f() { for (k, _) in std::env::vars() { drop(k); } }')
        self.assertEqual(self.names(), ["<env:vars>"])

    def test_a_use_of_var_os_without_a_call_is_not_a_read(self):
        self.rs('use std::env::var_os;\nfn f() {}')
        self.assertEqual(self.names(), [])

    def test_the_env_macro_alone_is_a_read(self):
        self.rs('const A: &str = env!("KF3_AU");')
        self.assertEqual(self.names(), ["KF3_AU"])


class TestCode(Tree):
    def test_an_inline_cfg_test_module_is_not_product_code(self):
        self.rs('fn f() {}\n#[cfg(test)]\nmod tests {\n    fn t() { let _ = std::env::var("KF3_V"); }\n}\n')
        self.assertEqual(self.names(), [])

    def test_code_after_a_cfg_test_module_is_product_code_again(self):
        self.rs('#[cfg(test)]\nmod tests { fn t() { let s = "}"; let _ = std::env::var("KF3_W"); } }\n'
                'fn f() { let _ = std::env::var("KF3_W2"); }\n')
        self.assertEqual(self.names(), ["KF3_W2"])

    def test_a_cfg_test_module_with_another_attribute_is_skipped_too(self):
        self.rs('#[cfg(test)]\n#[allow(clippy::all)]\nmod tests { fn t() { let _ = std::env::var("KF3_X"); } }\n')
        self.assertEqual(self.names(), [])

    def test_a_cfg_test_function_is_skipped(self):
        self.rs('#[cfg(test)]\nfn helper() { let _ = std::env::var("KF3_Y"); }\nfn f() { let _ = std::env::var("KF3_Y2"); }\n')
        self.assertEqual(self.names(), ["KF3_Y2"])

    def test_a_cfg_all_test_module_is_skipped(self):
        self.rs('#[cfg(all(test, unix))]\nmod tests { fn t() { let _ = std::env::var("KF3_Z"); } }\n')
        self.assertEqual(self.names(), [])

    def test_a_cfg_feature_module_is_product_code(self):
        self.rs('#[cfg(feature = "x")]\nmod m { pub fn f() { let _ = std::env::var("KF3_AA"); } }\n')
        self.assertEqual(self.names(), ["KF3_AA"])

    def test_a_file_declared_as_a_cfg_test_module_is_skipped(self):
        self.rs('#[cfg(test)]\nmod checks;\nfn f() {}\n', "crates/kf-x/src/lib.rs")
        self.rs('fn t() { let _ = std::env::var("KF3_AB"); }', "crates/kf-x/src/checks.rs")
        self.assertEqual(self.names(), [])

    def test_a_directory_declared_as_a_cfg_test_module_is_skipped(self):
        self.rs('#[cfg(test)]\nmod checks;\nfn f() {}\n', "crates/kf-x/src/lib.rs")
        self.rs('fn t() { let _ = std::env::var("KF3_AC"); }', "crates/kf-x/src/checks/inner.rs")
        self.assertEqual(self.names(), [])

    def test_a_tests_directory_is_skipped(self):
        self.rs('fn t() { let _ = std::env::var("KF3_AD"); }', "crates/kf-x/tests/it.rs")
        self.assertEqual(self.names(), [])

    def test_benches_and_examples_are_skipped(self):
        self.rs('fn t() { let _ = std::env::var("KF3_AE"); }', "crates/kf-x/benches/b.rs")
        self.rs('fn t() { let _ = std::env::var("KF3_AF"); }', "crates/kf-x/examples/e.rs")
        self.assertEqual(self.names(), [])

    def test_test_named_files_are_skipped(self):
        self.rs('fn t() { let _ = std::env::var("KF3_AG"); }', "crates/kf-x/src/tests.rs")
        self.rs('fn t() { let _ = std::env::var("KF3_AH"); }', "crates/kf-x/src/chan_tests.rs")
        self.write("broker/nb_session_test.c", 'int main(void) { return getenv("NB_AI") != 0; }')
        self.write("cuda/walk/kf_tests.cu", 'int main() { return getenv("KF_AJ") != 0; }')
        self.assertEqual(self.names(), [])

    def test_a_file_whose_name_only_contains_test_inside_a_word_is_scanned(self):
        self.rs('fn t() { let _ = std::env::var("KF3_AK"); }', "crates/kf-x/src/latest.rs")
        self.assertEqual(self.names(), ["KF3_AK"])

    def test_the_frozen_crates_and_the_archive_are_not_scanned(self):
        self.rs('fn t() { let _ = std::env::var("KF3_AL"); }', "crates/kayfabe-core/src/lib.rs")
        self.rs('fn t() { let _ = std::env::var("KF3_AM"); }', "archive/crates/kayfabe-core/src/lib.rs")
        self.rs('fn t() { let _ = std::env::var("KF3_AN"); }', "third_party/x/src/lib.rs")
        self.assertEqual(self.names(), [])

    def test_a_build_script_is_scanned(self):
        self.rs('fn main() { let _ = std::env::var("OUT_DIR"); }', "crates/kf-x/build.rs")
        self.assertEqual(self.names(), ["OUT_DIR"])

    def test_a_cfg_test_function_with_a_test_attribute_is_skipped(self):
        self.rs('#[cfg(test)]\n#[test]\nfn t() { let _ = std::env::var("KF3_AV"); }\nfn f() {}\n')
        self.assertEqual(self.names(), [])

    def test_a_cfg_all_test_with_a_second_condition_is_skipped(self):
        self.rs('#[cfg(all(test, feature = "x"))]\nmod tests { fn t() { let _ = std::env::var("KF3_AW"); } }\n')
        self.assertEqual(self.names(), [])

    def test_a_cfg_unix_item_is_product_code(self):
        self.rs('#[cfg(unix)]\nfn f() { let _ = std::env::var("KF3_AX"); }\n')
        self.assertEqual(self.names(), ["KF3_AX"])

    def test_a_nested_directory_of_a_test_module_is_skipped(self):
        self.rs('#[cfg(test)]\nmod checks;\nfn f() {}\n', "crates/kf-x/src/lib.rs")
        self.rs('fn t() { let _ = std::env::var("KF3_AY"); }', "crates/kf-x/src/checks/deeper/x.rs")
        self.assertEqual(self.names(), [])

    def test_a_nested_tests_directory_is_skipped(self):
        self.rs('fn t() { let _ = std::env::var("KF3_AZ"); }', "crates/kf-x/src/foo/tests/a.rs")
        self.assertEqual(self.names(), [])

    def test_a_benches_directory_is_skipped(self):
        self.rs('fn t() { let _ = std::env::var("KF3_BJ"); }', "crates/kf-x/benches/b.rs")
        self.assertEqual(self.names(), [])

    def test_an_examples_directory_is_skipped(self):
        self.rs('fn t() { let _ = std::env::var("KF3_BK"); }', "crates/kf-x/examples/e.rs")
        self.assertEqual(self.names(), [])

    def test_a_tests_rs_file_is_skipped(self):
        self.rs('fn t() { let _ = std::env::var("KF3_BL"); }', "crates/kf-x/src/tests.rs")
        self.assertEqual(self.names(), [])

    def test_contest_and_attest_names_are_scanned(self):
        self.rs('fn t() { let _ = std::env::var("KF3_BM"); }', "crates/kf-x/src/contest.rs")
        self.rs('fn t() { let _ = std::env::var("KF3_BN"); }', "crates/kf-x/src/attest.rs")
        self.assertEqual(self.names(), ["KF3_BM", "KF3_BN"])

    def test_a_directory_not_named_kf_is_not_scanned(self):
        self.rs('fn t() { let _ = std::env::var("KF3_BO"); }', "crates/other/src/lib.rs")
        self.assertEqual(self.names(), [])

    def test_the_archive_is_not_scanned(self):
        self.rs('fn t() { let _ = std::env::var("KF3_BP"); }', "archive/crates/kf-old/src/lib.rs")
        self.assertEqual(self.names(), [])


class CReads(Tree):
    def test_getenv_in_the_qemu_device_is_found(self):
        self.write("qemu/hw/misc/kf3/kf3.c", 'void f(void) { const char *e = getenv("KF3_BA"); }')
        self.assertEqual(self.names(), ["KF3_BA"])

    def test_secure_getenv_is_found(self):
        self.write("broker/b.c", 'void f(void) { secure_getenv("KF3_BB"); }')
        self.assertEqual(self.names(), ["KF3_BB"])

    def test_the_broker_the_walk_kernel_and_the_firmware_are_scanned(self):
        self.write("broker/b.c", 'int f(void) { return getenv("DISPLAY") != 0; }')
        self.write("cuda/walk/kf_walk.cu", 'int f() { return getenv("KF_BC") != 0; }')
        self.write("firmware/kf-gop/build.rs", 'fn main() { let _ = std::env::var("CARGO_CFG_TARGET_OS"); }')
        self.assertEqual(self.names(), ["CARGO_CFG_TARGET_OS", "DISPLAY", "KF_BC"])

    def test_getenv_in_a_c_comment_or_string_is_not_a_read(self):
        self.write("broker/b.c", '/* getenv("KF3_BD") */\n// getenv("KF3_BE")\nconst char *s = "getenv(\\"KF3_BF\\")";\n')
        self.assertEqual(self.names(), [])

    def test_a_computed_getenv_is_dynamic(self):
        self.write("broker/b.c", 'const char *f(const char *k) { return getenv(k); }')
        self.assertEqual(self.names(), ["<dynamic:broker/b.c:?>"])

    def test_a_longer_identifier_ending_in_getenv_is_not_a_read(self):
        self.write("broker/b.c", 'int f(void) { return my_getenv("KF3_BG") + xgetenv_all("KF3_BH"); }')
        self.assertEqual(self.names(), [])

    def test_a_source_outside_the_scanned_directories_is_not_scanned(self):
        self.write("tools/t.c", 'int main(void) { return getenv("KF3_BI") != 0; }')
        self.assertEqual(self.names(), [])

    def test_secure_getenv_in_the_device_is_found(self):
        self.write("qemu/hw/misc/kf3/kf3.c", 'void f(void) { secure_getenv("KF3_BQ"); }')
        self.assertEqual(self.names(), ["KF3_BQ"])

    def test_a_prefixed_getenv_wrapper_is_not_a_read(self):
        self.write("broker/b.c", 'int f(void) { return qemu_getenv("KF3_BR") != 0; }')
        self.assertEqual(self.names(), [])


class Findings(Tree):
    def read(self, name="KF3_A", rel="crates/kf-x/src/lib.rs"):
        self.rs('fn f() { let _ = std::env::var("%s"); }' % name, rel)

    def test_a_read_that_is_not_in_the_registry_is_a_finding(self):
        self.read()
        self.registry([])
        self.assertEqual(self.kinds(), [("UNREGISTERED", "KF3_A")])

    def test_an_unregistered_read_names_its_file_and_line(self):
        self.read()
        self.registry([])
        detail = E.findings(self.root)[0][2]
        self.assertIn("crates/kf-x/src/lib.rs:1", detail)

    def test_a_registered_debug_read_is_clean(self):
        self.read()
        self.registry([entry("KF3_A")])
        self.assertEqual(self.kinds(), [])

    def test_every_non_product_class_without_a_guest_effect_is_clean(self):
        names = [("KF3_C%d" % i, c) for i, c in enumerate(("debug", "measurement", "inject", "stress", "system"))]
        self.rs("fn f() { %s }" % " ".join('let _ = std::env::var("%s");' % n for n, _ in names))
        self.registry([entry(n, c) for n, c in names])
        self.assertEqual(self.kinds(), [])

    def test_a_missing_registry_is_a_finding(self):
        self.read()
        self.assertEqual(self.kinds(), [("NO_REGISTRY", E.REGISTRY)])

    def test_an_entry_that_nothing_reads_is_stale(self):
        self.read()
        self.registry([entry("KF3_A"), entry("KF3_GONE")])
        self.assertEqual(self.kinds(), [("STALE_ENTRY", "KF3_GONE")])

    def test_an_entry_whose_name_is_read_only_in_a_test_is_stale(self):
        self.rs('fn f() {}\n#[cfg(test)]\nmod t { fn x() { let _ = std::env::var("KF3_A"); } }')
        self.registry([entry("KF3_A")])
        self.assertEqual(self.kinds(), [("STALE_ENTRY", "KF3_A")])

    def test_an_entry_whose_file_does_not_read_the_name_is_a_mismatch(self):
        self.read()
        self.registry([entry("KF3_A", file="crates/kf-x/src/other.rs")])
        self.assertEqual(self.kinds(), [("FILE_MISMATCH", "KF3_A")])

    def test_an_entry_may_name_any_file_that_reads_the_name(self):
        self.read(rel="crates/kf-x/src/a.rs")
        self.read(rel="crates/kf-x/src/b.rs")
        self.registry([entry("KF3_A", file="crates/kf-x/src/b.rs")])
        self.assertEqual(self.kinds(), [])

    def test_an_unknown_class_is_a_bad_entry(self):
        self.read()
        self.registry([entry("KF3_A", cls="whatever")])
        self.assertEqual(self.kinds(), [("BAD_ENTRY", "KF3_A")])

    def test_a_guest_flag_that_is_not_a_boolean_is_a_bad_entry(self):
        self.read()
        self.registry([entry("KF3_A", guest="no")])
        self.assertEqual(self.kinds(), [("BAD_ENTRY", "KF3_A")])

    def test_a_why_of_one_or_two_words_is_a_bad_entry(self):
        self.read()
        self.registry([entry("KF3_A", why="debug knob")])
        self.assertEqual(self.kinds(), [("BAD_ENTRY", "KF3_A")])

    def test_a_why_with_a_newline_is_a_bad_entry(self):
        self.read()
        self.registry([entry("KF3_A", why="one line here\nand another line")])
        self.assertEqual(self.kinds(), [("BAD_ENTRY", "KF3_A")])

    def test_an_offender_without_a_proposal_is_a_bad_entry(self):
        self.read()
        e = entry("KF3_A", guest=True)
        del e["proposal"]
        self.registry([e], baseline=["KF3_A"])
        self.assertEqual(self.kinds(), [("BAD_ENTRY", "KF3_A")])

    def test_an_unknown_proposal_is_a_bad_entry(self):
        self.read()
        self.registry([entry("KF3_A", guest=True, proposal="someday")])
        self.assertEqual(self.kinds()[0], ("BAD_ENTRY", "KF3_A"))

    def test_a_name_listed_twice_is_a_bad_entry(self):
        self.read()
        self.registry([entry("KF3_A"), entry("KF3_A")])
        self.assertEqual(self.kinds(), [("BAD_ENTRY", "KF3_A")])

    def test_a_guest_affecting_entry_that_is_not_on_the_baseline_is_a_new_offender(self):
        self.read()
        self.registry([entry("KF3_A", guest=True)], baseline=[])
        self.assertEqual(self.kinds(), [("NEW_OFFENDER", "KF3_A")])

    def test_a_product_class_entry_that_is_not_on_the_baseline_is_a_new_offender(self):
        self.read()
        self.registry([entry("KF3_A", cls="product")], baseline=[])
        self.assertEqual(self.kinds(), [("NEW_OFFENDER", "KF3_A")])

    def test_an_offender_on_the_baseline_is_tolerated(self):
        self.read()
        self.registry([entry("KF3_A", guest=True)], baseline=["KF3_A"])
        self.assertEqual(self.kinds(), [])

    def test_an_offender_on_the_allowlist_needs_no_baseline_entry(self):
        self.read()
        self.registry([entry("KF3_A", guest=True)], allow=["KF3_A"], baseline=[])
        self.assertEqual(self.kinds(), [])

    def test_an_allowlisted_name_the_registry_lacks_is_a_finding(self):
        self.read()
        self.registry([entry("KF3_A")], allow=["KF3_NOPE"])
        self.assertEqual(self.kinds(), [("BAD_ALLOWLIST", "KF3_NOPE")])

    def test_a_baseline_name_that_is_no_longer_an_offender_is_stale(self):
        self.read()
        self.registry([entry("KF3_A", guest=False)], baseline=["KF3_A"])
        self.assertEqual(self.kinds(), [("STALE_BASELINE", "KF3_A")])

    def test_a_baseline_name_that_is_now_allowlisted_is_stale(self):
        self.read()
        self.registry([entry("KF3_A", guest=True)], allow=["KF3_A"], baseline=["KF3_A"])
        self.assertEqual(self.kinds(), [("STALE_BASELINE", "KF3_A")])

    def test_a_baseline_name_that_is_gone_from_the_registry_is_stale(self):
        self.read()
        self.registry([entry("KF3_A")], baseline=["KF3_OLD"])
        self.assertEqual(self.kinds(), [("STALE_BASELINE", "KF3_OLD")])

    def test_a_missing_baseline_file_is_an_empty_baseline(self):
        self.read()
        self.registry([entry("KF3_A", guest=True)])
        self.assertEqual(self.kinds(), [("NEW_OFFENDER", "KF3_A")])

    def test_a_dynamic_read_must_be_registered_under_its_pseudo_name(self):
        self.rs('fn lookup(k: &str) { let _ = std::env::var(k); }')
        self.registry([])
        self.assertEqual(self.kinds(), [("UNREGISTERED", "<dynamic:crates/kf-x/src/lib.rs:lookup>")])

    def test_an_entry_for_a_name_read_in_another_crate_is_a_mismatch(self):
        self.read(rel="crates/kf-y/src/lib.rs")
        self.registry([entry("KF3_A", file="crates/kf-x/src/lib.rs")])
        self.assertEqual(self.kinds(), [("FILE_MISMATCH", "KF3_A")])

    def test_an_entry_may_name_the_file_in_another_crate(self):
        self.read(rel="crates/kf-x/src/lib.rs")
        self.read(rel="crates/kf-y/src/lib.rs")
        self.read(rel="crates/kf-z/src/lib.rs")
        self.registry([entry("KF3_A", file="crates/kf-z/src/lib.rs")])
        self.assertEqual(self.kinds(), [])

    def test_a_missing_class_is_a_bad_entry(self):
        self.read()
        e = entry("KF3_A")
        del e["class"]
        self.registry([e])
        self.assertEqual(self.kinds(), [("BAD_ENTRY", "KF3_A")])

    def test_a_missing_guest_flag_is_a_bad_entry(self):
        self.read()
        e = entry("KF3_A")
        del e["changes_guest_served"]
        self.registry([e])
        self.assertEqual(self.kinds(), [("BAD_ENTRY", "KF3_A")])

    def test_a_one_word_why_is_a_bad_entry(self):
        self.read()
        self.registry([entry("KF3_A", why="knob")])
        self.assertEqual(self.kinds(), [("BAD_ENTRY", "KF3_A")])

    def test_only_the_names_missing_from_the_baseline_are_new(self):
        self.rs('fn f() { let _ = std::env::var("KF3_A"); let _ = std::env::var("KF3_B"); let _ = std::env::var("KF3_C"); }')
        self.registry([entry("KF3_A", guest=True), entry("KF3_B", guest=True), entry("KF3_C", guest=True)], baseline=["KF3_A"])
        self.assertEqual(self.kinds(), [("NEW_OFFENDER", "KF3_B"), ("NEW_OFFENDER", "KF3_C")])

    def test_an_empty_why_is_a_bad_entry(self):
        self.read()
        self.registry([entry("KF3_A", why="")])
        self.assertEqual(self.kinds(), [("BAD_ENTRY", "KF3_A")])

    def test_a_why_that_spans_a_blank_line_is_a_bad_entry(self):
        self.read()
        self.registry([entry("KF3_A", why="first line of it\n\nsecond paragraph here")])
        self.assertEqual(self.kinds(), [("BAD_ENTRY", "KF3_A")])

    def test_a_name_listed_twice_with_different_classes_is_a_bad_entry(self):
        self.read()
        self.registry([entry("KF3_A"), entry("KF3_A", cls="measurement")])
        self.assertEqual(self.kinds(), [("BAD_ENTRY", "KF3_A")])

    def test_an_allowlist_may_name_several_unknown_names(self):
        self.read()
        self.registry([entry("KF3_A")], allow=["KF3_NOPE", "KF3_NOPE2"])
        self.assertEqual(self.kinds(), [("BAD_ALLOWLIST", "KF3_NOPE"), ("BAD_ALLOWLIST", "KF3_NOPE2")])

    def test_two_new_offenders_are_both_reported(self):
        self.rs('fn f() { let _ = std::env::var("KF3_A"); let _ = std::env::var("KF3_B"); }')
        self.registry([entry("KF3_A", guest=True), entry("KF3_B", cls="product")], baseline=[])
        self.assertEqual(self.kinds(), [("NEW_OFFENDER", "KF3_A"), ("NEW_OFFENDER", "KF3_B")])


class Baseline(Tree):
    def setUp(self):
        super().setUp()
        self.rs('fn f() { let _ = std::env::var("KF3_A"); let _ = std::env::var("KF3_B"); }')

    def stored(self):
        return json.loads((self.root / E.BASELINE).read_text())["offenders"]

    def test_recording_creates_the_baseline_from_the_offenders(self):
        self.registry([entry("KF3_A", guest=True), entry("KF3_B")])
        E.record_baseline(self.root)
        self.assertEqual(self.stored(), ["KF3_A"])

    def test_recording_never_adds_a_name_to_an_existing_baseline(self):
        self.registry([entry("KF3_A", guest=True), entry("KF3_B", guest=True)], baseline=["KF3_A"])
        E.record_baseline(self.root)
        self.assertEqual(self.stored(), ["KF3_A"])

    def test_recording_removes_a_name_that_is_no_longer_an_offender(self):
        self.registry([entry("KF3_A", guest=True), entry("KF3_B")], baseline=["KF3_A", "KF3_B"])
        E.record_baseline(self.root)
        self.assertEqual(self.stored(), ["KF3_A"])

    def test_recording_leaves_allowlisted_names_out(self):
        self.registry([entry("KF3_A", guest=True), entry("KF3_B", guest=True)], allow=["KF3_B"])
        E.record_baseline(self.root)
        self.assertEqual(self.stored(), ["KF3_A"])

    def test_recording_leaves_every_allowlisted_name_out(self):
        self.registry([entry("KF3_A", guest=True), entry("KF3_B", guest=True)], allow=["KF3_A", "KF3_B"])
        E.record_baseline(self.root)
        self.assertEqual(self.stored(), [])

    def test_recording_includes_a_product_class_entry(self):
        self.registry([entry("KF3_A", cls="product"), entry("KF3_B")])
        E.record_baseline(self.root)
        self.assertEqual(self.stored(), ["KF3_A"])

    def test_recording_never_adds_even_when_the_baseline_is_empty(self):
        self.registry([entry("KF3_A", guest=True)], baseline=[])
        E.record_baseline(self.root)
        self.assertEqual(self.stored(), [])

    def test_recording_twice_gives_the_same_baseline(self):
        self.registry([entry("KF3_A", guest=True), entry("KF3_B")])
        E.record_baseline(self.root)
        E.record_baseline(self.root)
        self.assertEqual(self.stored(), ["KF3_A"])


class Report(Tree):
    def test_the_report_counts_names_and_guest_effects_per_class(self):
        self.rs('fn f() { let _ = std::env::var("KF3_A"); let _ = std::env::var("KF3_B"); let _ = std::env::var("KF3_C"); }')
        self.registry([entry("KF3_A", guest=True), entry("KF3_B"), entry("KF3_C", cls="measurement")])
        out = E.report(self.root)
        self.assertIn("| debug | 2 | 1 |", out)
        self.assertIn("| measurement | 1 | 0 |", out)
        self.assertIn("| **total** | 3 | 1 |", out)

    def test_the_report_lists_each_offender_with_file_line_and_proposal(self):
        self.rs('\nfn f() { let _ = std::env::var("KF3_A"); }')
        self.registry([entry("KF3_A", guest=True, proposal="device-property")])
        out = E.report(self.root)
        self.assertIn("| `KF3_A` | debug | `crates/kf-x/src/lib.rs:2` | device-property |", out)

    def test_the_report_leaves_out_names_that_do_not_change_the_guest(self):
        self.rs('fn f() { let _ = std::env::var("KF3_A"); }')
        self.registry([entry("KF3_A")])
        self.assertNotIn("| `KF3_A` |", E.report(self.root))

    def test_the_report_counts_guest_effects_in_every_class(self):
        self.rs('fn f() { let _ = std::env::var("KF3_A"); let _ = std::env::var("KF3_B"); }')
        self.registry([entry("KF3_A", cls="inject", guest=True), entry("KF3_B", cls="stress", guest=True)])
        out = E.report(self.root)
        self.assertIn("| inject | 1 | 1 |", out)
        self.assertIn("| stress | 1 | 1 |", out)

    def test_the_report_lists_all_six_classes(self):
        self.rs('fn f() {}')
        self.registry([])
        rows = [l for l in E.report(self.root).splitlines() if l.startswith("|") and l.split("|")[1].strip() in E.CLASSES]
        self.assertEqual(len(rows), 6)

    def test_a_class_with_no_names_is_listed_with_zero(self):
        self.rs('fn f() { let _ = std::env::var("KF3_A"); }')
        self.registry([entry("KF3_A")])
        out = E.report(self.root)
        self.assertIn("| stress | 0 | 0 |", out)
        self.assertIn("| product | 0 | 0 |", out)

    def test_the_offender_table_has_one_row_per_offender(self):
        self.rs('fn f() { let _ = std::env::var("KF3_A"); let _ = std::env::var("KF3_B"); let _ = std::env::var("KF3_C"); }')
        self.registry([entry("KF3_A", guest=True), entry("KF3_B", cls="product"), entry("KF3_C")])
        rows = [l for l in E.report(self.root).splitlines() if l.startswith("| `")]
        self.assertEqual(len(rows), 2)


class Cli(Tree):
    def run_gate(self, *args):
        return subprocess.run([sys.executable, "-B", str(HERE / "env_allowlist.py"), "--root", str(self.root), *args],
                              capture_output=True, text=True)

    def test_exit_zero_when_clean(self):
        self.rs('fn f() { let _ = std::env::var("KF3_A"); }')
        self.registry([entry("KF3_A")])
        p = self.run_gate()
        self.assertEqual(p.returncode, 0, p.stdout)
        self.assertIn("findings=0", p.stdout)

    def test_exit_one_with_a_named_line_when_not(self):
        self.rs('fn f() { let _ = std::env::var("KF3_A"); }')
        self.registry([])
        p = self.run_gate()
        self.assertEqual(p.returncode, 1)
        self.assertIn("UNREGISTERED KF3_A:", p.stdout)

    def test_the_report_mode_exits_zero_even_with_findings(self):
        self.rs('fn f() { let _ = std::env::var("KF3_A"); }')
        self.registry([entry("KF3_A", guest=True)])
        p = self.run_gate("report")
        self.assertEqual(p.returncode, 0)
        self.assertIn("| `KF3_A` |", p.stdout)
    def test_a_missing_registry_exits_one(self):
        self.rs('fn f() {}')
        p = self.run_gate()
        self.assertEqual(p.returncode, 1)
        self.assertIn("NO_REGISTRY", p.stdout)

    def test_an_offender_on_the_baseline_exits_zero(self):
        self.rs('fn f() { let _ = std::env::var("KF3_A"); }')
        self.registry([entry("KF3_A", guest=True)], baseline=["KF3_A"])
        self.assertEqual(self.run_gate().returncode, 0)

    def test_exit_one_for_a_stale_entry(self):
        self.rs('fn f() {}')
        self.registry([entry("KF3_A")])
        p = self.run_gate()
        self.assertEqual(p.returncode, 1)
        self.assertIn("STALE_ENTRY KF3_A:", p.stdout)

    def test_the_record_flag_prints_the_count_and_exits_zero(self):
        self.rs('fn f() { let _ = std::env::var("KF3_A"); }')
        self.registry([entry("KF3_A", guest=True)])
        p = self.run_gate("--record-baseline")
        self.assertEqual(p.returncode, 0)
        self.assertIn("baseline: 1 offenders", p.stdout)


if __name__ == "__main__":
    sys.exit(unittest.main())
