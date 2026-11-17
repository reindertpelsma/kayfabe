#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later
"""GPU-free tests of the identity-pin rule (owner, 2026-11-17; docs/design/V3_PINS_ARE_DEPLOYMENT_DATA.md): the sha256 of a user-supplied
image is a PIN that only a strict run (the Worker, strict CI) enforces. A standalone run records the computed sha256 and size and goes on;
`--expect-sha256 NAME=HASH` is the user's own opt-in. The matrix is {standalone, strict} x {pin matches, pin differs, no pin}, for a file, a URL, a
pin object and a directory; the downloaded third-party tools (swtpm, libtpms) keep their pins in every mode.

Uses the fakes of test_propimage_images.py (no VM, no network, no box)."""
import contextlib
import io
import json
from pathlib import Path
import shutil
import sys
import unittest
import unittest.mock
import zipfile

HERE = Path(__file__).resolve().parent
sys.path.insert(0, str(HERE))
import propimage_apps as pa  # noqa: E402
import propimage_images as images  # noqa: E402
import propimage_runspec as S  # noqa: E402
import test_propimage_images as T  # noqa: E402
import vast_report  # noqa: E402

H64, OTHER = T.H64, "e" * 64
APPS = {"3dmark": {"mode": "present"}}


def zip_file(path):
    with zipfile.ZipFile(path, "w") as z:
        for n in ("image.qcow2", "state.json", "manifest.json"):
            z.writestr(n, "x")
    return path


def spec(img, strict, **top):
    return dict(T.doc(img), **({"strict": True} if strict else {}), **top)


class Matrix(unittest.TestCase):
    """{standalone, strict} x {no pin, pin matches, pin differs} for a file source (the disk's computed sha256 is H64)."""

    def go(self, strict, pin):
        env = T.Env(self)
        z = zip_file(env.tmp / "mine.zip")
        disk = T.src(z) if pin is None else T.src(z, sha256=pin)
        rec = env.run(spec({"kind": "prebuilt", "disk": disk, "apps": APPS}, strict))
        return env, json.loads(json.dumps(rec))

    def check(self, rec, verdict, reason, identity, pin):
        im = rec["images"][0]
        self.assertEqual((im["verdict"], im["reason"], im["source"]["identity"], im["source"]["pin_sha256"]), (verdict, reason, identity, pin))
        self.assertEqual((rec["pins_enforced"], im["source"]["sha256"]), (rec["strict"], H64))      # the computed hash is recorded, refused or not
        self.assertIsInstance(im["source"]["size"], int)
        vast_report.validate_propimage_run(rec)

    def test_standalone_without_a_pin_runs_and_records_the_hash(self):
        env, rec = self.go(False, None)
        self.check(rec, "PASS", "", "none", None)
        self.assertEqual(len(env.cmds), 1)

    def test_standalone_with_a_matching_pin_runs(self):
        env, rec = self.go(False, H64)
        self.check(rec, "PASS", "", "match", H64)

    def test_standalone_with_a_differing_pin_runs_and_records_the_difference(self):
        env, rec = self.go(False, OTHER)
        self.check(rec, "PASS", "", "differs", OTHER)
        self.assertEqual((rec["verdict"], len(env.cmds)), ("PASS", 1))

    def test_strict_without_a_pin_runs_and_records_the_hash(self):
        env, rec = self.go(True, None)
        self.check(rec, "PASS", "", "none", None)

    def test_strict_with_a_matching_pin_runs(self):
        env, rec = self.go(True, H64)
        self.check(rec, "PASS", "", "match", H64)

    def test_strict_with_a_differing_pin_is_refused_before_any_boot(self):
        env, rec = self.go(True, OTHER)
        self.check(rec, "REFUSED", "SOURCE_SHA256_MISMATCH", "differs", OTHER)
        self.assertEqual((rec["verdict"], env.cmds), ("REFUSED", []))


