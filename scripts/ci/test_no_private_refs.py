import contextlib
import io
import tempfile
import unittest
from pathlib import Path

import no_private_refs as gate

ROOT = Path(__file__).resolve().parents[2]
NAME = "kayfabe" + "-private"  # built in two pieces so this file is not a hit itself


def run_gate(root):
    out = io.StringIO()
    with contextlib.redirect_stdout(out), contextlib.redirect_stderr(io.StringIO()):
        rc = gate.main(["--root", str(root)])
    return rc, out.getvalue()


class Tree:
    def __init__(self, files):
        self._tmp = tempfile.TemporaryDirectory()
        self.root = Path(self._tmp.name)
        for rel, text in files.items():
            path = self.root / rel
            path.parent.mkdir(parents=True, exist_ok=True)
            path.write_bytes(text if isinstance(text, bytes) else text.encode())

    def close(self):
        self._tmp.cleanup()


class NoPrivateRefs(unittest.TestCase):
    def check(self, files, expect_rc, expect_in_out=""):
        tree = Tree(files)
        self.addCleanup(tree.close)
        rc, out = run_gate(tree.root)
        self.assertEqual(rc, expect_rc, out)
        self.assertIn(expect_in_out, out)

    def test_clean_tree_passes_once_known_debt_is_gone(self):
        # KNOWN_DEBT entries must still match, so a scratch tree needs them present.
        files = {rel: f"see {NAME}\n" for rel in gate.KNOWN_DEBT}
        files["README.md"] = "nothing here\n"
        self.check(files, 0, "clean")

    def test_new_reference_fails(self):
        files = {rel: f"see {NAME}\n" for rel in gate.KNOWN_DEBT}
        files["docs/new.md"] = f"details live in reindertpelsma/{NAME}\n"
        self.check(files, 1, "FAIL docs/new.md")

    def test_underscore_and_case_variants_fail(self):
        files = {rel: f"see {NAME}\n" for rel in gate.KNOWN_DEBT}
        files["a.txt"] = "Kayfabe_Private notes\n"
        self.check(files, 1, "FAIL a.txt")

    def test_policy_documents_are_allowed(self):
        files = {rel: f"see {NAME}\n" for rel in gate.KNOWN_DEBT}
        files["docs/design/V3_PROVENANCE.md"] = f"RE stays in {NAME}\n"
        files["docs/OWNER_RULINGS.md"] = f"RE stays in {NAME}\n"
        self.check(files, 0, "clean")

    def test_stale_known_debt_fails(self):
        files = {rel: f"see {NAME}\n" for rel in gate.KNOWN_DEBT}
        victim = sorted(gate.KNOWN_DEBT)[0]
        files[victim] = "rephrased\n"
        self.check(files, 1, f"FAIL {victim}: listed in KNOWN_DEBT")

    def test_receipt_schema_id_is_not_a_reference(self):
        files = {rel: f"see {NAME}\n" for rel in gate.KNOWN_DEBT}
        files["traces/run1/receipt.json"] = '{"schema": "' + NAME + '-watchdog-recovery/1"}\n'
        self.check(files, 0, "clean")

    def test_schema_id_does_not_hide_a_real_reference_beside_it(self):
        files = {rel: f"see {NAME}\n" for rel in gate.KNOWN_DEBT}
        files["traces/run1/receipt.json"] = NAME + "-watchdog-recovery/1 and " + NAME + "\n"
        self.check(files, 1, "FAIL traces/run1/receipt.json")

    def test_binary_files_are_skipped(self):
        files = {rel: f"see {NAME}\n" for rel in gate.KNOWN_DEBT}
        files["x.bin"] = b"\0\0" + NAME.encode()
        self.check(files, 0, "clean")

    def test_the_real_tree_is_clean(self):
        rc, out = run_gate(ROOT)
        self.assertEqual(rc, 0, out)


if __name__ == "__main__":
    unittest.main()
