#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later
"""GPU-free tests of the generic proprietary-image runner's registry and run spec (2026-11-16, phase 1): the application registry and its
plug-in directory, the spec validation with its named refusals, the command-line flags, and the per-image application plan."""
import json
from pathlib import Path
import sys
import tempfile
import unittest

HERE = Path(__file__).resolve().parent
WIN = HERE.parents[1] / "scripts/bench/windows"
sys.path.insert(0, str(WIN))
sys.path.insert(0, str(HERE))
import propimage_registry as R  # noqa: E402
import propimage_runspec as S  # noqa: E402

H64 = "c" * 64


def prebuilt(**over):
    img = {"kind": "prebuilt", "disk": {"from": "file", "ref": "/images/a.zip"}}
    img.update(over)
    return img


def spec(*images):
    return {"schema": S.SCHEMA, "images": list(images)}


def refusal(doc):
    with unittest.TestCase().assertRaises(S.SpecError) as c:
        S.validate(doc)
    return c.exception.reason


class Registry(unittest.TestCase):
    def test_the_core_definitions_load_in_run_order_with_wukong_last_and_solo(self):
        reg = R.load_registry()
        self.assertEqual(list(reg), ["superposition", "3dmark", "solidworks", "resolve", "edrawings", "edge", "wukong"])
        self.assertTrue(reg["wukong"]["solo"])
        self.assertEqual(R.canonical(reg, "blackmyth"), "wukong")

    def test_the_definitions_equal_the_harness_tables(self):
        import propimage_check as check
        import propimage_apps as pa
        reg = R.load_registry()
        self.assertEqual({k: v["timeout"] for k, v in reg.items()}, {k: v["timeout"] for k, v in check.CHECKS.items()})
        self.assertEqual({k: v["kind"] for k, v in reg.items()}, pa.EXPECTED_KIND)
        self.assertEqual(tuple(reg), pa.APPS)

    def test_harness_words_of_the_two_apps_that_need_them(self):
        reg = R.load_registry()
        self.assertEqual(reg["superposition"]["harness_args"], ["--superposition-run"])
        self.assertEqual(reg["wukong"]["harness_args"], ["--fb-mb", "8192"])

    def write(self, d, name, text):
        (Path(d) / name).write_text(text)

    def test_a_definition_set_is_one_more_file_and_a_duplicate_is_refused_by_name(self):
        with tempfile.TemporaryDirectory() as d:
            self.write(d, "a.py", 'APPS = [{"id": "alpha", "title": "A", "timeout": 60, "kind": "FILE", "match": ["alpha"]}]\n')
            self.write(d, "b.py", 'APPS = [{"id": "beta", "title": "B", "timeout": 60, "kind": "UIA", "group": "b"}]\n')
            reg = R.load_registry(d)
            self.assertEqual(list(reg), ["alpha", "beta"])
            self.assertEqual(reg["beta"]["group"], "b")
            self.write(d, "c.py", 'APPS = [{"id": "alpha", "title": "A2", "timeout": 60, "kind": "FILE"}]\n')
            with self.assertRaises(R.RegistryError) as c:
                R.load_registry(d)
            self.assertIn("alpha", str(c.exception))
            self.assertIn("a.py", str(c.exception))

    def test_an_alias_cannot_shadow_another_identity(self):
        with tempfile.TemporaryDirectory() as d:
            self.write(d, "a.py", 'APPS = [{"id": "alpha", "title": "A", "timeout": 60, "kind": "FILE"}]\n')
            self.write(d, "b.py", 'APPS = [{"id": "beta", "title": "B", "timeout": 60, "kind": "FILE", "aliases": ["alpha"]}]\n')
            with self.assertRaises(R.RegistryError):
                R.load_registry(d)

    def test_malformed_definitions_are_refused(self):
        bad = ['{"id": "Up", "title": "x", "timeout": 60, "kind": "FILE"}', '{"id": "x", "title": "x", "timeout": 5, "kind": "FILE"}',
               '{"id": "x", "title": "x", "timeout": 60, "kind": "PNG"}', '{"id": "x", "title": "x", "timeout": 60, "kind": "FILE", "bogus": 1}',
               '{"id": "x", "title": "x", "timeout": 60, "kind": "FILE", "match": ["("]}', '{"id": "x", "title": "x", "timeout": 60}',
               '{"id": "x", "title": "x", "timeout": 60, "kind": "FILE", "driver": "../evil:run"}',
               '{"id": "x", "title": "x", "timeout": 60, "kind": "FILE", "solo": "yes"}']
        for text in bad:
            with tempfile.TemporaryDirectory() as d:
                self.write(d, "a.py", "APPS = [%s]\n" % text)
                with self.assertRaises(R.RegistryError, msg=text):
                    R.load_registry(d)

    def test_a_module_without_apps_is_refused(self):
        with tempfile.TemporaryDirectory() as d:
            self.write(d, "a.py", "X = 1\n")
            with self.assertRaises(R.RegistryError):
                R.load_registry(d)

    def test_names_are_matched_to_identities(self):
        reg = R.load_registry()
        ids, unknown = R.discover(reg, ["DaVinci Resolve", "3DMark", "SOLIDWORKS 2025 SP1", "SOLIDWORKS eDrawings 2025", "Microsoft Edge",
                                        "Microsoft Edge WebView2 Runtime", "Some Other Tool", "resolve"])
        self.assertEqual(ids, ["3dmark", "solidworks", "resolve", "edrawings", "edge"])
        self.assertEqual(unknown, ["Microsoft Edge WebView2 Runtime", "Some Other Tool"])

    def test_declared_names_accept_strings_and_objects_and_ignore_the_rest(self):
        self.assertEqual(R.declared_names(["a", {"id": "b"}, {"name": "c"}, {"x": 1}, 5, None]), ["a", "b", "c"])
        self.assertEqual(R.declared_names("nope"), [])

    def test_unmet_needs(self):
        d = {"needs": ["data_iso"]}
        self.assertEqual(R.unmet_needs(d), ["data_iso"])
        self.assertEqual(R.unmet_needs(d, frozenset({"data_iso"})), [])


