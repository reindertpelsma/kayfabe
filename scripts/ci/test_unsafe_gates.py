# SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later
"""Known positives for the perimeter gates (docs/design/V3_SEC_PERIMETER.md §1.4, §1.5).

Every rule must FIRE on a fixture built to violate it, and stay quiet on its control. A gate
that reports zero on the tree is only evidence when it has first reported one here (audit
S1-12). Pure Python, no cargo: this runs in the `stable` job's discover step.

The keyword is assembled from fragments so this file never contains it as a word.
"""

from __future__ import annotations

import json
import tempfile
import textwrap
import unittest
from pathlib import Path

import debt
import perimeter
import rslex

K = "un" + "safe"


def cfg(**extra) -> perimeter.Config:
    raw = {
        "crates": [{"path": "crates/u", "class": "U"}, {"path": "crates/p", "class": "P"}],
        "exempt": [{"path": "crates/frozen", "bound": "debt.py frozen", "reason": "fixture"}],
        "standalone": [{"path": "crates/u/gen", "runs": ["--all-targets"]}],
        "manifest_lints": {
            "crates/u": {"rust": {"unsafe_code": "allow"}, "clippy": {"undocumented_unsafe_blocks": "deny"}},
            "crates/u/gen": {"rust": {"unsafe_code": "forbid"}},
        },
        "build_scripts": {},
    }
    raw.update(extra)
    return perimeter.Config(raw)


class Fixture:
    def __init__(self, files: dict[str, str]):
        self.dir = tempfile.TemporaryDirectory()
        self.root = Path(self.dir.name)
        for rel, text in files.items():
            p = self.root / rel
            p.parent.mkdir(parents=True, exist_ok=True)
            p.write_text(textwrap.dedent(text).replace("UNSAFE", K))
        self.files = sorted(f for f in files if f.endswith(".rs"))

    def lex(self, c: perimeter.Config | None = None) -> list[perimeter.Finding]:
        findings, _ = perimeter.run_lex(self.root, c or cfg(), self.files)
        return findings

    def close(self) -> None:
        self.dir.cleanup()


SAFE_LIB = "pub fn ok() {}\n"
U_LIB = "mod a_unsafe;\npub fn ok() {}\n"


def rules(findings) -> list[str]:
    return sorted({f.rule for f in findings})


