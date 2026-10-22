"""Generated files that derive from NVIDIA's MIT-licensed sources carry NVIDIA's notice.

MIT asks that the copyright notice and the permission notice travel with copies and substantial
portions. The canonical texts are `tools/legal/nvidia_*_mit.txt`; each generator writes one as a
comment block at the top of what it emits, and the generated files that are committed carry it.
"""

import importlib.util
import subprocess
import sys
import tempfile
import unittest
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
LEGAL = ROOT / "tools" / "legal"
OGKM = (LEGAL / "nvidia_ogkm_mit.txt").read_text()
OPEN_GPU_DOC = (LEGAL / "nvidia_open_gpu_doc_mit.txt").read_text()
HEAD_LINES = 80

# Committed generated files that derive from ogkm, with their comment marker.
OGKM_FILES = {
    "crates/kf-abi/src/generated/classes.rs": "//",
    "crates/kf-abi/src/generated/ctrl.rs": "//",
    "crates/kf-abi/src/generated/mod.rs": "//",
    "crates/kf-abi/src/generated/nvos.rs": "//",
    "crates/kf-abi/src/generated/rpc.rs": "//",
    "crates/kf-abi/src/generated/vbios.rs": "//",
    "crates/kf-abi/src/generated/matrix.rs": "//",
    "crates/kf-chip/data/hwref-580.159.04.tsv": "#",
}
OPEN_GPU_DOC_FILES = {"crates/kf-chan/src/video_generated.rs": "//"}
# Generated files that carry no NVIDIA text: version strings only, with their own SPDX line.
EXEMPT = {"crates/kf-abi/src/generated/windows_twins.rs"}


def commented(text, marker):
    """The notice as comment lines, blank lines kept as a bare marker."""
    return [f"{marker} {line}" if line else marker for line in text.splitlines()]


def carries(text, notice, marker, head=HEAD_LINES):
    """True when the notice sits, whole and in order, in the first `head` lines of text."""
    block = commented(notice, marker)
    lines = text.splitlines()[:head]
    n = len(block)
    return any(lines[i : i + n] == block for i in range(len(lines) - n + 1))


def load(name, relpath):
    spec = importlib.util.spec_from_file_location(name, ROOT / relpath)
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


class NoticeTextTests(unittest.TestCase):
    def test_ogkm_text_is_the_upstream_mit_notice(self):
        for needle in (
            "SPDX-License-Identifier: MIT",
            "Copyright (c) 2021 NVIDIA CORPORATION & AFFILIATES. All rights reserved.",
            "Permission is hereby granted, free of charge",
            "The above copyright notice and this permission notice shall be included in",
            'THE SOFTWARE IS PROVIDED "AS IS", WITHOUT WARRANTY OF ANY KIND',
        ):
            self.assertIn(needle, OGKM)

    def test_open_gpu_doc_text_is_the_upstream_mit_notice(self):
        for needle in (
            "SPDX-License-Identifier: MIT",
            "Copyright (c) 2019, NVIDIA CORPORATION. All rights reserved.",
            "Permission is hereby granted, free of charge",
            'THE SOFTWARE IS PROVIDED "AS IS", WITHOUT WARRANTY OF ANY KIND',
        ):
            self.assertIn(needle, OPEN_GPU_DOC)

    def test_third_party_notices_quotes_both_texts_verbatim(self):
        doc = (ROOT / "THIRD_PARTY_NOTICES.md").read_text()
        for text in (OGKM, OPEN_GPU_DOC):
            body = text.split("\n\n", 1)[1].rstrip("\n")
            quoted = "\n".join(f"    {l}" if l else "" for l in body.splitlines())
            self.assertIn(quoted, doc)


class HelperTests(unittest.TestCase):
    def test_cli_prints_the_ogkm_notice_behind_the_marker(self):
        out = subprocess.run(
            [sys.executable, str(LEGAL / "notice.py"), "ogkm", "#"],
            check=True,
            capture_output=True,
            text=True,
        ).stdout
        self.assertEqual(out.splitlines(), commented(OGKM, "#"))

    def test_cli_prints_the_open_gpu_doc_notice_behind_the_marker(self):
        out = subprocess.run(
            [sys.executable, str(LEGAL / "notice.py"), "open-gpu-doc", "//"],
            check=True,
            capture_output=True,
            text=True,
        ).stdout
        self.assertEqual(out.splitlines(), commented(OPEN_GPU_DOC, "//"))

    def test_cli_refuses_an_unknown_source(self):
        run = subprocess.run(
            [sys.executable, str(LEGAL / "notice.py"), "nowhere", "#"], capture_output=True
        )
        self.assertNotEqual(run.returncode, 0)


class GeneratorTests(unittest.TestCase):
    def test_matrix_generator_writes_the_ogkm_notice(self):
        with tempfile.TemporaryDirectory() as tmp:
            out = Path(tmp) / "matrix.rs"
            subprocess.run(
                [
                    sys.executable,
                    str(ROOT / "tools/drivermatrix/emit_rust.py"),
                    "--ranges",
                    str(ROOT / "traces/driver_matrix/ranges.tsv"),
                    "--out",
                    str(out),
                ],
                check=True,
            )
            self.assertTrue(carries(out.read_text(), OGKM, "//"))

    def test_video_generator_header_writes_the_open_gpu_doc_notice(self):
        module = load("derive_video_methods", "tools/derive_video_methods.py")
        text = "\n".join(module.header("abc123", ["clc9b0.h"]))
        self.assertTrue(carries(text, OPEN_GPU_DOC, "//"))
        self.assertIn("NVIDIA/open-gpu-doc @ abc123", text)

    def test_hwref_generator_emits_the_notice_through_the_helper(self):
        script = (ROOT / "tools/derive_hwref.sh").read_text()
        self.assertTrue("legal/notice.py" in script, "derive_hwref.sh must call the helper")
        call = script.split("legal/notice.py", 1)[1].split("\n", 1)[0]
        self.assertTrue(" ogkm " in call, "the helper must be asked for the ogkm notice")


class CommittedFileTests(unittest.TestCase):
    def test_committed_ogkm_derived_files_carry_the_notice(self):
        for rel, marker in OGKM_FILES.items():
            with self.subTest(file=rel):
                self.assertTrue(carries((ROOT / rel).read_text(), OGKM, marker))

    def test_committed_open_gpu_doc_derived_files_carry_the_notice(self):
        for rel, marker in OPEN_GPU_DOC_FILES.items():
            with self.subTest(file=rel):
                self.assertTrue(carries((ROOT / rel).read_text(), OPEN_GPU_DOC, marker))

    def test_no_generated_module_escapes_the_list(self):
        listed = set(OGKM_FILES) | set(OPEN_GPU_DOC_FILES) | EXEMPT
        for path in (ROOT / "crates/kf-abi/src/generated").glob("*.rs"):
            self.assertIn(str(path.relative_to(ROOT)), listed)


if __name__ == "__main__":
    unittest.main()