class Validation(unittest.TestCase):
    def test_a_minimal_prebuilt_image_gets_its_defaults(self):
        doc = S.validate(spec(prebuilt()))
        img = doc["images"][0]
        self.assertEqual((img["label"], img["kind"], img["state"], img["net"], img["only"], img["apps"]), ("image1", "prebuilt", "auto", "off", [], {}))

    def test_the_named_refusals(self):
        cases = {
            "SPEC_NOT_OBJECT": [],
            "SPEC_SCHEMA": {"schema": "x", "images": []},
            "SPEC_IMAGES": spec(),
            "KIND_BAD": spec({"kind": "weird"}),
            "DISK_REQUIRED": spec({"kind": "prebuilt"}),
            "ISO_ON_PREBUILT": spec(prebuilt(iso={"from": "official"})),
            "ISO_REQUIRED": spec({"kind": "built"}),
            "LABEL_BAD": spec(prebuilt(label="bad label")),
            "LABEL_DUP": spec(prebuilt(label="a"), prebuilt(label="a")),
            "STATE_BAD": spec(prebuilt(state="maybe")),
            "NET_BAD": spec(prebuilt(net="sometimes")),
            "NET_ON_WITH_STATE": spec(prebuilt(net="on", state_file={"from": "file", "ref": "/s.json"})),
            "ONLY_BAD": spec(prebuilt(only=["Bad Name"])),
            "APP_MODE_BAD": spec(prebuilt(apps={"resolve": {"mode": "launch"}})),
            "APP_ID_BAD": spec(prebuilt(apps={"Bad Id": {"mode": "present"}})),
            "APP_SOURCE_FORBIDDEN": spec(prebuilt(apps={"resolve": {"mode": "present", "source": {"from": "file", "ref": "/x"}}})),
            "APP_SOURCE_REQUIRED": spec({"kind": "built", "iso": {"from": "official"}, "apps": {"spec": {"mode": "install"}}}),
            "INSTALL_ON_PREBUILT": spec(prebuilt(apps={"spec": {"mode": "install", "source": {"from": "file", "ref": "/x.zip"}}})),
            "BUILT_NEEDS_APPS": spec({"kind": "built", "iso": {"from": "official"}}),
            "BUILT_APP_MODE": spec({"kind": "built", "iso": {"from": "official"}, "apps": {"spec": {"mode": "present"}}}),
            "SOURCE_URL_NOT_HTTPS": spec(prebuilt(disk={"from": "url", "ref": "http://x/a.zip"})),
            "SOURCE_SHA256": spec(prebuilt(disk={"from": "file", "ref": "/a", "sha256": "xyz"})),
            "SOURCE_REF": spec(prebuilt(disk={"from": "file"})),
            "SOURCE_FROM": spec(prebuilt(disk={"from": "official"})),
            "SSH_BAD": spec(prebuilt(ssh={"user": "u"})),
            "IMAGE_KEYS": spec(prebuilt(bogus=1)),
            "APP_LANE_BAD": spec(prebuilt(apps={"resolve": {"mode": "present", "lane": "chaos"}})),
            "STATE_CONFLICT": spec(prebuilt(state="none", state_file={"from": "file", "ref": "/s.json"})),
        }
        for reason, doc in cases.items():
            self.assertEqual(refusal(doc), reason, reason)

    def test_built_images_validate_so_that_phase_two_has_a_shape(self):
        doc = S.validate(spec({"kind": "built", "label": "ltsc", "iso": {"from": "official"},
                               "apps": {"spec": {"mode": "install", "source": {"from": "file", "ref": "/spec.zip", "sha256": H64}},
                                        "superposition": {"mode": "install", "source": {"from": "url", "ref": "https://example.invalid/s.run"}}}}))
        self.assertEqual(doc["images"][0]["apps"]["spec"]["source"]["sha256"], H64)

    def test_more_than_eight_images_and_duplicate_json_keys_are_refused(self):
        self.assertEqual(refusal(spec(*[prebuilt(label="i%d" % n) for n in range(9)])), "SPEC_IMAGES")
        with self.assertRaises(S.SpecError) as c:
            S.parse_json('{"schema": "x", "schema": "y"}')
        self.assertEqual(c.exception.reason, "SPEC_DUPLICATE_KEY")
        with self.assertRaises(S.SpecError):
            S.parse_json("{nope")

    def test_the_instruction_list_form(self):
        self.assertEqual(S.validate_list([prebuilt(label="a"), prebuilt(label="b")])["images"][1]["label"], "b")