class LexTests(unittest.TestCase):
    def run_fixture(self, files: dict[str, str], c=None) -> list[perimeter.Finding]:
        fx = Fixture(files)
        try:
            return fx.lex(c)
        finally:
            fx.close()

    def assertFires(self, rule: str, files: dict[str, str], count: int | None = None, c=None):
        got = [f for f in self.run_fixture(files, c) if f.rule == rule]
        self.assertTrue(got, f"{rule} did not fire")
        if count is not None:
            self.assertEqual(len(got), count, [str(g) for g in got])
        return got

    def assertQuiet(self, files: dict[str, str], c=None):
        got = self.run_fixture(files, c)
        self.assertEqual(got, [], [str(g) for g in got])

    # -- controls ---------------------------------------------------------------------
    def test_control_a_perimeter_file_of_a_class_u_crate_may_hold_it(self):
        self.assertQuiet({"crates/u/src/lib.rs": U_LIB,
                          "crates/u/src/a_unsafe.rs": "pub fn f() -> u8 { UNSAFE { 1 } }\n"})

    def test_control_the_exempt_path_may_hold_it(self):
        self.assertQuiet({"crates/frozen/src/lib.rs": "pub fn f() { UNSAFE { } }\n"})

    # -- F1, F2, F3 -------------------------------------------------------------------
    def test_F1_a_line_start_dereferenced_block_is_seen(self):
        self.assertFires("L1", {"crates/s/src/lib.rs": """\
            pub fn f(p: &u8) -> u8 {
                let x =
                    *UNSAFE { &*(p as *const u8) };
                x
            }
            """}, count=1)

    def test_F2_a_comment_marker_inside_a_string_does_not_hide_code(self):
        self.assertFires("L1", {"crates/s/src/lib.rs": 'pub fn f() -> u8 { let _s = ":12:// "; UNSAFE { 1 } }\n'},
                         count=1)

    def test_F3_comments_strings_and_raw_identifiers_are_not_the_keyword(self):
        self.assertQuiet({"crates/s/src/lib.rs": """\
            // UNSAFE in a comment
            /* outer /* inner */ UNSAFE still the outer comment */
            pub fn f() -> &'static str { "UNSAFE" }
            pub fn g() -> &'static str { r#"UNSAFE { }"# }
            pub fn r#UNSAFE() {}
            """})

    # -- F4, F9, F16 ------------------------------------------------------------------
    def test_F4_path_and_include_in_a_class_u_crate(self):
        got = self.assertFires("L4", {"crates/u/src/lib.rs": """\
            mod a_unsafe;
            #[path = "other.rs"]
            mod other;
            include!("x.rs");
            """, "crates/u/src/a_unsafe.rs": "", "crates/u/src/other.rs": ""})
        self.assertEqual(len(got), 2)

    def test_F4_control_include_bytes_is_allowed(self):
        self.assertQuiet({"crates/u/src/lib.rs": U_LIB + 'pub static B: &[u8] = include_bytes!("x.bin");\n',
                          "crates/u/src/a_unsafe.rs": ""})

    def test_F4_a_perimeter_file_reused_under_its_own_stem_is_allowed_and_a_rename_is_not(self):
        self.assertQuiet({"crates/u/src/lib.rs": U_LIB,
                          "crates/u/src/a_unsafe.rs": "",
                          "crates/u/src/bin/t/main.rs": '#[path = "../../a_unsafe.rs"]\nmod a_unsafe;\nfn main() {}\n'})
        self.assertFires("L4", {"crates/u/src/lib.rs": U_LIB,
                                "crates/u/src/a_unsafe.rs": "",
                                "crates/u/src/bin/t/main.rs": '#[path = "../../a_unsafe.rs"]\nmod safe_name;\nfn main() {}\n'})
        self.assertFires("L4", {"crates/u/src/lib.rs": U_LIB,
                                "crates/u/src/a_unsafe.rs": "",
                                "crates/u/src/bin/t/main.rs": '#[path = "../../a_unsafe.rs"]\nmod b_unsafe;\nfn main() {}\n'})

    def test_F9_an_out_of_line_mod_inside_a_perimeter_file(self):
        self.assertFires("L4", {"crates/u/src/lib.rs": U_LIB,
                                "crates/u/src/a_unsafe.rs": "mod hidden;\n",
                                "crates/u/src/a_unsafe/hidden.rs": ""})

    def test_F16_macro_use_on_a_perimeter_module(self):
        self.assertFires("L4", {"crates/u/src/lib.rs": "#[macro_use] mod a_unsafe;\n",
                                "crates/u/src/a_unsafe.rs": ""})

    def test_a_perimeter_file_must_be_declared_exactly_once(self):
        self.assertFires("L4", {"crates/u/src/lib.rs": "pub fn ok() {}\n",
                                "crates/u/src/a_unsafe.rs": ""})
        self.assertFires("L4", {"crates/u/src/lib.rs": "mod a_unsafe;\n",
                                "crates/u/src/main.rs": "mod a_unsafe;\nfn main() {}\n",
                                "crates/u/src/a_unsafe.rs": ""})

    # -- F5, F13 ----------------------------------------------------------------------
    def test_F5_a_perimeter_name_in_a_forbid_crate_at_depth_1_and_2(self):
        got = self.assertFires("L2", {"crates/s/src/lib.rs": "mod x_unsafe;\nmod d;\n",
                                      "crates/s/src/x_unsafe.rs": "",
                                      "crates/s/src/d/y_unsafe.rs": ""})
        self.assertEqual(len(got), 2)

    def test_L1_a_class_u_crate_outside_its_src_perimeter_files(self):
        got = self.assertFires("L1", {"crates/u/src/lib.rs": U_LIB + "pub fn g() { UNSAFE { } }\n",
                                      "crates/u/src/a_unsafe.rs": "",
                                      "crates/u/tests/t.rs": "#[test] fn t() { UNSAFE { } }\n"})
        self.assertEqual(sorted(f.path for f in got), ["crates/u/src/lib.rs", "crates/u/tests/t.rs"])

    def test_L1_a_perimeter_name_outside_src_does_not_license_the_keyword(self):
        got = self.assertFires("L1", {"crates/u/src/lib.rs": U_LIB, "crates/u/src/a_unsafe.rs": "",
                                      "crates/u/tests/t_unsafe.rs": "#[test] fn t() { UNSAFE { } }\n"})
        self.assertEqual([f.path for f in got], ["crates/u/tests/t_unsafe.rs"])

    def test_L1_a_class_p_perimeter_file_may_not_hold_it(self):
        self.assertFires("L1", {"crates/p/src/lib.rs": "mod a_unsafe;\n",
                                "crates/p/src/a_unsafe.rs": "pub fn f() { UNSAFE { } }\n"})

    def test_F5_a_perimeter_name_outside_src_of_a_class_u_crate(self):
        self.assertFires("L2", {"crates/u/src/lib.rs": "pub fn ok() {}\n",
                                "crates/u/tests/t_unsafe.rs": "#[test] fn t() {}\n"})

    def test_F13_nothing_named_target_is_excluded(self):
        got = self.assertFires("L1", {"crates/s/src/lib.rs": "mod target;\n",
                                      "crates/s/src/target/mod.rs": "pub fn f() { UNSAFE { } }\n"})
        self.assertEqual(got[0].path, "crates/s/src/target/mod.rs")

    def test_F13_a_class_u_file_under_src_target_is_a_perimeter_file(self):
        self.assertTrue(perimeter.is_perimeter_file("crates/u/src/target/x_unsafe.rs", cfg()))

    # -- F6 ---------------------------------------------------------------------------
    def test_F6_every_allow_shape_outside_line_one_of_a_perimeter_file(self):
        got = self.assertFires("L3", {
            "crates/u/src/lib.rs": "mod a_unsafe;\n#[allow(unsafe_code)]\npub fn f() {}\n",
            "crates/u/src/a_unsafe.rs": "pub fn x() {}\n#![allow(unsafe_code)]\n",
            "crates/u/src/b.rs": "#![allow(unsafe_code)]\n",
            "crates/u/src/c.rs": "#![cfg_attr(test, allow(unsafe_code))]\n",
            "crates/s/src/lib.rs": "#![forbid(unsafe_code)]\n",
        })
        self.assertEqual(sorted(f.path for f in got),
                         ["crates/u/src/a_unsafe.rs", "crates/u/src/b.rs", "crates/u/src/c.rs", "crates/u/src/lib.rs"])

    def test_F6_control_line_one_allow_in_a_class_u_perimeter_file(self):
        self.assertQuiet({"crates/u/src/lib.rs": "#![forbid(missing_docs)]\nmod a_unsafe;\n",
                          "crates/u/src/a_unsafe.rs": "//! docs\n#![allow(unsafe_code)]\npub fn x() {}\n"})

    def test_F6_a_line_one_allow_in_a_class_p_perimeter_file_fails(self):
        self.assertFires("L3", {"crates/p/src/lib.rs": "mod a_unsafe;\n",
                                "crates/p/src/a_unsafe.rs": "#![allow(unsafe_code)]\n"})

    def test_F6_forbid_outside_a_crate_root_fails(self):
        self.assertFires("L3", {"crates/s/src/lib.rs": "mod m;\n", "crates/s/src/m.rs": "#![deny(unsafe_code)]\n"})

    # -- F7 ---------------------------------------------------------------------------
    EIGHT = """\
        pub fn a() { UNSAFE{ } }
        pub UNSAFE extern "C-unwind" fn b() {}
        pub UNSAFE extern "system" fn c() {}
        #[UNSAFE(no_mangle)]
        pub extern "C" fn d() {}
        #[UNSAFE(export_name = "e2")]
        pub extern "C" fn e() {}
        #[UNSAFE(link_section = ".x")]
        pub static F: u8 = 0;
        UNSAFE extern "C" { fn g(); }
        pub UNSAFE trait H {}
        """

    def test_F7_the_shapes_the_old_ratchet_missed_are_each_seen_in_a_safe_file(self):
        self.assertFires("L1", {"crates/s/src/lib.rs": self.EIGHT}, count=8)

    def test_F7_global_asm_needs_no_keyword_so_the_compiler_gate_owns_it(self):
        s = rslex.Structure(rslex.tokenize('core::arch::global_asm!("nop");'))
        self.assertEqual([x.kind for x in rslex.unsafe_sites(s)], ["asm"])

    def test_F7_every_kind_is_classified(self):
        s = rslex.Structure(rslex.tokenize(self.EIGHT.replace("UNSAFE", K) + """
            struct S; impl S { pub UNSAFE fn m(&self) {} } UNSAFE impl Send for S {}
            type P = UNSAFE extern "C" fn(u8);
            core::arch::global_asm!("nop");
            """.replace("UNSAFE", K)))
        kinds = sorted(x.kind for x in rslex.unsafe_sites(s))
        self.assertEqual(kinds, sorted(["blocks", "unsafe_fn", "unsafe_fn", "unsafe_attrs", "unsafe_attrs",
                                        "unsafe_attrs", "extern_blocks", "extern_items", "unsafe_trait",
                                        "unsafe_method", "unsafe_impl", "fn_pointer_type", "asm"]))

    # -- F8 ---------------------------------------------------------------------------
    def test_F8_an_exported_macro_with_the_keyword_in_its_body(self):
        self.assertFires("L5", {"crates/u/src/lib.rs": U_LIB, "crates/u/src/a_unsafe.rs": """\
            #[macro_export]
            macro_rules! m { () => { UNSAFE { } } }
            """})

    def test_F8_a_macro_that_defines_an_exported_macro(self):
        self.assertFires("L5", {"crates/u/src/lib.rs": U_LIB, "crates/u/src/a_unsafe.rs": """\
            macro_rules! outer { ($n:ident) => {
                #[macro_export] macro_rules! $n { () => { UNSAFE { } } }
            } }
            """})

    def test_F8_control_a_crate_local_macro_in_a_perimeter_file(self):
        self.assertQuiet({"crates/u/src/lib.rs": U_LIB,
                          "crates/u/src/a_unsafe.rs": "macro_rules! m { () => { UNSAFE { } } }\n"})

    # -- L6 ---------------------------------------------------------------------------
    def test_L6_a_safe_item_in_an_extern_block(self):
        self.assertFires("L6", {"crates/u/src/lib.rs": U_LIB,
                                "crates/u/src/a_unsafe.rs": 'UNSAFE extern "C" { pub safe fn abs(x: i32) -> i32; }\n'})

    # -- F12 --------------------------------------------------------------------------
    def test_F12_an_unterminated_literal_fails_closed(self):
        self.assertFires("L0", {"crates/s/src/lib.rs": 'pub fn f() {}\npub const S: &str = r#"open;\n'})
        self.assertFires("L0", {"crates/s/src/lib.rs": "pub fn f() {} /* open\n"})
        self.assertFires("L0", {"crates/s/src/lib.rs": 'pub fn f() { let _ = "open; }\n'})

    # -- F14 --------------------------------------------------------------------------
    def test_F14_a_tracked_symlink_fails(self):
        entries = "100644 abc 0\tcrates/s/src/lib.rs\x00" "120000 def 0\tcrates/s/src/link.rs\x00"
        got = perimeter.symlink_findings(entries)
        self.assertEqual([(f.rule, f.path) for f in got], [("L0", "crates/s/src/link.rs")])
        self.assertEqual(perimeter.symlink_findings("100644 abc 0\tcrates/s/src/lib.rs\0"), [])

    # -- F15 --------------------------------------------------------------------------
    def test_F15_a_doctest_fence(self):
        got = self.assertFires("L10", {"crates/s/src/lib.rs": """\
            /// ```
            /// let x = UNSAFE { 1 };
            /// ```
            pub fn a() {}
            /// ```rust,no_run
            /// # UNSAFE fn hidden() {}
            /// ```
            pub fn b() {}
            /**
             * ```edition2024
             * UNSAFE { }
             * ```
             */
            pub fn c() {}
            """}, count=3)
        self.assertEqual(sorted(f.line for f in got), [2, 6, 11])

    def test_F15_control_a_text_fence(self):
        self.assertQuiet({"crates/s/src/lib.rs": """\
            /// ```text
            /// UNSAFE { }
            /// ```
            pub fn a() {}
            """})


