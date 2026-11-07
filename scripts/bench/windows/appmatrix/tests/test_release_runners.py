# SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later
"""Runner wiring against synthetic files and fake processes/Blender; no CUDA, browser, or VM."""
import contextlib
import hashlib
import io
import json
from pathlib import Path
import subprocess
import sys
import tempfile
from types import SimpleNamespace as NS
import unittest
from unittest.mock import patch

import helpers as H
sys.path.insert(0, str(Path(H.AM) / "guest/py"))
import llama_release as L
import blender_release as B


class Llama(unittest.TestCase):
    def setUp(self):
        tmp = tempfile.TemporaryDirectory()
        self.addCleanup(tmp.cleanup)
        self.root = Path(tmp.name)
        for name in ("fake.exe", "model.gguf"):
            (self.root / name).write_bytes(b"test-only, never executed")
        self.answer = "A fixed prompt produced a sufficiently long deterministic answer."
        raw = self.answer.encode()
        (self.root / "llama_cuda.txt").write_bytes(raw)
        self.meta = dict(schema=1, kind="bare-metal", revision="1" * 40, date="2026-11-07", evidence="synthetic/test-only",
                         sha256=hashlib.sha256(raw).hexdigest(), prompt=L.PROMPT, seed=1, tokens=64, temperature=0,
                         executable_sha256=L.file_hash(self.root / "fake.exe"), model_sha256=L.file_hash(self.root / "model.gguf"))
        self.write_meta()

    def write_meta(self):
        (self.root / "llama_cuda.json").write_text(json.dumps(self.meta))

    def run_fake(self, output=None, log="Device 0: NVIDIA GeForce RTX 4090\noffloaded 29/29 layers to GPU", rc=0):
        p = subprocess.CompletedProcess([], rc, (self.answer if output is None else output).encode(), log.encode())
        with patch.object(L.subprocess, "run", return_value=p) as proc, contextlib.redirect_stdout(io.StringIO()) as out:
            result = L.main([str(self.root / "fake.exe"), str(self.root / "model.gguf"), str(self.root), str(self.root / "out")])
        return result, out.getvalue(), proc

    def test_success_checks_reference_and_fixed_command(self):
        rc, log, proc = self.run_fake()
        self.assertEqual(rc, 0, log)
        self.assertIn("LLM_PARITY_PASS", log)
        args = proc.call_args.args[0]
        for flag, value in (("-p", L.PROMPT), ("--seed", "1"), ("--temp", "0"), ("-n", "64"), ("-ngl", "99")):
            self.assertEqual(args[args.index(flag) + 1], value)
        self.assertEqual(proc.call_args.kwargs["timeout"], 480)

    def test_wrong_answer_is_a_failure(self):
        self.assertEqual(self.run_fake(output="A completely different sufficiently long answer.")[0], 1)

    def test_empty_output_is_a_failure(self):
        self.assertEqual(self.run_fake(output="")[0], 1)

    def test_cpu_fallback_is_a_failure(self):
        self.assertEqual(self.run_fake(log="CPU produced a matching output")[0], 1)

    def test_partial_offload_is_a_failure(self):
        self.assertEqual(self.run_fake(log="Device 0: NVIDIA\noffloaded 1/29 layers to GPU")[0], 1)

    def test_missing_reference_never_starts_inference(self):
        (self.root / "llama_cuda.txt").unlink()
        rc, log, proc = self.run_fake()
        self.assertEqual(rc, 3)
        self.assertIn("REFERENCE_NOTFOUND", log)
        proc.assert_not_called()

    def test_wrong_seed_reference_never_starts_inference(self):
        self.meta["seed"] = 2
        self.write_meta()
        rc, log, proc = self.run_fake()
        self.assertEqual(rc, 1)
        proc.assert_not_called()

    def test_guest_reference_never_starts_inference(self):
        self.meta["kind"] = "guest"
        self.write_meta()
        rc, log, proc = self.run_fake()
        self.assertEqual(rc, 1)
        proc.assert_not_called()


class Blender(unittest.TestCase):
    def setUp(self):
        tmp = tempfile.TemporaryDirectory()
        self.addCleanup(tmp.cleanup)
        self.root = Path(tmp.name)
        (self.root / "blender_render.py").write_text("# synthetic scene, never run")
        (self.root / "blender_CUDA.png").write_bytes(b"synthetic reference, not a real image")
        raw = (self.root / "blender_CUDA.png").read_bytes()
        self.meta = dict(schema=1, kind="bare-metal", revision="1" * 40, date="2026-11-07", evidence="synthetic/test-only",
                         sha256=hashlib.sha256(raw).hexdigest(), backend="CUDA", gpu="NVIDIA test GPU", seed=1, samples=256,
                         width=960, height=540, blender_version="4.5.0", scene_sha256=hashlib.sha256((self.root / "blender_render.py").read_bytes()).hexdigest())
        self.write_meta()
        prefs = NS(compute_device_type=None, get_devices=lambda: None, devices=[NS(type="CUDA", name="NVIDIA test GPU")])
        self.images = NS(load=lambda *a, **kw: NS(size=[960, 540], pixels=[1, 2, 3, 4]))
        self.bpy = NS(context=NS(preferences=NS(addons={"cycles": NS(preferences=prefs)})), app=NS(version_string="4.5.0"), data=NS(images=self.images))

    def write_meta(self):
        (self.root / "blender_CUDA.json").write_text(json.dumps(self.meta))

    def run_fake(self, error=None):
        argv = ["blender", "--", "CUDA", str(self.root / "out.png"), str(self.root)]
        with patch.dict(sys.modules, {"bpy": self.bpy}), patch.object(B, "__file__", str(self.root / "blender_release.py")), \
                patch.object(sys, "argv", argv), patch.object(B.runpy, "run_path") as render, \
                patch.object(B, "compare_pixels", return_value={"mae": 0, "max_error": 0}, side_effect=error) as compare, \
                contextlib.redirect_stdout(io.StringIO()) as out:
            rc = B.main()
        return rc, out.getvalue(), render, compare

    def test_success_uses_scene_and_pixel_oracle(self):
        rc, log, render, compare = self.run_fake()
        self.assertEqual(rc, 0, log)
        self.assertIn("CYCLES_PARITY_PASS CUDA", log)
        render.assert_called_once()
        self.assertEqual(compare.call_args.args[-2:], (960, 540))

    def test_pixel_mismatch_fails_the_runner(self):
        rc, log, *_ = self.run_fake(ValueError("image mismatch"))
        self.assertEqual(rc, 1)
        self.assertNotIn("CYCLES_PARITY_PASS", log)

    def test_blank_image_fails_the_runner(self):
        self.assertEqual(self.run_fake(ValueError("blank image"))[0], 1)

    def test_wrong_seed_reference_prevents_render(self):
        self.meta["seed"] = 2
        self.write_meta()
        rc, log, render, compare = self.run_fake()
        self.assertEqual(rc, 1)
        render.assert_not_called()

    def test_guest_reference_prevents_render(self):
        self.meta["kind"] = "guest"
        self.write_meta()
        rc, log, render, compare = self.run_fake()
        self.assertEqual(rc, 1)
        render.assert_not_called()


if __name__ == "__main__":
    unittest.main()
