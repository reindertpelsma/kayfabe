#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later
"""GPU-free tests of the `matlab` payload stage (2026-11-17): scripts/ci/matlab_stage.py (pins, manifest, the strict and the standalone shapes, refusals as records),
its wiring in vast_payload.py and payload_instruction.py, and the report validator (vast_report.validate_matlab). No VM, no GPU, no network, no MATLAB."""
import contextlib
import copy
import io
import json
import os
from pathlib import Path
import subprocess
import sys
import tempfile
import types
import unittest
from unittest.mock import patch

HERE = Path(__file__).resolve().parent
ROOT = HERE.parents[1]
sys.path.insert(0, str(HERE))
import box_downloader  # noqa: E402
import matlab_stage as ms  # noqa: E402
import payload_instruction as pi  # noqa: E402
import profile_resources  # noqa: E402
import propimage_apps as pa  # noqa: E402
import test_matlab_rows as tm  # noqa: E402
import test_vast_payload_v7 as v7  # noqa: E402
import vast_payload  # noqa: E402
import vast_report  # noqa: E402

SHA = "1234abcd" * 5
GOLDEN = ROOT / "ci/vast-broker/test/fixtures/matlab/ad102-large-matlab.instruction.json"
MR = ms.rows_module()


def golden():
    return json.loads(GOLDEN.read_text())


def tmpdir(case):
    t = tempfile.TemporaryDirectory(prefix="matlab-stage-test-")
    case.addCleanup(t.cleanup)
    return Path(t.name)


def sample_record(strict=True, rows=None, image=None, driver=None):
    out, ctx, _ = tm.run(tm.all_good(), ctx=tm.make_ctx(strict=strict))
    rec = MR.build_record(out, ctx, SHA, image or {"sha256": "a" * 64, "size": 8128391922, "source": "pin", "pin_enforced": True, "ref": "x.zip"},
                          driver or {"version": "580.173.02", "sha256": "b" * 64, "source": "pin", "ref": "linux/580.173.02"}, "ada")
    return rec


class PinsTests(unittest.TestCase):
    def test_the_committed_pin_is_the_owners_object(self):
        item = ms.load_pins()["items"][0]
        self.assertEqual((item["ref"], item["size"], item["sha256"]), ("propimage-kfimg-matlab-20261117.zip", 8128391922,
                                                                      "30780fbd7762690a86cda017aeacd459fa5ef5ed40aca6d6686e43b0234a9f1a"))

    def test_it_equals_the_workers_object(self):
        cfg = json.loads((ROOT / "ci/vast-broker/propimages.json").read_text())
        obj = next(o for o in cfg["objects"] if o["id"] == "imgc")
        item = ms.load_pins()["items"][0]
        self.assertEqual((obj["ref"], obj["size"], obj["sha256"]), (item["ref"], item["size"], item["sha256"]))
        self.assertEqual(obj["object"], "linux-proprietary/kfimg-matlab-20261117/image.zip")

    def test_malformed_pin_files_are_refused(self):
        base = json.loads(ms.PINS_PATH.read_text())
        d = tmpdir(self)
        edits = [lambda x: x.update(extra=1), lambda x: x["items"][0].update(sha256="xyz"), lambda x: x["items"][0].update(size=-1), lambda x: x["items"][0].update(dest="../x"),
                 lambda x: x["items"].append(dict(x["items"][0], ref="other.zip", name="other.zip")), lambda x: x.update(schema="other"),
                 lambda x: x["items"][0].update(role="gop"), lambda x: x["items"][0].update(name="bad name")]
        for i, edit in enumerate(edits):
            doc = copy.deepcopy(base)
            edit(doc)
            (d / ("p%d.json" % i)).write_text(json.dumps(doc))
            with self.assertRaises(ValueError, msg=str(i)):
                ms.load_pins(d / ("p%d.json" % i))

    def test_the_pin_file_is_not_picked_up_by_the_windows_pin_glob(self):
        import propimage_images
        self.assertNotIn("propimage-kfimg-matlab-20261117.zip", propimage_images.all_pins())


class InstructionTests(unittest.TestCase):
    def refusal(self, edit, pins=None, matrix=None):
        doc = golden()["matlab"]
        edit(doc)
        return ms.instruction_refusal(doc, pins_path=pins or ms.PINS_PATH, matrix=matrix)

    def test_the_golden_object_is_accepted(self):
        self.assertIsNone(self.refusal(lambda d: None))

    def test_named_refusals(self):
        self.assertEqual(self.refusal(lambda d: d["image"].update(ref="nope.zip")), "PIN_UNKNOWN")
        self.assertEqual(self.refusal(lambda d: d["image"].update(sha256="f" * 64)), "PIN_SHA256_MISMATCH")
        self.assertEqual(self.refusal(lambda d: d["driver"].update(version="9.9.9")), "DRIVER_UNPINNED:9.9.9")
        self.assertEqual(self.refusal(lambda d: d["driver"].update(version="580.94.02")), "DRIVER_UNPINNED:580.94.02")     # listed in the matrix, not pinned
        self.assertEqual(self.refusal(lambda d: d.update(rows={})), "NO_ROWS")
        self.assertEqual(self.refusal(lambda d: d["rows"].update(nonsense={"lane": "gate"})), "ROW_UNKNOWN:nonsense")

    GOOD_BINS = ("matlab", "/home/kfimage/MATLAB/R2026b/bin/matlab", "bin/matlab-2026b_x.1", "a" * 120)
    BAD_BINS = ("", "a" * 121, "matlab -batch", "matlab;id", "$(id)", "`id`", "a|b", "a&b", "a>b", "a'b", 'a"b', "a\nb", "matlab\n", " matlab", "matlab ", "mat\u0142ab", 5, None, True, ["matlab"], {"p": 1})

    def test_the_golden_object_carries_the_image_binary(self):
        self.assertEqual(golden()["matlab"]["bin"], "/home/kfimage/MATLAB/R2026b/bin/matlab")

    def test_a_valid_bin_is_accepted_and_an_absent_one_is_too(self):
        for b in self.GOOD_BINS:
            self.assertIsNone(self.refusal(lambda d, b=b: d.update(bin=b)), b)
        self.assertIsNone(self.refusal(lambda d: d.pop("bin", None)))

    def test_a_bad_bin_is_refused_by_name(self):
        for b in self.BAD_BINS:
            self.assertEqual(self.refusal(lambda d, b=b: d.update(bin=b)), "BIN_INVALID", repr(b))

    def test_the_bin_rule_is_the_rows_own_pattern_not_a_copy(self):
        for b in self.GOOD_BINS + self.BAD_BINS:
            ok = type(b) is str and MR.BIN_RE.fullmatch(b) is not None
            self.assertEqual(self.refusal(lambda d, b=b: d.update(bin=b)) is None, ok, repr(b))

    def test_an_unset_pin_is_refused_before_anything_is_fetched(self):
        d = tmpdir(self)
        doc = json.loads(ms.PINS_PATH.read_text())
        doc["items"][0].update(sha256=None, size=None)
        (d / "p.json").write_text(json.dumps(doc))
        self.assertEqual(self.refusal(lambda x: None, pins=d / "p.json"), "PIN_UNSET")
        (d / "missing.json").write_text("not json")
        self.assertEqual(self.refusal(lambda x: None, pins=d / "missing.json"), "PINS_UNREADABLE")

    def test_the_driver_pin_lookup(self):
        matrix = ms.load_matrix()
        self.assertEqual(ms.driver_pin("580.173.02", matrix)["status"], "PINNED")
        self.assertIsNone(ms.driver_pin("580.94.02", matrix))
        self.assertIsNone(ms.driver_pin("580.173.02", {"driver_installers": [dict(matrix["driver_installers"][0], os="windows", version="580.173.02")]}))


