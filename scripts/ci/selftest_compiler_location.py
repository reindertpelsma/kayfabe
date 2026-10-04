#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later
"""Known positives for G1, the compiler location gate (V3_SEC_PERIMETER.md §1.2, W1-W12).

Each case builds a small throwaway workspace (its own git repo, its own perimeter.toml), runs
`compiler_location.sh` against it with `KF_ROOT`, and asserts the gate ended the way it must:
the control (W2) passes, and every violation fails WITH ITS OWN REASON in the output, so a
case cannot pass by failing for an unrelated cause. Needs cargo and the pinned toolchain; not
named test_*.py because the discover step has no compiler.

Run: bash scripts/ci/compiler_location.sh --selftest
"""

from __future__ import annotations

import hashlib
import os
import shutil
import subprocess
import sys
import tempfile
import textwrap
from pathlib import Path

HERE = Path(__file__).resolve().parent
SCRIPT = HERE / "compiler_location.sh"
K = "un" + "safe"

ROOT_TOML = """\
[workspace]
resolver = "2"
members = ["crates/u", "crates/s"]
exclude = ["archive", "tools"]
[workspace.package]
version = "0.1.0"
edition = "2024"
[workspace.dependencies]
u = { path = "crates/u" }
[workspace.lints.rust]
unsafe_code = "forbid"
"""
U_TOML = """\
[package]
name = "u"
version.workspace = true
edition.workspace = true
[lints.rust]
unsafe_code = "allow"
"""
S_TOML = """\
[package]
name = "s"
version.workspace = true
edition.workspace = true
[dependencies]
u = { workspace = true }
[lints]
workspace = true
"""
U_LIB = "mod x_unsafe;\npub use x_unsafe::first;\n"
X_UNSAFE = """\
/// The first byte.
pub fn first(v: &[u8; 4]) -> u8 {
    // SAFETY: index 0 of a 4-byte array.
    UNSAFE { *v.get_unchecked(0) }
}
"""
S_LIB = "pub fn g() -> u8 { u::first(&[1, 2, 3, 4]) }\n"
PERIMETER = """\
format = 1
toolchain = "TOOLCHAIN"
crates = [{ path = "crates/u", class = "U" }]
[[location.pass]]
name = "x86_64"
args = "--workspace --all-targets --target x86_64-unknown-linux-gnu"
[manifest_lints."crates/u"]
rust = { unsafe_code = "allow" }
[build_scripts]
"""


def toolchain() -> str:
    import tomllib
    return tomllib.loads((HERE / "perimeter.toml").read_text())["toolchain"]


class Case:
    def __init__(self, name: str, files: dict[str, str], tmp: Path, untracked: dict[str, str] | None = None):
        self.name = name
        self.untracked = untracked or {}
        self.root = tmp / name
        self.base = tmp / f"{name}.run"
        self.files = {"Cargo.toml": ROOT_TOML, "crates/u/Cargo.toml": U_TOML, "crates/u/src/lib.rs": U_LIB,
                      "crates/u/src/x_unsafe.rs": X_UNSAFE, "crates/s/Cargo.toml": S_TOML,
                      "crates/s/src/lib.rs": S_LIB,
                      "scripts/ci/perimeter.toml": PERIMETER}
        for k, v in files.items():
            if v is None:
                self.files.pop(k, None)
            else:
                self.files[k] = v

    def write(self) -> None:
        for rel, text in self.files.items():
            p = self.root / rel
            p.parent.mkdir(parents=True, exist_ok=True)
            p.write_text(textwrap.dedent(text).replace("UNSAFE", K).replace("TOOLCHAIN", toolchain()))
        env = {**os.environ, "RUSTUP_TOOLCHAIN": toolchain()}
        for lock in ["Cargo.toml"] + [f for f in self.files if f.startswith("tools/") and f.endswith("Cargo.toml")]:
            subprocess.run(["cargo", "generate-lockfile", "--offline", "--manifest-path", str(self.root / lock)],
                           check=True, capture_output=True, env=env)
        git = ["git", "-C", str(self.root), "-c", "user.email=t@t", "-c", "user.name=t"]
        subprocess.run([*git, "init", "-q"], check=True)
        subprocess.run([*git, "add", "-A"], check=True)
        subprocess.run([*git, "commit", "-qm", "fixture"], check=True)
        for rel, text in self.untracked.items():
            (self.root / rel).write_text(textwrap.dedent(text).replace("UNSAFE", K))

    def run(self, reuse: bool = False) -> tuple[int, str]:
        env = {k: v for k, v in os.environ.items() if k not in ("KF_LOC_CLEAN",)}
        env.update({"KF_ROOT": str(self.root), "KF_TARGET_DIR": str(self.base / "target"),
                    "KF_GATE_DIR": str(self.base / "gate"), "RUNNER_TEMP": str(self.base)})
        if reuse:
            env["KF_ALLOW_REUSE_TARGET"] = "1"
        r = subprocess.run(["bash", str(SCRIPT)], env=env, capture_output=True, text=True)
        return r.returncode, r.stdout + r.stderr


