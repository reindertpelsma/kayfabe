import contextlib
import io
import json
import re
import tempfile
import unittest
from collections import Counter
from pathlib import Path

import no_gpu_uuids as gate

ROOT = Path(__file__).resolve().parents[2]


def uuid(seed):
    """A synthetic UUID (never a real board's): the seed repeated into the 8-4-4-4-12 shape."""
    h = (seed * 32)[:32]
    return f"{h[:8]}-{h[8:12]}-{h[12:16]}-{h[16:20]}-{h[20:]}"


A, B, C = uuid("a1"), uuid("b2"), uuid("c3")


class Tree:
    """A scratch repository tree (no git: the gate then walks the directories)."""

    def __init__(self):
        self._tmp = tempfile.TemporaryDirectory()
        self.root = Path(self._tmp.name)

    def write(self, rel, data):
        path = self.root / rel
        path.parent.mkdir(parents=True, exist_ok=True)
        path.write_bytes(data if isinstance(data, bytes) else data.encode())
        return path

    def remove(self, rel):
        (self.root / rel).unlink()

    def run(self, *args):
        """main() on this tree: (exit code, everything it printed)."""
        out, err = io.StringIO(), io.StringIO()
        base = self.root / "baseline.json"
        with contextlib.redirect_stdout(out), contextlib.redirect_stderr(err):
            code = gate.main(["--root", str(self.root), "--baseline", str(base), *args])
        return code, out.getvalue() + err.getvalue()

    def baseline(self):
        return json.loads((self.root / "baseline.json").read_text())


class ScanTests(unittest.TestCase):
    def test_finds_the_nvidia_smi_form(self):
        self.assertEqual(sum(gate.scan(f"GPU-{A}".encode()).values()), 1)

    def test_finds_the_gpu_uuid_assignment_form(self):
        self.assertEqual(sum(gate.scan(f"gpu-uuid={A}".encode()).values()), 1)

    def test_finds_the_underscore_and_colon_and_quoted_forms(self):
        self.assertEqual(sum(gate.scan(f'gpu_uuid: "{A}"'.encode()).values()), 1)

    def test_is_case_insensitive_in_marker_and_hex(self):
        for text in (f"gpu-{A.upper()}", f"GPU-{A.upper()}", f"GPU_UUID={A.upper()}"):
            with self.subTest(text=text[:12]):
                self.assertEqual(sum(gate.scan(text.encode()).values()), 1)

    def test_a_bare_uuid_without_a_gpu_marker_is_not_a_hit(self):
        self.assertEqual(gate.scan(A.encode()), Counter())
        self.assertEqual(gate.scan(f"pci-{A}".encode()), Counter())

    def test_a_truncated_or_overlong_uuid_is_not_a_hit(self):
        self.assertEqual(gate.scan(b"GPU-a1a1a1a1-a1a1-a1a1"), Counter())
        self.assertEqual(gate.scan(f"GPU-{A}f".encode()), Counter())

    def test_counts_every_occurrence_per_uuid(self):
        found = gate.scan(f"GPU-{A} GPU-{A}\ngpu-uuid={A}\nGPU-{B}".encode())
        self.assertEqual(sorted(found.values()), [1, 3])

    def test_the_same_uuid_in_either_form_is_one_identity(self):
        self.assertEqual(
            gate.scan(f"GPU-{A}".encode()).keys(), gate.scan(f"gpu-uuid={A}".encode()).keys()
        )

    def test_finds_a_hit_inside_binary_data(self):
        data = b"\x00\xff\xfe" * 50 + f"GPU-{A}".encode() + b"\x00\x80" * 50
        self.assertEqual(sum(gate.scan(data).values()), 1)

    def test_fingerprint_is_stable_and_never_the_uuid(self):
        fp = gate.fingerprint(A.encode())
        self.assertEqual(fp, gate.fingerprint(A.upper().encode()))
        self.assertNotEqual(fp, gate.fingerprint(B.encode()))
        self.assertNotIn(A[:8], fp)
        self.assertNotIn(A.replace("-", ""), fp)


