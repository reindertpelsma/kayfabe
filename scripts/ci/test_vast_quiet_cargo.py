"""Gate visibility falsifiers: cargo warnings must not flood the box log (measured 2026-10-26).

`cargo build -q` still prints every rustc warning (checked 2026-10-26: -q and CARGO_TERM_QUIET
both leave warnings on). In run 40241597724 the unredirected `cargo build -q` of provision_full.sh
(crates/kayfabe-linux-raw, `-W missing-docs`) filled the job log. The build there now writes its
output to a file and prints errors; lint level and build flags are untouched.
"""

import os
import re
import subprocess
import tempfile
import unittest
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
SCRIPT = Path(os.environ.get("KF_PROVISION_FULL") or ROOT / "scripts/bench/box/provision_full.sh")
BEGIN, END = "# KF_LADDER_BUILD_BEGIN", "# KF_LADDER_BUILD_END"


def ladder_block():
    text = SCRIPT.read_text()
    if BEGIN not in text or END not in text:
        raise AssertionError("provision_full.sh has no marked ladder build block")
    return text.split(BEGIN, 1)[1].split(END, 1)[0]


class QuietLadderBuild(unittest.TestCase):
    def run_block(self, cargo_body):
        with tempfile.TemporaryDirectory() as d:
            root = Path(d)
            (root / "bin").mkdir()
            cargo = root / "bin/cargo"
            cargo.write_text(
                '#!/bin/bash\necho "$@" > "$(dirname "$0")/../cargo.args"\n' + cargo_body
            )
            cargo.chmod(0o755)
            (root / "prov").mkdir()
            block = ladder_block().replace("/root/prov", str(root / "prov"))
            script = root / "block.sh"
            script.write_text("exec 2>&1\n" + block)
            env = dict(os.environ, PATH=str(root / "bin") + ":" + os.environ["PATH"])
            p = subprocess.run(
                ["bash", str(script)], env=env, text=True, capture_output=True, timeout=30
            )
            kept = {x.name: x.read_text() for x in (root / "prov").iterdir()}
            kept["cargo.args"] = (root / "cargo.args").read_text().strip()
            return p.stdout + p.stderr, kept

    WARN = 'for i in $(seq 1 800); do echo "warning: missing documentation for item $i" >&2; done\n'

    def test_warnings_do_not_flood_the_log(self):
        out, _ = self.run_block(self.WARN + "exit 0\n")
        self.assertLessEqual(len(out.splitlines()), 5, out[:500])
        self.assertEqual(re.findall(r"^LADDER_RC=0$", out, re.M), ["LADDER_RC=0"])

    def test_warnings_are_kept_in_a_file(self):
        out, kept = self.run_block(self.WARN + "exit 0\n")
        self.assertEqual(sorted(kept), ["cargo.args", "ladder.log"])
        self.assertEqual(kept["ladder.log"].count("warning: missing documentation"), 800)

    def test_errors_still_print(self):
        body = self.WARN + 'echo "error[E0432]: unresolved import \\`x\\`" >&2\nexit 101\n'
        out, _ = self.run_block(body)
        self.assertIn("error[E0432]: unresolved import", out)
        self.assertEqual(re.findall(r"^LADDER_RC=101$", out, re.M), ["LADDER_RC=101"])

    def test_a_failed_build_prints_its_tail_but_is_bounded(self):
        body = 'for i in $(seq 1 800); do echo "line $i of the failing build" >&2; done\nexit 101\n'
        out, _ = self.run_block(body)
        self.assertIn("line 800 of the failing build", out)
        self.assertLessEqual(len(out.splitlines()), 90)
        self.assertEqual(re.findall(r"^LADDER_RC=\d+$", out, re.M), ["LADDER_RC=101"])

    def test_lint_level_and_build_flags_are_untouched(self):
        block = ladder_block()
        self.assertIn(
            "cargo build -q --release -p kayfabe-rm-ladder --bin kayfabe-rm-ladder", block
        )
        for weakened in ("-A ", "-Awarnings", "cap-lints", "RUSTFLAGS", "--quiet --quiet"):
            self.assertNotIn(weakened, block)

    def test_cargo_gets_exactly_the_original_arguments(self):
        _, kept = self.run_block("exit 0\n")
        self.assertEqual(
            kept["cargo.args"], "build -q --release -p kayfabe-rm-ladder --bin kayfabe-rm-ladder"
        )

    def test_the_exit_status_is_still_the_cargo_status(self):
        block = ladder_block()
        self.assertRegex(block, r"LADDER_RC=\$\{?(?:LADDER_RC|rc)\}?|LADDER_RC=\$\?")
        self.assertNotRegex(block, r"cargo[^\n]*\|\s*(tail|head|grep)")


if __name__ == "__main__":
    unittest.main()