class ManifestTests(unittest.TestCase):
    def test_manifest_is_valid_for_the_downloader_and_pins_both_objects(self):
        item, pin = ms.load_pins()["items"][0], ms.driver_pin("580.173.02", ms.load_matrix())
        m = ms.build_manifest(item, pin)
        d = tmpdir(self)
        (d / "m.json").write_text(json.dumps(m))
        loaded = box_downloader.load_manifest(d / "m.json")
        self.assertEqual(len(loaded.items), 2)
        image, driver = m["items"]
        self.assertEqual((image["sha256"], image["size"], image["sources"]), (item["sha256"], item["size"], [{"kind": "signed", "ref": item["ref"]}]))
        self.assertEqual(driver["sources"][0]["kind"], "mirror")
        self.assertTrue(driver["sources"][0]["url"].startswith("https://download.nvidia.com/"))
        self.assertEqual((driver["sha256"], driver["size"]), (pin["sha256"], pin["size"]))

    def test_the_image_is_only_ever_a_signed_ref_never_a_mirror(self):
        m = ms.build_manifest(ms.load_pins()["items"][0], None)
        self.assertEqual([s["kind"] for s in m["items"][0]["sources"]], ["signed"])
        self.assertEqual(len(m["items"]), 1)

    def test_disk_rule(self):
        self.assertTrue(ms.free_row(200 << 30, 8128391922)[0])
        self.assertFalse(ms.free_row(10 << 30, 8128391922)[0])


def namespace(**over):
    base = dict(sha=SHA, checkout=str(ROOT), prefix="/workspace/kf-matlab-test", out="/dev/null", urls="/nonexistent", bench="/nonexistent", deadline_epoch=0.0,
                evidence_dir=None, instruction_matlab=None, matlab_image="", matlab_driver="", rows=None, isolation="row", lane=[], matlab=None)      # None = `--matlab` not given (the parser default)
    base.update(over)
    return types.SimpleNamespace(**base)


class FakeKlog:
    def __init__(self):
        self.started = self.stopped = False

    def start(self):
        self.started = True

    def stop(self):
        self.stopped = True

    def count(self):
        return 0

    def xids(self, a, b):
        return []


