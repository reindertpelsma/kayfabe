"""Compile the actual fixture selector without building the GPU client."""

import importlib.util
import os
from pathlib import Path
import subprocess
import tempfile
import unittest

SPEC = importlib.util.spec_from_file_location(
    "fuzz_fixture", Path(__file__).with_name("fuzz_fixture.py")
)
F = None
if Path(SPEC.origin).exists():
    F = importlib.util.module_from_spec(SPEC)
    SPEC.loader.exec_module(F)


class FixtureSelection(unittest.TestCase):
    def setUp(self):
        self.assertIsNotNone(F, "isolated fuzz fixture selector missing")

    def selected(self, arm=None):
        source = "fn run() -> bool {\n" + F.SELECTION
        source += '\nlet arms = [("unpinned",3),("percore",3),("crowd-free",9),("crowd",9)];'
        source += "\nfor (name, width) in arms {\n" + F.FILTER
        source += '\nprintln!("{}:{}",name,width);}\ntrue}\n'
        source += "fn main(){if !run(){std::process::exit(1);}}"
        with tempfile.TemporaryDirectory() as root:
            root = Path(root)
            (root / "selector.rs").write_text(source)
            subprocess.run(
                ["nice", "-n", "19", "rustc", str(root / "selector.rs"), "-o", str(root / "s")],
                capture_output=True,
                check=True,
            )
            env = dict(os.environ)
            env.pop("KF_BW3_FUZZ_ARM", None)
            if arm is not None:
                env["KF_BW3_FUZZ_ARM"] = arm
            return subprocess.run([str(root / "s")], env=env, capture_output=True, text=True)

    def test_unset_selection_keeps_all_four_default_arms(self):
        self.assertEqual(
            self.selected().stdout.splitlines(),
            ["unpinned:3", "percore:3", "crowd-free:9", "crowd:9"],
        )

    def test_pinned_selection_runs_exactly_one_nine_worker_arm(self):
        self.assertEqual(self.selected("crowd").stdout.strip(), "crowd:9")

    def test_unpinned_control_runs_exactly_one_same_width_arm(self):
        self.assertEqual(self.selected("crowd-free").stdout.strip(), "crowd-free:9")

    def test_unknown_selection_refuses_without_a_clean_fuzz_result(self):
        self.assertNotEqual(self.selected("other").returncode, 0)

    def test_empty_selection_also_refuses(self):
        self.assertNotEqual(self.selected("").returncode, 0)

    def test_instrumentation_changes_only_the_arm_loop(self):
        source = subprocess.check_output(
            ["git", "show", "6e3fd3fd6:crates/kayfabe-rm-ladder/src/main.rs"], text=True
        )
        patched = F.instrument(source)
        self.assertEqual(patched.replace(F.SELECTION, "").replace(F.FILTER, ""), source)


if __name__ == "__main__":
    unittest.main()