def manifest_fixture(files: dict[str, str]) -> Fixture:
    return Fixture(files)


ROOT_TOML = """\
[workspace]
members = ["crates/u", "crates/s"]
[workspace.lints.rust]
unsafe_code = "forbid"
"""
U_TOML = """\
[package]
name = "u"
[lints.rust]
unsafe_code = "allow"
[lints.clippy]
undocumented_unsafe_blocks = "deny"
"""
S_TOML = "[package]\nname = \"s\"\n[lints]\nworkspace = true\n"
GEN_TOML = "[package]\nname = \"gen\"\n[workspace]\n[lints.rust]\nunsafe_code = \"forbid\"\n"


class ManifestTests(unittest.TestCase):
    def manifest(self, files: dict[str, str], members=("crates/u", "crates/s"), c=None, workflows=None,
                 configs=None) -> list[perimeter.Finding]:
        base = {"Cargo.toml": ROOT_TOML, "crates/u/Cargo.toml": U_TOML, "crates/s/Cargo.toml": S_TOML,
                "crates/u/gen/Cargo.toml": GEN_TOML}
        base.update(files)
        fx = Fixture(base)
        try:
            manifests = sorted(k for k in base if k.endswith("Cargo.toml"))
            return perimeter.run_manifest(fx.root, c or cfg(), list(members), manifests,
                                          configs or [], workflows or [])
        finally:
            fx.close()

    def test_control_the_fixture_workspace_passes(self):
        self.assertEqual(self.manifest({}), [])

    def test_MF1_the_s1_01_manifest(self):
        got = self.manifest({"crates/s/Cargo.toml": "[package]\nname = \"s\"\n[lints]\n"
                                                    "[dependencies.x]\nworkspace = true\n"})
        self.assertEqual(rules(got), ["M1"])

    def test_MF2_a_class_u_manifest_missing_a_lint(self):
        got = self.manifest({"crates/u/Cargo.toml": "[package]\nname = \"u\"\n[lints.rust]\nunsafe_code = \"allow\"\n"})
        self.assertEqual(rules(got), ["M3"])

    def test_MF2_a_standalone_manifest_relaxing_its_forbid(self):
        got = self.manifest({"crates/u/gen/Cargo.toml": GEN_TOML.replace("forbid", "allow")})
        self.assertEqual(rules(got), ["M3"])

    def test_M2_the_workspace_forbid(self):
        got = self.manifest({"Cargo.toml": ROOT_TOML.replace("forbid", "deny")})
        self.assertEqual(rules(got), ["M2"])

    def test_MF3_a_package_that_is_neither_member_nor_standalone(self):
        got = self.manifest({"tools/x/Cargo.toml": S_TOML.replace('"s"', '"x"')})
        self.assertEqual(rules(got), ["M1"])

    def test_MF3_a_member_outside_crates_without_workspace_lints(self):
        got = self.manifest({"tools/x/Cargo.toml": "[package]\nname = \"x\"\n"},
                            members=("crates/u", "crates/s", "tools/x"))
        self.assertEqual(rules(got), ["M1"])

    def test_MF4_cargo_config_rustflags_and_a_workflow_cap_lints(self):
        got = self.manifest({".cargo/config.toml": '[build]\nrustflags = ["--cap-lints", "warn"]\n',
                             ".github/workflows/x.yml": "jobs:\n  a:\n    env:\n      RUSTFLAGS: --cap-lints warn\n"
                                                       "    steps:\n      - run: true\n        env: { RUSTC_WRAPPER: x }\n"},
                            configs=[".cargo/config.toml"], workflows=[".github/workflows/x.yml"])
        self.assertEqual(rules(got), ["M4"])
        self.assertEqual(len(got), 3, [str(g) for g in got])

    def test_MF4_control_a_workflow_env_without_compiler_flags(self):
        got = self.manifest({".github/workflows/x.yml": "env:\n  CARGO_TERM_COLOR: always\n"},
                            workflows=[".github/workflows/x.yml"])
        self.assertEqual(got, [])

    def test_M0_the_toolchain_pin_agrees_everywhere(self):
        c = cfg(toolchain="1.99.0")
        got = self.manifest({"rust-toolchain.toml": '[toolchain]\nchannel = "1.98.1"\n',
                             ".github/workflows/x.yml": "steps:\n  - with:\n      toolchain: 1.98.1   # old\n"},
                            c=c, workflows=[".github/workflows/x.yml"])
        self.assertEqual([(g.rule, g.path) for g in got],
                         [("M0", ".github/workflows/x.yml"), ("M0", "rust-toolchain.toml")])
        ok = self.manifest({"rust-toolchain.toml": '[toolchain]\nchannel = "1.99.0"\n',
                            ".github/workflows/x.yml": "steps:\n  - with:\n      toolchain: 1.99.0\n"},
                           c=c, workflows=[".github/workflows/x.yml"])
        self.assertEqual(ok, [])

    def test_MF5_a_proc_macro_package(self):
        got = self.manifest({"crates/s/Cargo.toml": S_TOML + "[lib]\nproc-macro = true\n"})
        self.assertEqual(rules(got), ["M5"])

    def test_MF6_an_unpinned_build_script(self):
        got = self.manifest({"crates/s/build.rs": "fn main() {}\n"})
        self.assertEqual(rules(got), ["M4"])

    def test_MF6_a_changed_build_script(self):
        c = cfg(build_scripts={"crates/s/build.rs": "0" * 64})
        got = self.manifest({"crates/s/build.rs": "fn main() {}\n"}, c=c)
        self.assertEqual([g.msg[:20] for g in got], ["build script changed"])

    def test_MF7_a_frozen_file_hash_change(self):
        fx = Fixture({"crates/kayfabe-doorbell/Cargo.toml": "[package]\n", "crates/kayfabe-doorbell/src/lib.rs": "x"})
        saved = debt.ROOT
        try:
            debt.ROOT = fx.root
            before = debt.frozen()
            (fx.root / "crates/kayfabe-doorbell/src/lib.rs").write_text("y")
            self.assertNotEqual(before, debt.frozen())
        finally:
            debt.ROOT = saved
            fx.close()