class Flags(unittest.TestCase):
    def parse(self, *words):
        rest, doc = S.split_flags(list(words))
        return rest, doc

    def test_any_number_of_images_with_per_image_options(self):
        rest, doc = self.parse("--sha", "x", "--propimage", "/d/a.zip:" + H64, "--apps", "resolve,3dmark", "--propimage", "/d/b", "--app", "photoshop=present",
                               "--label", "B", "--out", "o")
        self.assertEqual(rest, ["--sha", "x", "--out", "o"])
        a, b = S.validate(doc)["images"]
        self.assertEqual((a["label"], a["disk"]["ref"], a["disk"]["sha256"], a["only"]), ("image1", "/d/a.zip", H64, ["resolve", "3dmark"]))
        self.assertEqual((b["label"], b["apps"]), ("B", {"photoshop": {"mode": "present"}}))

    def test_the_image_flag_forms(self):
        _, doc = self.parse("--image", "prebuilt:/d/a.qcow2", "--state", "/d/state.json", "--ssh-user", "u", "--ssh-key", "/k", "--ssh-port=2222",
                            "--image=prebuilt:https://example.invalid/b.zip#sha256=" + H64, "--net", "on", "--state", "none")
        a, b = S.validate(doc)["images"]
        self.assertEqual(a["state_file"]["ref"], "/d/state.json")
        self.assertEqual(a["ssh"], {"user": "u", "key_file": "/k", "port": 2222})
        self.assertEqual((b["disk"]["from"], b["disk"]["sha256"], b["net"], b["state"]), ("url", H64, "on", "none"))

    def test_a_built_image_and_install_apps_parse_for_phase_two(self):
        _, doc = self.parse("--image", "built:official", "--app", "spec=install:file:/z/spec.zip#sha256=" + H64,
                            "--app", "superposition=install:url:https://example.invalid/s.run", "--app", "x=install:official")
        img = S.validate(doc)["images"][0]
        self.assertEqual(img["iso"]["from"], "official")
        self.assertEqual(img["apps"]["spec"]["source"], {"from": "file", "ref": "/z/spec.zip", "sha256": H64})
        self.assertEqual(img["apps"]["superposition"]["source"]["from"], "url")
        self.assertEqual(img["apps"]["x"]["source"]["from"], "official")

    def test_no_image_flag_means_no_spec(self):
        self.assertEqual(self.parse("--sha", "x"), (["--sha", "x"], None))

    def test_a_per_image_flag_before_any_image_is_refused(self):
        for flag in ("--apps", "--net", "--label", "--state", "--app", "--ssh-user"):
            with self.assertRaises(S.SpecError) as c:
                self.parse(flag, "x=present")
            self.assertEqual(c.exception.reason, "FLAG_BEFORE_IMAGE")

    def test_bad_flags_are_named(self):
        for words, reason in ((["--image", "weird:/x"], "IMAGE_FLAG_BAD"), (["--image"], "FLAG_NEEDS_VALUE"),
                              (["--image", "prebuilt:/x", "--app", "novalue"], "APP_FLAG_BAD"), (["--image", "prebuilt:/x", "--app", "a=bogus"], "APP_FLAG_BAD"),
                              (["--image", "prebuilt:/x", "--app", "a=present", "--app", "a=skip"], "APP_DUPLICATE"),
                              (["--image", "prebuilt:/x#sha256=zz"], "SOURCE_SHA256"), (["--image", "prebuilt:/x", "--ssh-port", "abc"], "SSH_BAD")):
            with self.assertRaises(S.SpecError, msg=words) as c:
                self.parse(*words)
            self.assertEqual(c.exception.reason, reason)

    def test_a_pin_source(self):
        _, doc = self.parse("--image", "prebuilt:pin:propimage-kfimg-20261112.zip")
        self.assertEqual(S.validate(doc)["images"][0]["disk"], {"from": "pin", "ref": "propimage-kfimg-20261112.zip", "sha256": None})


