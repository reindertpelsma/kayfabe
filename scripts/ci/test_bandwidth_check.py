"""GPU-free tests of the bandwidth table guard and the calibration proposal (scripts/ci/bandwidth_check.py, bandwidth_calibrate.py)."""

import copy
import datetime
import json
from pathlib import Path
import tempfile
import unittest

import bandwidth_calibrate as cal
import bandwidth_check as chk

TODAY = datetime.date(2026, 11, 17)
DATA = chk.load()
PROPS = json.loads(chk.PROPS.read_text())
MATRIX = json.loads(chk.MATRIX.read_text())
STAGES = chk.payload_stage_names()


def problems(data, today=TODAY, stages=STAGES):
    return chk.validate(data, stages, PROPS, MATRIX, today)


def edited(fn):
    data = copy.deepcopy(DATA)
    fn(data)
    return data


def entry(data, id):
    for e in data["fixed"] + [e for s in data["stages"].values() for e in s["entries"]]:
        if e["id"] == id:
            return e
    raise KeyError(id)


class Committed(unittest.TestCase):
    def test_the_committed_table_is_clean(self):
        self.assertEqual(problems(DATA), [])

    def test_every_payload_stage_is_in_the_table_and_the_other_way_round(self):
        self.assertEqual(set(DATA["stages"]), STAGES)

    def test_the_payload_stage_list_includes_the_manual_only_stages(self):
        self.assertIn("install_flow", STAGES)
        self.assertIn("propimage_apps", STAGES)


