"""B1g: export the console's host-key lines with their key type; the guard decision is unchanged.

Run: KF_BW3_D=/workspace/bw3-round-b1g-a1-run python3 -m unittest test_pin_evidence
"""

import hashlib
import json
import os
from pathlib import Path
import sys
import unittest

D = Path(os.environ.get("KF_BW3_D", "/workspace/bw3-round-b1g-a1-run"))
BW = Path("/workspace/bw-native-edge-wt/scripts/bench/blackwell")
sys.path.insert(0, str(BW))
sys.path.insert(0, str(D))
import console_pin as C  # noqa: E402
import pin_evidence as E  # noqa: E402

KEY = "AAAAC3NzaC1lZDI1NTE5AAAAICqdfloi34PeBXNmyxDrU291Lh1LityrWqJ7w8ZPiYp/"
FP = "SHA256:wS+QN5rGbi64x35ezg9cr+SdIpgGOUNMUK6AqAPBbyE"
ECDSA_FP = "SHA256:" + "E" * 43
UNTYPED_FP = "SHA256:" + "U" * 43
CONSOLE = "\n".join([
    "booting the instance",
    "<14>Oct 22 21:08:55 cloud-init: 256 %s root@ubuntu (ED25519)" % FP,
    "<14>Oct 22 21:08:55 cloud-init: 256 %s root@ubuntu (ECDSA)" % ECDSA_FP,
    "ssh-ed25519 %s root@ubuntu" % KEY,
    "an unlabelled line %s" % UNTYPED_FP,
    "export SECRET_TOKEN=do-not-export",
])


class KeyLines(unittest.TestCase):
    def test_kinds(self):
        rows = E.key_lines(CONSOLE)
        kinds = [(r["kind"], r["fingerprint"]) for r in rows]
        self.assertIn(("fingerprint_typed:ED25519", FP), kinds)
        self.assertIn(("fingerprint_typed:ECDSA", ECDSA_FP), kinds)
        self.assertIn(("ed25519_key", FP), kinds)
        self.assertIn(("fingerprint_untyped", UNTYPED_FP), kinds)

    def test_only_key_lines_exported(self):
        text = json.dumps(E.key_lines(CONSOLE))
        self.assertNotIn("SECRET_TOKEN", text)
        self.assertNotIn("booting the instance", text)

    def test_guard_view_annotated(self):
        rows = E.key_lines(CONSOLE)
        guard = C.console_pins(CONSOLE, KEY)
        for row in rows:
            self.assertEqual(row["counted_by_guard"], row["fingerprint"] in guard)
        typed_ecdsa = [r for r in rows if r["kind"] == "fingerprint_typed:ECDSA"][0]
        self.assertFalse(typed_ecdsa["counted_by_guard"])
        untyped = [r for r in rows if r["kind"] == "fingerprint_untyped"][0]
        self.assertTrue(untyped["counted_by_guard"])

    def test_line_bound(self):
        rows = E.key_lines("x" * 5000 + " " + FP + " (ED25519)")
        self.assertLessEqual(len(rows[0]["line"]), 300)

    def test_malformed_key_is_recorded_not_raised(self):
        rows = E.key_lines("ssh-ed25519 AAAAnotakey root@x")
        self.assertEqual(rows[0]["kind"], "ed25519_key_malformed")


    def test_long_untyped_line_bounded(self):
        rows = E.key_lines("y" * 4000 + " " + UNTYPED_FP + " " + "z" * 4000)
        self.assertEqual(rows[0]["kind"], "fingerprint_untyped")
        self.assertLessEqual(len(rows[0]["line"]), 300)
        self.assertIn(UNTYPED_FP, rows[0]["line"])

    def test_malformed_key_does_not_hide_other_lines(self):
        rows = E.key_lines("ssh-ed25519 AAAAnotakey root@x\n256 %s h (ECDSA)" % ECDSA_FP)
        self.assertEqual([r["kind"] for r in rows],
                         ["ed25519_key_malformed", "fingerprint_typed:ECDSA"])

    def test_untyped_and_typed_guard_flags(self):
        rows = {r["fingerprint"]: r for r in E.key_lines(CONSOLE) if r["kind"] != "ed25519_key"}
        self.assertFalse(rows[ECDSA_FP]["counted_by_guard"])
        self.assertTrue(rows[UNTYPED_FP]["counted_by_guard"])


class Wiring(unittest.TestCase):
    def test_ops_captures_console_before_returning(self):
        ops = (D / "ops.py").read_text()
        self.assertIn("import pin_evidence as PE", ops)
        start = ops.index("    def console_text(timeout):")
        end = ops.index("    while True:", start)
        body = ops[start:end]
        self.assertIn("PE.capture(D, iid, text)", body)
        self.assertLess(body.index("PE.capture(D, iid, text)"), body.index("return text"))

    def test_capture_writes_private_raw_and_exportable_lines(self):
        import tempfile

        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            E.capture(root, 7, CONSOLE)
            E.capture(root, 7, CONSOLE)
            raws = sorted(root.glob("console_raw_7_*.txt"))
            self.assertEqual(len(raws), 2)
            lines = (root / "console_keylines_7.jsonl").read_text().splitlines()
            self.assertEqual(len(lines), 2)
            row = json.loads(lines[0])
            self.assertEqual(row["console_sha256"], hashlib.sha256(CONSOLE.encode()).hexdigest())
            self.assertNotIn("SECRET_TOKEN", "\n".join(lines))

    def test_ops_capture_adjacent_to_return(self):
        ops = (D / "ops.py").read_text()
        self.assertEqual(ops.count("PE.capture(D, iid, text)"), 1)
        self.assertIn("        PE.capture(D, iid, text)\n        return text\n", ops)

    def test_capture_raw_is_private_full_text_and_row_has_no_text(self):
        import tempfile

        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            row = E.capture(root, 9, CONSOLE)
            self.assertEqual((root / "console_raw_9_0.txt").read_text(), CONSOLE)
            self.assertEqual(set(row), {"utc", "lease_id", "poll", "console_sha256", "key_lines"})

    def test_guard_source_unchanged(self):
        rehearsal = json.loads((D / "rehearsal_pass.json").read_text())
        digest = hashlib.sha256((BW / "console_pin.py").read_bytes()).hexdigest()
        self.assertEqual(digest, rehearsal["guard_source_sha256"])


if __name__ == "__main__":
    unittest.main()
