#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later
"""Known positives for G5, the export table, through rustdoc (V3_SEC_PERIMETER.md §3.6).

Each case is a one-crate fixture workspace (its own git repo, perimeter.toml and table). The
exports gate runs the real rustdoc JSON generator over it, and the case passes only if the gate
ends the way it must: the control with zero findings, every violation with ITS OWN rule (and
marker) in the output. Not named test_*.py: it needs cargo and rustdoc, which the discover step
lacks.

Run: python3 scripts/ci/selftest_perimeter_cargo.py
"""

from __future__ import annotations

import os
import shutil
import subprocess
import sys
import tempfile
import textwrap
from pathlib import Path

HERE = Path(__file__).resolve().parent
K = "un" + "safe"

PERIMETER = """\
format = 1
toolchain = "TOOLCHAIN"
rustdoc_format = FORMAT
crates = [{ path = "crates/u", class = "U", kf3 = true }]
[cfg]
allowed = ['test', 'target_arch = "x86_64"']
[skip_guards]
macros = ["require_kvm"]
[exports]
targets = ["x86_64-unknown-linux-gnu"]
e9_baseline = E9
"""
ROOT_TOML = '[workspace]\nresolver = "2"\nmembers = ["crates/u"]\n'
U_TOML = '[package]\nname = "u"\nversion = "0.1.0"\nedition = "2024"\n[lints.rust]\nunsafe_code = "allow"\n'
LIB = "//! fixture\nmod a_unsafe;\npub mod bounds;\npub use a_unsafe::*;\n"
BOUNDS = "/// In range.\npub fn in_range(len: usize, i: usize) -> bool {\n    i < len\n}\n"
A = """\
//! fixture perimeter
use crate::bounds::in_range;
/// Reads byte `i`, refusing an out-of-range index.
pub fn checked(x: &[u8], i: usize) -> Option<u8> {
    if !in_range(x.len(), i) {
        return None;
    }
    // SAFETY: `i < x.len()` was checked above.
    Some(UNSAFE { *x.get_unchecked(i) })
}
EXTRA
#[cfg(test)]
mod tests {
    #[test]
    fn refuses_out_of_range() {
        assert_eq!(super::checked(&[1], 1), None);
    }
    TESTS
}
"""
ROW_CHECKED = "| `checked` | pub | safe fn | range | range=t:crates/u/src/a_unsafe.rs::refuses_out_of_range | OK |"
VD = "\n## Validation dependencies\n\n| file | role | why |\n|---|---|---|\n| `crates/u/src/bounds.rs` | VALIDATES | in_range |\n"


def table(rows: list[str], vd: str = VD) -> str:
    return ("# fixture\n\n## crates/u/src/a_unsafe.rs\n\n| item | vis | kind | checks | tests | status |\n"
            "|---|---|---|---|---|---|\n" + "\n".join(rows) + "\n" + vd)


def open_row(item: str, vis: str = "pub", kind: str = "safe fn") -> str:
    return f"| `{item}` | {vis} | {kind} |  |  | OPEN: 2026-10-04: fixture |"