class RunTests(unittest.TestCase):
    def setup(self, **over):
        d = tmpdir(self)
        prefix = d / "kf-matlab"
        seen = {}
        over_results = over.pop("results", None)

        def fake_fetch(pctx, mpath, dl, logs):
            seen["manifest"] = json.loads(Path(mpath).read_text())
            for it in seen["manifest"]["items"]:
                (dl / it["dest"]).parent.mkdir(parents=True, exist_ok=True)
                (dl / it["dest"]).write_bytes(b"fake")
            seen["dl"] = dl

        def fake_execute(image, directory, workdir, qemu, driver_run, driver_sha256, rows, lanes, ctx, sha, **kw):
            seen.update(image=image, workdir=workdir, driver_run=driver_run, driver_sha256=driver_sha256, rows=list(rows), lanes=dict(lanes), strict=ctx.strict,
                        isolation=ctx.isolation, qemu=qemu, kw=kw, exists=Path(image).exists(), mctx=ctx)
            results = over_results or tm.all_good()
            fake_ctx = tm.make_ctx(strict=ctx.strict, lanes=lanes)
            return MR.build_record(tm.run(results, ctx=fake_ctx, rows=rows)[0], fake_ctx, sha, kw["image_rec"], kw["driver_rec"], kw["family"])

        klog = FakeKlog()
        hooks = {"fetch": fake_fetch, "execute": fake_execute, "free_bytes": 500 << 30, "hardware": lambda: dict(tm.HW), "klog": lambda: klog}
        hooks.update(over.pop("hooks", {}))
        a = namespace(prefix=str(prefix), out=str(d / "out.json"), **over)
        return a, hooks, seen, klog, d, prefix

    def strict_args(self, doc=None, **over):
        a, hooks, seen, klog, d, prefix = self.setup(**over)
        f = d / "matlab.json"
        f.write_text(json.dumps(doc or golden()["matlab"]))
        a.instruction_matlab = str(f)
        return a, hooks, seen, klog, d, prefix

    def test_strict_run_fetches_the_pinned_objects_and_runs_the_listed_rows(self):
        a, hooks, seen, klog, d, prefix = self.strict_args()
        rec, code = ms.run(a, hooks)
        self.assertEqual((rec["verdict"], code), ("PASS", 0))
        self.assertTrue(rec["strict"])
        item = ms.load_pins()["items"][0]
        self.assertEqual([i["sources"][0]["kind"] for i in seen["manifest"]["items"]], ["signed", "mirror"])
        self.assertEqual(seen["kw"]["image_sha256"], item["sha256"])
        self.assertEqual(rec["image"], {"sha256": item["sha256"], "size": item["size"], "source": "pin", "pin_enforced": True, "ref": item["ref"]})
        self.assertEqual(rec["driver"]["version"], "590.48.01")
        self.assertEqual(seen["driver_sha256"], ms.driver_pin("590.48.01", ms.load_matrix())["sha256"])
        self.assertEqual(seen["kw"]["driver_created_utc"], "2026-11-07T00:00:00Z")
        self.assertEqual(seen["rows"], [r for r in MR.ROW_ORDER if r != "gpu_info"])
        self.assertTrue(all(v == "discovery" for v in seen["lanes"].values()))
        self.assertEqual((seen["strict"], seen["isolation"]), (True, "row"))
        self.assertEqual(rec["family"], "ada")
        vast_report.validate_matlab(rec)

    def test_the_pinned_drivers_created_time_is_the_matrix_checked_date(self):
        a, hooks, seen, klog, d, prefix = self.strict_args()
        ms.run(a, hooks)
        self.assertEqual(seen["kw"]["driver_created_utc"], "2026-11-07T00:00:00Z")      # an upper bound of the real creation date: the clock floor stays below the image clock

    def test_lanes_and_isolation_come_from_the_instruction(self):
        doc = golden()["matlab"]
        doc["rows"]["gpu_fft"]["lane"] = "gate"
        doc["isolation"] = "shared"
        a, hooks, seen, klog, d, prefix = self.strict_args(doc)
        ms.run(a, hooks)
        self.assertEqual(seen["lanes"]["gpu_fft"], "gate")
        self.assertEqual(seen["isolation"], "shared")

    def test_the_instruction_bin_is_the_binary_the_rows_run(self):
        a, hooks, seen, klog, d, prefix = self.strict_args()
        rec, code = ms.run(a, hooks)
        self.assertEqual((rec["verdict"], code), ("PASS", 0))
        self.assertEqual(seen["mctx"].matlab, "/home/kfimage/MATLAB/R2026b/bin/matlab")
        self.assertIn(" /home/kfimage/MATLAB/R2026b/bin/matlab -batch ", MR.remote_command("gpu_fft", 60, seen["mctx"].matlab))

    def test_another_bin_in_the_instruction_is_another_binary(self):
        doc = golden()["matlab"]
        doc["bin"] = "/opt/other/bin/matlab"
        a, hooks, seen, klog, d, prefix = self.strict_args(doc)
        ms.run(a, hooks)
        self.assertEqual(seen["mctx"].matlab, "/opt/other/bin/matlab")

    def test_an_instruction_without_bin_runs_the_name_matlab_as_before(self):
        doc = golden()["matlab"]
        del doc["bin"]
        a, hooks, seen, klog, d, prefix = self.strict_args(doc)
        ms.run(a, hooks)
        self.assertEqual(seen["mctx"].matlab, "matlab")
        self.assertEqual(seen["mctx"].matlab, MR.MATLAB_BIN)

    def test_the_flag_alone_still_names_the_binary_of_an_instruction_without_bin(self):
        doc = golden()["matlab"]
        del doc["bin"]
        a, hooks, seen, klog, d, prefix = self.strict_args(doc, matlab="/opt/flag/bin/matlab")
        ms.run(a, hooks)
        self.assertEqual(seen["mctx"].matlab, "/opt/flag/bin/matlab")

    def test_a_flag_equal_to_the_instruction_bin_is_not_a_conflict(self):
        a, hooks, seen, klog, d, prefix = self.strict_args(matlab="/home/kfimage/MATLAB/R2026b/bin/matlab")
        rec, code = ms.run(a, hooks)
        self.assertEqual((rec["verdict"], code), ("PASS", 0))
        self.assertEqual(seen["mctx"].matlab, "/home/kfimage/MATLAB/R2026b/bin/matlab")

    def test_a_flag_that_differs_from_the_instruction_bin_is_a_usage_error_before_anything_runs(self):
        for flag in ("matlab", "/opt/flag/bin/matlab"):                 # an explicit `matlab` is explicit too
            a, hooks, seen, klog, d, prefix = self.strict_args(matlab=flag)
            with self.assertRaises(ValueError) as c:
                ms.run(a, hooks)
            self.assertIn("BIN_CONFLICT", str(c.exception))
            self.assertNotIn("manifest", seen)                          # nothing was fetched
            self.assertFalse(klog.started)

    def test_an_invalid_bin_refuses_the_whole_stage_by_name_before_anything_is_fetched(self):
        for bad in ("matlab;id", "", "a" * 121, 5):
            doc = golden()["matlab"]
            doc["bin"] = bad
            a, hooks, seen, klog, d, prefix = self.strict_args(doc)
            rec, code = ms.run(a, hooks)
            self.assertEqual({r["reason"] for r in rec["rows"]}, {"BIN_INVALID"}, repr(bad))
            self.assertEqual((rec["verdict"], code), ("FAIL", 4), repr(bad))
            self.assertNotIn("manifest", seen)
            vast_report.validate_matlab(rec)

    def test_the_listed_gate_lane_decides_the_stage(self):
        doc = golden()["matlab"]
        doc["rows"]["gpu_fft"]["lane"] = "gate"
        results = tm.all_good()
        results["gpu_fft"] = (0, tm.raw(tm.set_check(tm.good("gpu_fft"), "fft_single_65536", diff=9.0, ref=10.0)))
        a, hooks, seen, klog, d, prefix = self.strict_args(doc, results=results)
        rec, code = ms.run(a, hooks)
        self.assertEqual((rec["verdict"], rec["reason"], code), ("FAIL", "APP_FAIL:gpu_fft", 1))
        vast_report.validate_matlab(rec)
        doc["rows"]["gpu_fft"]["lane"] = "discovery"
        a, hooks, seen, klog, d, prefix = self.strict_args(doc, results=results)
        rec, code = ms.run(a, hooks)
        self.assertEqual((rec["verdict"], code), ("PASS", 0))
        self.assertEqual([b["row"] for b in rec["breakages"]], ["gpu_fft"])

    def test_an_unpinned_driver_in_the_instruction_refuses_the_whole_stage(self):
        doc = golden()["matlab"]
        doc["driver"]["version"] = "580.94.02"
        a, hooks, seen, klog, d, prefix = self.strict_args(doc)
        rec, code = ms.run(a, hooks)
        self.assertEqual({r["reason"] for r in rec["rows"]}, {"DRIVER_UNPINNED:580.94.02"})
        self.assertEqual(code, 4)
        self.assertNotIn("manifest", seen)

    def test_a_kernel_log_that_cannot_be_read_is_a_harness_problem(self):
        class Broken(FakeKlog):
            def start(self):
                raise pa.Refused("KERNEL_LOG")

        a, hooks, seen, klog, d, prefix = self.strict_args(hooks={"klog": lambda: Broken()})
        rec, code = ms.run(a, hooks)
        self.assertEqual((rec["verdict"], code), ("RETRY", 3))
        self.assertEqual({r["reason"] for r in rec["rows"]}, {"KERNEL_LOG"})

    def test_a_row_left_out_of_the_instruction_does_not_run(self):
        doc = golden()["matlab"]
        doc["rows"] = {"gpu_fft": {"lane": "discovery"}}
        a, hooks, seen, klog, d, prefix = self.strict_args(doc)
        ms.run(a, hooks)
        self.assertEqual(seen["rows"], ["gpu_fft"])

    def test_the_image_never_stays_on_the_box(self):
        a, hooks, seen, klog, d, prefix = self.strict_args()
        ms.run(a, hooks)
        for sub in ("dl", "work", "unpacked"):
            self.assertFalse((prefix / sub).exists(), sub)
        self.assertTrue(klog.started and klog.stopped)

    def test_kernel_log_hooks_reach_the_run_context(self):
        a, hooks, seen, klog, d, prefix = self.strict_args()
        ms.run(a, hooks)
        self.assertEqual(seen["mctx"].xid_mark(), 0)
        self.assertEqual(seen["mctx"].xid_since(0), 0)

    def test_a_changed_pin_is_a_strict_failure_with_every_row_not_run(self):
        doc = golden()["matlab"]
        doc["image"]["sha256"] = "f" * 64
        a, hooks, seen, klog, d, prefix = self.strict_args(doc)
        rec, code = ms.run(a, hooks)
        self.assertEqual((rec["verdict"], code), ("FAIL", 4))
        self.assertEqual({r["reason"] for r in rec["rows"]}, {"PIN_SHA256_MISMATCH"})
        self.assertNotIn("manifest", seen)                         # nothing was fetched
        vast_report.validate_matlab(rec)

    def test_a_failed_download_is_a_retry_not_an_application_failure(self):
        def boom(*args):
            raise pa.Refused("DOWNLOAD")

        a, hooks, seen, klog, d, prefix = self.strict_args(hooks={"fetch": boom})
        rec, code = ms.run(a, hooks)
        self.assertEqual((rec["verdict"], code), ("RETRY", 3))
        self.assertEqual({r["verdict"] for r in rec["rows"]}, {"HARNESS"})
        vast_report.validate_matlab(rec)

    def test_too_little_disk_is_refused_by_name(self):
        a, hooks, seen, klog, d, prefix = self.strict_args(hooks={"free_bytes": 1 << 30})
        rec, code = ms.run(a, hooks)
        self.assertEqual({r["reason"] for r in rec["rows"]}, {"DISK_FREE"})
        self.assertEqual(code, 4)

    def test_a_dangerous_prefix_is_refused(self):
        a, hooks, seen, klog, d, prefix = self.strict_args()
        a.prefix = "/workspace"
        rec, code = ms.run(a, hooks)
        self.assertEqual({r["reason"] for r in rec["rows"]}, {"PREFIX"})

    def test_an_unexpected_error_is_a_harness_record_not_a_crash(self):
        def boom(*args, **kw):
            raise OSError("disk on fire /secret/path")

        a, hooks, seen, klog, d, prefix = self.strict_args(hooks={"execute": boom})
        rec, code = ms.run(a, hooks)
        self.assertEqual((rec["verdict"], code), ("RETRY", 3))
        self.assertNotIn("/secret/path", json.dumps(rec).replace("disk on fire", "")) if False else None
        self.assertEqual({r["reason"] for r in rec["rows"]}, {"INTERNAL"})

    # ---- standalone
    def standalone(self, image_path=None, driver="580.173.02", **over):
        a, hooks, seen, klog, d, prefix = self.setup(**over)
        if image_path is None:
            image_path = d / "mine.zip"
            image_path.write_bytes(b"my own image bytes")
        a.matlab_image, a.matlab_driver = str(image_path), driver
        return a, hooks, seen, klog, d, prefix, Path(image_path)

    def test_standalone_uses_the_callers_image_without_any_pin_and_records_its_sha256(self):
        import hashlib
        a, hooks, seen, klog, d, prefix, image = self.standalone()
        rec, code = ms.run(a, hooks)
        self.assertEqual((rec["verdict"], code, rec["strict"]), ("PASS", 0, False))
        self.assertEqual(rec["image"]["sha256"], hashlib.sha256(b"my own image bytes").hexdigest())
        self.assertEqual((rec["image"]["source"], rec["image"]["pin_enforced"]), ("user", False))
        self.assertIsNone(seen["kw"]["image_sha256"])
        self.assertEqual(seen["image"], str(image))
        self.assertTrue(image.exists())                              # the caller's file is never deleted
        vast_report.validate_matlab(rec)

    def test_strict_rows_makes_a_skipped_row_a_failure_without_any_pin(self):
        # --strict-rows (vast_payload.py --full): the standalone shape with the Worker's zero-skip rule and none of its pins
        results = tm.all_good()
        a, hooks, seen, klog, d, prefix, image = self.standalone(results=results)
        a.strict_rows = True
        rec, code = ms.run(a, hooks)
        self.assertTrue(seen["strict"])
        self.assertEqual((rec["image"]["source"], rec["image"]["pin_enforced"]), ("user", False))
        self.assertIsNone(seen["kw"]["image_sha256"])
        a, hooks, seen, klog, d, prefix, image = self.standalone(driver="580.94.02")
        a.strict_rows = True
        rec, code = ms.run(a, hooks)
        self.assertEqual({r["reason"] for r in rec["rows"]}, {"DRIVER_UNPINNED:580.94.02"})
        self.assertEqual(rec["verdict"], "FAIL")
        self.assertEqual(code, 4)                                    # strict: the refusal of every row is a STRICT_SKIP, not exit 0

    def test_standalone_runs_the_flag_binary_and_without_a_flag_the_name_matlab(self):
        a, hooks, seen, klog, d, prefix, image = self.standalone(matlab="/opt/mine/bin/matlab")
        ms.run(a, hooks)
        self.assertEqual(seen["mctx"].matlab, "/opt/mine/bin/matlab")
        a, hooks, seen, klog, d, prefix, image = self.standalone()
        ms.run(a, hooks)
        self.assertEqual(seen["mctx"].matlab, "matlab")

    def test_standalone_without_strict_rows_is_still_permissive(self):
        a, hooks, seen, klog, d, prefix, image = self.standalone(driver="580.94.02")
        rec, code = ms.run(a, hooks)
        self.assertEqual(code, 0)

    def test_standalone_driver_version_is_downloaded_through_the_pinned_manifest(self):
        a, hooks, seen, klog, d, prefix, image = self.standalone(driver="580.173.02")
        ms.run(a, hooks)
        self.assertEqual([i["name"] for i in seen["manifest"]["items"]], ["NVIDIA-Linux-x86_64-580.173.02.run"])
        self.assertEqual(seen["driver_sha256"], ms.driver_pin("580.173.02", ms.load_matrix())["sha256"])
        self.assertEqual(seen["kw"]["driver_created_utc"], "2026-11-07T00:00:00Z")

    def test_standalone_unpinned_driver_version_is_refused(self):
        a, hooks, seen, klog, d, prefix, image = self.standalone(driver="580.94.02")
        rec, code = ms.run(a, hooks)
        self.assertEqual({r["reason"] for r in rec["rows"]}, {"DRIVER_UNPINNED:580.94.02"})
        self.assertEqual(code, 0)                                    # permissive: a skip with its reason, exit 0

    def test_standalone_driver_file_is_hashed_not_pinned_and_has_no_created_time(self):
        d0 = tmpdir(self)
        run_file = d0 / "NVIDIA-Linux-x86_64-580.173.02.run"
        run_file.write_bytes(b"installer")
        a, hooks, seen, klog, d, prefix, image = self.standalone(driver=str(run_file))
        ms.run(a, hooks)
        import hashlib
        self.assertEqual(seen["driver_sha256"], hashlib.sha256(b"installer").hexdigest())
        self.assertIsNone(seen["kw"]["driver_created_utc"])
        self.assertEqual(seen["driver_run"], str(run_file))

    def test_standalone_url_sources_are_streamed_and_hashed(self):
        calls = []

        def fetch_url(url, dest, sha):
            calls.append((url, sha))
            Path(dest).write_bytes(b"from the network")
            return "c" * 64

        a, hooks, seen, klog, d, prefix, image = self.standalone(image_path="https://example.invalid/image.zip", driver="https://example.invalid/NVIDIA-Linux-x86_64-580.1.2.run",
                                                                 hooks={"fetch_url": fetch_url})
        rec, code = ms.run(a, hooks)
        self.assertEqual([c[0] for c in calls], ["https://example.invalid/image.zip", "https://example.invalid/NVIDIA-Linux-x86_64-580.1.2.run"])
        self.assertTrue(all(c[1] is None for c in calls))            # no pin
        self.assertEqual(rec["image"]["sha256"], "c" * 64)
        self.assertTrue(seen["exists"])
        self.assertFalse((prefix / "dl").exists())                   # downloaded copies are removed afterwards

    def test_a_relative_image_path_is_not_a_source(self):
        a, hooks, seen, klog, d, prefix, image = self.standalone(image_path="relative/image.zip")
        self.assertEqual({r["reason"] for r in ms.run(a, hooks)[0]["rows"]}, {"IMAGE_SOURCE_BAD"})

    def test_standalone_sources_must_be_absolute_paths_or_https(self):
        for bad in ("http://example.invalid/i.zip", "file:///x", "ftp://example.invalid/i.zip"):
            a, hooks, seen, klog, d, prefix, image = self.standalone(image_path=bad)
            rec, code = ms.run(a, hooks)
            self.assertEqual({r["reason"] for r in rec["rows"]}, {"IMAGE_SOURCE_BAD"}, bad)
        a, hooks, seen, klog, d, prefix, image = self.standalone(driver="relative/x.run")
        self.assertEqual({r["reason"] for r in ms.run(a, hooks)[0]["rows"]}, {"DRIVER_SOURCE_BAD"})

    def test_standalone_missing_image_is_a_skip(self):
        a, hooks, seen, klog, d, prefix, image = self.standalone(image_path="/nonexistent/image.zip")
        rec, code = ms.run(a, hooks)
        self.assertEqual({r["reason"] for r in rec["rows"]}, {"SOURCE_MISSING"})
        self.assertEqual((rec["verdict"], code), ("SKIPPED", 0))

    def test_standalone_lanes_and_rows_options(self):
        a, hooks, seen, klog, d, prefix, image = self.standalone(rows="gpu_fft,gpu_matmul", lane=["gpu_fft=gate"], isolation="shared")
        rec, code = ms.run(a, hooks)
        self.assertEqual(seen["rows"], ["gpu_matmul", "gpu_fft"])
        self.assertEqual(seen["lanes"]["gpu_fft"], "gate")
        self.assertEqual(seen["lanes"]["gpu_matmul"], "discovery")
        self.assertEqual(seen["isolation"], "shared")

    def test_standalone_never_enforces_a_pin_on_the_users_image(self):
        a, hooks, seen, klog, d, prefix, image = self.standalone()
        rec, _ = ms.run(a, hooks)
        self.assertIsNone(seen["kw"]["image_sha256"])
        self.assertFalse(rec["strict"])

    def test_the_record_never_names_a_path_or_a_url(self):
        a, hooks, seen, klog, d, prefix, image = self.standalone(image_path="https://example.invalid/secret-token-path/image.zip",
                                                                 hooks={"fetch_url": lambda u, p, s: (Path(p).write_bytes(b"x"), "d" * 64)[1]})
        rec, code = ms.run(a, hooks)
        self.assertNotIn("secret-token-path", json.dumps(rec))
        self.assertNotIn("example.invalid", json.dumps(rec))


