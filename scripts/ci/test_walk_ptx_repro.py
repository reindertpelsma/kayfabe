"""Tests of walk_ptx_repro.py (GPU-free, no NVRTC, no clang): every check must fire on a hostile tree.

STATUS: LIVE, 2026-11-13. The real NVRTC/clang rebuilds run in the `ptx` CI job; here the builder is a
fake runner, so what is tested is the comparison and refusal logic around it.
"""

import hashlib
import shutil
import subprocess
import sys
import tempfile
import unittest
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))
import walk_ptx_repro as w  # noqa: E402

ROOT = Path(__file__).resolve().parents[2]
FILES = [
    "cuda/display/kf_scanout.ptx",
    "cuda/display/kf_sum.ptx",
    "cuda/walk/kf_walk.cu",
    "cuda/walk/kf_walk.ptx",
    "crates/kf-cuda/src/walk.rs",
    "cuda/display/kf_yuv.ptx",
    "cuda/display/kf_color.ptx",
    "cuda/display/kf_bl_pack.ptx",
]


class Tree:
    def __enter__(self):
        self.td = tempfile.TemporaryDirectory()
        self.root = Path(self.td.name)
        for rel in FILES:
            (self.root / rel).parent.mkdir(parents=True, exist_ok=True)
            shutil.copyfile(ROOT / rel, self.root / rel)
        return self.root

    def __exit__(self, *a):
        self.td.cleanup()


def edit(path, old, new):
    s = path.read_text()
    assert s.count(old) >= 1, old
    path.write_text(s.replace(old, new, 1))


def fake_runner(write: bytes | None, rc=0):
    """A stand-in for make_ptx.py / make_*_ptx.sh: writes `write` to the output path it was given."""

    def run(cmd, **kw):
        if write is not None:
            if cmd[0] == "bash":  # display: the script writes next to itself
                script = Path(cmd[1])
                name = {v: k for k, v in w.GENERATED_DISPLAY.items()}[script.name]
                (script.parent / name).write_bytes(write.get(name) if isinstance(write, dict) else write)
            else:
                Path(cmd[-2]).write_bytes(write)
        return subprocess.CompletedProcess(cmd, rc, b"", b"boom" if rc else b"")

    return run


class Blobs(unittest.TestCase):
    def test_the_tree_matches_its_pins(self):
        self.assertEqual(w.check_blobs(ROOT), [])

    def test_an_edited_handwritten_ptx_fails(self):
        with Tree() as r:
            p = r / "cuda/display/kf_scanout.ptx"
            p.write_bytes(p.read_bytes() + b"\n// sneaky\n")
            self.assertTrue(any("kf_scanout.ptx" in f and "pinned" in f for f in w.check_blobs(r)))

    def test_each_blob_is_pinned_separately(self):
        with Tree() as r:
            p = r / "cuda/display/kf_sum.ptx"
            p.write_bytes(p.read_bytes().replace(b"add", b"sub", 1))
            out = w.check_blobs(r)
            self.assertEqual(len(out), 1)
            self.assertIn("kf_sum.ptx", out[0])

    def test_a_missing_blob_fails(self):
        with Tree() as r:
            (r / "cuda/display/kf_sum.ptx").unlink()
            self.assertTrue(any("missing" in f for f in w.check_blobs(r)))

    def test_both_missing_blobs_are_named(self):
        with Tree() as r:
            for rel in w.ACCEPTED_BLOBS:
                (r / rel).unlink()
            out = w.check_blobs(r)
            self.assertEqual(len(out), 2)
            self.assertTrue(all("missing" in f for f in out))

    def test_pins_are_real_sha256(self):
        for rel, want in w.ACCEPTED_BLOBS.items():
            self.assertEqual(hashlib.sha256((ROOT / rel).read_bytes()).hexdigest(), want)