class Sources(unittest.TestCase):
    """The same rule for the other source kinds: a URL, a pin object, a directory, a state file."""

    def test_a_url_standalone_is_fetched_without_a_pin_check_and_the_hash_is_recorded(self):
        for strict, pin, verdict in ((False, OTHER, "PASS"), (True, OTHER, "REFUSED"), (False, H64, "PASS"), (True, H64, "PASS"), (False, None, "PASS"), (True, None, "PASS")):
            env, seen = T.Env(self), []

            def fetch(url, dest, sha=None, seen=seen):
                seen.append(sha)
                zip_file(dest)
                if sha and sha != H64:
                    raise pa.Refused("SOURCE_SHA256_MISMATCH")          # what the real fetcher does while the bytes arrive
                return H64
            env.hooks.fetch_url = fetch
            disk = {"from": "url", "ref": "https://example.invalid/a.zip", "sha256": pin}
            rec = env.run(spec({"kind": "prebuilt", "disk": disk, "apps": APPS}, strict))
            im = rec["images"][0]
            self.assertEqual((strict, pin, im["verdict"], seen), (strict, pin, verdict, [pin] if strict else [None]))
            self.assertEqual(im["source"]["sha256"], None if verdict == "REFUSED" else H64)   # strict refuses while the bytes arrive: nothing was read back
            self.assertEqual(rec["pins_enforced"], strict)

    def test_a_pin_object_standalone_ignores_a_spec_hash_that_differs_from_the_deployments_pin_and_records_it(self):
        ref = "propimage-kfimg-20261112.zip"
        real = images.all_pins()[ref]["sha256"]
        env = T.Env(self)
        rec = env.run(spec({"kind": "prebuilt", "disk": {"from": "pin", "ref": ref, "sha256": OTHER}, "apps": APPS}, False))
        src = rec["images"][0]["source"]
        self.assertEqual((rec["images"][0]["verdict"], src["sha256"], src["pin_sha256"], src["identity"]), ("PASS", real, OTHER, "differs"))
        env2 = T.Env(self)
        rec2 = env2.run(spec({"kind": "prebuilt", "disk": {"from": "pin", "ref": ref, "sha256": OTHER}, "apps": APPS}, True))
        self.assertEqual((rec2["images"][0]["reason"], env2.cmds), ("PIN_SHA256_MISMATCH", []))

    def test_a_pin_object_with_no_hash_in_the_spec_runs_in_both_modes(self):
        for strict in (False, True):
            env = T.Env(self)
            rec = env.run(spec({"kind": "prebuilt", "disk": {"from": "pin", "ref": "propimage-kfimg-20261112.zip"}, "apps": APPS}, strict))
            self.assertEqual((rec["images"][0]["verdict"], rec["images"][0]["source"]["identity"]), ("PASS", "none"))

    def test_a_pin_that_the_deployment_does_not_know_is_refused_in_both_modes(self):
        for strict in (False, True):
            rec = T.Env(self).run(spec({"kind": "prebuilt", "disk": {"from": "pin", "ref": "no-such-object.zip"}, "apps": APPS}, strict))
            self.assertEqual(rec["images"][0]["reason"], "PIN_UNKNOWN")

    def own_pins(self, env):
        doc = json.loads((HERE / "propimage_apps_pins.json").read_text())
        for item in doc["items"]:
            if item["role"] == "image":
                item.update(name="my-image.zip", dest="image/my.zip", ref="my-image.zip", sha256="a" * 64, size=5)
            if item["role"] == "gop":
                item.update(sha256="b" * 64)                     # same ref as the repository's, YOUR hash
        mine = env.tmp / "my_pins.json"
        mine.write_text(json.dumps(doc))
        return doc, mine

    def test_your_own_pins_file_adds_its_objects(self):
        env = T.Env(self)
        doc, mine = self.own_pins(env)
        self.assertIn("my-image.zip", images.all_pins(extra=mine))
        self.assertNotIn("my-image.zip", images.all_pins())

    def test_your_own_pins_file_wins_over_the_repositorys_of_the_same_name(self):
        env = T.Env(self)
        doc, mine = self.own_pins(env)
        gop_ref = next(i["ref"] for i in doc["items"] if i["role"] == "gop")
        self.assertEqual((images.all_pins(extra=mine)[gop_ref]["sha256"], images.all_pins()[gop_ref]["sha256"] == "b" * 64), ("b" * 64, False))

    def test_your_own_pins_file_reaches_the_runner_and_its_object_is_the_one_fetched(self):
        env = T.Env(self)
        doc, mine = self.own_pins(env)
        env.ctx.pins_path = mine
        rec = env.run(spec({"kind": "prebuilt", "disk": {"from": "pin", "ref": "my-image.zip"}, "apps": APPS}, True))
        self.assertEqual((rec["images"][0]["verdict"], rec["images"][0]["source"]["sha256"]), ("PASS", "a" * 64))

    def test_a_directory_has_no_hash_so_a_pin_on_it_is_recorded_standalone_and_refused_strict(self):
        env = T.Env(self)
        rec = env.run(spec({"kind": "prebuilt", "disk": T.src(env.state_dir(), sha256=H64), "apps": APPS}, False))
        im = rec["images"][0]
        self.assertEqual((im["verdict"], im["source"]["identity"], im["source"]["sha256"], im["source"]["pin_sha256"]), ("PASS", "unverifiable", None, H64))
        self.assertIsInstance(im["source"]["size"], int)
        vast_report.validate_propimage_run(json.loads(json.dumps(rec)))
        env2 = T.Env(self)
        rec2 = env2.run(spec({"kind": "prebuilt", "disk": T.src(env2.state_dir(), sha256=H64), "apps": APPS}, True))
        self.assertEqual((rec2["images"][0]["reason"], env2.cmds), ("SHA256_ON_DIRECTORY", []))

    def test_a_url_with_a_differing_pin_runs_standalone_and_is_noted(self):
        env = T.Env(self)

        def fetch(url, dest, sha=None):
            zip_file(dest)
            if sha and sha != H64:
                raise pa.Refused("SOURCE_SHA256_MISMATCH")
            return H64
        env.hooks.fetch_url = fetch
        rec = env.run(spec({"kind": "prebuilt", "disk": {"from": "url", "ref": "https://example.invalid/a.zip", "sha256": OTHER}, "apps": APPS}, False))
        src = rec["images"][0]["source"]
        self.assertEqual((rec["images"][0]["verdict"], src["identity"], src["sha256"], src["pin_sha256"]), ("PASS", "differs", H64, OTHER))

    def test_a_standalone_directory_pin_is_noted_on_one_line_and_exits_0(self):
        env = T.Env(self)
        rec = env.run(spec({"kind": "prebuilt", "disk": T.src(env.state_dir(), sha256=H64), "apps": APPS}, False))
        out = io.StringIO()
        with contextlib.redirect_stdout(out):
            rc = images.finish(json.loads(json.dumps(rec)), None)
        self.assertEqual(rc, 0)
        self.assertIn("identity=unverifiable", out.getvalue())

    def test_your_pin_of_the_repositorys_ref_is_the_one_fetched(self):
        env = T.Env(self)
        doc = json.loads((HERE / "propimage_apps_pins.json").read_text())
        for item in doc["items"]:
            if item["role"] == "image":
                item.update(sha256="c" * 64, size=7)           # the repository's ref, YOUR object
                ref = item["ref"]
        mine = env.tmp / "p.json"
        mine.write_text(json.dumps(doc))
        env.ctx.pins_path = mine
        rec = env.run(spec({"kind": "prebuilt", "disk": {"from": "pin", "ref": ref}, "apps": APPS}, False))
        self.assertEqual(rec["images"][0]["source"]["sha256"], "c" * 64)

    def test_a_state_file_pin_is_enforced_only_when_strict_and_its_hash_is_recorded(self):
        for strict, verdict in ((False, "PASS"), (True, "REFUSED")):
            env = T.Env(self)
            disk, state = env.tmp / "d.qcow2", env.tmp / "s.json"
            disk.write_text("x")
            state.write_text("{}")
            rec = env.run(spec({"kind": "prebuilt", "disk": T.src(disk), "state_file": T.src(state, sha256=OTHER), "apps": APPS}, strict))
            im = rec["images"][0]
            self.assertEqual((im["verdict"], im["reason"] if strict else ""), (verdict, "STATE_FILE_SHA256_MISMATCH" if strict else ""))
            self.assertEqual(im["source"]["state_sha256"], T.H64)

    def test_the_computed_hash_and_size_are_recorded_for_every_run_and_it_validates(self):
        env = T.Env(self)
        z = zip_file(env.tmp / "x.zip")
        rec = env.run(spec({"kind": "prebuilt", "disk": T.src(z), "apps": APPS}, False))
        src = rec["images"][0]["source"]
        self.assertEqual((src["sha256"], src["size"]), (H64, z.stat().st_size))
        vast_report.validate_propimage_run(json.loads(json.dumps(rec)))