class Refusals(unittest.TestCase):
    def has(self, data, token, **kw):
        found = problems(data, **kw)
        self.assertIn(token, found, found)

    def test_a_stage_missing_from_the_table(self):
        self.has(edited(lambda d: d["stages"].pop("windows_apps")), "STAGE_MISSING:windows_apps")

    def test_a_new_payload_stage_cannot_be_forgotten(self):
        self.has(DATA, "STAGE_MISSING:brand_new", stages=STAGES | {"brand_new"})

    def test_a_table_stage_the_payload_no_longer_has(self):
        self.has(DATA, "STAGE_UNKNOWN:windows_gate_2", stages=STAGES - {"windows_gate_2"})

    def test_another_table_stage_the_payload_no_longer_has(self):
        self.has(DATA, "STAGE_UNKNOWN:apps", stages=STAGES - {"apps"})

    def test_an_unknown_class_on_a_fixed_entry(self):
        self.has(edited(lambda d: entry(d, "fixed.setup_cache").update({"class": "cdn"})), "CLASS:fixed.setup_cache")

    def test_an_unknown_field_on_a_fixed_entry(self):
        self.has(edited(lambda d: entry(d, "fixed.repo_clone").update({"bytes": 1})), "UNKNOWN_FIELD:fixed.repo_clone:bytes")

    def test_a_stale_margin_and_a_stale_upload_are_estimates_too(self):
        found = problems(DATA, today=datetime.date(2027, 2, 1))
        self.assertIn("ESTIMATE_STALE:margin", found)
        self.assertIn("ESTIMATE_STALE:apps.upload", found)

    def test_a_driver_size_copied_beside_its_pin_is_refused(self):
        self.has(edited(lambda d: entry(d, "provision.host_driver").update({"down_bytes": 397162548})), "SIZE_SOURCE:provision.host_driver")

    def test_an_entry_without_a_class(self):
        self.has(edited(lambda d: entry(d, "provision.packages").pop("class")), "MISSING:provision.packages:class")

    def test_an_unknown_class(self):
        self.has(edited(lambda d: entry(d, "provision.packages").update({"class": "cdn"})), "CLASS:provision.packages")

    def test_an_entry_without_a_source(self):
        self.has(edited(lambda d: entry(d, "apps.media").pop("source")), "MISSING:apps.media:source")

    def test_an_empty_source(self):
        self.has(edited(lambda d: entry(d, "apps.media").update({"source": "  "})), "SOURCE:apps.media")

    def test_an_entry_without_a_date(self):
        self.has(edited(lambda d: entry(d, "apps.media").pop("date")), "MISSING:apps.media:date")

    def test_a_date_that_is_not_a_date(self):
        self.has(edited(lambda d: entry(d, "apps.media").update({"date": "2026-13-40"})), "DATE:apps.media")

    def test_a_date_in_the_future(self):
        self.has(edited(lambda d: entry(d, "apps.media").update({"date": "2027-01-01"})), "DATE:apps.media")

    def test_an_estimate_older_than_sixty_days(self):
        self.has(DATA, "ESTIMATE_STALE:apps.media", today=datetime.date(2027, 2, 1))

    def test_an_estimate_of_exactly_sixty_days_is_still_fine(self):
        self.assertNotIn("ESTIMATE_STALE:apps.media", problems(DATA, today=datetime.date(2027, 1, 15)))

    def test_a_measured_value_does_not_go_stale(self):
        self.assertNotIn("ESTIMATE_STALE:propimage_apps.image", problems(DATA, today=datetime.date(2028, 1, 1)))

    def test_an_estimate_without_a_note(self):
        self.has(edited(lambda d: entry(d, "apps.media").pop("note")), "ESTIMATE_NO_NOTE:apps.media")

    def test_an_estimate_with_an_empty_note(self):
        self.has(edited(lambda d: entry(d, "apps.media").update({"note": ""})), "ESTIMATE_NO_NOTE:apps.media")

    def test_an_unknown_field_on_an_entry(self):
        self.has(edited(lambda d: entry(d, "apps.media").update({"bytes": 1})), "UNKNOWN_FIELD:apps.media:bytes")

    def test_an_unknown_field_on_a_stage(self):
        self.has(edited(lambda d: d["stages"]["apps"].update({"extra": 1})), "UNKNOWN_FIELD:apps:extra")

    def test_an_unknown_top_level_field(self):
        self.has(edited(lambda d: d.update({"extra": 1})), "UNKNOWN_FIELD:extra")

    def test_an_unknown_field_on_an_upload(self):
        self.has(edited(lambda d: d["stages"]["apps"]["upload"].update({"mb": 1})), "UNKNOWN_FIELD:apps.upload:mb")

    def test_bytes_must_be_a_non_negative_integer(self):
        for bad in (-1, 1.5, "7", True):
            self.has(edited(lambda d: entry(d, "apps.media").update({"down_bytes": bad})), "BYTES:apps.media")

    def test_an_entry_with_no_size(self):
        self.has(edited(lambda d: entry(d, "apps.media").pop("down_bytes")), "SIZE_SOURCE:apps.media")

    def test_a_pinned_size_copied_beside_its_pin_is_refused(self):
        self.has(edited(lambda d: entry(d, "propimage_apps.gop").update({"down_bytes": 11224})), "SIZE_SOURCE:propimage_apps.gop")

    def test_a_size_from_that_does_not_resolve(self):
        self.has(edited(lambda d: entry(d, "propimage_apps.gop").update({"size_from": "propimage:nothing"})), "SIZE_FROM:propimage_apps.gop")
        self.has(edited(lambda d: entry(d, "provision.host_driver").update({"size_from": "driver:linux/1.0"})), "SIZE_FROM:provision.host_driver")

    def test_a_per_image_entry_needs_the_image_reference(self):
        self.has(edited(lambda d: entry(d, "propimage_apps.image").update({"per_image": True, "size_from": "propimage:gop"})),
                 "SIZE_FROM:propimage_apps.image")

    def test_a_when_with_an_unknown_key_or_image(self):
        self.has(edited(lambda d: entry(d, "propimage_apps.imga_media").update({"when": {"colour": ["red"]}})), "WHEN:propimage_apps.imga_media")
        self.has(edited(lambda d: entry(d, "propimage_apps.imga_media").update({"when": {"images": ["imgz"]}})), "WHEN:propimage_apps.imga_media")

    def test_a_duplicate_id(self):
        self.has(edited(lambda d: d["stages"]["apps"]["entries"].append(copy.deepcopy(d["stages"]["apps"]["entries"][0]))), "DUPLICATE_ID:apps.media")

    def test_an_id_that_does_not_start_with_its_stage(self):
        self.has(edited(lambda d: entry(d, "apps.media").update({"id": "provision.media"})), "MISSING:provision.media:id")

    def test_an_empty_stage_needs_a_note(self):
        self.has(edited(lambda d: d["stages"]["cuda_host"].update({"note": ""})), "EMPTY_STAGE:cuda_host")

    def test_two_calibrate_entries_in_one_stage(self):
        self.has(edited(lambda d: d["stages"]["provision"]["entries"][0].update({"calibrate": True})), "CALIBRATE_MANY:provision")

    def test_the_margin_carries_its_provenance(self):
        self.has(edited(lambda d: d["margin"].pop("source")), "MISSING:margin:source")
        self.has(edited(lambda d: d["margin"].update({"percent": 400})), "BYTES:margin.percent")

    def test_an_upload_without_a_date(self):
        self.has(edited(lambda d: d["stages"]["apps"]["upload"].pop("date")), "MISSING:apps.upload:date")