def meta_fixture(root: str, pkgs, edges, members, targets=None, sources=None):
    targets = targets or {}
    sources = sources or {}
    packages, nodes = [], []
    for name, rel in pkgs:
        manifest = f"{root}/{rel}/Cargo.toml" if rel else f"/reg/{name}/Cargo.toml"
        src = sources.get(name, None if rel else "registry+https://github.com/rust-lang/crates.io-index")
        t = targets.get(name, [{"kind": ["lib"], "name": name, "src_path": manifest.replace("Cargo.toml", "src/lib.rs")}])
        packages.append({"id": f"{name}-id", "name": name, "manifest_path": manifest, "source": src, "targets": t})
        nodes.append({"id": f"{name}-id", "features": [],
                      "deps": [{"pkg": f"{b}-id", "dep_kinds": [{"kind": k, "target": None}]}
                               for a, b, k in edges if a == name]})
    return {"packages": packages, "workspace_members": [f"{m}-id" for m in members], "resolve": {"nodes": nodes}}


class MetadataTests(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory()
        self.root = Path(self.tmp.name)
        for d in ("crates/u", "crates/s", "tools/stray"):
            (self.root / d / "src").mkdir(parents=True)

    def tearDown(self):
        self.tmp.cleanup()

    def c(self, **ext):
        e = {"packages": ["libc", "trybuild", "serde_derive"],
             "edges": ["u -> libc (normal)", "u -> trybuild (dev)"],
             "proc_macros_reachable": ["serde_derive"]}
        e.update(ext)
        return cfg(external=e, standalone=[])

    def base(self, pkgs=(), edges=(), members=("u", "s"), targets=None, sources=None):
        r = str(self.root)
        p = [("u", "crates/u"), ("s", "crates/s"), ("libc", None), ("trybuild", None), ("serde_derive", None), *pkgs]
        e = [("u", "libc", None), ("u", "trybuild", "dev"), ("trybuild", "serde_derive", None), *edges]
        t = {"serde_derive": [{"kind": ["proc-macro"], "name": "serde_derive", "src_path": "/reg/sd/src/lib.rs"}]}
        t.update(targets or {})
        return meta_fixture(r, p, e, members, t, sources)

    def run_meta(self, meta, c=None):
        return perimeter.metadata_violations(self.root, c or self.c(), meta, {})

    def test_control_the_reviewed_graph_passes(self):
        self.assertEqual(self.run_meta(self.base()), [])

    def test_M7_a_new_external_package_and_edge(self):
        got = self.run_meta(self.base(pkgs=[("memchr", None)], edges=[("s", "memchr", None)]))
        self.assertEqual(rules(got), ["M7"])
        self.assertEqual(len(got), 2)

    def test_MF5_a_direct_proc_macro_dependency(self):
        got = self.run_meta(self.base(pkgs=[("thiserror_impl", None)], edges=[("s", "thiserror_impl", None)],
                                      targets={"thiserror_impl": [{"kind": ["proc-macro"], "name": "t",
                                                                   "src_path": "/reg/t/src/lib.rs"}]}),
                            self.c(packages=["libc", "trybuild", "serde_derive", "thiserror_impl"],
                                   edges=["u -> libc (normal)", "u -> trybuild (dev)", "s -> thiserror_impl (normal)"]))
        self.assertEqual([g.rule for g in got], ["M7"])
        self.assertIn("thiserror_impl", got[0].msg)

    def test_MF8_a_proc_macro_reached_through_a_re_exporting_crate(self):
        got = self.run_meta(self.base(pkgs=[("facade", None), ("hidden_derive", None)],
                                      edges=[("s", "facade", None), ("facade", "hidden_derive", None)],
                                      targets={"hidden_derive": [{"kind": ["proc-macro"], "name": "h",
                                                                  "src_path": "/reg/h/src/lib.rs"}]}),
                            self.c(packages=["libc", "trybuild", "serde_derive", "facade", "hidden_derive"],
                                   edges=["u -> libc (normal)", "u -> trybuild (dev)", "s -> facade (normal)"]))
        self.assertEqual([g.rule for g in got], ["M7"])
        self.assertIn("hidden_derive", got[0].msg)

    def test_M7_a_git_source(self):
        got = self.run_meta(self.base(sources={"libc": "git+https://example.invalid/libc"}))
        self.assertIn("M7", rules(got))

    def test_MF9_a_non_member_path_package(self):
        got = self.run_meta(self.base(pkgs=[("stray", "tools/stray")], edges=[("s", "stray", None)]))
        self.assertEqual(rules(got), ["M8"])

    def test_MF9_a_borrowed_source_path(self):
        borrowed = [{"kind": ["lib"], "name": "s", "src_path": str(self.root / "crates/u/src/lib.rs")}]
        got = self.run_meta(self.base(targets={"s": borrowed}))
        self.assertEqual(rules(got), ["M9"])


def row(**kw) -> dict:
    r = {c: 0 for c in perimeter.SIZE_COLUMNS}
    r.update(kw)
    return r


class SizeTests(unittest.TestCase):
    """G6 (§2): exact counts, every perimeter file has a row, a rise carries a dated reason."""

    A = "crates/u/src/a_unsafe.rs"
    OK = "2026-10-04: baseline — fixture"

    def test_SF2_code_moves_without_a_tsv_change(self):
        for delta in (1, -1):
            got = perimeter.size_findings({self.A: row(code=10 + delta)}, {self.A: {**row(code=10), "reason": self.OK}},
                                          None)
            self.assertEqual([g.rule for g in got], ["SF2"])

    def test_SF3_a_file_without_a_row_and_a_stale_row(self):
        got = perimeter.size_findings({self.A: row(code=1)}, {"crates/u/src/gone_unsafe.rs": {**row(), "reason": self.OK}},
                                      None)
        self.assertEqual(sorted(g.rule for g in got), ["SF3", "SF3"])

    def test_SF1_first_landing_needs_a_dated_baseline_reason(self):
        got = perimeter.size_findings({self.A: row(code=1)}, {self.A: {**row(code=1), "reason": "because"}}, None)
        self.assertEqual([g.rule for g in got], ["SF1"])
        self.assertEqual(perimeter.size_findings({self.A: row(code=1)}, {self.A: {**row(code=1), "reason": self.OK}},
                                                 None), [])

    def test_SF1_a_rise_with_an_unchanged_reason(self):
        base = {self.A: {**row(code=10, blocks=1), "reason": self.OK}}
        got = perimeter.size_findings({self.A: row(code=12, blocks=2)}, {self.A: {**row(code=12, blocks=2),
                                                                                 "reason": self.OK}}, base)
        self.assertEqual([g.rule for g in got], ["SF1"])

    def test_SF1_a_rise_reusing_the_previous_reason_verbatim(self):
        prior = "2026-10-05: code+2 blocks+1 — an earlier rise"
        base = {self.A: {**row(code=10, blocks=1), "reason": prior}}
        got = perimeter.size_findings({self.A: row(code=12, blocks=2)},
                                      {self.A: {**row(code=12, blocks=2), "reason": prior}}, base)
        self.assertEqual([g.rule for g in got], ["SF1"])

    def test_SF1_first_landing_reason_must_say_baseline(self):
        got = perimeter.size_findings({self.A: row(code=1)},
                                      {self.A: {**row(code=1), "reason": "2026-10-04: code+1 — new"}}, None)
        self.assertEqual([g.rule for g in got], ["SF1"])

    def test_SF1_a_rise_missing_one_token(self):
        base = {self.A: {**row(code=10, blocks=1), "reason": self.OK}}
        new = {**row(code=12, blocks=2), "reason": "2026-10-05: blocks+1 — a new mmap"}
        got = perimeter.size_findings({self.A: row(code=12, blocks=2)}, {self.A: new}, base)
        self.assertEqual([g.rule for g in got], ["SF1"])

    def test_SF1_control_a_rise_with_every_token(self):
        base = {self.A: {**row(code=10, blocks=1), "reason": self.OK}}
        new = {**row(code=12, blocks=2), "reason": "2026-10-05: code+2 blocks+1 — a new mmap"}
        self.assertEqual(perimeter.size_findings({self.A: row(code=12, blocks=2)}, {self.A: new}, base), [])

    def test_SF1_a_decrease_needs_only_the_number(self):
        base = {self.A: {**row(code=10, blocks=2), "reason": self.OK}}
        self.assertEqual(perimeter.size_findings({self.A: row(code=9, blocks=1)},
                                                 {self.A: {**row(code=9, blocks=1), "reason": self.OK}}, base), [])

    def test_SF1_a_new_row_after_the_baseline_needs_its_tokens(self):
        base = {"crates/u/src/b_unsafe.rs": {**row(code=1), "reason": self.OK}}
        stored = {**base, self.A: {**row(code=3, blocks=1), "reason": "2026-10-05: code+3 — new"}}
        actual = {"crates/u/src/b_unsafe.rs": row(code=1), self.A: row(code=3, blocks=1)}
        self.assertEqual([g.rule for g in perimeter.size_findings(actual, stored, base)], ["SF1"])
        stored[self.A]["reason"] = "2026-10-05: code+3 blocks+1 — new"
        self.assertEqual(perimeter.size_findings(actual, stored, base), [])

    def test_the_tsv_round_trips(self):
        rows = {self.A: {**row(code=3, blocks=1), "reason": self.OK}}
        self.assertEqual(perimeter.read_tsv(perimeter.write_tsv(rows)), rows)

    def measure(self, files: dict[str, str]) -> dict[str, dict]:
        fx = Fixture(files)
        try:
            tree = perimeter.Tree(fx.root, fx.files)
            return perimeter.size_rows(tree, cfg(), [f for f in files if f.endswith((".c", ".h"))])
        finally:
            fx.close()

    def test_code_excludes_cfg_test_items_and_counts_kinds_and_macros(self):
        rows = self.measure({"crates/u/src/lib.rs": U_LIB, self.A: """\
            macro_rules! m { () => { UNSAFE { 1 } } }
            pub fn a() -> u8 { m!() + m!() + UNSAFE { 2 } }
            // a comment line is not code
            #[cfg(test)]
            mod tests {
                fn t() { UNSAFE { } }
            }
            """})
        r = rows[self.A]
        self.assertEqual((r["code"], r["blocks"], r["macro_unsafe"]), (2, 3, 2))

    def test_F13_a_file_under_src_target_is_counted(self):
        rows = self.measure({"crates/u/src/lib.rs": "mod target;\n", "crates/u/src/target/mod.rs": "mod x_unsafe;\n",
                             "crates/u/src/target/x_unsafe.rs": "pub fn f() { UNSAFE { } }\n"})
        self.assertEqual(rows["crates/u/src/target/x_unsafe.rs"]["blocks"], 1)

    def test_c_code_lines_strip_comments_but_not_strings(self):
        rows = self.measure({"k.c": '/* a\n b */\nint x; // c\n// only a comment\n\nchar *s = "/*";\nint y;\n/* real */\n'})
        self.assertEqual(rows["k.c"]["code"], 3)

    def test_F10_a_new_caller_obligation_comment_in_a_safe_fn(self):
        fx = Fixture({"crates/u/src/lib.rs": U_LIB, self.A: """\
            pub fn f(p: *const u8) -> u8 {
                // SAFETY: every caller passes a live pointer.
                UNSAFE { *p }
            }
            pub UNSAFE fn g(p: *const u8) -> u8 {
                // SAFETY: every caller passes a live pointer (the # Safety contract).
                UNSAFE { *p }
            }
            pub fn h() {
                // SAFETY: the CALLING thread; no caller memory is read.
                UNSAFE { }
            }
            """})
        try:
            sites = perimeter.l8_sites(perimeter.Tree(fx.root, fx.files), cfg())
        finally:
            fx.close()
        self.assertEqual([(f, fn) for f, fn, _ in sites], [(self.A, "f")])
        got = perimeter.l8_findings(sites, [])
        self.assertEqual([g.rule for g in got], ["L8"])
        self.assertEqual(perimeter.l8_findings(sites, [f"{self.A}::f"]), [])
        self.assertEqual([g.rule for g in perimeter.l8_findings([], [f"{self.A}::f"])], ["L8"])

    def test_F11_a_new_mint_site_and_an_alias(self):
        c = cfg(mint={"names": ["Nvos46Parameters"]})
        fx = Fixture({"crates/k/src/lib.rs": """\
            use abi::Nvos46Parameters as P;
            pub fn f() -> P { P::default() }
            #[cfg(test)]
            mod tests { fn t() -> super::P { todo!() } }
            """, "crates/u/src/a_unsafe.rs": "pub fn g() -> abi::Nvos46Parameters { todo!() }\n",
               "crates/abi2/src/lib.rs": "pub struct Nvos46Parameters;\n"})
        try:
            sites = perimeter.mint_sites(perimeter.Tree(fx.root, fx.files), c, ["crates/k", "crates/u"])
        finally:
            fx.close()
        self.assertEqual({f: len(v) for f, v in sites.items()}, {"crates/k/src/lib.rs": 4})
        self.assertEqual([g.rule for g in perimeter.mint_findings(sites, {"crates/k/src/lib.rs": 3})], ["L9"])
        self.assertEqual([g.rule for g in perimeter.mint_findings(sites, {"crates/k/src/lib.rs": 5})], ["L9"])
        self.assertEqual(perimeter.mint_findings(sites, {"crates/k/src/lib.rs": 4}), [])


def diag(file, line, message="usage of an `unsafe` block", callsites=(), code="unsafe_code"):
    return {"code": code, "level": "warning", "message": message, "file": file, "line": line, "col": 5,
            "callsites": [list(c) for c in callsites]}


def unit(manifest_dir, diags=(), klass="U", src=None, test=False, pass_="x86_64"):
    return {"unit": {"pass": pass_, "manifest_dir": manifest_dir, "class": klass, "crate_name": "c",
                     "src": src or f"{manifest_dir}/src/lib.rs", "test": test, "rc": 0},
            "diags": list(diags)}


class LocationTests(unittest.TestCase):
    """G1's verdict over a synthetic wrapper log (the cargo-driven W cases are in
    selftest_compiler_location.py)."""

    def verdict(self, recs, frozen=True):
        return perimeter.location_findings(recs, cfg(), frozen)

    def test_a_perimeter_file_of_the_units_own_class_u_crate_passes(self):
        got, st = self.verdict([unit("crates/u", [diag("crates/u/src/a_unsafe.rs", 3)])])
        self.assertEqual((got, st["outside"]), ([], 0))

    def test_a_safe_file_fails(self):
        got, _ = self.verdict([unit("crates/u", [diag("crates/u/src/lib.rs", 3)])])
        self.assertEqual([(g.rule, g.path, g.line) for g in got], [("G1", "crates/u/src/lib.rs", 3)])

    def test_another_packages_perimeter_file_fails(self):
        got, _ = self.verdict([unit("crates/b", [diag("crates/u/src/a_unsafe.rs", 3)], klass="F")])
        self.assertEqual(len(got), 1)

    def test_a_safe_call_site_of_a_perimeter_macro_fails(self):
        got, _ = self.verdict([unit("crates/u", [diag("crates/u/src/a_unsafe.rs", 1,
                                                       callsites=[("crates/u/src/lib.rs", 9, 1, "m!")])])])
        self.assertIn("via crates/u/src/lib.rs:9", got[0].msg)

    def test_E0453_always_fails(self):
        got, _ = self.verdict([unit("crates/s", [diag("crates/s/src/lib.rs", 1, "allow(unsafe_code) incompatible",
                                                      code="E0453")], klass="F")])
        self.assertIn("E0453", got[0].msg)

    def test_the_exempt_path_needs_the_frozen_binding(self):
        recs = [unit("crates/frozen", [diag("crates/frozen/src/lib.rs", 2)], klass="exempt")]
        self.assertEqual(self.verdict(recs, frozen=True)[1]["exempt"], 1)
        self.assertEqual(len(self.verdict(recs, frozen=False)[0]), 1)

    def test_SF4_a_compiler_count_above_the_tokenizers(self):
        recs = [unit("crates/u", [diag("crates/u/src/a_unsafe.rs", 3), diag("crates/u/src/a_unsafe.rs", 7),
                                  diag("crates/u/src/a_unsafe.rs", 7)])]
        counts = perimeter.compiler_counts(recs)
        self.assertEqual(len(counts["crates/u/src/a_unsafe.rs"]["blocks"]), 2)  # deduplicated
        actual = {"crates/u/src/a_unsafe.rs": row(blocks=1)}
        self.assertEqual([g.rule for g in perimeter.cross_check(actual, counts)], ["SF4"])
        actual = {"crates/u/src/a_unsafe.rs": row(blocks=2)}
        self.assertEqual(perimeter.cross_check(actual, counts), [])

    def test_SF4_macro_expansions_count_against_macro_unsafe(self):
        recs = [unit("crates/u", [diag("crates/u/src/a_unsafe.rs", 1, callsites=[("crates/u/src/a_unsafe.rs", 9, c, "m!")])
                                  for c in (1, 20, 40)])]
        counts = perimeter.compiler_counts(recs)
        self.assertEqual([g.rule for g in perimeter.cross_check({"crates/u/src/a_unsafe.rs": row(blocks=1, macro_unsafe=2)},
                                                                counts)], ["SF4"])
        self.assertEqual(perimeter.cross_check({"crates/u/src/a_unsafe.rs": row(blocks=1, macro_unsafe=3)}, counts), [])

    def test_every_compiler_message_maps_to_a_kind(self):
        for msg, kind in [("usage of an `unsafe` block", "blocks"),
                          ("declaration of an `unsafe` function", "unsafe_fn"),
                          ("implementation of an `unsafe` method", "unsafe_method"),
                          ("declaration of an `unsafe` method", "unsafe_method"),
                          ("implementation of an `unsafe` trait", "unsafe_impl"),
                          ("declaration of an `unsafe` trait", "unsafe_trait"),
                          ("usage of an `unsafe extern` block", "extern_blocks"),
                          ("usage of the unsafe `no_mangle` attribute", "unsafe_attrs"),
                          ("usage of `core::arch::global_asm`", "asm")]:
            self.assertEqual(perimeter.diag_kind(msg), kind)

    def test_reached_names_a_unit_that_never_ran(self):
        meta = {"workspace_members": ["u-id"], "packages": [{"id": "u-id", "name": "u", "features": {}, "targets": [
            {"kind": ["lib"], "name": "u", "src_path": "/r/crates/u/src/lib.rs", "test": True},
            {"kind": ["test"], "name": "t", "src_path": "/r/crates/u/tests/t.rs", "test": True},
            {"kind": ["bin"], "name": "x", "src_path": "/r/crates/u/src/bin/x.rs", "test": True,
             "required-features": ["extra"]}]}]}
        args = "--workspace --all-targets".split()
        recs = [unit("crates/u", src="crates/u/src/lib.rs"), unit("crates/u", src="crates/u/src/lib.rs", test=True)]
        got, total = perimeter.reached_findings(recs, {"x86_64": (meta, args)}, Path("/r"))
        self.assertEqual([g.path for g in got], ["crates/u/tests/t.rs"])
        recs.append(unit("crates/u", src="crates/u/tests/t.rs", test=True))
        got, total = perimeter.reached_findings(recs, {"x86_64": (meta, args)}, Path("/r"))
        self.assertEqual((got, total), ([], 3))


class StructureTests(unittest.TestCase):
    """The tokenizer's own edge cases; each is a way a gate could mis-read a file."""

    def toks(self, src):
        return [(t.kind, t.text) for t in rslex.tokenize(src) if t.is_code]

    def test_lifetimes_versus_char_literals(self):
        self.assertEqual(self.toks("fn f<'a>(x: &'a u8) -> char { 'x' }")[2:4], [("punct", "<"), ("lifetime", "'a")])
        self.assertIn(("char", "'\\''"), self.toks("let c = '\\'';"))
        self.assertIn(("char", "b'x'"), self.toks("let c = b'x';"))

    def test_raw_strings_with_hashes_hold_quotes(self):
        self.assertEqual(self.toks('let s = r##"a "# b"##;')[3], ("str", 'r##"a "# b"##'))

    def test_byte_and_c_strings(self):
        kinds = [k for k, _ in self.toks('b"x" c"y" br"z" cr#"w"#')]
        self.assertEqual(kinds, ["str"] * 4)

    def test_doc_comments_are_kept_as_text(self):
        t = [x for x in rslex.tokenize("/// a\n//! b\n//// c\n/** d */\n/*! e */\n/**/") if not x.is_code]
        self.assertEqual([x.kind for x in t], ["doc", "doc", "comment", "doc", "doc", "comment"])

    def test_items_find_impl_methods_and_cfg_test(self):
        s = rslex.Structure(rslex.tokenize("impl A { fn m(&self) {} }\n#[cfg(test)]\nmod tests { fn t() {} }\n"))
        m = [it for it in s.items if it.kind == "fn" and it.name == "m"][0]
        self.assertEqual(m.parent.kind, "impl")
        self.assertEqual(len(s.test_ranges()), 1)

    def test_findings_are_json_serializable_for_logs(self):
        f = perimeter.Finding("L1", "a.rs", 1, "m")
        self.assertEqual(json.loads(json.dumps(f.__dict__))["rule"], "L1")


if __name__ == "__main__":
    unittest.main()