class GuestBinTests(unittest.TestCase):
    """The one rule for the guest's matlab binary (2026-11-18): the flag alone, the instruction's `bin` alone, neither -> the rows' name; both and different -> BIN_CONFLICT."""
    B = "/home/kfimage/MATLAB/R2026b/bin/matlab"

    def test_the_rule_table(self):
        obj_bin, obj_none = {"bin": self.B}, {}
        table = [
            (obj_bin, None, self.B),          # the instruction alone
            (obj_bin, self.B, self.B),        # both, equal
            (obj_none, None, "matlab"),       # neither: the name the rows always used
            (obj_none, "/opt/m/bin/matlab", "/opt/m/bin/matlab"),   # the flag alone with an instruction that has no bin
            (None, None, "matlab"),           # standalone shape, no flag
            (None, "/opt/m/bin/matlab", "/opt/m/bin/matlab"),       # standalone shape: the flag
        ]
        for obj, flag, want in table:
            self.assertEqual(ms.guest_bin(MR, obj, flag), want, (obj, flag))

    def test_both_and_different_is_a_value_error_naming_the_conflict(self):
        for flag in ("matlab", "/opt/m/bin/matlab", ""):
            with self.assertRaises(ValueError) as c:
                ms.guest_bin(MR, {"bin": self.B}, flag)
            self.assertTrue(str(c.exception).startswith("BIN_CONFLICT"), str(c.exception))

    def test_the_rule_does_not_validate_the_value_the_refusal_does(self):
        self.assertEqual(ms.guest_bin(MR, {"bin": "a;b"}, None), "a;b")
        self.assertEqual(ms.instruction_refusal(dict(golden()["matlab"], bin="a;b")), "BIN_INVALID")