class Constants(unittest.TestCase):
    def test_the_tree_agrees(self):
        self.assertEqual(w.check_constants(ROOT), [])

    def test_every_shared_constant_is_found_on_both_sides(self):
        k = w.kernel_constants((ROOT / "cuda/walk/kf_walk.cu").read_text())
        r = w.rust_constants((ROOT / "crates/kf-cuda/src/walk.rs").read_text())
        for name in w.SHARED_CONSTANTS:
            self.assertIn(name, k)
            self.assertIn(name, r)
        self.assertEqual(k["KF_SHWORDS"], 576)

    def test_launcher_shared_memory_disagreement_fails(self):
        with Tree() as r:
            edit(r / "crates/kf-cuda/src/walk.rs", "const KF_SHWORDS: u32 = KF_MAX_ENT + 64;",
                 "const KF_SHWORDS: u32 = KF_MAX_ENT + 32;")
            self.assertTrue(any(f.startswith("KF_SHWORDS") for f in w.check_constants(r)))

    def test_kernel_frontier_disagreement_fails(self):
        with Tree() as r:
            edit(r / "cuda/walk/kf_walk.cu", "#define KF_MAX_FRONTIER 131072u", "#define KF_MAX_FRONTIER 262144u")
            self.assertTrue(any(f.startswith("KF_MAX_FRONTIER") for f in w.check_constants(r)))

    def test_a_derived_constant_follows_its_base(self):
        with Tree() as r:
            edit(r / "cuda/walk/kf_walk.cu", "#define KF_MAX_ENT 512u", "#define KF_MAX_ENT 1024u")
            out = w.check_constants(r)
            self.assertTrue(any(f.startswith("KF_SHWORDS") for f in out))
            self.assertTrue(any(f.startswith("KF_MAX_ENT") for f in out))

    def test_a_missing_constant_fails(self):
        with Tree() as r:
            edit(r / "crates/kf-cuda/src/walk.rs", "const KF_WARP: u32 = 32;", "const KF_WARPS: u32 = 32;")
            self.assertTrue(any("KF_WARP" in f and "not defined" in f for f in w.check_constants(r)))

    def test_a_missing_launcher_constant_is_named_alone(self):
        with Tree() as r:
            edit(r / "crates/kf-cuda/src/walk.rs", "const KF_PAR_GRID: u32 = 128;", "const KF_GRID: u32 = 128;")
            self.assertEqual(w.check_constants(r), ["KF_PAR_GRID: not defined in crates/kf-cuda/src/walk.rs"])

    def test_an_unevaluable_expression_is_an_error_not_a_pass(self):
        with self.assertRaises(ValueError):
            w._eval_const("os.system(1)", {})
        with self.assertRaises(ValueError):
            w._eval_const("KF_UNKNOWN + 1", {})

    def test_a_call_in_a_constant_is_an_error(self):
        for bad in ("__import__('os')", "u32::MAX", "max(1, 2)"):
            with self.assertRaises(ValueError):
                w._eval_const(bad, {})


class Compare(unittest.TestCase):
    def test_identical(self):
        self.assertEqual(w.compare("x", b"a\nb\n", b"a\nb\n"), [])

    def test_one_line_differs(self):
        out = w.compare("x", b"a\nb\nc\n", b"a\nB\nc\n")
        self.assertEqual(len(out), 1)
        self.assertIn("line 2", out[0])

    def test_trailing_difference(self):
        self.assertTrue(w.compare("x", b"a\nb\n", b"a\nb\nc\n"))


class Walk(unittest.TestCase):
    def committed(self):
        return (ROOT / "cuda/walk/kf_walk.ptx").read_bytes()

    def test_identical_rebuild_passes(self):
        self.assertEqual(w.check_walk(ROOT, "x.so", run=fake_runner(self.committed())), [])

    def test_a_rebuild_that_differs_fails(self):
        out = w.check_walk(ROOT, "x.so", run=fake_runner(self.committed().replace(b"setp.lt", b"setp.le", 1)))
        self.assertTrue(out and "differs" in out[0])

    def test_a_failed_rebuild_fails(self):
        out = w.check_walk(ROOT, "x.so", run=fake_runner(None, rc=1))
        self.assertTrue(out and "rebuild failed" in out[0])

    def test_a_nonzero_exit_fails_even_with_an_output(self):
        out = w.check_walk(ROOT, "x.so", run=fake_runner(self.committed(), rc=1))
        self.assertTrue(out and "rebuild failed" in out[0])

    def test_a_crashed_builder_with_a_matching_output_still_fails(self):
        out = w.check_walk(ROOT, "x.so", run=fake_runner(self.committed(), rc=139))
        self.assertEqual(len(out), 1)
        self.assertIn("rc=139", out[0])

    def test_a_rebuild_without_a_compiler_header_is_refused(self):
        body = b"\n".join(l for l in self.committed().splitlines() if b"Cuda compilation tools" not in l)
        out = w.check_walk(ROOT, "x.so", run=fake_runner(body))
        self.assertTrue(out and "not 12.2.140" in out[0])

    def test_another_nvrtc_is_refused(self):
        other = self.committed().replace(b"release 12.2, V12.2.140", b"release 12.9, V12.9.86")
        out = w.check_walk(ROOT, "x.so", run=fake_runner(other))
        self.assertTrue(out and "not 12.2.140" in out[0])

    def test_a_committed_ptx_from_another_compiler_is_refused(self):
        with Tree() as r:
            p = r / "cuda/walk/kf_walk.ptx"
            p.write_bytes(p.read_bytes().replace(b"V12.2.140", b"V12.9.86", 1))
            out = w.check_walk(r, "x.so", run=fake_runner(p.read_bytes()))
            self.assertTrue(out and "pinned compiler" in out[0])

    def test_a_committed_ptx_without_a_header_is_refused(self):
        with Tree() as r:
            p = r / "cuda/walk/kf_walk.ptx"
            body = b"\n".join(l for l in p.read_bytes().splitlines() if b"Cuda compilation tools" not in l)
            p.write_bytes(body)
            out = w.check_walk(r, "x.so", run=fake_runner(self.committed()))
            self.assertTrue(out and "pinned compiler" in out[0])