class ExpectSha256(unittest.TestCase):
    """`--expect-sha256 NAME=HASH` is the user's own request: enforced in EVERY mode, by name."""

    def go(self, strict, expect, name="mine"):
        env = T.Env(self)
        z = zip_file(env.tmp / "mine.zip")
        rec = env.run(spec({"kind": "prebuilt", "label": "mine", "disk": T.src(z), "apps": APPS}, strict, expect_sha256={name: expect}))
        return env, rec

    def test_a_matching_expectation_runs_standalone_and_strict(self):
        for strict in (False, True):
            env, rec = self.go(strict, H64)
            self.assertEqual((rec["images"][0]["verdict"], rec["images"][0]["source"]["expect_sha256"]), ("PASS", H64))
            vast_report.validate_propimage_run(json.loads(json.dumps(rec)))

    def test_a_differing_expectation_is_refused_standalone_too_before_any_boot(self):
        for strict in (False, True):
            env, rec = self.go(strict, OTHER)
            self.assertEqual((rec["images"][0]["reason"], env.cmds, rec["images"][0]["source"]["sha256"]), ("EXPECT_SHA256_MISMATCH", [], H64))

    def test_an_expectation_on_a_directory_is_refused_in_a_strict_run_too(self):
        env = T.Env(self)
        rec = env.run(spec({"kind": "prebuilt", "label": "mine", "disk": T.src(env.state_dir()), "apps": APPS}, True, expect_sha256={"mine": H64}))
        self.assertEqual(rec["images"][0]["reason"], "EXPECT_SHA256_ON_DIRECTORY")

    def cli(self, env, argv, **patches):
        """pa.main with the context faked; patches are `pa_<name>` / `images_<name>` keyword functions."""
        out = io.StringIO()
        with contextlib.redirect_stdout(out), contextlib.ExitStack() as stack:
            stack.enter_context(unittest.mock.patch.object(pa, "Context", lambda *a, **k: env.ctx))
            for key, fn in patches.items():
                module, name = key.split("_", 1)
                stack.enter_context(unittest.mock.patch.object(pa if module == "pa" else images, name, fn))
            rc = pa.main(argv)
        return rc, out.getvalue()

    def test_a_malformed_expectation_on_the_command_line_is_a_usage_error(self):
        env = T.Env(self)
        rc, out = self.cli(env, ["run", "--sha", T.SHA, "--out", "/tmp/never.json", "--image", "prebuilt:/x.zip", "--expect-sha256", "x.zip=abc"])
        self.assertEqual(rc, 2)
        self.assertIn("KF_PROPIMAGE_APPS_USAGE SPEC:EXPECT_SHA256_BAD", out)          # refused as a usage error, before any run record

    def test_an_expectation_without_an_image_never_starts_the_legacy_stage(self):
        env = T.Env(self)

        def boom(*a, **k):
            raise AssertionError("the legacy stage must not start")
        rc, out = self.cli(env, ["run", "--sha", T.SHA, "--out", "/tmp/never.json", "--expect-sha256", "x.zip=" + H64], pa_run=boom)
        self.assertEqual(rc, 2)
        self.assertIn("EXPECT_WITHOUT_IMAGES", out)

    def test_strict_with_an_expectation_reaches_the_runner(self):
        env, seen = T.Env(self), []

        def run_images(ctx, spec_doc, *a, **k):
            seen.append(spec_doc)
            return {"verdict": "PASS"}
        rc, out = self.cli(env, ["run", "--sha", T.SHA, "--out", "/tmp/never.json", "--strict", "--image", "prebuilt:/x.zip", "--app", "3dmark=present",
                                 "--expect-sha256", "x.zip=" + H64], images_run_images=run_images, images_finish=lambda rec, out: 0)
        self.assertEqual((rc, seen[0]["strict"], seen[0]["expect_sha256"]), (0, True, {"x.zip": H64}))

    def test_the_file_name_of_the_source_names_it_too(self):
        env, rec = self.go(False, OTHER, name="mine.zip")
        self.assertEqual(rec["images"][0]["reason"], "EXPECT_SHA256_MISMATCH")

    def test_an_expectation_on_a_directory_cannot_be_checked_and_is_refused(self):
        env = T.Env(self)
        rec = env.run(spec({"kind": "prebuilt", "label": "mine", "disk": T.src(env.state_dir()), "apps": APPS}, False, expect_sha256={"mine": H64}))
        self.assertEqual(rec["images"][0]["reason"], "EXPECT_SHA256_ON_DIRECTORY")

    def test_an_expectation_is_checked_against_the_pin_object_it_names(self):
        env = T.Env(self)
        ref = "propimage-kfimg-20261112.zip"
        rec = env.run(spec({"kind": "prebuilt", "disk": {"from": "pin", "ref": ref}, "apps": APPS}, False, expect_sha256={ref: OTHER}))
        self.assertEqual(rec["images"][0]["reason"], "EXPECT_SHA256_MISMATCH")

    def test_a_name_that_calls_no_image_is_a_spec_error_never_silent(self):
        env = T.Env(self)
        rec = env.run(spec({"kind": "prebuilt", "label": "mine", "disk": T.src(zip_file(env.tmp / "m.zip")), "apps": APPS}, False, expect_sha256={"typo": H64}))
        self.assertEqual((rec["verdict"], rec["reason"]), ("REFUSED", "SPEC:EXPECT_SHA256_UNKNOWN:typo"))

    def test_spec_validation_names_its_refusals(self):
        base = T.doc({"kind": "prebuilt", "label": "mine", "disk": T.src("/x.zip")})
        for bad, reason in (({"mine": "xyz"}, "EXPECT_SHA256_BAD"), ({"mine": 5}, "EXPECT_SHA256_BAD"), ({"nobody": H64}, "EXPECT_SHA256_UNKNOWN")):
            with self.assertRaises(S.SpecError) as c:
                S.validate(dict(base, expect_sha256=bad))
            self.assertEqual(c.exception.reason, reason)
        with self.assertRaises(S.SpecError) as c:
            S.validate(dict(base, expect_sha256={"mine": H64, "x.zip": OTHER}))
        self.assertEqual(c.exception.reason, "EXPECT_SHA256_DUP")
        self.assertEqual(S.validate(dict(base, expect_sha256={"x.zip": H64}))["expect_sha256"], {"mine": H64})
        self.assertEqual(S.validate(base)["expect_sha256"], {})

    def test_the_command_line_flag_builds_the_expectation(self):
        words, got = S.split_flags(["run", "--image", "prebuilt:/d/a.zip", "--label", "mine", "--expect-sha256", "mine=" + H64, "--expect-sha256=a.zip=" + H64])
        self.assertEqual((words, got["expect_sha256"]), (["run"], {"mine": H64, "a.zip": H64}))
        self.assertNotIn("strict", got)
        for argv, reason in ((["--expect-sha256", "mine=" + H64], "EXPECT_WITHOUT_IMAGES"),
                             (["--image", "prebuilt:/a.zip", "--expect-sha256", "mine"], "EXPECT_SHA256_BAD"),
                             (["--image", "prebuilt:/a.zip", "--expect-sha256", "mine=abc"], "EXPECT_SHA256_BAD"),
                             (["--image", "prebuilt:/a.zip", "--expect-sha256"], "FLAG_NEEDS_VALUE"),
                             (["--image", "prebuilt:/a.zip", "--expect-sha256", "m=" + H64, "--expect-sha256", "m=" + OTHER], "EXPECT_SHA256_DUP")):
            with self.assertRaises(S.SpecError) as c:
                S.split_flags(argv)
            self.assertEqual(c.exception.reason, reason, argv)

    def test_strict_and_an_expectation_together_from_the_command_line(self):
        words, got = S.split_flags(["--strict", "--image", "prebuilt:/d/a.zip", "--app", "3dmark=present", "--expect-sha256", "image1=" + H64])
        self.assertEqual((got["strict"], got["expect_sha256"]), (True, {"image1": H64}))
        self.assertTrue(S.pins_enforced(S.validate(got)))

    def test_the_cli_runs_with_an_expectation_standalone_and_refuses_a_wrong_one(self):
        env = T.Env(self)
        z = zip_file(env.tmp / "mine.zip")
        for sha, code in ((OTHER, 2), (H64, 0)):
            out = io.StringIO()
            with contextlib.redirect_stdout(out), unittest.mock.patch.object(images, "Hooks", lambda *a, **k: env.hooks), \
                    unittest.mock.patch.object(pa, "Context", lambda *a, **k: env.ctx):
                rc = pa.main(["run", "--sha", T.SHA, "--out", str(env.tmp / "o.json"), "--image", "prebuilt:" + str(z), "--app", "3dmark=present",
                              "--expect-sha256", "mine.zip=" + sha])
            self.assertEqual(rc, code, out.getvalue()[-300:])