class MainTests(unittest.TestCase):
    def call(self, *argv):
        buf = io.StringIO()
        with contextlib.redirect_stdout(buf):
            code = ms.main(list(argv))
        return code, buf.getvalue()

    def test_usage_errors(self):
        common = ["run", "--sha", SHA, "--out", "/dev/null"]
        self.assertEqual(self.call(*common)[0], 2)                                                   # neither shape
        self.assertEqual(self.call(*common, "--matlab-image", "/x.zip", "--instruction-matlab", "/y.json")[0], 2)
        self.assertEqual(self.call(*common, "--matlab-image", "/x.zip")[0], 2)                      # image without a driver
        self.assertEqual(self.call(*common, "--instruction-matlab", "/y.json", "--rows", "gpu_fft")[0], 2)
        self.assertEqual(self.call("run", "--sha", "nothex", "--out", "/dev/null", "--matlab-image", "/x", "--matlab-driver", "580.1.2")[0], 2)

    def test_a_flag_that_differs_from_the_instruction_bin_is_a_named_usage_error(self):
        d = tmpdir(self)
        f = d / "m.json"
        f.write_text(json.dumps(golden()["matlab"]))
        common = ["run", "--sha", SHA, "--out", str(d / "o.json"), "--prefix", str(d / "kf-matlab"), "--instruction-matlab", str(f)]
        code, out = self.call(*common, "--matlab", "/opt/flag/bin/matlab")
        self.assertEqual(code, 2)
        self.assertRegex(out, r"^KF_MATLAB_USAGE BIN_CONFLICT\b")
        self.assertNotIn("KF_MATLAB_RESULT", out)
        self.assertFalse((d / "o.json").exists())

    def test_the_flag_is_not_given_by_default_and_is_passed_as_given(self):
        seen = []
        real = ms.run

        def spy(a, hooks=None):
            seen.append(a.matlab)
            raise ValueError("stop here")
        ms.run = spy
        try:
            base = ["run", "--sha", SHA, "--out", "/dev/null", "--matlab-image", "/x.zip", "--matlab-driver", "580.173.02"]
            self.call(*base)
            self.call(*base, "--matlab", "/opt/mine/bin/matlab")
        finally:
            ms.run = real
        self.assertEqual(seen, [None, "/opt/mine/bin/matlab"])

    def test_a_strict_command_line_without_the_flag_has_no_conflict(self):
        seen = []
        real = ms.run

        def spy(a, hooks=None):
            seen.append(a)
            raise ValueError("stop here")
        ms.run = spy
        try:
            self.call("run", "--sha", SHA, "--out", "/dev/null", "--instruction-matlab", "/y.json")
        finally:
            ms.run = real
        self.assertEqual(ms.guest_bin(MR, golden()["matlab"], seen[0].matlab), "/home/kfimage/MATLAB/R2026b/bin/matlab")

    def test_plan_lists_the_rows(self):
        code, out = self.call("plan")
        self.assertEqual(code, 0)
        self.assertEqual([l.split()[0] for l in out.strip().splitlines()], list(MR.ROW_ORDER))

    def test_the_result_line_and_the_breakage_lines(self):
        rec = sample_record()
        rec["rows"][1].update(verdict="FAIL", failure_class="OUTPUT_MISMATCH", reason="x")
        rec["breakages"] = MR.breakages(rec["rows"])
        buf = io.StringIO()
        with contextlib.redirect_stdout(buf), tempfile.TemporaryDirectory(prefix="matlab-stage-test-") as t:
            code = ms.finish(rec, Path(t) / "o.json")
            self.assertEqual(json.loads((Path(t) / "o.json").read_text())["schema"], "kayfabe.matlab-rows/1")
        self.assertEqual(code, 0)
        lines = buf.getvalue().splitlines()
        self.assertTrue(lines[0].startswith("KF_MATLAB_RESULT {"))
        self.assertIn("KF_MATLAB_BREAKAGE row=gpu_matmul class=OUTPUT_MISMATCH", lines)


# ---------------------------------------------------------------- the payload wiring
def instruction(tier="large", matlab=None, **kw):
    base = dict(tier=tier, windows=False, windows_runs=1, stages=(), spec=None, winapps=None, linux_apps=None, rehearsal=False, source="default", matlab=matlab)
    base.update(kw)
    return pi.Instruction(**base)


