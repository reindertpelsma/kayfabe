# SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later
import json
from pathlib import Path
import subprocess
import sys
import tempfile
import unittest

from adobe_census import APPS, compare, parse, render_table, validate

FIXTURES = Path(__file__).with_name("adobe_census") / "fixtures"


def observed(app, name, state, reason="Observed setting"):
    return parse(app, [("observations", "observation.tsv",
                       f"version\t27.0\nfeature\t{name}\t{state}\t{reason}\n")])


def by_name(record):
    return {item["name"]: item for item in record["features"]}


class Census(unittest.TestCase):
    def test_photoshop_preference_and_capability_are_distinct(self):
        path = FIXTURES / "photoshop-system-info.txt"
        record = parse("Photoshop", [("system-info", path.name, path.read_text())])
        self.assertEqual(record["version"], "27.0.0")
        self.assertEqual(by_name(record)["opencl"]["state"], "disabled")
        self.assertEqual(by_name(record)["opencl-capable"]["state"], "enabled")
        self.assertEqual(by_name(record)["graphics-processor"]["state"], "unknown")

    def test_after_effects_selected_renderer(self):
        path = FIXTURES / "after-effects.tsv"
        record = parse("After Effects", [("observations", path.name, path.read_text())])
        self.assertEqual(by_name(record)["mercury.cuda"]["state"], "enabled")
        self.assertEqual(by_name(record)["mercury.opencl"]["state"], "disabled")
        self.assertEqual(by_name(record)["advanced-3d"]["state"], "disabled")

    def test_sniffer_reports_enumeration_without_claiming_renderer(self):
        path = FIXTURES / "mercury-sniffer.txt"
        for app in ("Premiere Pro", "Media Encoder", "After Effects"):
            with self.subTest(app=app):
                record = parse(app, [("sniffer", path.name, path.read_text())])
                self.assertEqual(by_name(record)["sniffer-device.cuda"]["state"], "enabled")
                self.assertEqual(by_name(record)["mercury.cuda"]["state"], "unknown")

    def test_photoshop_sniffer_only_records_complete_probe(self):
        for end, state in (("", "unknown"), ("End platform OpenCL\n", "enabled")):
            record = parse("Photoshop", [("sniffer", "sniffer.txt", "Start platform OpenCL\n" + end)])
            self.assertEqual(by_name(record)["sniffer-opencl-probe"]["state"], state)
            self.assertEqual(by_name(record)["opencl"]["state"], "unknown")

    def test_observations_for_other_apps(self):
        for app in ("Illustrator", "InDesign", "Acrobat", "Creative Cloud"):
            with self.subTest(app=app):
                name = APPS[app][0]
                record = observed(app, name, "unsupported")
                self.assertEqual(by_name(record)[name]["state"], "unsupported")
                self.assertEqual(record["raw_files"], ["observation.tsv"])

    def test_validator_rejects_invalid_records(self):
        record = observed("Acrobat", APPS["Acrobat"][0], "enabled")
        with self.assertRaises(ValueError):
            validate([record, record])
        with self.assertRaises(ValueError):
            validate([{**record, "raw_files": ["../observation.tsv"]}])
        with self.assertRaises(ValueError):
            validate([{**record, "raw_files": []}])
        with self.assertRaises(ValueError):
            validate([{**record, "features": record["features"] * 2}])

    def test_conflicting_input_rejected(self):
        with self.assertRaises(ValueError):
            parse("After Effects", [("observations", "obs.tsv", "renderer\tcuda\nrenderer\tsoftware")])

    def test_cli_manifest_parses_eight_apps_and_renders_both_formats(self):
        runner = [sys.executable, "-m", "adobe_census"]
        here = Path(__file__).parent

        def run(*args):
            return subprocess.check_output([*runner, *args], cwd=here, text=True)

        with tempfile.TemporaryDirectory() as folder:
            path = Path(folder)
            source = path / "acrob.tsv"
            source.write_text("feature\t2d-graphics-acceleration\tenabled\tChecked\n", encoding="utf-16")
            manifest = path / "manifest.json"
            manifest.write_text(json.dumps({"Acrobat": [{"path": source.name, "format": "observations"}]}))
            reference = path / "reference.json"
            reference.write_text(run("parse", manifest))
            self.assertEqual(len(json.loads(reference.read_text())), 8)
            run("validate", reference)
            guest = path / "kf3.json"
            guest.write_text("[]")
            self.assertEqual(json.loads(run("diff", reference, guest, "--format", "json"))[0]["direction"],
                             "reference-only")
            self.assertIn("Checked", run("diff", reference, guest))


class Diff(unittest.TestCase):
    def setUp(self):
        self.name = APPS["Acrobat"][0]
        self.enabled = [observed("Acrobat", self.name, "enabled", "Use is checked")]
        self.disabled = [observed("Acrobat", self.name, "disabled", "Use is unchecked")]
        self.unknown = [parse("Acrobat", [])]

    def test_reference_only(self):
        self.assertEqual([row["direction"] for row in compare(self.enabled, self.disabled)],
                         ["reference-only"])

    def test_both_directions(self):
        app = "Illustrator"
        name = APPS[app][0]
        ref = self.enabled + [observed(app, name, "unsupported")]
        guest = self.disabled + [observed(app, name, "enabled")]
        self.assertEqual([row["direction"] for row in compare(ref, guest)],
                         ["reference-only", "kf3-only"])

    def test_reasons_and_sources_preserved(self):
        row, = compare(self.enabled, self.disabled)
        self.assertEqual(row["reference"]["reason"], "Use is checked")
        self.assertEqual(row["kf3"]["reason"], "Use is unchecked")
        self.assertEqual(row["kf3"]["source"], "observation.tsv")

    def test_table_contains_both_reasons(self):
        text = render_table(compare(self.enabled, self.disabled))
        self.assertIn("Use is checked", text)
        self.assertIn("Use is unchecked", text)

    def test_unknown_is_not_enabled(self):
        self.assertEqual([row["kf3"]["state"] for row in compare(self.enabled, self.unknown)],
                         ["unknown"])

    def test_unknown_and_disabled_do_not_differ(self):
        self.assertEqual(compare(self.unknown, self.disabled), [])
        self.assertEqual(compare(self.disabled, self.unknown), [])

    def test_missing_app_and_equal_states(self):
        row, = compare([], self.enabled)
        self.assertEqual(row["reference"]["reason"], "Feature missing from reference census")
        self.assertEqual(compare(self.enabled, self.enabled), [])


if __name__ == "__main__":
    unittest.main()