# (name, extra source, extra tests, table rows, expected marker substrings, overrides)
CASES = [
    ("control", "", "", [ROW_CHECKED], ["PERIMETER_EXPORTS findings=0"], {}),
    ("EF1_a_new_pub_fn_with_no_row_and_a_stale_row",
     "/// x\npub fn extra() {}\n", "", [ROW_CHECKED, open_row("gone")],
     ["E1 docs/design/PERIMETER_EXPORTS.md:0: add a row under `## crates/u/src/a_unsafe.rs`: | `extra`",
      "remove the row `gone`"], {}),
    ("EF2_empty_checks", "", "", [ROW_CHECKED.replace("| range | range=", "|  | range=")],
     ["E3 ", "is OK with an empty checks cell"], {}),
    ("EF2_a_label_with_no_test", "", "", [ROW_CHECKED.replace("| range |", "| range; overflow |")],
     ["check `overflow` has no t:/ui: test"], {}),
    ("EF2_a_ref_that_resolves_to_nothing", "", "", [ROW_CHECKED.replace("::refuses_out_of_range", "::nope")],
     ["no fn `nope`"], {}),
    ("EF2b_a_test_that_never_names_the_item", "", "#[test]\n    fn unrelated() { assert!(true); }",
     [ROW_CHECKED.replace("::refuses_out_of_range", "::unrelated")], ["E3b ", "never names `checked`"], {}),
    ("EF3_an_unsafe_fn_with_no_safety_heading",
     "/// No contract stated.\npub UNSAFE fn raw(p: *const u8) -> u8 {\n    UNSAFE { *p }\n}\n", "",
     [ROW_CHECKED, "| `raw` | pub | unsafe fn | contract | contract=t:crates/u/src/a_unsafe.rs::refuses_out_of_range | OK |"],
     ["E4 ", "has no `# Safety` heading"], {}),
    ("EF4_a_drop_type_that_derives_clone",
     "/// h\n#[derive(Clone)]\npub struct H {\n    fd: i32,\n}\nimpl Drop for H {\n    fn drop(&mut self) {}\n}\n", "",
     [ROW_CHECKED, "| `H` | pub | owning handle |  |  | OK |", open_row("<H as Clone>", "default", "trait impl"),
      open_row("<H as Clone>::clone", "default"), open_row("<H as Drop>", "default", "trait impl"),
      open_row("<H as Drop>::drop", "default"), open_row("H: Send", kind="auto trait"),
      open_row("H: Sync", kind="auto trait")],
     ["E5 ", "implements ['Clone']"], {}),
    ("EF5_plain_data_on_a_raw_pointer_type",
     "/// p\npub struct P {\n    p: *mut u8,\n}\n", "",
     [ROW_CHECKED, "| `P` | pub | plain data |  |  | OK |"], ["E6 ", "carries an address: never `plain data`"], {}),
    ("EF6_an_address_carrying_type_with_a_pub_field",
     "/// a\npub struct Ad {\n    /// the address\n    pub addr: u64,\n}\n", "",
     [ROW_CHECKED, "| `Ad` | pub | owning handle |  |  | OK |", open_row("Ad: Send", kind="auto trait"),
      open_row("Ad: Sync", kind="auto trait")],
     ["E6 ", "has non-private fields ['addr']"], {}),
    ("EF7_a_borrowed_view_with_no_lifetime",
     "/// v\npub struct Vw {\n    x: u8,\n}\n", "",
     [ROW_CHECKED, "| `Vw` | pub | borrowed view |  |  | OK |", open_row("Vw: Send", kind="auto trait"),
      open_row("Vw: Sync", kind="auto trait")],
     ["E8 ", "has no lifetime parameter"], {}),
    ("EF8_an_e9_entry_above_the_baseline",
     "/// e\npub extern \"C\" fn e(p: *mut u8) -> u8 {\n    p.is_null() as u8\n}\n", "",
     [ROW_CHECKED, open_row("e", kind="safe extern fn")], ["E9 ", "the baseline is 0"], {}),
    ("EF9_an_unlisted_validation_dependency", "", "", [ROW_CHECKED],
     ["E11 ", "add `crates/u/src/bounds.rs` to Validation dependencies"], {"vd": "\n## Validation dependencies\n"}),
    ("EF10_a_format_version_mismatch", "", "", [ROW_CHECKED], ["EF10: rustdoc JSON format_version 61 != pinned 60"],
     {"format": "60"}),
    ("EF11_doc_hidden_items_and_modules",
     "/// h\n#[doc(hidden)]\npub fn hidden() {}\n/// m\n#[doc(hidden)]\npub mod hm {\n    /// i\n    pub fn inner() {}\n}\n",
     "", [ROW_CHECKED], ["add a row under `## crates/u/src/a_unsafe.rs`: | `hidden`",
                         "add a row under `## crates/u/src/a_unsafe.rs`: | `inner`"], {}),
    ("EF12_a_provided_trait_method_with_no_row",
     "/// t\npub trait Tr {\n    /// p\n    fn provided(&self) {}\n}\n", "",
     [ROW_CHECKED, open_row("trait Tr", kind="trait")], ["add a row under `## crates/u/src/a_unsafe.rs`: | `Tr::provided`"], {}),
    ("EF13_a_blanket_impl_and_an_impl_for_a_foreign_type",
     "/// t\npub trait Tr {}\nimpl<X: Copy> Tr for X {}\n/// t2\npub trait Tr2 {}\nimpl Tr2 for Vec<u8> {}\n", "",
     [ROW_CHECKED, open_row("trait Tr", kind="trait"), open_row("trait Tr2", kind="trait")],
     ["| `<X as Tr>` | default | trait impl", "| `<Vec<u8> as Tr2>` | default | trait impl"], {}),
    ("EF14_a_debug_assertions_cfg",
     "/// nd\n#[cfg(not(debug_assertions))]\npub fn nd() {}\n", "", [ROW_CHECKED],
     ["E1c crates/u/src/a_unsafe.rs:", "`debug_assertions`"], {}),
    ("EF15_an_ok_row_citing_only_a_skip_guarded_test", "macro_rules! require_kvm { () => {} }\n",
     "#[test]\n    fn guarded() {\n        require_kvm!();\n        assert_eq!(super::checked(&[1], 1), None);\n    }",
     [ROW_CHECKED.replace("::refuses_out_of_range", "::guarded")], ["E3d ", "is skip-guarded"], {}),
    ("EF16_a_generic_export_without_untrusted_impls",
     "/// g\npub fn generic_len<T: AsRef<[u8]>>(x: T) -> usize {\n    x.as_ref().len()\n}\n",
     "#[test]\n    fn generic_len_counts() {\n        assert_eq!(super::generic_len([1u8]), 1);\n    }",
     [ROW_CHECKED, "| `generic_len` | pub | safe fn | len | len=t:crates/u/src/a_unsafe.rs::generic_len_counts | OK |"],
     ["E12 ", "`untrusted-impls`"], {}),
    ("EF17_a_type_alias_with_no_row_and_a_public_variant_field",
     "/// a\npub type Addr = u64;\n/// e\npub enum Ev {\n    /// v\n    V {\n        /// p\n        p: *mut u8,\n    },\n}\n", "",
     [ROW_CHECKED, "| `Ev` | pub | owning handle |  |  | OK |"],
     ["add a row under `## crates/u/src/a_unsafe.rs`: | `Addr` | pub | type alias", "E6 ", "has non-private fields ['p']"],
     {}),
    # `pub(super)` from a perimeter file one level down is restricted to a module in ANOTHER file:
    # visible outside the perimeter, so exported.
    ("E1_a_restricted_item_visible_outside_its_file_is_exported", "", "", [ROW_CHECKED],
     ["add a row under `## crates/u/src/outer/inner_unsafe.rs`: | `ps` | pub(in ::outer)"],
     {"files": {"crates/u/src/lib.rs": LIB + "pub mod outer;\n",
                "crates/u/src/outer.rs": "//! o\nmod inner_unsafe;\n/// q\npub fn q() {\n    inner_unsafe::ps();\n}\n",
                "crates/u/src/outer/inner_unsafe.rs": "/// s\npub(super) fn ps() {}\n/// private\nfn hidden_in_file() {}\n"}}),
    ("E1_a_derived_clone_is_a_row", "/// c\n#[derive(Clone, Debug)]\npub struct C2;\n", "",
     [ROW_CHECKED, open_row("C2", kind="plain data")],
     ["add a row under `## crates/u/src/a_unsafe.rs`: | `<C2 as Clone>` | default | trait impl"], {}),
    ("E3e_a_ui_ref_whose_stderr_lacks_its_error_code", "", "",
     [ROW_CHECKED.replace("range=t:crates/u/src/a_unsafe.rs::refuses_out_of_range",
                          "range=ui:crates/u/tests/ui/private.rs#E0603")],
     ["E3e ", "lacks 'E0603'"],
     {"files": {"crates/u/tests/ui/private.rs": "fn main() {}\n",
                "crates/u/tests/ui/private.stderr": "error[E0425]: cannot find value\n"}}),
    ("E3e_control_a_ui_ref_whose_stderr_names_its_code", "", "",
     [ROW_CHECKED.replace("range=t:crates/u/src/a_unsafe.rs::refuses_out_of_range",
                          "range=ui:crates/u/tests/ui/private.rs#E0603")],
     ["PERIMETER_EXPORTS findings=0"],
     {"files": {"crates/u/tests/ui/private.rs": "fn main() {}\n",
                "crates/u/tests/ui/private.stderr": "error[E0603]: function `x` is private\n"}}),
    ("E2_a_wrong_mechanical_kind", "", "", [ROW_CHECKED.replace("| safe fn |", "| unsafe fn |")],
     ["E2 ", "kind 'unsafe fn' != generated 'safe fn'"], {}),
    ("E10_an_undated_open_row", "/// o\npub fn o() {}\n", "",
     [ROW_CHECKED, "| `o` | pub | safe fn |  |  | OPEN: later |"], ["E10 ", "status must be OK"], {}),
    ("E13_a_safe_trait_ok_without_sealed", "/// t\npub trait Tr {}\n", "",
     [ROW_CHECKED, "| `trait Tr` | pub | trait |  |  | OK |"], ["E13 "], {}),
    ("E14_an_ok_static_of_an_address_type",
     "/// s\npub static S: AtomicPtrish = AtomicPtrish(0);\n/// a\npub struct AtomicPtrish(usize);\n", "",
     [ROW_CHECKED, "| `S` | pub | static |  |  | OK |", open_row("AtomicPtrish", kind="owning handle"),
      open_row("AtomicPtrish: Send", kind="auto trait"), open_row("AtomicPtrish: Sync", kind="auto trait")],
     ["E14 "], {}),
]