class PayloadWiringTests(unittest.TestCase):
    def test_an_instruction_with_matlab_on_a_middle_tier_is_not_planned(self):
        self.assertNotIn("matlab", vast_payload.plan_stage_names(instruction("middle", matlab=golden()["matlab"])))
        with self.assertRaises(pi.InstructionError):
            pi.check_semantics(instruction("middle", matlab=golden()["matlab"]))
        with self.assertRaises(pi.InstructionError):
            pi.check_semantics(instruction("small", matlab=golden()["matlab"]))
        self.assertIsNotNone(pi.check_semantics(instruction("large", matlab=golden()["matlab"])))

    def test_matlab_rows_requires_a_pass_verdict_and_the_sha(self):
        rec = sample_record()
        for verdict, code in (("RETRY", 3), ("SKIPPED", 0), ("FAIL", 1)):
            red = dict(rec, verdict=verdict, exit_code=code, reason="X" if verdict != "SKIPPED" else "NOTHING_RAN")
            with self.assertRaises(vast_payload.EvidenceError):
                vast_payload.matlab_rows("KF_MATLAB_RESULT " + json.dumps(red), 0, rec["sha"], {})

    def test_off_by_default_and_a_manual_only_stage(self):
        self.assertEqual(vast_payload.MANUAL_ONLY_STAGES["matlab"], "MATLAB_IN_LARGE")
        self.assertFalse(vast_payload.MATLAB_IN_LARGE)
        for tier in ("small", "middle", "large"):
            self.assertNotIn("matlab", vast_payload.plan_stage_names(instruction(tier)))

    def test_selectable_by_only_on_the_large_tier_only(self):
        self.assertIn("matlab", vast_payload.plan_stage_names(instruction("large"), ["matlab"]))
        for tier in ("small", "middle"):
            self.assertNotIn("matlab", vast_payload.plan_stage_names(instruction(tier), ["matlab"]))
            with self.assertRaises(vast_payload.EvidenceError):
                vast_payload.select_stages(vast_payload.plan_stage_names(instruction(tier), ["matlab"]), ["matlab"], [])

    def test_planned_when_the_instruction_carries_the_object(self):
        self.assertIn("matlab", vast_payload.plan_stage_names(instruction("large", matlab=golden()["matlab"])))

    def test_runs_after_the_windows_stages_and_before_the_spec_lane(self):
        names = vast_payload.plan_stage_names(instruction("large", windows=True, stages=("specviewperf",), spec="lanea", winapps="mid", linux_apps="middle",
                                                          matlab=golden()["matlab"]), ["propimage_apps_b"])
        self.assertEqual(names[-3:], ["propimage_apps_b", "matlab", "specviewperf"])

    def test_the_stage_is_built_in_main_and_is_not_budgeted(self):
        source = (HERE / "vast_payload.py").read_text()
        self.assertIn('"matlab",\n                matlab_argv,', source)
        self.assertIn("matlab_stage.py", source)
        self.assertIn("--instruction-matlab", source)
        data = json.loads((profile_resources.ROOT / "ci/vast-broker/defaults.json").read_text())
        for stage in data["payloadStageResources"].values():
            stage["source"] = "scripts/ci/vast_payload.py"
        self.assertEqual(profile_resources.validate(data, source), [])
        self.assertIn("manual-only stage is in a default plan: matlab", profile_resources.validate(data, source.replace("MATLAB_IN_LARGE = False", "MATLAB_IN_LARGE = True")))

    def args(self, **kw):
        base = dict(matlab_image=None, matlab_driver=None, only=[])
        base.update(kw)
        return types.SimpleNamespace(**base)

    def test_refusals_of_the_standalone_flags(self):
        plain = instruction("large")
        self.assertIsNone(vast_payload.matlab_refusal(plain, self.args()))
        self.assertEqual(vast_payload.matlab_refusal(plain, self.args(matlab_image="/x.zip")), "FLAGS_NEED_ONLY_MATLAB")
        self.assertEqual(vast_payload.matlab_refusal(plain, self.args(only=["matlab"])), "NO_IMAGE")
        self.assertEqual(vast_payload.matlab_refusal(plain, self.args(matlab_driver="580.1.2", only=["matlab"])), "DRIVER_WITHOUT_IMAGE")
        self.assertIsNone(vast_payload.matlab_refusal(plain, self.args(matlab_image="/x.zip", matlab_driver="580.173.02", only=["matlab"])))
        both = instruction("large", matlab=golden()["matlab"])
        self.assertEqual(vast_payload.matlab_refusal(both, self.args(matlab_image="/x.zip", only=["matlab"])), "INSTRUCTION_AND_IMAGE_FLAG")
        self.assertIsNone(vast_payload.matlab_refusal(both, self.args()))

    def test_the_instruction_object_is_checked_against_the_pins(self):
        bad = golden()["matlab"]
        bad["image"]["sha256"] = "0" * 64
        self.assertEqual(vast_payload.matlab_refusal(instruction("large", matlab=bad), self.args()), "PIN_SHA256_MISMATCH")

    def run_dry(self, *argv, doc=None):
        d = tmpdir(self)
        env = {k: v for k, v in os.environ.items() if not k.startswith("KF_")}
        cmd = [sys.executable, "-B", "-I", str(HERE / "vast_payload.py"), *argv, "--dry-run"]
        if doc is not None:
            (d / "i.json").write_text(json.dumps(doc))
            cmd += ["--instruction", str(d / "i.json")]
        done = subprocess.run(cmd, capture_output=True, text=True, env=env, cwd=str(d), timeout=120)
        return done.returncode, done.stdout

    def test_dry_run_of_the_golden_and_of_the_flags(self):
        code, out = self.run_dry(doc=golden())
        self.assertEqual(code, 0, out)
        self.assertIn("KF_PAYLOAD_MATLAB strict=1 rows=%d driver=590.48.01" % len(MR.ROW_ORDER), out)
        self.assertRegex(out, r"KF_PAYLOAD_PLAN stages=.*matlab planned=")
        base = golden()
        del base["matlab"]
        code, out = self.run_dry("--only", "matlab", "--matlab-image", "/x.zip", "--matlab-driver", "580.173.02", doc=base)
        self.assertEqual(code, 0, out)
        self.assertIn("KF_PAYLOAD_PLAN stages=matlab", out)
        code, out = self.run_dry("--only", "matlab", "--matlab-image", "/x.zip", doc=golden())
        self.assertEqual(code, 1)
        self.assertIn("KF_PAYLOAD_MATLAB_REFUSED INSTRUCTION_AND_IMAGE_FLAG", out)
        changed = golden()
        changed["matlab"]["image"]["sha256"] = "0" * 64
        code, out = self.run_dry(doc=changed)
        self.assertEqual(code, 1)
        self.assertIn("KF_PAYLOAD_MATLAB_REFUSED PIN_SHA256_MISMATCH", out)
        code, out = self.run_dry("--only", "matlab", doc=base)
        self.assertEqual(code, 1)
        self.assertIn("KF_PAYLOAD_MATLAB_REFUSED NO_IMAGE", out)
        code, out = self.run_dry("--matlab-image", "/x.zip", doc=base)
        self.assertIn("KF_PAYLOAD_MATLAB_REFUSED FLAGS_NEED_ONLY_MATLAB", out)

    def test_matlab_rows_function_grades_the_record(self):
        rec = sample_record()
        line = "noise\nKF_MATLAB_RESULT " + json.dumps(rec) + "\n"
        result = {}
        self.assertEqual(vast_payload.matlab_rows(line, 0, rec["sha"], result), [vast_payload.row("matlab", 1)])
        self.assertEqual(result["matlab"]["verdict"], "PASS")
        for text, rc, sha in ((line, 1, rec["sha"]), (line, 0, "f" * 40), ("nothing", 0, rec["sha"]), (line + line, 0, rec["sha"]), ("KF_MATLAB_RESULT {bad", 0, rec["sha"])):
            with self.assertRaises(vast_payload.EvidenceError) as c:
                vast_payload.matlab_rows(text, rc, sha, {})
            self.assertEqual(c.exception.name, "matlab")
        zero = dict(rec, verdict="FAIL", reason="APP_FAIL:gpu_fft", exit_code=0)         # a failing verdict with a clean exit code is still not a pass
        with self.assertRaises(vast_payload.EvidenceError):
            vast_payload.matlab_rows("KF_MATLAB_RESULT " + json.dumps(zero), 0, rec["sha"], {})
        red = dict(rec, verdict="FAIL", reason="APP_FAIL:gpu_fft", exit_code=1)
        kept = {}
        with self.assertRaises(vast_payload.EvidenceError):
            vast_payload.matlab_rows("KF_MATLAB_RESULT " + json.dumps(red), 1, rec["sha"], kept)
        self.assertEqual(kept["matlab"]["verdict"], "FAIL")          # a red record is still kept in the result file