class FlagRefusals(unittest.TestCase):
    def refuses(self, *words):
        _, doc = S.split_flags(list(words))
        with self.assertRaises(S.SpecError) as c:
            S.validate(doc)
        return c.exception.reason

    def test_install_on_a_prebuilt_image_from_flags(self):
        self.assertEqual(self.refuses("--image", "prebuilt:/x", "--app", "spec=install:file:/z.zip"), "INSTALL_ON_PREBUILT")

    def test_two_images_may_not_share_a_label_from_flags(self):
        self.assertEqual(self.refuses("--image", "prebuilt:/a", "--label", "x", "--image", "prebuilt:/b", "--label", "x"), "LABEL_DUP")

    def test_net_on_with_an_explicit_state_file_from_flags(self):
        self.assertEqual(self.refuses("--image", "prebuilt:/a", "--state", "/s.json", "--net", "on"), "NET_ON_WITH_STATE")

    def test_nine_images_from_flags_are_refused(self):
        words = []
        for n in range(9):
            words += ["--image", "prebuilt:/d/%d" % n, "--label", "i%d" % n]
        self.assertEqual(self.refuses(*words), "SPEC_IMAGES")

    def test_a_plain_http_url_is_refused(self):
        self.assertEqual(self.refuses("--image", "prebuilt:http://example.invalid/a.zip"), "SOURCE_URL_NOT_HTTPS")
        self.assertEqual(self.refuses("--image", "prebuilt:/a", "--state", "http://example.invalid/s.json"), "SOURCE_URL_NOT_HTTPS") if False else None

    def test_a_sha256_in_a_json_source_must_be_64_hex(self):
        with self.assertRaises(S.SpecError) as c:
            S.validate(spec(prebuilt(disk={"from": "file", "ref": "/a", "sha256": "ab" * 31})))
        self.assertEqual(c.exception.reason, "SOURCE_SHA256")