def toolchain() -> str:
    import tomllib
    return tomllib.loads((HERE / "perimeter.toml").read_text())["toolchain"]


def run_case(tmp: Path, name: str, extra: str, tests: str, rows: list[str], over: dict) -> tuple[int, str]:
    root = tmp / name
    files = {
        "Cargo.toml": ROOT_TOML, "crates/u/Cargo.toml": U_TOML, "crates/u/src/lib.rs": LIB,
        "crates/u/src/bounds.rs": BOUNDS,
        "crates/u/src/a_unsafe.rs": A.replace("EXTRA", extra).replace("TESTS", tests),
        "scripts/ci/perimeter.toml": PERIMETER.replace("TOOLCHAIN", toolchain()).replace(
            "FORMAT", over.get("format", "61")).replace("E9", over.get("e9", "0")),
        "docs/design/PERIMETER_EXPORTS.md": table(rows, over.get("vd", VD)),
    }
    files.update(over.get("files", {}))
    for rel, text in files.items():
        p = root / rel
        p.parent.mkdir(parents=True, exist_ok=True)
        p.write_text(text.replace("UNSAFE", K))
    env = {**os.environ, "RUSTUP_TOOLCHAIN": toolchain(), "RUNNER_TEMP": str(tmp / f"{name}.rt")}
    subprocess.run(["cargo", "generate-lockfile", "--offline"], cwd=root, check=True, capture_output=True, env=env)
    git = ["git", "-C", str(root), "-c", "user.email=t@t", "-c", "user.name=t"]
    subprocess.run([*git, "init", "-q"], check=True)
    subprocess.run([*git, "add", "-A"], check=True)
    subprocess.run([*git, "commit", "-qm", "fixture"], check=True)
    r = subprocess.run([sys.executable, str(HERE / "perimeter.py"), "--root", str(root), "exports"],
                       env=env, capture_output=True, text=True)
    return r.returncode, r.stdout + r.stderr


def main() -> int:
    if shutil.which("cargo") is None:
        print("SELFTEST INFRASTRUCTURE: no cargo")
        return 2
    failures = []
    with tempfile.TemporaryDirectory(prefix="kf-exports-selftest-") as t:
        tmp = Path(t)
        for name, extra, tests, rows, markers, over in CASES:
            rc, out = run_case(tmp, name, extra, tests, rows, over)
            want_ok = name == "control" or "_control_" in name
            ok = (rc == 0) == want_ok and all(m in out for m in markers)
            print(f"{'ok  ' if ok else 'FAIL'} {name}: rc={rc}"
                  + ("" if ok else f" missing={[m for m in markers if m not in out]}"))
            if not ok:
                failures.append(name)
                print(textwrap.indent(out[-3000:], "    | "))
    print(f"EXPORTS_SELFTEST cases={len(CASES)} failed={len(failures)} {failures}")
    return 1 if failures else 0


if __name__ == "__main__":
    sys.exit(main())