class Explain(unittest.TestCase):
    def instruction(self, **kw):
        base = {"schema": "kayfabe.payload-instruction/1", "tier": "large", "windows": True, "windows_runs": 2, "stages": [],
                "spec": None, "winapps": "mid", "linux_apps": "middle", "rehearsal": False}
        base.update(kw)
        return base

    def image(self, label):
        return {"label": label, "kind": "prebuilt", "disk": {"from": "pin", "ref": "x"}, "apps": {"resolve": {"mode": "present"}}}

    def test_an_image_adds_exactly_its_pinned_size(self):
        plain = chk.explain(self.instruction(), None, DATA, PROPS, MATRIX)
        with_a = chk.explain(self.instruction(propimages=[self.image("imga")], propimages_strict=True), None, DATA, PROPS, MATRIX)
        size = {o["id"]: o["size"] for o in PROPS["objects"]}["imga"]
        got = [i for i in with_a if i["id"] == "propimage_apps.image"]
        self.assertEqual([(i["label"], i["down"]) for i in got], [("imga", size)])
        self.assertEqual([i for i in plain if i["stage"] == "propimage_apps"], [])

    def test_images_not_selected_add_nothing(self):
        items = chk.explain(self.instruction(propimages=[self.image("imgb")], propimages_strict=True), None, DATA, PROPS, MATRIX)
        self.assertEqual([i["label"] for i in items if i["id"] == "propimage_apps.image"], ["imgb"])
        self.assertEqual([i for i in items if i["id"] == "propimage_apps.imga_media"], [])
        self.assertEqual(len([i for i in items if i["id"] == "propimage_apps.imgb_test_assets"]), 1)

    def test_a_skipped_stage_adds_nothing(self):
        items = chk.explain(self.instruction(), None, DATA, PROPS, MATRIX, skip=["windows_gate_2", "apps"])
        self.assertEqual([i["stage"] for i in items if i["stage"] in ("windows_gate_2", "apps")], [])


def runs_file(directory, name, stages):
    path = Path(directory) / name
    path.write_text(json.dumps({"verdict": "PASS", "suites": [], "net_bytes": {"interface": "eth0", "stages": stages}}))
    return str(path)


