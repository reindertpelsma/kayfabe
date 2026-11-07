#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later
"""GPU-free fixtures for the clean clone preflight and the real shell/clone path."""

import os
from pathlib import Path
import shutil
import subprocess
import sys
import tempfile
import unittest

sys.path.insert(0, str(Path(__file__).resolve().parent))
import clean_clone_inputs as check

HERE = Path(__file__).resolve().parent
FIXTURES = HERE / "fixtures/release_checks"


class Tree(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory(prefix="kf-clone-test-")
        self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name) / "source"
        self.root.mkdir()
        for name, text in {
            "README.md": (FIXTURES / "README.md.txt").read_text(),
            "docs/howto/building.md": (FIXTURES / "building.md.txt").read_text(),
            "Cargo.toml": '[workspace]\nmembers = ["crates/kf-oprom"]\n',
            "Cargo.lock": "# fixture\n",
            "rust-toolchain.toml": '[toolchain]\nchannel = "1.99.0"\ntargets = ["x86_64-unknown-linux-musl", "x86_64-unknown-uefi"]\n',
            "firmware/kf-gop/Cargo.toml": "# fixture\n",
            "firmware/kf-gop/Cargo.lock": "# fixture\n",
            "crates/kf-oprom/Cargo.toml": "# fixture\n",
            "crates/kf-oprom/src/lib.rs": "// fixture\n",
        }.items():
            p = self.root / name
            p.parent.mkdir(parents=True, exist_ok=True)
            p.write_text(text)

    def errors(self):
        return "\n".join(check.validate(self.root))


class Fixture(Tree):
    def test_documented_steps_and_inputs_pass(self):
        self.assertEqual(self.errors(), "")

    def test_readme_missing_build_fails(self):
        (self.root / "README.md").write_text("# No command\n")
        self.assertIn("documented build", self.errors())

    def test_manual_changed_to_check_fails(self):
        p = self.root / "docs/howto/building.md"
        p.write_text(p.read_text().replace("cargo build", "cargo check"))
        self.assertIn("documented build", self.errors())

    def test_readme_missing_test_fails(self):
        p = self.root / "README.md"
        p.write_text(p.read_text().replace("cargo test", "cargo check"))
        self.assertIn("documented test", self.errors())

    def test_manual_missing_test_fails(self):
        p = self.root / "docs/howto/building.md"
        p.write_text(p.read_text().replace("cargo test", "cargo check"))
        self.assertIn("documented test", self.errors())

    def test_missing_workspace_lock_fails(self):
        (self.root / "Cargo.lock").unlink()
        self.assertIn("required build input", self.errors())

    def test_missing_firmware_lock_fails(self):
        (self.root / "firmware/kf-gop/Cargo.lock").unlink()
        self.assertIn("required build input", self.errors())

    def test_missing_include_bytes_fails(self):
        (self.root / "crates/kf-oprom/src/lib.rs").write_text(
            'const X: &[u8] = include_bytes!("missing.bin");\n'
        )
        self.assertIn("missing.bin", self.errors())

    def test_missing_include_str_fails(self):
        (self.root / "crates/kf-oprom/src/lib.rs").write_text(
            'const X: &str = include_str!("data.tsv");\n'
        )
        self.assertIn("data.tsv", self.errors())

    def moved(self, statement):
        path = self.root / "traces/old/MANIFEST.tsv"
        path.parent.mkdir(parents=True)
        path.write_text(
            "path\tbytes\tsha256\ntraces/old/data.rec\t1\t" + "a" * 64 + "\n"
        )
        (self.root / "crates/kf-oprom/src/lib.rs").write_text(statement)

    def test_runtime_reader_cannot_depend_on_archived_file(self):
        self.moved('let p = root.join("traces/old/data.rec");\n')
        self.assertIn("archived input", self.errors())

    def test_relative_runtime_reader_cannot_depend_on_archive(self):
        self.moved('let p = root.join("../../../traces/old/data.rec");\n')
        self.assertIn("archived input", self.errors())

    def test_archived_comment_is_not_a_read(self):
        self.moved('// see "traces/old/data.rec"\n')
        self.assertEqual(self.errors(), "")

    def test_present_manifest_member_is_allowed(self):
        self.moved('let p = root.join("traces/old/data.rec");\n')
        (self.root / "traces/old/data.rec").write_text("x")
        self.assertEqual(self.errors(), "")


