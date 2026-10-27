"""Publication failures must be visible even when a PDF exists."""
import hashlib
import importlib.util
import json
from pathlib import Path
import subprocess
import tempfile
import unittest


class PublicationGate(unittest.TestCase):
    def setUp(self):
        spec = importlib.util.spec_from_file_location(
            "build_v3", Path(__file__).with_name("build_v3.py")
        )
        self.gate = importlib.util.module_from_spec(spec)
        spec.loader.exec_module(self.gate)
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name)
        self.git("init", "-q")
        self.git("config", "user.email", "test@example.invalid")
        self.git("config", "user.name", "Test")
        (self.root / "paper.tex").write_text("committed paper\n")
        self.git("add", "paper.tex")
        self.git("commit", "-qm", "source")

    def git(self, *args):
        return subprocess.check_output(["git", *args], cwd=self.root, text=True).strip()

    def test_dirty_main_source_is_refused(self):
        (self.root / "paper.tex").write_text("uncommitted correction\n")
        with self.assertRaisesRegex(ValueError, "committed"):
            self.gate.source_records(self.root, [self.root / "paper.tex"])

    def test_dirty_included_figure_is_refused(self):
        figure = self.root / "figure.tex"
        figure.write_text("original figure\n")
        self.git("add", "figure.tex")
        self.git("commit", "-qm", "figure")
        figure.write_text("draft figure\n")
        with self.assertRaisesRegex(ValueError, "committed"):
            self.gate.source_records(self.root, [figure])

    def test_untracked_included_figure_is_refused(self):
        figure = self.root / "figure.tex"
        figure.write_text("untracked figure\n")
        with self.assertRaisesRegex(ValueError, "committed"):
            self.gate.source_records(self.root, [self.root / "paper.tex", figure])

    def test_undefined_reference_is_refused(self):
        with self.assertRaises(ValueError):
            self.gate.check_log("LaTeX Warning: There were undefined references.\n")

    def test_missing_glyph_is_refused(self):
        with self.assertRaises(ValueError):
            self.gate.check_log("Missing character: There is no X in font Y!\n")

    def test_tex_error_is_refused_despite_pdf(self):
        with self.assertRaises(ValueError):
            self.gate.check_log("! Undefined control sequence.\nOutput written on paper.pdf")

    def test_clean_sources_and_log_are_accepted(self):
        records = self.gate.source_records(self.root, [self.root / "paper.tex"])
        self.assertEqual(records[0]["path"], "paper.tex")
        self.gate.check_log("Output written on paper.pdf (3 pages).\n")

    def manifest(self):
        pdf = self.root / "paper.pdf"
        pdf.write_bytes(b"original pdf")
        data = {
            "schema": 1,
            "source_commit": self.git("rev-parse", "HEAD"),
            "inputs": self.gate.source_records(self.root, [self.root / "paper.tex"]),
            "pdf_sha256": hashlib.sha256(pdf.read_bytes()).hexdigest(),
        }
        manifest = self.root / "paper.build.json"
        manifest.write_text(json.dumps(data))
        return pdf, manifest

    def test_changed_pdf_is_refused(self):
        pdf, manifest = self.manifest()
        pdf.write_bytes(b"stale or substituted pdf")
        with self.assertRaises(ValueError):
            self.gate.verify(self.root, pdf, manifest)

    def test_changed_manifest_digest_is_refused(self):
        pdf, manifest = self.manifest()
        data = json.loads(manifest.read_text())
        data["pdf_sha256"] = "0" * 64
        manifest.write_text(json.dumps(data))
        with self.assertRaises(ValueError):
            self.gate.verify(self.root, pdf, manifest)

    def test_manifest_accepts_matching_committed_source(self):
        self.gate.verify(self.root, *self.manifest())


if __name__ == "__main__":
    unittest.main()