class CompareTests(unittest.TestCase):
    def test_nothing_new_and_nothing_stale_when_equal(self):
        base = {"traces/a.log": Counter({"x": 2})}
        self.assertEqual(gate.compare({"traces/a.log": Counter({"x": 2})}, base), ([], []))

    def test_a_new_file_with_a_hit_is_new(self):
        new, stale = gate.compare({"traces/n.log": Counter({"x": 1})}, {})
        self.assertEqual(new, [("traces/n.log", 1)])
        self.assertEqual(stale, [])

    def test_another_occurrence_in_a_known_file_is_new(self):
        new, _ = gate.compare(
            {"traces/a.log": Counter({"x": 3})}, {"traces/a.log": Counter({"x": 2})}
        )
        self.assertEqual(new, [("traces/a.log", 1)])

    def test_a_different_uuid_in_a_known_file_is_new_and_the_old_one_stale(self):
        new, stale = gate.compare(
            {"traces/a.log": Counter({"y": 1})}, {"traces/a.log": Counter({"x": 1})}
        )
        self.assertEqual(new, [("traces/a.log", 1)])
        self.assertEqual(stale, [("traces/a.log", 1)])

    def test_the_same_uuid_in_a_different_file_is_new(self):
        new, stale = gate.compare(
            {"traces/b.log": Counter({"x": 1})}, {"traces/a.log": Counter({"x": 1})}
        )
        self.assertEqual(new, [("traces/b.log", 1)])
        self.assertEqual(stale, [("traces/a.log", 1)])

    def test_fewer_occurrences_leave_the_baseline_stale(self):
        new, stale = gate.compare(
            {"traces/a.log": Counter({"x": 1})}, {"traces/a.log": Counter({"x": 3})}
        )
        self.assertEqual(new, [])
        self.assertEqual(stale, [("traces/a.log", 2)])


class ScopeTests(unittest.TestCase):
    def test_only_traces_and_docs_are_scanned(self):
        t = Tree()
        t.write("traces/a.log", f"GPU-{A}")
        t.write("docs/design/b.md", f"GPU-{B}")
        t.write("crates/kf-x/src/c.rs", f"GPU-{C}")
        t.write("README.md", f"GPU-{C}")
        self.assertEqual(sorted(gate.tracked(t.root)), ["docs/design/b.md", "traces/a.log"])

    def test_scan_tree_keys_files_by_relative_path(self):
        t = Tree()
        t.write("traces/run/a.log", f"x GPU-{A} y")
        found = gate.scan_tree(t.root, ["traces/run/a.log"])
        self.assertEqual(list(found), ["traces/run/a.log"])