class Shell(Tree):
    # Keep fixture helpers, without rerunning the preflight tests in this class.
    def setUp(self):
        super().setUp()
        self.log = Path(self.temp.name) / "cargo.log"
        self.bin = Path(self.temp.name) / "bin"
        self.bin.mkdir()
        cargo = self.bin / "cargo"
        cargo.write_text("""#!/usr/bin/env bash
set -eu
printf '%s\\n' "$*" >> "$TEST_CARGO_LOG"
[ "$(git rev-parse --is-shallow-repository)" = true ]
[ ! -e uncommitted-input ]
[ ! -e ignored-input ]
if [ "$1" = "build" ]; then
  [ -z "$(ls -A "$CARGO_TARGET_DIR")" ]
  touch "$CARGO_TARGET_DIR/fixture-built"
  exit "${TEST_BUILD_RC:-0}"
fi
[ -f "$CARGO_TARGET_DIR/fixture-built" ]
exit "${TEST_TEST_RC:-0}"
""")
        cargo.chmod(0o755)
        for args in (
            ["init", "-q"],
            ["add", "."],
            [
                "-c",
                "user.name=Fixture",
                "-c",
                "user.email=fixture@example.invalid",
                "commit",
                "-qm",
                "fixture",
            ],
        ):
            subprocess.run(
                ["git", *args], cwd=self.root, check=True, capture_output=True
            )
        (self.root / "uncommitted-input").write_text("must not clone")
        (self.root / ".gitignore").write_text("ignored-input\n")
        (self.root / "ignored-input").write_text("must not clone")
        self.target = Path(self.temp.name) / "target"

    def run_shell(self, **updates):
        env = dict(
            os.environ,
            PATH=str(self.bin) + os.pathsep + os.environ["PATH"],
            CARGO_TARGET_DIR=str(self.target),
            TEST_CARGO_LOG=str(self.log),
        )
        env.update(updates)
        return subprocess.run(
            ["bash", str(HERE / "clean_clone_check.sh"), "--source", str(self.root)],
            env=env,
            capture_output=True,
            text=True,
            timeout=30,
        )

    def test_fresh_shallow_clone_builds_and_tests(self):
        r = self.run_shell()
        self.assertEqual(r.returncode, 0, r.stderr + r.stdout)
        self.assertIn("CLEAN_CLONE_PASS", r.stdout)
        self.assertEqual(
            self.log.read_text().splitlines(),
            [
                "build --workspace --release --locked -j2",
                "test --workspace --locked --no-fail-fast -j2",
            ],
        )
        self.assertEqual(list(self.target.iterdir()), [])

    def test_build_failure_is_not_success(self):
        r = self.run_shell(TEST_BUILD_RC="7")
        self.assertNotEqual(r.returncode, 0)
        self.assertIn("release build failed", r.stderr)
        self.assertEqual(len(self.log.read_text().splitlines()), 1)

    def test_failed_build_still_cleans_private_target(self):
        r = self.run_shell(TEST_BUILD_RC="42")
        self.assertNotEqual(r.returncode, 0)
        self.assertNotIn("CLEAN_CLONE_PASS", r.stdout)
        self.assertEqual(list(self.target.iterdir()), [])

    def test_selected_ref_is_cloned(self):
        subprocess.run(["git", "branch", "selected"], cwd=self.root, check=True)
        env = dict(
            os.environ,
            PATH=str(self.bin) + os.pathsep + os.environ["PATH"],
            CARGO_TARGET_DIR=str(self.target),
            TEST_CARGO_LOG=str(self.log),
        )
        r = subprocess.run(
            [
                "bash",
                str(HERE / "clean_clone_check.sh"),
                "--source",
                str(self.root),
                "--ref",
                "selected",
            ],
            env=env,
            capture_output=True,
            text=True,
            timeout=30,
        )
        self.assertEqual(r.returncode, 0, r.stderr)
        self.assertIn("CLEAN_CLONE_PASS", r.stdout)

    def test_test_failure_is_not_success(self):
        r = self.run_shell(TEST_TEST_RC="8")
        self.assertNotEqual(r.returncode, 0)
        self.assertIn("GPU-free tests failed", r.stderr)
        self.assertNotIn("CLEAN_CLONE_PASS", r.stdout)

    def test_failed_tests_clean_target_and_propagate_status(self):
        r = self.run_shell(TEST_TEST_RC="42")
        self.assertNotEqual(r.returncode, 0)
        self.assertEqual(list(self.target.iterdir()), [])

    def commit_file(self, name):
        subprocess.run(["git", "add", "--", name], cwd=self.root, check=True)
        subprocess.run(
            [
                "git",
                "-c",
                "user.name=Fixture",
                "-c",
                "user.email=fixture@example.invalid",
                "commit",
                "-qm",
                "change fixture",
            ],
            cwd=self.root,
            check=True,
        )

    def test_missing_committed_firmware_input_stops_before_cargo(self):
        (self.root / "firmware/kf-gop/Cargo.lock").unlink()
        self.commit_file("firmware/kf-gop/Cargo.lock")
        r = self.run_shell()
        self.assertNotEqual(r.returncode, 0)
        self.assertIn("required build input", r.stderr)
        self.assertFalse(self.log.exists())

    def test_changed_documented_command_stops_before_cargo(self):
        (self.root / "docs/howto/building.md").write_text("cargo check\n")
        self.commit_file("docs/howto/building.md")
        r = self.run_shell()
        self.assertNotEqual(r.returncode, 0)
        self.assertIn("documented build", r.stderr)
        self.assertFalse(self.log.exists())

    def disk_fixture(self, free_kib):
        tool = self.bin / "df"
        tool.write_text(
            "#!/bin/sh\nprintf 'Filesystem 1024-blocks Used Available Capacity Mounted\\nfixture 10000000 0 "
            + str(free_kib)
            + " 0%% /\\n'\n"
        )
        tool.chmod(0o755)

    def test_zero_free_space_stops_before_clone_and_cargo(self):
        self.disk_fixture(0)
        r = self.run_shell()
        self.assertNotEqual(r.returncode, 0)
        self.assertIn("less than 6 GiB", r.stderr)
        self.assertFalse(self.log.exists())

    def test_space_just_below_threshold_stops_before_build(self):
        self.disk_fixture(6291455)
        r = self.run_shell()
        self.assertNotEqual(r.returncode, 0)
        self.assertIn("less than 6 GiB", r.stderr)
        self.assertFalse(self.target.exists())

    def test_no_target_directory_is_an_error(self):
        r = self.run_shell(CARGO_TARGET_DIR="")
        self.assertNotEqual(r.returncode, 0)
        self.assertIn("CARGO_TARGET_DIR", r.stderr)
        self.assertFalse(self.log.exists())


if __name__ == "__main__":
    unittest.main()