class PinsFlag(unittest.TestCase):
    def test_the_pins_flag_reaches_the_context_and_defaults_to_the_repositorys_file(self):
        env, seen = T.Env(self), []

        def context(*a, **k):
            seen.append(k["pins_path"])
            return env.ctx
        for flag, want in (([], pa.PINS_PATH), (["--pins", "/my/pins.json"], Path("/my/pins.json"))):
            for extra in ([], ["--image", "prebuilt:/x.zip", "--app", "3dmark=present"]):
                seen.clear()
                with contextlib.redirect_stdout(io.StringIO()), unittest.mock.patch.object(pa, "Context", context), \
                        unittest.mock.patch.object(images, "run_images", lambda *a, **k: {"verdict": "PASS"}), \
                        unittest.mock.patch.object(images, "finish", lambda rec, out: 0), unittest.mock.patch.object(pa, "run", lambda ctx, ev: {}), \
                        unittest.mock.patch.object(pa, "finish", lambda rec, out: 0):
                    pa.main(["run", "--sha", T.SHA, "--out", "/tmp/never.json", *flag, *extra])
                self.assertEqual(seen, [want])


class Notes(unittest.TestCase):
    def test_a_standalone_run_with_a_differing_pin_says_so_on_one_line_and_exits_0(self):
        env = T.Env(self)
        z = zip_file(env.tmp / "mine.zip")
        rec = env.run(spec({"kind": "prebuilt", "disk": T.src(z, sha256=OTHER), "apps": APPS}, False))
        out = io.StringIO()
        with contextlib.redirect_stdout(out):
            rc = images.finish(json.loads(json.dumps(rec)), None)
        self.assertEqual(rc, 0)
        self.assertIn("KF_PROPIMAGE_RUN_PIN_NOTE image=image1 identity=differs", out.getvalue())

    def test_a_standalone_pin_object_with_a_differing_spec_hash_exits_zero_with_the_note(self):
        env = T.Env(self)
        rec = env.run(spec({"kind": "prebuilt", "disk": {"from": "pin", "ref": "propimage-kfimg-20261112.zip", "sha256": OTHER}, "apps": APPS}, False))
        out = io.StringIO()
        with contextlib.redirect_stdout(out):
            rc = images.finish(json.loads(json.dumps(rec)), None)
        self.assertEqual(rc, 0)
        self.assertIn("identity=differs", out.getvalue())

    def test_a_standalone_state_file_with_a_differing_pin_runs_and_validates(self):
        env = T.Env(self)
        disk, state = env.tmp / "d.qcow2", env.tmp / "s.json"
        disk.write_text("x")
        state.write_text("{}")
        rec = env.run(spec({"kind": "prebuilt", "disk": T.src(disk), "state_file": T.src(state, sha256=OTHER), "apps": APPS}, False))
        self.assertEqual(rec["images"][0]["verdict"], "PASS")
        vast_report.validate_propimage_run(json.loads(json.dumps(rec)))

    def test_the_switch_is_the_strict_flag_and_nothing_else(self):
        self.assertFalse(S.pins_enforced({"strict": False}))
        self.assertFalse(S.pins_enforced({}))
        self.assertTrue(S.pins_enforced({"strict": True}))
        self.assertTrue(S.pins_enforced(S.validate_list([{"kind": "prebuilt", "disk": T.src("/x.zip"), "apps": APPS}], True)))
        self.assertFalse(S.pins_enforced(S.validate_list([{"kind": "prebuilt", "disk": T.src("/x.zip")}], False)))