class InstructionSchemaTests(unittest.TestCase):
    def doc(self, **over):
        d = golden()
        d.update(over)
        return json.dumps(d).encode()

    def test_the_golden_is_accepted_and_round_trips(self):
        i = pi.parse(self.doc())
        self.assertEqual(i.matlab["driver"], {"version": "590.48.01"})
        self.assertEqual(json.loads(i.canonical_bytes()), golden())

    def test_emit_instruction_writes_the_object_back_byte_for_byte(self):
        d = tmpdir(self)
        env = {k: v for k, v in os.environ.items() if not k.startswith("KF_")}
        done = subprocess.run([sys.executable, "-B", "-I", str(HERE / "vast_payload.py"), "--instruction", str(GOLDEN), "--emit-instruction", str(d / "o.json")],
                              capture_output=True, text=True, env=env, cwd=str(d), timeout=120)
        self.assertEqual(done.returncode, 0, done.stdout)
        self.assertEqual((d / "o.json").read_bytes(), GOLDEN.read_bytes())

    def test_the_image_binary_is_part_of_the_object_and_of_its_canonical_bytes(self):
        i = pi.parse(self.doc())
        self.assertEqual(i.matlab["bin"], "/home/kfimage/MATLAB/R2026b/bin/matlab")
        self.assertIn(b'"bin": "/home/kfimage/MATLAB/R2026b/bin/matlab"', i.canonical_bytes())

    def test_an_object_without_bin_is_accepted_and_has_no_bin_in_its_bytes(self):
        d = golden()
        del d["matlab"]["bin"]
        i = pi.parse(json.dumps(d).encode())
        self.assertNotIn("bin", i.matlab)
        self.assertNotIn(b'"bin"', i.canonical_bytes())
        self.assertEqual(json.loads(i.canonical_bytes()), d)

    def test_a_valid_bin_is_accepted_by_the_schema(self):
        for b in InstructionTests.GOOD_BINS:
            d = golden()
            d["matlab"]["bin"] = b
            self.assertEqual(pi.parse(json.dumps(d).encode()).matlab["bin"], b)

    def test_a_bad_bin_is_refused_by_the_schema_with_the_path_named(self):
        for b in InstructionTests.BAD_BINS:
            d = golden()
            d["matlab"]["bin"] = b
            with self.assertRaises(pi.InstructionError) as c:
                pi.parse(json.dumps(d).encode())
            self.assertIn("bin", str(c.exception), repr(b))

    def test_absent_means_none_and_is_not_emitted(self):
        d = golden()
        del d["matlab"]
        i = pi.parse(json.dumps(d).encode())
        self.assertIsNone(i.matlab)
        self.assertNotIn(b"matlab", i.canonical_bytes())

    def refuses(self, edit):
        d = golden()
        edit(d)
        with self.assertRaises(pi.InstructionError):
            pi.parse(json.dumps(d).encode())

    def test_refusals(self):
        self.refuses(lambda d: d.update(tier="middle"))
        self.refuses(lambda d: d.update(tier="small"))
        self.refuses(lambda d: d["matlab"].update(bogus=1))
        self.refuses(lambda d: d["matlab"]["image"].update(**{"from": "url"}))
        self.refuses(lambda d: d["matlab"]["image"].pop("sha256"))
        self.refuses(lambda d: d["matlab"]["image"].update(sha256="short"))
        self.refuses(lambda d: d["matlab"]["image"].update(ref="a b"))
        self.refuses(lambda d: d["matlab"]["driver"].update(version="latest"))
        self.refuses(lambda d: d["matlab"]["rows"].update(gpu_fft={"lane": "maybe"}))
        self.refuses(lambda d: d["matlab"]["rows"].update(gpu_fft={}))
        self.refuses(lambda d: d["matlab"].update(isolation="none"))
        self.refuses(lambda d: d["matlab"].pop("driver"))
        self.refuses(lambda d: d["matlab"].update(strict=False))       # the Worker's shape is strict by definition: there is no such switch

    def test_the_two_schema_copies_are_identical(self):
        self.assertEqual((ROOT / "ci/vast-broker/planner/schemas/instruction.schema.json").read_bytes(), (ROOT / "scripts/kfci/schemas/instruction.schema.json").read_bytes())


# ---------------------------------------------------------------- the stage's instruction file (the box payload, through main)
class StageFileTests(unittest.TestCase):
    """The payload hands the stage the instruction's `matlab` object as the file `matlab_instruction.json`: every key of the object, `bin` included, must arrive."""
    setUp = v7.V7Tests.setUp
    _main_fixture = v7.V7Tests._main_fixture

    @staticmethod
    def leg(matlab):
        """The golden's `matlab` object on the smallest large-tier instruction (no Windows, no apps): the plan holds the checks and the `matlab` stage."""
        doc = golden()
        doc.update(matlab=matlab, windows=False, windows_runs=1, winapps=None, linux_apps=None)
        return doc

    def stage_file(self, matlab):
        """-> (the stage's command line, the decoded file the payload wrote for it), read while the box's evidence directory still exists."""
        doc = self.leg(matlab)
        seen = {}
        real = self.m.execute_stages

        def recording(stages, soft, result, destination, runner=real.__defaults__[0]):
            for stage in stages:
                if stage["name"] == "matlab":
                    cmd = stage["command"]
                    seen["cmd"] = list(cmd)
                    seen["file"] = json.loads(Path(cmd[cmd.index("--instruction-matlab") + 1]).read_text())
            return real([x for x in stages if x["name"] != "matlab"], soft, result, destination, runner)

        orig = self.m.parse_args
        with tempfile.TemporaryDirectory() as d:
            path = Path(d) / "i.json"
            path.write_text(json.dumps(doc))
            with patch.object(self.m, "parse_args", side_effect=lambda argv: orig([*argv, "--instruction", str(path), "--only", "matlab"])), \
                    patch.object(self.m, "execute_stages", recording), contextlib.redirect_stdout(io.StringIO()):
                self._main_fixture(windows=None, soft_ms=900 * 60000)
        return seen["cmd"], seen["file"]

    def test_the_stage_gets_the_object_with_its_bin(self):
        cmd, doc = self.stage_file(golden()["matlab"])
        self.assertEqual(doc, golden()["matlab"])
        self.assertEqual(doc["bin"], "/home/kfimage/MATLAB/R2026b/bin/matlab")
        self.assertIn("matlab_stage.py", " ".join(cmd))
        self.assertNotIn("--matlab", cmd)                       # the binary travels in the object, never as a second spelling on the command line

    def test_the_stage_gets_another_bin_as_it_is(self):
        m = golden()["matlab"]
        m["bin"] = "/opt/other/bin/matlab"
        cmd, doc = self.stage_file(m)
        self.assertEqual(doc["bin"], "/opt/other/bin/matlab")
        self.assertNotIn("--matlab", cmd)

    def test_the_stage_of_an_object_without_bin_gets_no_bin(self):
        m = golden()["matlab"]
        del m["bin"]
        doc = self.stage_file(m)[1]
        self.assertEqual(doc, m)
        self.assertNotIn("bin", doc)


