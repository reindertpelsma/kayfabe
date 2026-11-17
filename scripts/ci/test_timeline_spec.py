#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later
"""GPU-free, network-free contracts for scripts/bench/assets/timeline_spec.py.

Falsifier: two generations differ; the OTIO has the wrong schema names, a source range outside its
available range, durations that do not sum, or a transition that is not between clips; the FCPXML
does not parse or has the wrong number of asset-clips, transitions or titles; an EDL line does not
match CMX3600 layout or record times are not continuous; the export matrix misses a field; `check`
accepts a directory in which one byte changed.

Run: python3 -I scripts/ci/test_timeline_spec.py
"""

import contextlib
import io
import json
import re
import shutil
import sys
import tempfile
import unittest
import xml.etree.ElementTree as ET
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parents[1] / "bench" / "assets"))
import timeline_spec as ts  # noqa: E402


def quiet(fn, *a):
    with contextlib.redirect_stdout(io.StringIO()) as buf:
        rc = fn(*a)
    return rc, buf.getvalue()


class TimelineSpecTest(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        cls.tmp = Path(tempfile.mkdtemp(prefix="timeline_spec_"))
        cls.out = cls.tmp / "spec"
        rc, _ = quiet(ts.cmd_spec, cls.out)
        assert rc == 0
        cls.otio = json.loads((cls.out / "timeline.otio").read_text())

    @classmethod
    def tearDownClass(cls):
        shutil.rmtree(cls.tmp, ignore_errors=True)

    def test_deterministic(self):
        a, b = ts.generate(), ts.generate()
        self.assertEqual(a, b)
        quiet(ts.cmd_spec, self.tmp / "again")
        for name in ts.FILES:
            self.assertEqual((self.out / name).read_bytes(), (self.tmp / "again" / name).read_bytes())

    def test_check_passes_on_fresh_spec(self):
        rc, out = quiet(ts.cmd_check, self.out)
        self.assertEqual(rc, 0, out)

    def test_otio_structure(self):
        d = self.otio
        self.assertEqual(d["OTIO_SCHEMA"], "Timeline.1")
        self.assertEqual(d["tracks"]["OTIO_SCHEMA"], "Stack.1")
        tracks = d["tracks"]["children"]
        self.assertEqual([t["OTIO_SCHEMA"] for t in tracks], ["Track.1"] * 3)
        v1 = tracks[0]["children"]
        clips = [c for c in v1 if c["OTIO_SCHEMA"] == "Clip.2"]
        self.assertEqual(len(clips), 12)
        self.assertEqual(sum(1 for c in v1 if c["OTIO_SCHEMA"] == "Transition.1"), 3)
        for c in clips:
            ref = c["media_references"][c["active_media_reference_key"]]
            self.assertEqual(ref["OTIO_SCHEMA"], "ExternalReference.1")
            self.assertEqual(c["source_range"]["OTIO_SCHEMA"], "TimeRange.1")
            self.assertEqual(c["source_range"]["start_time"]["OTIO_SCHEMA"], "RationalTime.1")
            g = c["metadata"]["kayfabe"]["grade"]
            for key in ("lift", "gamma", "gain", "saturation", "tone_curve"):
                self.assertIn(key, g)
            self.assertEqual(len(g["tone_curve"]), 5)
        gaps = [c for t in tracks for c in t["children"] if c["OTIO_SCHEMA"] == "Gap.1"]
        self.assertTrue(gaps)
        scalars = sorted(e["time_scalar"] for c in clips for e in c["effects"]
                         if e["OTIO_SCHEMA"] == "LinearTimeWarp.1")
        self.assertEqual(scalars, [-1.0, 0.0, 0.5, 0.5, 2.0])
        self.assertEqual(sum(ts._dur(c["source_range"]) for c in clips), 1500)
        names = {Path(c["media_references"]["DEFAULT_MEDIA"]["target_url"]).name for c in clips}
        self.assertIn("sintel_trailer_1080p.mp4", names)
        self.assertIn("bbb_trailer_720p.mov", names)
        titles = [c for c in tracks[1]["children"] if c["OTIO_SCHEMA"] == "Clip.2"]
        self.assertEqual(len(titles), 2)
        self.assertEqual(titles[0]["effects"][0]["effect_name"], "Title")
        self.assertEqual(len(tracks[2]["children"][0]["metadata"]["kayfabe"]["gain_keyframes"]),
                         len(ts.GAIN_KEYFRAMES))

    def test_pinned_names_exist_in_manifest(self):
        manifest = json.loads((Path(ts.__file__).with_name("test_assets.json")).read_text())
        names = {a["name"] for a in manifest["assets"]}
        for s in ts.SOURCES.values():
            if s["kind"] == "pinned":
                self.assertIn(s["path"], names)

    def test_synthetic_command_lines(self):
        cmds = ts.synthetic_sources()
        self.assertEqual(len(cmds), 4)
        for c in cmds:
            self.assertTrue(c["output"].startswith("synthetic/") and c["output"].endswith(".mp4"))
            self.assertIn("-bitexact", c["argv"])
        v = [c for c in cmds if c["name"] == "testsrc2"][0]["argv"]
        for tok in ("libx264", "veryfast", "18", "yuv420p", "+bitexact", "testsrc2=size=3840x2160:rate=25"):
            self.assertIn(tok, v)

    def test_fcpxml(self):
        text = (self.out / "timeline.fcpxml").read_text()
        root = ET.fromstring(text.split("<!DOCTYPE fcpxml>", 1)[1])
        self.assertEqual(root.get("version"), "1.9")
        self.assertEqual(len(root.findall("./resources/asset")), 6)
        self.assertGreaterEqual(len(root.findall("./resources/format")), 1)
        spine = root.find("./library/event/project/sequence/spine")
        self.assertEqual(len(spine.findall("asset-clip")), 12)
        self.assertEqual(len(spine.findall("transition")), 3)
        self.assertEqual(len(root.findall(".//title")), 2)
        self.assertEqual(len(root.findall(".//timeMap")), 5)
        self.assertEqual(len(spine.findall("asset-clip/adjust-transform")), 12)

    def test_edl_format(self):
        lines = (self.out / "timeline.edl").read_text().splitlines()
        self.assertEqual(lines[0], "TITLE: KAYFABE_TIMELINE")
        self.assertEqual(lines[1], "FCM: NON-DROP FRAME")
        events = [ln for ln in lines if re.match(r"^\d{3}  ", ln)]
        self.assertEqual(len(events), 12)
        for ln in events:
            self.assertRegex(ln, ts.EDL_LINE)
        self.assertTrue(events[0].endswith("01:00:00:00 01:00:06:00"))
        self.assertTrue(events[-1].endswith("01:01:00:00"))

    def test_export_matrix_schema(self):
        m = json.loads((self.out / "export_matrix.json").read_text())
        jobs = m["jobs"]
        need = {"name", "container", "codec", "bit_depth", "chroma", "resolution", "fps",
                "hardware", "expected_duration_s", "expected_video_codec_fourcc"}
        for j in jobs:
            self.assertFalse(need - set(j), j["name"])
            self.assertTrue(j["crf"] is not None or j["bitrate_mbps"] is not None
                            or j.get("profile") or j["container"] == "image_sequence")
        self.assertEqual(len({j["name"] for j in jobs}), len(jobs))
        self.assertEqual({j["resolution"] for j in jobs}, {"3840x2160", "1920x1080", "1280x720"})
        self.assertEqual({j["fps"] for j in jobs}, {25, 50})
        self.assertEqual({j["hardware"] for j in jobs}, {"nvenc", "software", "either"})
        self.assertTrue(any(j["codec"] == "av1" and j.get("requires_ada") for j in jobs))
        self.assertTrue(any(j["codec"] == "hevc" and j["bit_depth"] == 10 for j in jobs))
        self.assertTrue(any(j["codec"] == "png" and j["bit_depth"] == 16 for j in jobs))
        self.assertTrue(any(j["codec"] == "prores_422_hq" for j in jobs))
        self.assertTrue(any(j["codec"] == "dnxhr_hqx" for j in jobs))
        self.assertTrue(all(j["expected_duration_s"] == 60.0 for j in jobs))

    def _mutated(self, name, fn):
        d = self.tmp / ("mut_" + name.replace(".", "_"))
        shutil.copytree(self.out, d, dirs_exist_ok=True)
        p = d / name
        p.write_bytes(fn(p.read_bytes()))
        return d

    def test_check_detects_modified_byte(self):
        d = self._mutated("timeline.edl", lambda b: b.replace(b"SINTEL01", b"SINTEL02", 1))
        rc, out = quiet(ts.cmd_check, d)
        self.assertEqual(rc, 1)
        self.assertIn("timeline.edl: sha256", out)

    def test_check_detects_missing_file(self):
        d = self.tmp / "missing"
        shutil.copytree(self.out, d)
        (d / "export_matrix.json").unlink()
        self.assertEqual(quiet(ts.cmd_check, d)[0], 1)

    def test_validator_detects_structural_breaks(self):
        # The structural validators must fail on their own, independent of the hash comparison.
        import copy
        doc = copy.deepcopy(self.otio)
        v1 = doc["tracks"]["children"][0]["children"]
        clip = next(c for c in v1 if c["name"] == "bbb_b")
        clip["source_range"]["start_time"]["value"] = 590.0  # runs past the available range
        errs = []
        ts.validate_otio(doc, errs)
        self.assertTrue(any("outside available range" in e for e in errs), errs)

        doc = copy.deepcopy(self.otio)
        v1 = doc["tracks"]["children"][0]["children"]
        next(c for c in v1 if c["OTIO_SCHEMA"] == "Transition.1")["in_offset"]["value"] = 3.0
        errs = []
        ts.validate_otio(doc, errs)
        self.assertTrue(any("offsets" in e for e in errs), errs)

        doc = copy.deepcopy(self.otio)
        v1 = doc["tracks"]["children"][0]["children"]
        v1.insert(0, v1.pop(1))  # transition becomes the first item
        errs = []
        ts.validate_otio(doc, errs)
        self.assertTrue(any("not between two clips" in e for e in errs), errs)

        doc = copy.deepcopy(self.otio)
        next(c for c in doc["tracks"]["children"][0]["children"]
             if c.get("name") == "mandel_slow")["metadata"]["kayfabe"]["speed"]["source_frames"] = 80
        errs = []
        ts.validate_otio(doc, errs)
        self.assertTrue(any("speed metadata" in e for e in errs), errs)

        v1 = ts.validate_otio(self.otio, [])
        bad = (self.out / "timeline.edl").read_text().replace("01:00:06:00 01:00:11:00",
                                                             "01:00:06:01 01:00:11:00", 1)
        errs = []
        ts.validate_edl(bad, v1, errs)
        self.assertTrue(errs)

    def test_speed_math(self):
        self.assertEqual(ts.consumed_frames(150, 0.5), 75)
        self.assertEqual(ts.consumed_frames(100, 2.0), 200)
        self.assertEqual(ts.consumed_frames(100, -1.0), 100)
        self.assertEqual(ts.consumed_frames(75, 0.0), 1)

    @unittest.skipUnless(shutil.which("ffmpeg"), "ffmpeg not installed")
    def test_make_sources_smoke(self):
        dest = self.tmp / "src"
        rc, _ = quiet(ts.cmd_make_sources, dest, None)
        self.assertEqual(rc, 0)
        self.assertTrue((dest / "sources.sha256").is_file())


if __name__ == "__main__":
    unittest.main()