class ReportInvariants(unittest.TestCase):
    """The report validator refuses a record that contradicts the rule."""

    def record(self, strict, pin):
        env = T.Env(self)
        z = zip_file(env.tmp / "m.zip")
        return json.loads(json.dumps(env.run(spec({"kind": "prebuilt", "disk": T.src(z, sha256=pin), "apps": APPS}, strict))))

    def test_a_differing_pin_under_enforcement_cannot_sit_under_a_pass(self):
        rec = self.record(True, H64)
        rec["images"][0]["source"].update(pin_sha256=OTHER, identity="differs")
        with self.assertRaises(ValueError):
            vast_report.validate_propimage_run(rec)

    def test_a_claimed_match_needs_equal_hashes(self):
        rec = self.record(False, H64)
        rec["images"][0]["source"]["pin_sha256"] = OTHER
        with self.assertRaises(ValueError):
            vast_report.validate_propimage_run(rec)

    def test_pins_enforced_must_equal_strict(self):
        rec = self.record(False, H64)
        rec["pins_enforced"] = True
        with self.assertRaises(ValueError):
            vast_report.validate_propimage_run(rec)

    def test_a_strict_record_that_did_not_enforce_is_refused(self):
        rec = self.record(True, H64)
        rec["pins_enforced"] = False
        with self.assertRaises(ValueError):
            vast_report.validate_propimage_run(rec)

    def test_an_unverifiable_pin_under_enforcement_cannot_sit_under_a_pass(self):
        rec = self.record(True, H64)
        rec["images"][0]["source"].update(identity="unverifiable", sha256=None)
        with self.assertRaises(ValueError):
            vast_report.validate_propimage_run(rec)

    def test_a_differing_pin_whose_hashes_are_equal_is_a_contradiction(self):
        rec = self.record(False, H64)
        rec["images"][0]["source"].update(identity="differs")
        with self.assertRaises(ValueError):
            vast_report.validate_propimage_run(rec)

    def test_a_source_with_identity_none_cannot_carry_a_pin(self):
        rec = self.record(False, H64)
        rec["images"][0]["source"].update(identity="none")
        with self.assertRaises(ValueError):
            vast_report.validate_propimage_run(rec)

    def test_a_match_with_no_computed_hash_is_a_contradiction(self):
        rec = self.record(False, H64)
        rec["images"][0]["source"].update(sha256=None)
        with self.assertRaises(ValueError):
            vast_report.validate_propimage_run(rec)

    def test_a_differing_pin_without_a_computed_hash_is_a_contradiction(self):
        rec = self.record(False, OTHER)
        rec["images"][0]["source"].update(sha256=None)
        with self.assertRaises(ValueError):
            vast_report.validate_propimage_run(rec)

    def test_a_pin_less_source_says_none(self):
        rec = self.record(False, H64)
        rec["images"][0]["source"].update(pin_sha256=None, identity="differs")
        with self.assertRaises(ValueError):
            vast_report.validate_propimage_run(rec)


