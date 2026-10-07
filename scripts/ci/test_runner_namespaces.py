from pathlib import Path
import unittest
from runner_namespaces import profile_text


class RunnerNamespaceTests(unittest.TestCase):
    def test_only_checkout_target_executables_are_attached(self):
        text = profile_text(Path("/ci/work/kayfabe"))
        self.assertIn('"/ci/work/kayfabe/target/**"', text)
        self.assertIn("userns,", text)
        self.assertNotIn('"/**"', text)

    def test_paths_cannot_inject_additional_policy(self):
        for path in ("relative", "/ci/*", '/ci/"bad', "/ci/bad\nrule", "/ci/[ab]"):
            with self.subTest(path=path), self.assertRaises(ValueError):
                profile_text(Path(path))