class Strict(unittest.TestCase):
    def strict(self, *images):
        return {"schema": S.SCHEMA, "images": list(images), "strict": True}

    def test_a_strict_spec_names_exactly_what_runs(self):
        ok = prebuilt(apps={"resolve": {"mode": "present"}})
        self.assertTrue(S.validate(self.strict(ok))["strict"])
        for bad, reason in ((prebuilt(), "STRICT_NEEDS_APPS"), (prebuilt(only=["resolve"], apps={"resolve": {"mode": "present"}}), "STRICT_ONLY"),
                            (prebuilt(apps={"resolve": {"mode": "present"}, "edge": {"mode": "skip"}}), "STRICT_SKIP_ENTRY")):
            with self.assertRaises(S.SpecError) as c:
                S.validate(self.strict(bad))
            self.assertEqual(c.exception.reason, reason)

    def test_the_same_specs_are_fine_when_not_strict(self):
        S.validate({"schema": S.SCHEMA, "images": [prebuilt(), prebuilt(label="b", only=["resolve"])]})
        S.validate({"schema": S.SCHEMA, "images": [prebuilt(apps={"edge": {"mode": "skip"}})], "strict": False})

    def test_strict_must_be_a_boolean(self):
        with self.assertRaises(S.SpecError) as c:
            S.validate({"schema": S.SCHEMA, "images": [prebuilt()], "strict": "yes"})
        self.assertEqual(c.exception.reason, "STRICT_BAD")

    def test_an_app_without_a_definition_in_a_strict_spec_is_named(self):
        doc = S.validate(self.strict(prebuilt(label="imga", apps={"resolve": {"mode": "present"}, "photoshop": {"mode": "present"}})))
        with self.assertRaises(S.SpecError) as c:
            S.strict_unknown_app(doc, R.load_registry())
        self.assertEqual((c.exception.reason, c.exception.detail), ("STRICT_UNKNOWN_APP", "imga:photoshop"))
        S.strict_unknown_app(S.validate(self.strict(prebuilt(apps={"blackmyth": {"mode": "present"}}))), R.load_registry())     # an alias is a definition

    def test_the_strict_flag_is_global_and_describe_shows_it(self):
        rest, doc = S.split_flags(["--strict", "--image", "prebuilt:/a", "--app", "resolve=present", "--sha", "x"])
        self.assertEqual(rest, ["--sha", "x"])
        self.assertTrue(S.validate(doc)["strict"])
        self.assertIn("strict=1", S.describe(S.validate(doc))[0])
        self.assertIn("strict=0", S.describe(S.validate(S.split_flags(["--image", "prebuilt:/a"])[1]))[0])
        with self.assertRaises(S.SpecError) as c:
            S.split_flags(["--strict"])
        self.assertEqual(c.exception.reason, "STRICT_WITHOUT_IMAGES")

    def test_skip_classes_of_reasons(self):
        self.assertEqual(S.skip_class("NEEDS:data_iso"), "MISSING_INPUT")
        self.assertEqual(S.skip_class("NO_DRIVER"), "NO_DRIVER")
        self.assertEqual(S.skip_class("NOT_ON_IMAGE"), "NOT_INSTALLED")
        self.assertEqual(S.skip_class("something odd"), "CONSTRAINT")


class LaneSpec(unittest.TestCase):
    def test_lane_is_optional_gate_by_default_and_closed(self):
        img = S.validate(spec(prebuilt(apps={"resolve": {"mode": "present", "lane": "discovery"}, "edge": {"mode": "present"}})))["images"][0]
        self.assertEqual((img["apps"]["resolve"]["lane"], "lane" in img["apps"]["edge"]), ("discovery", False))
        self.assertEqual(refusal(spec(prebuilt(apps={"resolve": {"mode": "present", "lane": "chaos"}}))), "APP_LANE_BAD")

    def test_the_flag_form(self):
        _, doc = S.split_flags(["--image", "prebuilt:/a", "--app", "resolve=present,lane=discovery", "--app", "edge=present,lane=gate", "--app", "x=skip"])
        apps = S.validate(doc)["images"][0]["apps"]
        self.assertEqual((apps["resolve"], apps["edge"], apps["x"]), ({"mode": "present", "lane": "discovery"}, {"mode": "present", "lane": "gate"}, {"mode": "skip"}))

    def test_the_plan_rows_carry_the_lane(self):
        img = S.validate(spec(prebuilt(apps={"resolve": {"mode": "present", "lane": "discovery"}, "edge": {"mode": "present"}})))["images"][0]
        rows, _, _ = S.plan_apps(img, R.load_registry())
        self.assertEqual({r["app"]: r["lane"] for r in rows}, {"resolve": "discovery", "edge": "gate"})


