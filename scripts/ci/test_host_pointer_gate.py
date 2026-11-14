#!/usr/bin/env python3
"""Known-positive and known-negative fixtures for scripts/ci/host_pointer_gate.py (S1-12: a gate
with no known positive proves nothing). Stdlib only; GPU-free.

    python3 -B scripts/ci/test_host_pointer_gate.py
"""

import subprocess
import sys
import tempfile
import unittest
from pathlib import Path

HERE = Path(__file__).resolve().parent
sys.path.insert(0, str(HERE))
import host_pointer_gate as gate  # noqa: E402


def tree(files: dict) -> Path:
    root = Path(tempfile.mkdtemp(prefix="hpgate-"))
    for rel, text in files.items():
        p = root / rel
        p.parent.mkdir(parents=True, exist_ok=True)
        p.write_text(text)
    return root


class Positives(unittest.TestCase):
    def hits(self, text, name="crates/kf-x/src/lib.rs"):
        return gate.scan(tree({name: text}))

    def test_review1_f01_exact_line(self):
        line = "params[24..32].copy_from_slice(&(ops.as_mut_ptr() as u64).to_le_bytes());\n"
        self.assertEqual(len(self.hits(line)), 1)

    def test_as_ptr_to_u64(self):
        self.assertTrue(self.hits("let a = buf.as_ptr() as u64;\n"))

    def test_cast_to_i64(self):
        self.assertTrue(self.hits("let a = v.as_mut_ptr() as i64;\n"))

    def test_cast_to_u128(self):
        self.assertTrue(self.hits("let a = v.as_ptr() as u128;\n"))

    def test_cast_to_i128(self):
        self.assertTrue(self.hits("let a = v.as_mut_ptr() as i128;\n"))

    def test_cast_split_across_lines(self):
        found = self.hits("let a = ops\n    .as_mut_ptr()\n    as u64;\n")
        self.assertEqual(len(found), 1)
        self.assertTrue(found[0].startswith("crates/kf-x/src/lib.rs:2:"), found)

    def test_cast_split_before_the_type(self):
        self.assertTrue(self.hits("let a = ops.as_ptr() as\n    u64;\n"))

    def test_mut_ptr_type(self):
        self.assertTrue(self.hits("let p: *mut u8 = x;\n"))

    def test_const_ptr_type(self):
        self.assertTrue(self.hits("let p: *const u8 = x;\n"))

    def test_non_null(self):
        self.assertTrue(self.hits("let p = NonNull::dangling();\n"))

    def test_transmute(self):
        self.assertTrue(self.hits("let p = std::mem::transmute(x);\n"))

    def test_raw_const_borrow(self):
        self.assertTrue(self.hits("let p = &raw const x;\n"))

    def test_raw_mut_borrow(self):
        self.assertTrue(self.hits("let p = &raw mut x;\n"))

    def test_addr_of(self):
        self.assertTrue(self.hits("let p = addr_of!(x);\n"))

    def test_addr_of_mut(self):
        self.assertTrue(self.hits("let p = std::ptr::addr_of_mut!(x);\n"))

    def test_expose_provenance(self):
        self.assertTrue(self.hits("let a = p.expose_provenance();\n"))

    def test_with_exposed_provenance(self):
        self.assertTrue(self.hits("let p = std::ptr::with_exposed_provenance::<u8>(a);\n"))

    def test_test_files_are_in_scope(self):
        self.assertTrue(self.hits("let a = b.as_ptr() as u64;\n", "crates/kf-x/tests/t.rs"))

    def test_test_support_files_are_in_scope(self):
        self.assertTrue(self.hits("let p: *const u8 = x;\n", "crates/kf-x/tests/support/s.rs"))

    def test_main_exits_one_and_names_the_file(self):
        root = tree({"crates/kf-x/src/a.rs": "let a = b.as_ptr() as u64;\n"})
        out = subprocess.run([sys.executable, "-B", str(HERE / "host_pointer_gate.py"), str(root)],
                             capture_output=True, text=True)
        self.assertEqual(out.returncode, 1)
        self.assertIn("crates/kf-x/src/a.rs:1:", out.stdout)

    def test_main_exits_one_for_a_type_name(self):
        root = tree({"crates/kf-x/src/b.rs": "fn f() {}\nlet p: *mut u8 = q;\n"})
        out = subprocess.run([sys.executable, "-B", str(HERE / "host_pointer_gate.py"), str(root)],
                             capture_output=True, text=True)
        self.assertEqual(out.returncode, 1)
        self.assertIn("crates/kf-x/src/b.rs:2:", out.stdout)


class Negatives(unittest.TestCase):
    def hits(self, text, name="crates/kf-x/src/lib.rs"):
        return gate.scan(tree({name: text}))

    def test_unsafe_files_are_the_perimeter(self):
        self.assertEqual(self.hits("let a = b.as_ptr() as u64;\n", "crates/kf-x/src/io_unsafe.rs"), [])

    def test_comments_may_name_the_forms(self):
        self.assertEqual(self.hits("// never write `ops.as_mut_ptr() as u64` here\n"), [])
        self.assertEqual(self.hits("let n = 1; // *mut u8 is forbidden\n"), [])

    def test_phantom_data_is_erased(self):
        self.assertEqual(self.hits("struct S(PhantomData<*mut ()>);\n"), [])

    def test_usize_identity_keys_are_out_of_scope(self):
        self.assertEqual(self.hits("let k = what.as_ptr() as usize;\n"), [])

    def test_unrelated_casts(self):
        self.assertEqual(self.hits("let a = len as u64; let b = v.as_slice().len() as u64;\n"), [])

    def test_excluded_trees(self):
        for d in ("archive/x.rs", "third_party/y/z.rs", "target/debug/a.rs",
                  "crates/kayfabe-doorbell/src/a.rs", "mutants.out/a.rs"):
            self.assertEqual(self.hits("let a = b.as_ptr() as u64;\n", d), [], d)

    def test_clean_tree_exits_zero(self):
        root = tree({"crates/kf-x/src/a.rs": "fn f() {}\n"})
        out = subprocess.run([sys.executable, "-B", str(HERE / "host_pointer_gate.py"), str(root)],
                             capture_output=True, text=True)
        self.assertEqual(out.returncode, 0, out.stdout)


class TheRealTree(unittest.TestCase):
    def test_repository_is_clean(self):
        self.assertEqual(gate.scan(HERE.parents[1]), [])


if __name__ == "__main__":
    unittest.main()