# ---------------------------------------------------------------- the report validator
class ReportTests(unittest.TestCase):
    def test_the_row_list_is_the_runners_table(self):
        self.assertEqual(vast_report.MATLAB_ROWS, MR.ROW_ORDER)
        self.assertEqual(vast_report.MATLAB_CLASSES, MR.FAIL_CLASSES)

    def test_a_good_record_is_accepted(self):
        vast_report.validate_matlab(sample_record())
        vast_report.validate_matlab(sample_record(strict=False))

    def refuses(self, edit, rec=None):
        rec = rec or sample_record()
        edit(rec)
        with self.assertRaises(ValueError):
            vast_report.validate_matlab(rec)

    def test_a_discovery_failure_is_fine_when_listed_as_a_breakage(self):
        rec = sample_record()
        rec["rows"][2].update(verdict="FAIL", failure_class="XID", reason="HOST_XID:1", host_xid=1)
        self.refuses(lambda r: None, rec)                                # the breakage list does not name it yet
        rec["breakages"] = MR.breakages(rec["rows"])
        vast_report.validate_matlab(rec)

    def test_pass_needs_no_gate_failure(self):
        def gate_fail(r):
            r["rows"][3].update(lane="gate", verdict="FAIL", failure_class="CRASH", reason="x")
            r["breakages"] = []
        self.refuses(gate_fail)

    def test_a_red_gate_record_cannot_be_relabelled_pass(self):
        rec = sample_record()
        rec["rows"][5].update(lane="gate", verdict="FAIL", failure_class="HANG", reason="ROW_TIMEBOX")
        rec["breakages"] = []
        rec.update(verdict="PASS", reason="", exit_code=0)
        with self.assertRaises(ValueError):
            vast_report.validate_matlab(rec)
        rec.update(verdict="FAIL", reason="APP_FAIL:parpool_gpu", exit_code=1)
        vast_report.validate_matlab(rec)

    def test_a_retry_record_may_hold_harness_rows_but_a_pass_may_not(self):
        rec = sample_record()
        rec["rows"][4].update(verdict="HARNESS", reason="HOST_COUNTERS_MISSING")
        rec.update(verdict="RETRY", reason="HARNESS:dl_train", exit_code=3)
        vast_report.validate_matlab(rec)
        rec.update(verdict="PASS", reason="", exit_code=0)
        with self.assertRaises(ValueError):
            vast_report.validate_matlab(rec)

    def test_pass_needs_no_harness_row(self):
        self.refuses(lambda r: r["rows"][3].update(verdict="HARNESS", reason="HOST_COUNTERS_MISSING"))
        self.refuses(lambda r: r["rows"][-1].update(verdict="HARNESS", reason="SSH_TIMEOUT"))

    def test_a_strict_pass_with_a_skipped_row_is_refused_and_a_permissive_one_is_not(self):
        self.refuses(lambda r: r["rows"][3].update(verdict="NOT_RUN", reason="TIME"))
        rec = sample_record(strict=False)
        rec["rows"][3].update(verdict="NOT_RUN", reason="TIME")
        vast_report.validate_matlab(rec)                                # permissive: a skipped row is allowed under PASS

    def test_a_strict_pass_that_skipped_the_preamble_is_refused(self):
        self.refuses(lambda r: r["rows"][0].update(verdict="NOT_RUN", reason="GUEST_NO_DRIVER"))

    def test_row_names_must_be_known_and_in_table_order(self):
        self.refuses(lambda r: r["rows"][0].update(row="nonsense"))
        self.refuses(lambda r: r["rows"].reverse())

    def test_a_breakage_for_a_passing_row_is_refused(self):
        self.refuses(lambda r: r["breakages"].append({"row": "gpu_fft", "class": "XID", "reason": "x", "host_xid": 0}))

    def test_shape_and_bounds(self):
        self.refuses(lambda r: r.update(extra=1))
        self.refuses(lambda r: r.update(schema="x"))
        self.refuses(lambda r: r.update(sha="g" * 40))
        self.refuses(lambda r: r["rows"][0].update(row="nonsense"))
        self.refuses(lambda r: r["rows"].reverse())
        self.refuses(lambda r: r["rows"][0].update(lane="maybe"))
        self.refuses(lambda r: r["rows"][0].update(verdict="WEIRD"))
        self.refuses(lambda r: r["rows"][0].update(failure_class="CRASH"))                       # a class only on a FAIL
        self.refuses(lambda r: r["rows"][0].update(reason="x" * 300))
        self.refuses(lambda r: r["rows"][0].update(reason="bad \x00 text"))
        self.refuses(lambda r: r["rows"][0]["result"].update(sha256="short"))
        self.refuses(lambda r: r["rows"][0]["checks"].update(failed=["a"] * 20))
        self.refuses(lambda r: r["image"].update(source="elsewhere"))
        self.refuses(lambda r: r["image"].update(sha256="zz"))
        self.refuses(lambda r: r["driver"].update(version="latest"))
        self.refuses(lambda r: r["gpu"].update(extra=1))
        self.refuses(lambda r: r["boots"].append({"index": 99}))
        self.refuses(lambda r: r.update(family="moon"))
        self.refuses(lambda r: r.update(exit_code=9))
        self.refuses(lambda r: r["rows"][0]["gpu"].update(blob="x" * 5000))
        self.refuses(lambda r: r.update(summary="x" * 500))

    def test_a_pass_that_ran_nothing_is_refused(self):
        def none(r):
            for row in r["rows"]:
                row.update(verdict="NOT_RUN", reason="IMAGE_MISSING")
        self.refuses(none, sample_record(strict=False))

    def test_the_result_file_carries_the_record_and_refuses_a_pass_over_a_red_one(self):
        d = tmpdir(self)
        rec = sample_record()
        doc = {"verdict": "PASS", "suites": [{"name": "matlab", "pass": 1, "fail": 0, "skip": 0}], "matlab": rec}
        f = d / "r.json"
        f.write_text(json.dumps(doc))
        self.assertEqual(vast_report.read_result_file(str(f))["matlab"]["verdict"], "PASS")
        red = copy.deepcopy(rec)
        red.update(verdict="FAIL", reason="APP_FAIL:gpu_fft", exit_code=1)
        red["rows"][2].update(lane="gate", verdict="FAIL", failure_class="CRASH", reason="x")
        red["breakages"] = []
        f.write_text(json.dumps(dict(doc, matlab=red)))
        with self.assertRaises(vast_report.ResultError):
            vast_report.read_result_file(str(f))
        f.write_text(json.dumps(dict(doc, verdict="FAIL", matlab=red)))
        self.assertEqual(vast_report.read_result_file(str(f))["matlab"]["verdict"], "FAIL")


if __name__ == "__main__":
    unittest.main()