class EndToEndTests(unittest.TestCase):
    def seeded(self):
        t = Tree()
        t.write("traces/old.log", f"GPU-{A}\nGPU-{A}\n")
        t.write("docs/old.md", f"gpu-uuid={B}\n")
        t.write("traces/clean.log", "nothing here\n")
        code, _ = t.run("--init")
        self.assertEqual(code, 0)
        return t

    def test_the_existing_hits_pass_once_baselined(self):
        t = self.seeded()
        self.assertEqual(t.run()[0], 0)

    def test_a_newly_added_file_with_a_uuid_fails_and_names_the_file_not_the_uuid(self):
        t = self.seeded()
        t.write("traces/new/run.log", f"boot GPU-{C}\n")
        code, text = t.run()
        self.assertEqual(code, 1)
        self.assertIn("traces/new/run.log", text)
        for needle in (C, C.replace("-", ""), C[:8]):
            self.assertNotIn(needle, text)

    def test_a_changed_file_gaining_a_second_uuid_fails(self):
        t = self.seeded()
        t.write("docs/old.md", f"gpu-uuid={B}\nGPU-{C}\n")
        code, text = t.run()
        self.assertEqual(code, 1)
        self.assertIn("docs/old.md", text)

    def test_a_changed_file_gaining_another_copy_of_a_known_uuid_fails(self):
        t = self.seeded()
        t.write("traces/old.log", f"GPU-{A}\nGPU-{A}\nGPU-{A}\n")
        self.assertEqual(t.run()[0], 1)

    def test_a_changed_file_that_keeps_its_hits_passes(self):
        t = self.seeded()
        t.write("traces/old.log", f"new line\nGPU-{A}\nmore\nGPU-{A}\n")
        self.assertEqual(t.run()[0], 0)

    def test_a_new_file_without_a_uuid_passes(self):
        t = self.seeded()
        t.write("traces/new.log", "no board identifiers\n")
        self.assertEqual(t.run()[0], 0)

    def test_a_scrubbed_file_makes_the_baseline_stale_and_fails(self):
        t = self.seeded()
        t.write("traces/old.log", "scrubbed\n")
        code, text = t.run()
        self.assertEqual(code, 1)
        self.assertIn("--shrink", text)

    def test_a_deleted_file_makes_the_baseline_stale_and_fails(self):
        t = self.seeded()
        t.remove("docs/old.md")
        self.assertEqual(t.run()[0], 1)

    def test_shrink_drops_what_is_gone_and_then_the_gate_passes(self):
        t = self.seeded()
        t.write("traces/old.log", f"GPU-{A}\n")
        t.remove("docs/old.md")
        self.assertEqual(t.run("--shrink")[0], 0)
        self.assertEqual(t.run()[0], 0)
        files = t.baseline()["files"]
        self.assertEqual(list(files), ["traces/old.log"])
        self.assertEqual(list(files["traces/old.log"].values()), [1])

    def test_shrink_never_admits_a_new_hit(self):
        t = self.seeded()
        t.write("traces/new.log", f"GPU-{C}\n")
        self.assertEqual(t.run("--shrink")[0], 1)
        self.assertNotIn("traces/new.log", t.baseline()["files"])
        self.assertEqual(t.run()[0], 1)

    def test_init_refuses_to_overwrite_a_baseline(self):
        t = self.seeded()
        t.write("traces/new.log", f"GPU-{C}\n")
        self.assertEqual(t.run("--init")[0], 2)
        self.assertNotIn("traces/new.log", t.baseline()["files"])

    def test_a_missing_baseline_is_an_error_not_a_pass(self):
        t = Tree()
        t.write("traces/new.log", f"GPU-{C}\n")
        self.assertEqual(t.run()[0], 2)

    def test_the_baseline_holds_fingerprints_only(self):
        t = self.seeded()
        text = (t.root / "baseline.json").read_text()
        self.assertEqual(re.findall(r"[0-9a-fA-F]{8}-[0-9a-fA-F]{4}", text), [])
        for needle in (A, B, A[:8], B[:8]):
            self.assertNotIn(needle, text)

    def test_the_baseline_is_deterministic(self):
        t = self.seeded()
        first = (t.root / "baseline.json").read_text()
        t.run("--shrink")
        self.assertEqual((t.root / "baseline.json").read_text(), first)


class RepositoryTests(unittest.TestCase):
    def test_the_committed_baseline_matches_the_tree(self):
        out = io.StringIO()
        with contextlib.redirect_stdout(out), contextlib.redirect_stderr(out):
            code = gate.main([])
        self.assertEqual(code, 0, out.getvalue())

    def test_the_committed_baseline_holds_no_uuid(self):
        text = (ROOT / "scripts/ci/gpu-uuids-baseline.json").read_text()
        self.assertEqual(re.findall(r"[0-9a-fA-F]{8}-[0-9a-fA-F]{4}-", text), [])

    def test_the_committed_baseline_only_names_files_under_traces_or_docs(self):
        files = json.loads((ROOT / "scripts/ci/gpu-uuids-baseline.json").read_text())["files"]
        self.assertGreater(len(files), 100)
        for rel in files:
            self.assertTrue(rel.startswith(("traces/", "docs/")), rel)

    def test_the_gate_is_a_step_of_the_stable_job(self):
        ci = (ROOT / ".github/workflows/ci.yml").read_text()
        self.assertTrue("python3 scripts/ci/no_gpu_uuids.py" in ci, "not a ci.yml step")


if __name__ == "__main__":
    unittest.main()