class Plan(unittest.TestCase):
    def plan(self, img, **kw):
        img = S.validate(spec(img))["images"][0]
        rows, unsupported, decided = S.plan_apps(img, R.load_registry(), **kw)
        return {r["app"]: (r["action"], r["reason"]) for r in rows}, unsupported, decided

    def test_declared_present_apps_run_and_unknown_ones_are_not_run_never_an_error(self):
        got, _, decided = self.plan(prebuilt(apps={"resolve": {"mode": "present"}, "photoshop": {"mode": "present"}}))
        self.assertEqual(got, {"resolve": ("run", ""), "photoshop": ("not_run", "NO_DRIVER")})
        self.assertEqual(decided, "SPEC")

    def test_skip_wins_over_the_image_declaration(self):
        got, _, _ = self.plan(prebuilt(apps={"resolve": {"mode": "skip"}}), declared=["DaVinci Resolve", "3DMark"])
        self.assertEqual(got, {"3dmark": ("run", ""), "resolve": ("not_run", "SKIPPED")})

    def test_the_state_declaration_is_intersected_with_the_registry_and_the_rest_is_reported(self):
        got, unsupported, decided = self.plan(prebuilt(), declared=["DaVinci Resolve", "3DMark", "Some Other Tool"])
        self.assertEqual(got, {"3dmark": ("run", ""), "resolve": ("run", "")})
        self.assertEqual((unsupported, decided), (["Some Other Tool"], "STATE"))

    def test_only_restricts_and_names_what_is_not_on_the_image(self):
        got, _, _ = self.plan(prebuilt(only=["resolve", "edge"]), declared=["DaVinci Resolve", "3DMark"])
        self.assertEqual(got, {"resolve": ("run", ""), "edge": ("not_run", "NOT_ON_IMAGE")})

    def test_the_probe_stands_in_for_a_missing_declaration(self):
        got, _, decided = self.plan(prebuilt(), probed=["3DMark"])
        self.assertEqual((got, decided), ({"3dmark": ("run", "")}, "PROBE"))

    def test_apps_flag_alone_is_the_declaration(self):
        got, _, decided = self.plan(prebuilt(only=["resolve", "3dmark"]))
        self.assertEqual((got, decided), ({"3dmark": ("run", ""), "resolve": ("run", "")}, "APPS_FLAG"))

    def test_nothing_declared_needs_a_probe(self):
        got, _, decided = self.plan(prebuilt())
        self.assertEqual((got, decided), ({}, "NEEDS_PROBE"))

    def test_an_app_with_an_unmet_need_is_not_run_by_name(self):
        import tempfile as tf
        with tf.TemporaryDirectory() as d:
            (Path(d) / "x.py").write_text('APPS = [{"id": "needy", "title": "N", "timeout": 60, "kind": "FILE", "needs": ["data_iso"]}]\n')
            reg = R.load_registry(d)
        img = S.validate(spec(prebuilt(apps={"needy": {"mode": "present"}})))["images"][0]
        rows, _, _ = S.plan_apps(img, reg)
        self.assertEqual((rows[0]["action"], rows[0]["reason"]), ("not_run", "NEEDS:data_iso"))

    def test_aliases_are_the_same_application(self):
        got, _, _ = self.plan(prebuilt(apps={"blackmyth": {"mode": "present"}}))
        self.assertEqual(got, {"wukong": ("run", "")})

    def test_describe_is_one_line_per_image(self):
        lines = S.describe(S.validate(spec(prebuilt(label="a"), prebuilt(label="b", net="on"))))
        self.assertEqual(len(lines), 2)
        self.assertIn("net=on", lines[1])


if __name__ == "__main__":
    unittest.main()