def build_script_sha(text: str) -> str:
    return hashlib.sha256(textwrap.dedent(text).replace("UNSAFE", K).encode()).hexdigest()


W10_BUILD = """\
fn main() {
    let p = concat!(env!("CARGO_MANIFEST_DIR"), "/NOTES.md");
    std::fs::write(p, "rewritten by a build script\\n").unwrap();
}
"""

CASES: list[tuple[str, dict, int, list[str]] | tuple[str, dict, int, list[str], dict]] = [
    # (name, files, expected rc 0 or nonzero (1), markers that must appear)
    ("W2_control_class_u_perimeter_file", {}, 0, ["outside=0"]),
    ("W1_the_s1_01_crate", {
        "crates/s/Cargo.toml": S_TOML.replace("[dependencies]\nu = { workspace = true }\n[lints]\nworkspace = true\n",
                                              "[lints]\n[dependencies.u]\nworkspace = true\n"),
        "crates/s/src/lib.rs": "pub fn g() -> u8 { let v = [1u8; 4]; UNSAFE { *v.get_unchecked(0) } }\n",
    }, 1, ["G1 crates/s/src/lib.rs:1", "M1 crates/s/Cargo.toml"]),
    ("W3_unsafe_in_a_file_not_named_unsafe", {
        "crates/u/src/lib.rs": U_LIB + "pub fn h(v: &[u8; 4]) -> u8 { UNSAFE { *v.get_unchecked(1) } }\n",
    }, 1, ["G1 crates/u/src/lib.rs:3"]),
    ("W4_an_outer_allow_in_a_class_u_safe_file", {
        "crates/u/src/lib.rs": U_LIB + "#[allow(unsafe_code)]\npub fn h(v: &[u8; 4]) -> u8 { UNSAFE { *v.get_unchecked(1) } }\n",
    }, 1, ["G1 crates/u/src/lib.rs:4"]),
    ("W5_cap_lints_through_cargo_config", {
        ".cargo/config.toml": '[build]\nrustflags = ["--cap-lints", "warn"]\n',
    }, 1, ["REFUSED: --cap-lints", "M4 .cargo/config.toml"]),
    ("W7_a_non_member_path_package_under_an_excluded_dir", {
        "archive/evil/Cargo.toml": '[package]\nname = "evil"\nversion = "0.1.0"\nedition = "2024"\n',
        "archive/evil/src/lib.rs": "pub fn e(v: &[u8; 4]) -> u8 { UNSAFE { *v.get_unchecked(2) } }\n",
        "crates/s/Cargo.toml": S_TOML.replace("[dependencies]\n", '[dependencies]\nevil = { path = "../../archive/evil" }\n'),
    }, 1, ["G1 archive/evil/src/lib.rs:1"]),
    ("W8_a_lib_path_borrowed_from_a_class_u_crate", {
        "Cargo.toml": ROOT_TOML.replace('members = ["crates/u", "crates/s"]', 'members = ["crates/u", "crates/s", "crates/b"]'),
        "crates/b/Cargo.toml": '[package]\nname = "b"\nversion.workspace = true\nedition.workspace = true\n'
                               '[lib]\npath = "../u/src/x_unsafe.rs"\n[lints]\nworkspace = true\n',
    }, 1, ["G1 crates/u/src/x_unsafe.rs:4", "crates/b"]),
    ("W9_a_perimeter_macro_expanded_in_a_safe_file", {
        "crates/u/src/lib.rs": U_LIB + "mod m_unsafe;\npub fn k() -> u8 { m_unsafe::m!() }\n",
        "crates/u/src/m_unsafe.rs": "macro_rules! m { () => { UNSAFE { core::hint::black_box(7u8) } } }\npub(crate) use m;\n",
    }, 1, ["G1 crates/u/src/m_unsafe.rs:1", "via crates/u/src/lib.rs:4"]),
    ("W10_a_build_script_that_edits_a_tracked_file", {
        "crates/s/build.rs": W10_BUILD,
        "crates/s/NOTES.md": "tracked\n",
        "scripts/ci/perimeter.toml": PERIMETER + f'"crates/s/build.rs" = "{build_script_sha(W10_BUILD)}"\n',
    }, 1, ["a build step changed the checkout", "crates/s/NOTES.md"]),
    ("W11_an_interposed_rustc", {
        ".cargo/config.toml": '[build]\nrustc-workspace-wrapper = "/bin/true"\n',
    }, 1, ["REFUSED: argv[1] '/bin/true' is not the pinned rustc", "M4 .cargo/config.toml"]),
    ("W12_a_standalone_package_with_unsafe_in_a_safe_file", {
        "tools/gen/Cargo.toml": '[package]\nname = "gen"\nversion = "0.1.0"\nedition = "2024"\n[workspace]\n'
                                '[lints.rust]\nunsafe_code = "forbid"\n',
        "tools/gen/src/main.rs": "fn main() { let v = [1u8; 4]; let _ = UNSAFE { *v.get_unchecked(3) }; }\n",
        "scripts/ci/perimeter.toml": PERIMETER + '[manifest_lints."tools/gen"]\nrust = { unsafe_code = "forbid" }\n'
                                     '[[standalone]]\npath = "tools/gen"\nruns = ["--all-targets"]\n',
    }, 1, ["G1 tools/gen/src/main.rs:1"]),
    # Beyond the design's twelve: the early forbid, and the two dep-info (L0) rules.
    # s inherits no workspace lints here, so only the wrapper's own `-F` can turn the allow into E0453.
    ("W13_an_allow_in_a_forbid_crate_fails_early", {
        "crates/s/Cargo.toml": S_TOML.replace("[lints]\nworkspace = true\n", ""),
        "crates/s/src/lib.rs": "#![allow(unsafe_code)]\n" + S_LIB,
    }, 1, ["G1 crates/s/src/lib.rs:1: E0453: an `allow(unsafe_code)` in a forbid unit"]),
    ("W14_a_unit_compiles_an_untracked_file", {
        "crates/s/src/lib.rs": "mod extra;\n" + S_LIB,
    }, 1, ["L0 crates/s/src/extra.rs:0: unit s compiled an untracked file"], {"crates/s/src/extra.rs": "pub fn x() {}\n"}),
    ("W15_a_perimeter_file_no_unit_compiles", {
        "crates/u/src/lib.rs": U_LIB + "#[cfg(any())]\nmod y_unsafe;\n",
        "crates/u/src/y_unsafe.rs": "pub fn y() {}\n",
    }, 1, ["L0 crates/u/src/y_unsafe.rs:0: a class U perimeter file no compiled unit read"]),
]