class Display(unittest.TestCase):
    def recorded(self):
        return w.recorded_compiler((ROOT / "cuda/display/kf_yuv.ptx").read_bytes())

    def committed(self):
        return {n: (ROOT / "cuda/display" / n).read_bytes() for n in w.GENERATED_DISPLAY}

    def tree_with_scripts(self):
        t = Tree()
        r = t.__enter__()
        for s in w.GENERATED_DISPLAY.values():
            shutil.copyfile(ROOT / "cuda/display" / s, r / "cuda/display" / s)
        return t, r

    def test_every_generated_file_records_one_compiler(self):
        names = {w.recorded_compiler((ROOT / "cuda/display" / n).read_bytes()) for n in w.GENERATED_DISPLAY}
        self.assertEqual(len(names), 1)
        self.assertIsNotNone(self.recorded())

    def test_another_clang_is_refused_not_compared(self):
        out = w.check_display(ROOT, clang_version="Ubuntu clang version 18.1.3", run=fake_runner(self.committed()))
        self.assertEqual(len(out), len(w.GENERATED_DISPLAY))
        self.assertTrue(all("refusing" in f for f in out))

    def test_the_clang_on_path_is_what_is_compared(self):
        def run(cmd, **kw):
            if cmd[:2] == ["clang", "--version"]:
                return subprocess.CompletedProcess(cmd, 0, b"clang version 99.0.0\nTarget: x\n", b"")
            return fake_runner(self.committed())(cmd, **kw)

        out = w.check_display(ROOT, run=run)
        self.assertTrue(out and all("refusing" in f for f in out))

    def test_identical_rebuild_passes(self):
        t, r = self.tree_with_scripts()
        try:
            self.assertEqual(w.check_display(r, clang_version=self.recorded(), run=fake_runner(self.committed())), [])
        finally:
            t.__exit__()

    def test_a_rebuild_that_differs_fails(self):
        t, r = self.tree_with_scripts()
        try:
            bad = self.committed()
            bad["kf_color.ptx"] = bad["kf_color.ptx"] + b"\n"
            out = w.check_display(r, clang_version=self.recorded(), run=fake_runner(bad))
            self.assertEqual(len(out), 1)
            self.assertIn("kf_color.ptx", out[0])
        finally:
            t.__exit__()

    def test_a_failed_script_fails(self):
        t, r = self.tree_with_scripts()
        try:
            out = w.check_display(r, clang_version=self.recorded(), run=fake_runner(None, rc=1))
            self.assertEqual(len(out), len(w.GENERATED_DISPLAY))
        finally:
            t.__exit__()

    def test_a_failed_script_fails_even_with_an_output(self):
        t, r = self.tree_with_scripts()
        try:
            out = w.check_display(r, clang_version=self.recorded(), run=fake_runner(self.committed(), rc=1))
            self.assertEqual(len(out), len(w.GENERATED_DISPLAY))
            self.assertTrue(all("failed" in f for f in out))
        finally:
            t.__exit__()

    def test_each_generated_file_is_compared(self):
        t, r = self.tree_with_scripts()
        try:
            bad = self.committed()
            bad["kf_yuv.ptx"] = bad["kf_yuv.ptx"] + b"// x\n"
            out = w.check_display(r, clang_version=self.recorded(), run=fake_runner(bad))
            self.assertEqual(len(out), 1)
            self.assertIn("kf_yuv.ptx", out[0])
        finally:
            t.__exit__()


class Main(unittest.TestCase):
    def test_unknown_command_is_usage(self):
        self.assertEqual(w.main(["nope"]), 2)

    def test_walk_needs_the_nvrtc_path(self):
        self.assertEqual(w.main(["walk"]), 2)


if __name__ == "__main__":
    unittest.main()