class SupplyChainStays(unittest.TestCase):
    """Pins of what the repository itself downloads hold in every mode: the generic runner never lets a spec weaken them."""

    def test_the_downloaded_tools_keep_their_pins_whatever_the_mode(self):
        pins = pa.load_pins()
        for role in ("libtpms", "swtpm", "gop"):
            item = next(i for i in pins["items"] if i["role"] == role)
            self.assertRegex(item["sha256"], "^[0-9a-f]{64}$")
            self.assertTrue(item["size"])
        unpinned = json.loads(json.dumps(pins))
        next(i for i in unpinned["items"] if i["role"] == "swtpm")["sha256"] = None
        with self.assertRaises(pa.Refused) as c:
            pa.build_manifest(unpinned, True)
        self.assertEqual(c.exception.reason, "PIN_UNSET:swtpm")

    def test_the_downloader_check_of_a_pin_object_is_not_bypassed_standalone(self):
        env = T.Env(self)
        env.runner.calls.clear()
        env.run(spec({"kind": "prebuilt", "disk": {"from": "pin", "ref": "propimage-kfimg-20261112.zip"}, "apps": APPS}, False))
        verbs = [c[4] for c in env.runner.calls if len(c) > 4 and str(c[3]).endswith("box_downloader.py")]
        self.assertEqual(verbs, ["check", "fetch", "verify"])


if __name__ == "__main__":
    unittest.main()