def main() -> int:
    if shutil.which("cargo") is None:
        print("SELFTEST INFRASTRUCTURE: no cargo")
        return 2
    failures = []
    with tempfile.TemporaryDirectory(prefix="kf-loc-selftest-") as t:
        tmp = Path(t)
        for name, files, want_rc, markers, *untracked in CASES:
            case = Case(name, files, tmp, untracked[0] if untracked else None)
            case.write()
            rc, out = case.run()
            ok = (rc == 0) == (want_rc == 0) and all(m in out for m in markers)
            print(f"{'ok  ' if ok else 'FAIL'} {name}: rc={rc} markers={'all' if ok else [m for m in markers if m not in out]}")
            if not ok:
                failures.append(name)
                print(textwrap.indent(out[-6000:], "    | "))
        # W6: the same target directory twice. Refused outright; and if forced, nothing is
        # reached, because a warm target dir compiles nothing.
        case = Case("W6_a_reused_target_directory", {}, tmp)
        case.write()
        rc1, _ = case.run()
        rc2, out2 = case.run()
        rc3, out3 = case.run(reuse=True)
        ok = rc1 == 0 and rc2 == 2 and "already exists" in out2 and rc3 != 0 and "never reached" in out3
        print(f"{'ok  ' if ok else 'FAIL'} W6_a_reused_target_directory: rc={rc1},{rc2},{rc3}")
        if not ok:
            failures.append("W6")
            print(textwrap.indent((out2 + out3)[-6000:], "    | "))
        # W11, the wrapper on its own: any rustc but the pinned one is refused with 97.
        env = {"KF_PINNED_RUSTC": subprocess.run(["rustup", "which", "rustc", "--toolchain", toolchain()],
                                                 capture_output=True, text=True).stdout.strip(), "PATH": os.environ["PATH"]}
        r = subprocess.run([sys.executable, str(HERE / "rustc_location_wrapper.py"), "/bin/true", "-vV"],
                           env=env, capture_output=True, text=True)
        ok = r.returncode == 97 and "REFUSED" in r.stderr
        r2 = subprocess.run([sys.executable, str(HERE / "rustc_location_wrapper.py"), env["KF_PINNED_RUSTC"], "-vV"],
                            env={**env, "RUSTC_BOOTSTRAP": "1"}, capture_output=True, text=True)
        ok = ok and r2.returncode == 97 and "RUSTC_BOOTSTRAP" in r2.stderr
        print(f"{'ok  ' if ok else 'FAIL'} W11_the_wrapper_alone: rc={r.returncode},{r2.returncode}")
        if not ok:
            failures.append("W11-direct")
    print(f"COMPILER_LOCATION_SELFTEST cases={len(CASES) + 2} failed={len(failures)} {failures}")
    return 1 if failures else 0


if __name__ == "__main__":
    sys.exit(main())