class Calibrate(unittest.TestCase):
    def setUp(self):
        self.dir = tempfile.TemporaryDirectory()
        self.addCleanup(self.dir.cleanup)
        self.items = chk.explain(Explain().instruction(), None, DATA, PROPS, MATRIX)

    def run_for(self, stages, files=1):
        paths = [runs_file(self.dir.name, "r%d.json" % n, stages) for n in range(files)]
        runs, errors = cal.read_runs(paths)
        self.assertEqual(errors, [])
        return cal.propose(DATA, runs, self.items, ["r.json"], TODAY)

    def test_the_calibrate_entry_becomes_the_actual_minus_the_other_entries(self):
        others = sum(i["down"] for i in self.items if i["stage"] == "provision" and i["id"] != "provision.packages")
        new, notes = self.run_for({"provision": {"rx": others + 5_000_000_000, "tx": 123456}})
        e = entry(new, "provision.packages")
        self.assertEqual((e["down_bytes"], e["basis"], e["date"]), (5_000_000_000, "measured", TODAY.isoformat()))
        self.assertIn("calibrated from 1 run(s)", e["source"])
        self.assertEqual(notes, [])

    def test_the_upload_is_the_actual_transmit_rounded_up(self):
        new, _ = self.run_for({"provision": {"rx": 1, "tx": 123456}})
        self.assertEqual(new["stages"]["provision"]["upload"]["up_bytes"], 130000)

    def test_several_runs_are_averaged(self):
        others = sum(i["down"] for i in self.items if i["stage"] == "provision" and i["id"] != "provision.packages")
        paths = [runs_file(self.dir.name, "a.json", {"provision": {"rx": others + 4_000_000_000, "tx": 0}}),
                 runs_file(self.dir.name, "b.json", {"provision": {"rx": others + 6_000_000_000, "tx": 0}})]
        runs, _ = cal.read_runs(paths)
        new, _ = cal.propose(DATA, runs, self.items, ["a", "b"], TODAY)
        self.assertEqual(entry(new, "provision.packages")["down_bytes"], 5_000_000_000)

    def test_a_negative_remainder_is_zero_and_reported(self):
        new, notes = self.run_for({"provision": {"rx": 1, "tx": 1}})
        self.assertEqual(entry(new, "provision.packages")["down_bytes"], 0)
        self.assertTrue(any(n.startswith("NEGATIVE_REMAINDER:provision") for n in notes), notes)

    def test_a_stage_without_a_calibrate_entry_is_only_reported(self):
        new, notes = self.run_for({"windows_apps": {"rx": 1_000_000, "tx": 1}})
        self.assertEqual(new["stages"]["windows_apps"]["entries"], DATA["stages"]["windows_apps"]["entries"])
        self.assertTrue(any(n.startswith("NO_CALIBRATE_ENTRY:windows_apps") for n in notes), notes)

    def test_the_proposal_is_a_diff_and_the_file_is_not_written(self):
        before = chk.DATA.read_bytes()
        runs = runs_file(self.dir.name, "r.json", {"provision": {"rx": 20_000_000_000, "tx": 5}})
        instruction = Path(self.dir.name) / "i.json"
        instruction.write_text(json.dumps(Explain().instruction()))
        import contextlib
        import io
        out = io.StringIO()
        with contextlib.redirect_stdout(out):
            self.assertEqual(cal.main(["--instruction", str(instruction), "--today", "2026-11-16", runs]), 0)
        self.assertIn("bandwidth.json (proposed)", out.getvalue())
        self.assertEqual(chk.DATA.read_bytes(), before)

    def test_a_result_without_net_bytes_is_named(self):
        path = Path(self.dir.name) / "none.json"
        path.write_text(json.dumps({"verdict": "PASS", "suites": []}))
        runs, errors = cal.read_runs([str(path)])
        self.assertEqual(runs, {})
        self.assertTrue(errors[0].startswith("NO_NET_BYTES:"))

    def test_a_negative_remainder_leaves_a_table_that_still_passes_the_check(self):
        new, _ = self.run_for({"provision": {"rx": 1, "tx": 1}})
        self.assertEqual(problems(new), [])

    def test_each_stage_gets_its_own_upload(self):
        new, _ = self.run_for({"provision": {"rx": 1, "tx": 40001}, "apps": {"rx": 1, "tx": 90000}})
        self.assertEqual([new["stages"][n]["upload"]["up_bytes"] for n in ("provision", "apps")], [50000, 90000])

    def test_the_upload_is_marked_measured_with_its_source(self):
        new, _ = self.run_for({"provision": {"rx": 1, "tx": 1}})
        up = new["stages"]["provision"]["upload"]
        self.assertEqual((up["basis"], up["date"]), ("measured", TODAY.isoformat()))
        self.assertIn("calibrated from 1 run(s)", up["source"])

    def test_the_source_names_the_mean_the_proposal_rests_on(self):
        paths = [runs_file(self.dir.name, "a.json", {"provision": {"rx": 4_000_000_000, "tx": 0}}),
                 runs_file(self.dir.name, "b.json", {"provision": {"rx": 6_000_000_000, "tx": 0}})]
        runs, _ = cal.read_runs(paths)
        new, _ = cal.propose(DATA, runs, self.items, ["a", "b"], TODAY)
        self.assertIn("stage rx mean 5000000000, min 4000000000, max 6000000000", entry(new, "provision.packages")["source"])

    def test_the_calibrated_table_still_passes_the_check(self):
        others = sum(i["down"] for i in self.items if i["stage"] == "provision" and i["id"] != "provision.packages")
        new, _ = self.run_for({"provision": {"rx": others + 7_000_000_000, "tx": 9}})
        self.assertEqual(problems(new), [])


if __name__ == "__main__":
    unittest.main()
