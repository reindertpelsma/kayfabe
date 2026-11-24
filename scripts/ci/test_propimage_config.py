#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later
"""GPU-free tests of scripts/ci/propimage_config.py (2026-11-19): the bundle schema and its validator, the resolver `bundles_for`, the CLI, and
the committed entries (ci/vast-broker/propimage-bundles.json) against what the stages and the Worker's propimages.json say today.
Mutants: propimage_config_mutants.py (two named killers each)."""
import copy
import importlib.util
import io
import json
import re
import sys
import tempfile
import unittest
from contextlib import redirect_stdout
from pathlib import Path

HERE = Path(__file__).resolve().parent
ROOT = HERE.parents[1]
sys.path.insert(0, str(HERE))
import propimage_config as pc  # noqa: E402

H64 = "ab" * 32


def doc():
    """A small valid configuration: two Windows bundles that overlap on one app, one Linux bundle."""
    return {
        "schema": pc.SCHEMA,
        "bundles": {
            "w1": {"os": "windows", "r2": "win/one.zip", "size_bytes": 100, "apps": ["alpha", "beta"]},
            "w2": {"os": "windows", "r2": "win/two.zip", "size_bytes": 200, "apps": ["beta", "gamma"]},
            "l1": {"os": "linux", "r2": "lin/one.zip", "size_bytes": 50, "apps": ["delta"]},
        },
    }


def windows_run():
    return {"rows_file": "scripts/bench/windows/table.py", "pins_file": "scripts/ci/pins.json"}


def with_bundle(**changes):
    d = doc()
    d["bundles"]["w1"].update(changes)
    return d


def with_run(**changes):
    run = windows_run()
    run.update(changes)
    return with_bundle(run=run)


def reason(fn, *args, **kwargs):
    try:
        fn(*args, **kwargs)
    except pc.ConfigError as exc:
        return exc.reason
    return None


def cli(*argv):
    out = io.StringIO()
    with redirect_stdout(out):
        rc = pc.main(list(argv))
    return rc, out.getvalue()


def cli_file(document, *argv):
    with tempfile.TemporaryDirectory() as tmp:
        path = Path(tmp) / "bundles.json"
        path.write_text(document if type(document) is str else json.dumps(document))
        return cli("--config", str(path), *argv)


class Schema(unittest.TestCase):
    def test_small_valid_document_is_accepted_and_returned(self):
        d = doc()
        self.assertIs(pc.validate(d), d)

    def test_wrong_schema_is_refused(self):
        d = doc()
        d["schema"] = "kayfabe.propimage-bundles/2"
        self.assertEqual(reason(pc.validate, d), "SCHEMA")

    def test_missing_schema_and_non_object_are_refused(self):
        d = doc()
        del d["schema"]
        self.assertEqual(reason(pc.validate, d), "SCHEMA")
        self.assertEqual(reason(pc.validate, []), "SCHEMA")

    def test_unknown_top_level_key_is_refused(self):
        d = doc()
        d["colour"] = 1
        self.assertEqual(reason(pc.validate, d), "TOP_KEY")

    def test_missing_bundles_is_refused(self):
        d = doc()
        del d["bundles"]
        self.assertEqual(reason(pc.validate, d), "TOP_KEY")

    def test_empty_bundle_table_is_refused(self):
        d = doc()
        d["bundles"] = {}
        self.assertEqual(reason(pc.validate, d), "BUNDLES")

    def test_too_many_bundles_are_refused(self):
        d = doc()
        d["bundles"] = {"b%d" % i: {"os": "linux", "r2": "k/%d.zip" % i, "size_bytes": 1, "apps": ["a%d" % i]} for i in range(pc.MAX_BUNDLES + 1)}
        self.assertEqual(reason(pc.validate, d), "BUNDLES")
        d["bundles"] = {"b%d" % i: {"os": "linux", "r2": "k/%d.zip" % i, "size_bytes": 1, "apps": ["a%d" % i]} for i in range(pc.MAX_BUNDLES)}
        pc.validate(d)

    def test_sixteen_bundles_are_the_limit(self):
        d = doc()
        d["bundles"] = {"b%d" % i: {"os": "linux", "r2": "k/%d.zip" % i, "size_bytes": 1, "apps": ["a%d" % i]} for i in range(pc.MAX_BUNDLES)}
        pc.validate(d)

    def test_status_of_4000_characters_is_the_limit(self):
        d = doc()
        d["status"] = "x" * 4000
        pc.validate(d)

    def test_sixty_four_apps_are_the_limit(self):
        pc.validate(with_bundle(apps=["a%d" % i for i in range(pc.MAX_APPS)]))

    def test_sixteen_words_are_the_limit(self):
        pc.validate(with_run(harness_args={"before_apps": ["w"] * 16}))

    def test_status_must_be_a_bounded_string(self):
        d = doc()
        d["status"] = 5
        self.assertEqual(reason(pc.validate, d), "STATUS")
        d["status"] = "x" * 4001
        self.assertEqual(reason(pc.validate, d), "STATUS")
        d["status"] = "STATUS: fine"
        pc.validate(d)

    def test_bundle_id_is_a_lower_case_token(self):
        for bad in ("A", "has space", "", "x" * 33, "-x"):
            d = doc()
            d["bundles"][bad] = d["bundles"].pop("w1")
            self.assertEqual(reason(pc.validate, d), "BUNDLE_ID:" + bad[:32], bad)

    def test_bundle_must_be_an_object(self):
        d = doc()
        d["bundles"]["w1"] = ["os"]
        self.assertEqual(reason(pc.validate, d), "BUNDLE_SHAPE:w1")

    def test_unknown_bundle_key_is_refused(self):
        self.assertEqual(reason(pc.validate, with_bundle(slot=1)), "BUNDLE_KEY:w1")

    def test_every_required_bundle_key_is_required(self):
        for key in pc.BUNDLE_REQUIRED:
            d = doc()
            del d["bundles"]["w1"][key]
            self.assertEqual(reason(pc.validate, d), "BUNDLE_MISSING:w1", key)

    def test_os_is_windows_or_linux(self):
        for bad in ("macos", "Windows", None, 1):
            self.assertEqual(reason(pc.validate, with_bundle(os=bad)), "BUNDLE_OS:w1", bad)

    def test_object_key_shapes(self):
        for bad in ("", "/abs.zip", "a b.zip", "a//b.zip", "a/b/", "a\\b.zip", 5, "x" * 201):
            self.assertEqual(reason(pc.validate, with_bundle(r2=bad)), "BUNDLE_R2:w1", repr(bad))

    def test_an_absolute_object_key_is_refused(self):
        self.assertEqual(reason(pc.validate, with_bundle(r2="/abs/image.zip")), "BUNDLE_R2:w1")

    def test_object_key_with_a_double_slash_is_refused(self):
        self.assertEqual(reason(pc.validate, with_bundle(r2="a//b.zip")), "BUNDLE_R2:w1")

    def test_object_key_with_a_trailing_slash_is_refused(self):
        self.assertEqual(reason(pc.validate, with_bundle(r2="a/b/")), "BUNDLE_R2:w1")

    def test_a_double_slash_in_a_path_is_refused(self):
        self.assertEqual(reason(pc.validate, with_run(rows_file="scripts//x.py")), "RUN_ROWS_FILE:w1")

    def test_object_key_may_not_climb(self):
        self.assertEqual(reason(pc.validate, with_bundle(r2="win/../other.zip")), "BUNDLE_R2:w1")

    def test_object_key_climb_in_the_last_segment_is_refused(self):
        self.assertEqual(reason(pc.validate, with_bundle(r2="a/b/..")), "BUNDLE_R2:w1")

    def test_object_key_with_dots_inside_a_name_is_fine(self):
        pc.validate(with_bundle(r2="win/a..b/image-v2.zip"))

    def test_size_is_a_positive_bounded_integer(self):
        for bad in (0, -1, True, 1.5, "10", pc.MAX_SIZE):
            self.assertEqual(reason(pc.validate, with_bundle(size_bytes=bad)), "BUNDLE_SIZE:w1", repr(bad))
        pc.validate(with_bundle(size_bytes=1))
        pc.validate(with_bundle(size_bytes=pc.MAX_SIZE - 1))

    def test_apps_are_a_non_empty_list_of_unique_tokens(self):
        for bad in ([], "alpha", ["alpha", "alpha"], ["Alpha"], ["a b"], [1], ["x" * 33], ["a%d" % i for i in range(pc.MAX_APPS + 1)]):
            self.assertEqual(reason(pc.validate, with_bundle(apps=bad)), "BUNDLE_APPS:w1", repr(bad)[:40])

    def test_an_app_may_be_on_two_bundles(self):
        pc.validate(doc())      # beta is on w1 and w2

    def test_sha256_is_64_lower_case_hex(self):
        pc.validate(with_bundle(sha256=H64))
        for bad in (H64.upper(), H64[:-1], "g" * 64, 5):
            self.assertEqual(reason(pc.validate, with_bundle(sha256=bad)), "BUNDLE_SHA256:w1", repr(bad)[:20])

    def test_ref_is_a_pins_name(self):
        pc.validate(with_bundle(ref="propimage-kfimg-1.zip"))
        for bad in ("", "a/b", "a b", 7):
            self.assertEqual(reason(pc.validate, with_bundle(ref=bad)), "BUNDLE_REF:w1", repr(bad))

    def test_duplicate_object_keys_are_refused(self):
        d = doc()
        d["bundles"]["w2"]["r2"] = d["bundles"]["w1"]["r2"]
        self.assertEqual(reason(pc.validate, d), "DUPLICATE_R2")

    def test_duplicate_object_keys_across_platforms_are_refused(self):
        d = doc()
        d["bundles"]["l1"]["r2"] = d["bundles"]["w2"]["r2"]
        self.assertEqual(reason(pc.validate, d), "DUPLICATE_R2")

    def test_gpu_families_are_known_and_unique(self):
        pc.validate(with_bundle(needs_gpu_families=["ada", "ampere"]))
        for bad in ([], ["ada", "ada"], ["pascal"], "ada"):
            self.assertEqual(reason(pc.validate, with_bundle(needs_gpu_families=bad)), "BUNDLE_FAMILIES:w1", repr(bad))


class NoSecrets(unittest.TestCase):
    """The notes are free text, so the validator refuses the shapes of what must never be in the repository."""

    def test_a_plain_note_is_accepted(self):
        pc.validate(with_bundle(licence_note="named-user licence; the token is not yet revoked", basis="measured 2026-11-19"))

    def test_licence_note_with_an_email_address_is_refused(self):
        self.assertEqual(reason(pc.validate, with_bundle(licence_note="owner is someone@example.org")), "BUNDLE_LICENCE_NOTE_SECRETISH:w1")

    def test_basis_with_an_email_address_is_refused(self):
        self.assertEqual(reason(pc.validate, with_bundle(basis="built by someone@example.org")), "BUNDLE_BASIS_SECRETISH:w1")

    def test_licence_note_with_a_key_sized_token_is_refused(self):
        self.assertEqual(reason(pc.validate, with_bundle(licence_note="key ABCDEF0123456789ABCDEF01")), "BUNDLE_LICENCE_NOTE_SECRETISH:w1")

    def test_basis_with_a_sha256_sized_token_is_refused(self):
        self.assertEqual(reason(pc.validate, with_bundle(basis="hash " + H64)), "BUNDLE_BASIS_SECRETISH:w1")

    def test_a_23_character_word_is_not_a_token(self):
        pc.validate(with_bundle(basis="a" * 23))

    def test_notes_are_printable_bounded_text(self):
        for key, name in (("licence_note", "BUNDLE_LICENCE_NOTE"), ("basis", "BUNDLE_BASIS")):
            for bad in ("", "x " * 151, "tab\there", 5):
                self.assertEqual(reason(pc.validate, with_bundle(**{key: bad})), name + ":w1", (key, repr(bad)[:20]))


class WindowsRun(unittest.TestCase):
    def test_minimal_run_is_accepted(self):
        pc.validate(with_run())

    def test_full_run_is_accepted(self):
        pc.validate(with_run(rows=["r1", "r2"], solo_rows=["r2"], data_iso=True, guest_ram_mb=16384, audio=True,
                             harness_args={"before_apps": ["--app-set", "b"], "after_apps": ["--fb-mb", "8192"], "per_row": {"r1": ["--x"]}}))

    def test_run_must_be_an_object(self):
        self.assertEqual(reason(pc.validate, with_bundle(run=[])), "RUN_SHAPE:w1")

    def test_unknown_run_key_is_refused(self):
        self.assertEqual(reason(pc.validate, with_run(app_set="a")), "RUN_KEY:w1")

    def test_linux_key_on_a_windows_run_is_refused(self):
        self.assertEqual(reason(pc.validate, with_run(isolation="row")), "RUN_KEY:w1")

    def test_rows_file_and_pins_file_are_required(self):
        for key in pc.WINDOWS_RUN_REQUIRED:
            run = windows_run()
            del run[key]
            self.assertEqual(reason(pc.validate, with_bundle(run=run)), "RUN_MISSING:w1", key)

    def test_paths_are_relative_and_do_not_climb(self):
        for key, name in (("rows_file", "RUN_ROWS_FILE:w1"), ("pins_file", "RUN_PINS_FILE:w1")):
            for bad in ("/etc/x.py", "a/../b.py", "a//b.py", "a/", "", "a b.py", 3):
                self.assertEqual(reason(pc.validate, with_run(**{key: bad})), name, (key, bad))

    def test_rows_are_unique_tokens(self):
        for bad in ([], ["a", "a"], ["A"], "a", [1]):
            self.assertEqual(reason(pc.validate, with_run(rows=bad)), "RUN_ROWS:w1", repr(bad))

    def test_solo_rows_are_unique_tokens(self):
        for bad in ([], ["a", "a"], ["A"], "a"):
            self.assertEqual(reason(pc.validate, with_run(solo_rows=bad)), "RUN_SOLO_ROWS:w1", repr(bad))

    def test_solo_rows_must_be_rows_when_rows_are_listed(self):
        self.assertEqual(reason(pc.validate, with_run(rows=["r1"], solo_rows=["r2"])), "RUN_SOLO_ROWS_UNKNOWN:w1")

    def test_solo_rows_are_free_when_the_table_decides_the_rows(self):
        pc.validate(with_run(solo_rows=["r2"]))

    def test_data_iso_and_audio_are_booleans(self):
        for key, name in (("data_iso", "RUN_DATA_ISO:w1"), ("audio", "RUN_AUDIO:w1")):
            for bad in (1, "yes", None):
                self.assertEqual(reason(pc.validate, with_run(**{key: bad})), name, (key, bad))

    def test_guest_ram_is_zero_or_a_whole_number_of_gib(self):
        pc.validate(with_run(guest_ram_mb=0))
        pc.validate(with_run(guest_ram_mb=1024))
        for bad in (-1024, 512, 1500, True, "16384", 1 << 21):
            self.assertEqual(reason(pc.validate, with_run(guest_ram_mb=bad)), "RUN_GUEST_RAM:w1", repr(bad))

    def test_harness_args_keys_are_fixed(self):
        self.assertEqual(reason(pc.validate, with_run(harness_args={"middle": []})), "ARGS_KEY:w1")
        self.assertEqual(reason(pc.validate, with_run(harness_args=[])), "ARGS_KEY:w1")

    def test_harness_words_are_short_plain_tokens(self):
        for where in ("before_apps", "after_apps"):
            for bad in (["a b"], ["$(x)"], ["x" * 65], [""], [1], "x", ["w"] * 17):
                self.assertEqual(reason(pc.validate, with_run(harness_args={where: bad})), "ARGS:w1:" + where, (where, repr(bad)[:20]))

    def test_per_row_words_are_checked_like_the_others(self):
        self.assertEqual(reason(pc.validate, with_run(harness_args={"per_row": {"r1": ["a;b"]}})), "ARGS:w1:per_row:r1")
        self.assertEqual(reason(pc.validate, with_run(harness_args={"per_row": []})), "ARGS_PER_ROW:w1")
        self.assertEqual(reason(pc.validate, with_run(harness_args={"per_row": {"R1": []}})), "ARGS_PER_ROW_NAME:w1")

    def test_per_row_names_must_be_rows_when_rows_are_listed(self):
        self.assertEqual(reason(pc.validate, with_run(rows=["r1"], harness_args={"per_row": {"r2": ["--x"]}})), "ARGS_PER_ROW_UNKNOWN:w1:r2")

    def test_the_data_volume_flag_is_set_by_the_field_not_by_words(self):
        self.assertEqual(reason(pc.validate, with_run(data_iso=True, harness_args={"before_apps": ["--data-iso"]})), "ARGS_DATA_ISO_SET_BY_FIELD:w1")
        self.assertEqual(reason(pc.validate, with_run(data_iso=True, harness_args={"per_row": {"r1": ["--data-iso"]}})), "ARGS_DATA_ISO_SET_BY_FIELD:w1")


class LinuxRun(unittest.TestCase):
    def linux(self, **changes):
        run = {"rows_file": "scripts/bench/linux/rows.py", "pins_file": "scripts/ci/pins.json", "isolation": "row"}
        run.update(changes)
        d = doc()
        d["bundles"]["l1"]["run"] = run
        return d

    def test_linux_run_is_accepted(self):
        pc.validate(self.linux())
        pc.validate(self.linux(isolation="shared"))

    def test_isolation_is_row_or_shared(self):
        self.assertEqual(reason(pc.validate, self.linux(isolation="vm")), "RUN_ISOLATION:l1")

    def test_all_three_linux_keys_are_required(self):
        for key in pc.LINUX_RUN_REQUIRED:
            d = self.linux()
            del d["bundles"]["l1"]["run"][key]
            self.assertEqual(reason(pc.validate, d), "RUN_MISSING:l1", key)

    def test_windows_keys_on_a_linux_run_are_refused(self):
        self.assertEqual(reason(pc.validate, self.linux(audio=True)), "RUN_KEY:l1")
        self.assertEqual(reason(pc.validate, self.linux(data_iso=True)), "RUN_KEY:l1")

    def test_linux_paths_are_checked(self):
        self.assertEqual(reason(pc.validate, self.linux(rows_file="../x.py")), "RUN_ROWS_FILE:l1")
        self.assertEqual(reason(pc.validate, self.linux(pins_file="/x.json")), "RUN_PINS_FILE:l1")


class Parse(unittest.TestCase):
    def test_parse_returns_the_document(self):
        self.assertEqual(pc.parse(json.dumps(doc())), doc())

    def test_invalid_json_is_a_named_refusal(self):
        self.assertEqual(reason(pc.parse, "{"), "JSON")

    def test_duplicate_object_key_in_the_text_is_refused(self):
        text = json.dumps(doc()).replace('"w1"', '"w2"', 1)
        self.assertEqual(reason(pc.parse, text), "DUPLICATE_KEY:w2")

    def test_duplicate_key_inside_a_bundle_is_refused(self):
        text = '{"schema":"%s","bundles":{"w":{"os":"linux","os":"windows","r2":"a.zip","size_bytes":1,"apps":["a"]}}}' % pc.SCHEMA
        self.assertEqual(reason(pc.parse, text), "DUPLICATE_KEY:os")

    def test_parse_validates(self):
        self.assertEqual(reason(pc.parse, json.dumps(with_bundle(os="bsd"))), "BUNDLE_OS:w1")


class Resolver(unittest.TestCase):
    def test_one_app_resolves_to_its_bundle(self):
        self.assertEqual(pc.bundles_for(doc(), ["alpha"]), ["w1"])
        self.assertEqual(pc.bundles_for(doc(), ["gamma"]), ["w2"])

    def test_apps_on_one_bundle_need_only_that_bundle(self):
        self.assertEqual(pc.bundles_for(doc(), ["alpha", "beta"]), ["w1"])

    def test_apps_on_two_bundles_need_both(self):
        self.assertEqual(pc.bundles_for(doc(), ["alpha", "gamma"]), ["w1", "w2"])

    def test_an_overlapping_app_goes_to_the_smaller_bundle(self):
        self.assertEqual(pc.bundles_for(doc(), ["beta"]), ["w1"])

    def test_unknown_app_is_refused_by_name(self):
        self.assertEqual(reason(pc.bundles_for, doc(), ["nope"]), "UNKNOWN_APP:nope")

    def test_unknown_app_among_known_ones_is_still_refused(self):
        self.assertEqual(reason(pc.bundles_for, doc(), ["alpha", "nope", "gamma"]), "UNKNOWN_APP:nope")

    def test_the_first_unknown_app_in_name_order_is_the_one_named(self):
        self.assertEqual(reason(pc.bundles_for, doc(), ["zeta", "eta"]), "UNKNOWN_APP:eta")

    def test_bad_app_name_is_refused_by_name(self):
        self.assertEqual(reason(pc.bundles_for, doc(), ["Alpha"]), "BAD_APP:Alpha")
        self.assertEqual(reason(pc.bundles_for, doc(), ["a b"]), "BAD_APP:a b")

    def test_request_shape_is_checked(self):
        self.assertEqual(reason(pc.bundles_for, doc(), "alpha"), "APPS_SHAPE")
        self.assertEqual(reason(pc.bundles_for, doc(), [1]), "APPS_SHAPE")
        self.assertEqual(reason(pc.bundles_for, doc(), None), "APPS_SHAPE")

    def test_empty_request_needs_no_bundle(self):
        self.assertEqual(pc.bundles_for(doc(), []), [])
        self.assertEqual(pc.bundles_for(doc(), set()), [])

    def test_result_is_sorted_by_bundle_id(self):
        d = doc()
        d["bundles"]["a0"] = {"os": "linux", "r2": "lin/zero.zip", "size_bytes": 5, "apps": ["omega"]}
        self.assertEqual(pc.bundles_for(d, ["omega", "gamma", "alpha"]), ["a0", "w1", "w2"])

    def test_request_order_and_repeats_do_not_change_the_result(self):
        want = pc.bundles_for(doc(), ["alpha", "gamma"])
        self.assertEqual(pc.bundles_for(doc(), ["gamma", "alpha", "gamma", "alpha"]), want)
        self.assertEqual(pc.bundles_for(doc(), ("gamma", "alpha")), want)
        self.assertEqual(pc.bundles_for(doc(), {"alpha", "gamma"}), want)

    def test_fewest_bytes_beat_fewest_bundles(self):
        d = doc()
        d["bundles"]["huge"] = {"os": "windows", "r2": "win/huge.zip", "size_bytes": 10_000, "apps": ["alpha", "gamma"]}
        self.assertEqual(pc.bundles_for(d, ["alpha", "gamma"]), ["w1", "w2"])

    def test_one_bundle_wins_when_it_is_cheaper_than_two(self):
        d = doc()
        d["bundles"]["all"] = {"os": "windows", "r2": "win/all.zip", "size_bytes": 250, "apps": ["alpha", "gamma"]}
        self.assertEqual(pc.bundles_for(d, ["alpha", "gamma"]), ["all"])

    def test_equal_bytes_prefer_fewer_bundles(self):
        d = {"schema": pc.SCHEMA, "bundles": {
            "aa": {"os": "windows", "r2": "k/aa.zip", "size_bytes": 30, "apps": ["one"]},
            "bb": {"os": "windows", "r2": "k/bb.zip", "size_bytes": 30, "apps": ["two"]},
            "zz": {"os": "windows", "r2": "k/zz.zip", "size_bytes": 60, "apps": ["one", "two"]}}}
        self.assertEqual(pc.bundles_for(d, ["one", "two"]), ["zz"])

    def test_equal_bytes_and_bundles_prefer_the_smaller_ids(self):
        d = {"schema": pc.SCHEMA, "bundles": {
            "bb": {"os": "windows", "r2": "k/bb.zip", "size_bytes": 30, "apps": ["one"]},
            "aa": {"os": "windows", "r2": "k/aa.zip", "size_bytes": 30, "apps": ["one"]}}}
        self.assertEqual(pc.bundles_for(d, ["one"]), ["aa"])
        self.assertEqual(pc.bundles_for(d, ["one"]), pc.bundles_for(json.loads(json.dumps(d)), ["one"]))

    def test_declaration_order_of_the_bundles_does_not_matter(self):
        d = doc()
        reordered = {"schema": pc.SCHEMA, "bundles": dict(reversed(list(d["bundles"].items())))}
        for apps in (["alpha"], ["beta"], ["alpha", "gamma"], ["gamma", "beta"]):
            self.assertEqual(pc.bundles_for(d, apps), pc.bundles_for(reordered, apps), apps)

    def test_an_app_on_both_platforms_is_ambiguous_without_an_os(self):
        d = doc()
        d["bundles"]["l1"]["apps"].append("alpha")
        self.assertEqual(reason(pc.bundles_for, d, ["alpha"]), "AMBIGUOUS_APP:alpha")

    def test_the_os_argument_settles_an_ambiguous_app(self):
        d = doc()
        d["bundles"]["l1"]["apps"].append("alpha")
        self.assertEqual(pc.bundles_for(d, ["alpha"], os="linux"), ["l1"])
        self.assertEqual(pc.bundles_for(d, ["alpha"], os="windows"), ["w1"])

    def test_the_os_argument_hides_the_other_platform(self):
        self.assertEqual(reason(pc.bundles_for, doc(), ["delta"], os="windows"), "UNKNOWN_APP:delta")
        self.assertEqual(reason(pc.bundles_for, doc(), ["alpha"], os="linux"), "UNKNOWN_APP:alpha")

    def test_both_platforms_in_one_request(self):
        self.assertEqual(pc.bundles_for(doc(), ["alpha", "delta"]), ["l1", "w1"])

    def test_the_os_argument_is_checked(self):
        self.assertEqual(reason(pc.bundles_for, doc(), ["alpha"], os="macos"), "BUNDLE_OS:macos")

    def test_an_invalid_configuration_is_refused_before_resolving(self):
        d = doc()
        d["bundles"]["w1"]["os"] = "bsd"
        self.assertEqual(reason(pc.bundles_for, d, ["alpha"]), "BUNDLE_OS:w1")
        self.assertEqual(reason(pc.bundles_for, {}, []), "SCHEMA")

    def test_download_bytes_sums_the_named_bundles(self):
        self.assertEqual(pc.download_bytes(doc(), ["w1", "w2", "l1"]), 350)
        self.assertEqual(pc.download_bytes(doc(), []), 0)

    def test_the_unknown_app_named_does_not_depend_on_the_request_order(self):
        self.assertEqual(reason(pc.bundles_for, doc(), ["eta", "zeta"]), "UNKNOWN_APP:eta")
        self.assertEqual(reason(pc.bundles_for, doc(), ["zeta", "eta"]), "UNKNOWN_APP:eta")

    def test_a_bare_string_is_not_a_request(self):
        self.assertEqual(reason(pc.bundles_for, doc(), "alpha"), "APPS_SHAPE")

    def test_a_list_of_numbers_is_not_a_request(self):
        self.assertEqual(reason(pc.bundles_for, doc(), [1, 2]), "APPS_SHAPE")

    def test_an_unknown_os_is_refused_even_for_an_empty_request(self):
        self.assertEqual(reason(pc.bundles_for, doc(), [], os="macos"), "BUNDLE_OS:macos")

    def test_an_invalid_bundle_table_is_refused_even_for_an_empty_request(self):
        d = doc()
        d["bundles"]["w1"]["size_bytes"] = 0
        self.assertEqual(reason(pc.bundles_for, d, []), "BUNDLE_SIZE:w1")

    def test_the_cheaper_bundle_wins_even_with_a_later_id(self):
        d = {"schema": pc.SCHEMA, "bundles": {
            "aa": {"os": "windows", "r2": "k/aa.zip", "size_bytes": 500, "apps": ["one"]},
            "zz": {"os": "windows", "r2": "k/zz.zip", "size_bytes": 10, "apps": ["one"]}}}
        self.assertEqual(pc.bundles_for(d, ["one"]), ["zz"])

    def test_two_cheap_bundles_beat_one_dear_one_for_three_apps(self):
        d = {"schema": pc.SCHEMA, "bundles": {
            "all": {"os": "windows", "r2": "k/all.zip", "size_bytes": 1000, "apps": ["one", "two", "three"]},
            "b1": {"os": "windows", "r2": "k/b1.zip", "size_bytes": 10, "apps": ["one", "two"]},
            "b2": {"os": "windows", "r2": "k/b2.zip", "size_bytes": 10, "apps": ["three"]}}}
        self.assertEqual(pc.bundles_for(d, ["one", "two", "three"]), ["b1", "b2"])

    def test_equal_bytes_prefer_fewer_bundles_for_three_apps(self):
        d = {"schema": pc.SCHEMA, "bundles": {
            "a1": {"os": "windows", "r2": "k/a1.zip", "size_bytes": 10, "apps": ["one"]},
            "a2": {"os": "windows", "r2": "k/a2.zip", "size_bytes": 10, "apps": ["two"]},
            "a3": {"os": "windows", "r2": "k/a3.zip", "size_bytes": 10, "apps": ["three"]},
            "z9": {"os": "windows", "r2": "k/z9.zip", "size_bytes": 30, "apps": ["one", "two", "three"]}}}
        self.assertEqual(pc.bundles_for(d, ["one", "two", "three"]), ["z9"])

    def test_equal_cost_covers_are_broken_by_the_smaller_ids(self):
        d = {"schema": pc.SCHEMA, "bundles": {
            "dd": {"os": "windows", "r2": "k/dd.zip", "size_bytes": 10, "apps": ["two"]},
            "cc": {"os": "windows", "r2": "k/cc.zip", "size_bytes": 10, "apps": ["one"]},
            "bb": {"os": "windows", "r2": "k/bb.zip", "size_bytes": 10, "apps": ["two"]},
            "aa": {"os": "windows", "r2": "k/aa.zip", "size_bytes": 10, "apps": ["one"]}}}
        self.assertEqual(pc.bundles_for(d, ["one", "two"]), ["aa", "bb"])

    def test_download_bytes_of_only_unknown_bundles_is_refused(self):
        self.assertEqual(reason(pc.download_bytes, doc(), ["nobody"]), "UNKNOWN_BUNDLE")

    def test_download_bytes_refuses_an_unknown_bundle(self):
        self.assertEqual(reason(pc.download_bytes, doc(), ["w1", "w9"]), "UNKNOWN_BUNDLE")


class Cli(unittest.TestCase):
    def test_validate_lists_the_bundles(self):
        rc, out = cli_file(doc(), "validate")
        self.assertEqual((rc, json.loads(out)), (0, {"ok": True, "bundles": ["l1", "w1", "w2"]}))

    def test_validate_names_the_refusal_and_exits_2(self):
        rc, out = cli_file(with_bundle(os="bsd"), "validate")
        self.assertEqual((rc, out.strip()), (2, "KF_PROPIMAGE_CONFIG_REFUSED BUNDLE_OS:w1"))

    def test_resolve_prints_bundles_and_bytes(self):
        rc, out = cli_file(doc(), "resolve", "--apps", "alpha,gamma")
        self.assertEqual((rc, json.loads(out)), (0, {"bundles": ["w1", "w2"], "bytes": 300}))

    def test_resolve_refuses_an_unknown_app(self):
        rc, out = cli_file(doc(), "resolve", "--apps", "alpha,nope")
        self.assertEqual((rc, out.strip()), (2, "KF_PROPIMAGE_CONFIG_REFUSED UNKNOWN_APP:nope"))

    def test_resolve_takes_the_os(self):
        d = doc()
        d["bundles"]["l1"]["apps"].append("alpha")
        self.assertEqual(cli_file(d, "resolve", "--apps", "alpha")[0], 2)
        rc, out = cli_file(d, "resolve", "--apps", "alpha", "--os", "linux")
        self.assertEqual((rc, json.loads(out)["bundles"]), (0, ["l1"]))

    def test_resolve_of_an_empty_list_is_empty(self):
        rc, out = cli_file(doc(), "resolve", "--apps", "")
        self.assertEqual((rc, json.loads(out)), (0, {"bundles": [], "bytes": 0}))

    def test_cli_refuses_a_bad_app_name(self):
        rc, out = cli_file(doc(), "resolve", "--apps", "Alpha")
        self.assertEqual((rc, out.strip()), (2, "KF_PROPIMAGE_CONFIG_REFUSED BAD_APP:Alpha"))

    def test_cli_refuses_an_ambiguous_app(self):
        d = doc()
        d["bundles"]["l1"]["apps"].append("alpha")
        rc, out = cli_file(d, "resolve", "--apps", "alpha")
        self.assertEqual((rc, out.strip()), (2, "KF_PROPIMAGE_CONFIG_REFUSED AMBIGUOUS_APP:alpha"))

    def test_an_unreadable_file_is_a_named_refusal(self):
        rc, out = cli("--config", "/nonexistent/none.json", "validate")
        self.assertEqual((rc, out.strip()), (2, "KF_PROPIMAGE_CONFIG_UNREADABLE FileNotFoundError"))

    def test_invalid_json_exits_2(self):
        rc, out = cli_file("{", "validate")
        self.assertEqual((rc, out.strip()), (2, "KF_PROPIMAGE_CONFIG_REFUSED JSON"))


def rename(d, old, new):
    d["bundles"][new] = d["bundles"].pop(old)


def many(n, **extra):
    return lambda d: d.update(bundles={"b%d" % i: {"os": "linux", "r2": "k/%d.zip" % i, "size_bytes": 1, "apps": ["a%d" % i], **extra} for i in range(n)})


def run_of(**changes):
    return lambda d: d["bundles"]["w1"].update(run={**windows_run(), **changes})


def linux_run_of(**changes):
    return lambda d: d["bundles"]["l1"].update(run={"rows_file": "scripts/bench/linux/rows.py", "pins_file": "scripts/ci/pins.json", "isolation": "row", **changes})


def bundle_of(**changes):
    return lambda d: d["bundles"]["w1"].update(changes)


# Every refusal of the validator, once more through the command line (the exit status and the one printed line a script reads). Each entry
# becomes a test of its own, `test_<key>`, so a mutant that disables a check is killed by the unit test AND by this one.
CLI_REFUSALS = {
    "a_typo_in_the_file_is_refused_by_the_cli": (lambda d: d.update(colour=1), "TOP_KEY"),
    "a_file_without_bundles_is_refused_by_the_cli": (lambda d: d.pop("bundles"), "TOP_KEY"),
    "a_file_with_no_bundles_is_refused_by_the_cli": (lambda d: d.update(bundles={}), "BUNDLES"),
    "seventeen_bundles_are_refused_by_the_cli": (many(17), "BUNDLES"),
    "a_status_of_4001_characters_is_refused_by_the_cli": (lambda d: d.update(status="x" * 4001), "STATUS"),
    "a_bundle_id_with_a_space_is_refused_by_the_cli": (lambda d: rename(d, "w1", "w 1"), "BUNDLE_ID:w 1"),
    "a_bundle_that_is_a_string_is_refused_by_the_cli": (lambda d: d["bundles"].update(w1="x"), "BUNDLE_SHAPE:w1"),
    "a_typo_in_a_bundle_is_refused_by_the_cli": (bundle_of(slot=1), "BUNDLE_KEY:w1"),
    "a_bundle_without_apps_is_refused_by_the_cli": (lambda d: d["bundles"]["w1"].pop("apps"), "BUNDLE_MISSING:w1"),
    "an_object_key_with_a_space_is_refused_by_the_cli": (bundle_of(r2="a b.zip"), "BUNDLE_R2:w1"),
    "a_zero_size_is_refused_by_the_cli": (bundle_of(size_bytes=0), "BUNDLE_SIZE:w1"),
    "a_size_of_the_limit_is_refused_by_the_cli": (bundle_of(size_bytes=pc.MAX_SIZE), "BUNDLE_SIZE:w1"),
    "a_true_size_is_refused_by_the_cli": (bundle_of(size_bytes=True), "BUNDLE_SIZE:w1"),
    "a_bundle_with_sixty_five_apps_is_refused_by_the_cli": (bundle_of(apps=["a%d" % i for i in range(pc.MAX_APPS + 1)]), "BUNDLE_APPS:w1"),
    "an_uppercase_sha256_is_refused_by_the_cli": (bundle_of(sha256=H64.upper()), "BUNDLE_SHA256:w1"),
    "a_ref_with_a_slash_is_refused_by_the_cli": (bundle_of(ref="a/b"), "BUNDLE_REF:w1"),
    "an_unknown_family_is_refused_by_the_cli": (bundle_of(needs_gpu_families=["pascal"]), "BUNDLE_FAMILIES:w1"),
    "a_repeated_family_is_refused_by_the_cli": (bundle_of(needs_gpu_families=["ada", "ada"]), "BUNDLE_FAMILIES:w1"),
    "an_empty_family_list_is_refused_by_the_cli": (bundle_of(needs_gpu_families=[]), "BUNDLE_FAMILIES:w1"),
    "a_note_with_a_control_character_is_refused_by_the_cli": (bundle_of(basis="a\tb"), "BUNDLE_BASIS:w1"),
    "a_run_that_is_a_list_is_refused_by_the_cli": (bundle_of(run=[]), "RUN_SHAPE:w1"),
    "a_windows_run_without_a_pins_file_is_refused_by_the_cli": (lambda d: d["bundles"]["w1"].update(run={"rows_file": "scripts/x.py"}), "RUN_MISSING:w1"),
    "a_climbing_rows_file_is_refused_by_the_cli": (run_of(rows_file="a/../b.py"), "RUN_ROWS_FILE:w1"),
    "an_absolute_pins_file_is_refused_by_the_cli": (run_of(pins_file="/etc/pins.json"), "RUN_PINS_FILE:w1"),
    "a_run_with_an_uppercase_row_is_refused_by_the_cli": (run_of(rows=["R"]), "RUN_ROWS:w1"),
    "a_run_with_a_repeated_solo_row_is_refused_by_the_cli": (run_of(solo_rows=["r", "r"]), "RUN_SOLO_ROWS:w1"),
    "a_solo_row_that_is_not_a_row_is_refused_by_the_cli": (run_of(rows=["r1"], solo_rows=["r2"]), "RUN_SOLO_ROWS_UNKNOWN:w1"),
    "data_iso_one_is_refused_by_the_cli": (run_of(data_iso=1), "RUN_DATA_ISO:w1"),
    "audio_yes_is_refused_by_the_cli": (run_of(audio="yes"), "RUN_AUDIO:w1"),
    "guest_ram_of_1500_is_refused_by_the_cli": (run_of(guest_ram_mb=1500), "RUN_GUEST_RAM:w1"),
    "a_guest_ram_of_two_tib_is_refused_by_the_cli": (run_of(guest_ram_mb=1 << 21), "RUN_GUEST_RAM:w1"),
    "a_typo_in_harness_args_is_refused_by_the_cli": (run_of(harness_args={"middle": []}), "ARGS_KEY:w1"),
    "a_run_with_seventeen_words_is_refused_by_the_cli": (run_of(harness_args={"after_apps": ["w"] * 17}), "ARGS:w1:after_apps"),
    "a_word_with_a_semicolon_is_refused_by_the_cli": (run_of(harness_args={"before_apps": ["a;b"]}), "ARGS:w1:before_apps"),
    "per_row_that_is_a_list_is_refused_by_the_cli": (run_of(harness_args={"per_row": []}), "ARGS_PER_ROW:w1"),
    "per_row_name_with_a_space_is_refused_by_the_cli": (run_of(harness_args={"per_row": {"a b": []}}), "ARGS_PER_ROW_NAME:w1"),
    "per_row_name_outside_rows_is_refused_by_the_cli": (run_of(rows=["r1"], harness_args={"per_row": {"r2": []}}), "ARGS_PER_ROW_UNKNOWN:w1:r2"),
    "the_data_volume_flag_after_the_apps_is_refused": (run_of(harness_args={"after_apps": ["--data-iso"]}), "ARGS_DATA_ISO_SET_BY_FIELD:w1"),
    "the_data_volume_flag_in_a_per_row_list_is_refused": (run_of(harness_args={"per_row": {"r1": ["--data-iso"]}}), "ARGS_DATA_ISO_SET_BY_FIELD:w1"),
    "linux_run_with_a_typo_is_refused_by_the_cli": (linux_run_of(audio=True), "RUN_KEY:l1"),
    "a_linux_run_without_isolation_is_refused_by_the_cli": (lambda d: d["bundles"]["l1"].update(run={"rows_file": "scripts/x.py", "pins_file": "scripts/p.json"}), "RUN_MISSING:l1"),
    "a_linux_run_with_a_climbing_path_is_refused_by_the_cli": (linux_run_of(rows_file="../x.py"), "RUN_ROWS_FILE:l1"),
    "a_linux_run_with_isolation_vm_is_refused_by_the_cli": (linux_run_of(isolation="vm"), "RUN_ISOLATION:l1"),
    "a_duplicate_object_key_is_refused_by_the_cli": (lambda d: d["bundles"]["l1"].update(r2="win/one.zip"), "DUPLICATE_R2"),
}


def _cli_refusal(mutate, want):
    def test(self):
        d = doc()
        mutate(d)
        rc, out = cli_file(d, "validate")
        self.assertEqual((rc, out.strip()), (2, "KF_PROPIMAGE_CONFIG_REFUSED " + want))
    return test


class CliRefusals(unittest.TestCase):
    pass


for _key, (_mutate, _want) in CLI_REFUSALS.items():
    setattr(CliRefusals, "test_" + _key, _cli_refusal(_mutate, _want))


def load_by_path(path, name):
    spec = importlib.util.spec_from_file_location(name, path)
    mod = importlib.util.module_from_spec(spec)
    sys.path.insert(0, str(ROOT / "scripts/bench/windows"))
    sys.path.insert(0, str(ROOT / "scripts/bench/linux"))
    spec.loader.exec_module(mod)
    return mod


class Committed(unittest.TestCase):
    """The repository's own entries (ci/vast-broker/propimage-bundles.json) agree with what the code and the Worker's data say today."""

    @classmethod
    def setUpClass(cls):
        cls.cfg = pc.load()
        cls.bundles = cls.cfg["bundles"]
        cls.worker = json.loads((ROOT / "ci/vast-broker/propimages.json").read_text())
        cls.objects = {o["id"]: o for o in cls.worker["objects"]}

    def test_committed_file_is_valid_and_the_cli_agrees(self):
        self.assertEqual(cli("validate")[0], 0)

    def test_the_five_project_bundles_are_there(self):
        self.assertEqual(sorted(self.bundles), ["imga", "imgb", "imgc", "imgd", "imge"])
        self.assertEqual({k: v["os"] for k, v in self.bundles.items()}, {"imga": "windows", "imgb": "windows", "imgc": "linux", "imgd": "windows", "imge": "linux"})

    def test_windows_and_linux_apps_do_not_collide(self):
        owner = {}
        for bid, b in self.bundles.items():
            for app in b["apps"]:
                owner.setdefault(app, set()).add(b["os"])
        self.assertEqual([a for a, oses in owner.items() if len(oses) > 1], [])

    def test_windows_bundles_a_and_b_match_the_workers_objects_exactly(self):
        for bid in ("imga", "imgb"):
            obj, b = self.objects[bid], self.bundles[bid]
            self.assertEqual((b["r2"], b["size_bytes"], b["sha256"], b["ref"]), (obj["object"], obj["size"], obj["sha256"], obj["ref"]), bid)

    def test_the_matlab_bundle_names_the_workers_object(self):
        # size and sha256 differ on purpose until PR 467 corrects propimages.json and matlab_pins.json (the bundle carries the object's read-back);
        # the day those files carry the same values, drop this exemption and compare all four fields as for the Windows bundles.
        obj, b = self.objects["imgc"], self.bundles["imgc"]
        self.assertEqual((b["r2"], b["ref"]), (obj["object"], obj["ref"]))

    def test_worker_apps_are_the_bundle_apps(self):
        for image in self.worker["images"]:
            self.assertEqual(self.bundles[image["object"]]["apps"], list(image["apps"]), image["object"])

    def test_the_matlab_bundle_apps_are_the_one_application(self):
        self.assertEqual(self.bundles["imgc"]["apps"], ["matlab"])

    def test_no_secret_shapes_in_the_file(self):
        text = (ROOT / "ci/vast-broker/propimage-bundles.json").read_text()
        self.assertEqual(re.findall(r"[\w.+-]+@[\w-]+\.[\w.]+", text), [])
        self.assertEqual(re.findall(r"(?i)password|passwd|secret|api[_-]?key|licen[cs]e[_-]?key|serial", text), [])

    def test_unverified_bundles_say_so_and_carry_no_identity_pin(self):
        for bid in ("imgd", "imge"):
            self.assertIn("UNVERIFIED", self.bundles[bid]["basis"], bid)
            self.assertNotIn("sha256", self.bundles[bid], bid)

    def test_the_autodesk_note_is_on_the_windows_bundle_that_has_the_token(self):
        self.assertIn("Autodesk token not yet revoked", self.bundles["imgd"]["licence_note"])

    def test_every_gpu_family_is_a_family_of_the_stage_table(self):
        pa = load_by_path(HERE / "propimage_apps.py", "kf_pc_pa")
        self.assertEqual(set(pc.FAMILIES), set(pa.FAMILY_BY_CC.values()))

    def test_the_stage_pins_files_exist_and_name_the_bundles_objects(self):
        for bid in ("imga", "imgb", "imgc"):
            run = self.bundles[bid]["run"]
            pins = json.loads((ROOT / run["pins_file"]).read_text())
            image = next(i for i in pins["items"] if i["role"] == "image")
            self.assertEqual(image["ref"], self.bundles[bid]["ref"], bid)

    def test_data_volume_is_declared_exactly_for_the_bundle_whose_pins_carry_one(self):
        for bid in ("imga", "imgb"):
            run = self.bundles[bid]["run"]
            roles = {i["role"] for i in json.loads((ROOT / run["pins_file"]).read_text())["items"]}
            self.assertEqual(run["data_iso"], "data" in roles, bid)

    def test_windows_a_rows_are_the_first_stage_applications_in_run_order(self):
        pa = load_by_path(HERE / "propimage_apps.py", "kf_pc_pa2")
        run = self.bundles["imga"]["run"]
        self.assertEqual(tuple(run["rows"]), pa.APPS)
        self.assertEqual(run["solo_rows"], [pa.APPS[-1]])
        check = load_by_path(ROOT / run["rows_file"], "kf_pc_check")
        self.assertTrue(set(run["rows"]) <= set(check.CHECKS))

    def test_windows_a_per_row_words_are_the_first_stage_command(self):
        pa = load_by_path(HERE / "propimage_apps.py", "kf_pc_pa3")
        words = self.bundles["imga"]["run"]["harness_args"]["per_row"]
        self.assertEqual(words["superposition"], ["--superposition-run"])
        self.assertEqual(words["wukong"], ["--fb-mb", str(pa.WUKONG_FB_MB)])

    def test_windows_b_run_matches_the_second_stage_constants(self):
        sb = load_by_path(HERE / "propimage_apps_b_stage.py", "kf_pc_sb")
        run = self.bundles["imgb"]["run"]
        self.assertEqual((run["guest_ram_mb"], run["audio"]), (sb.GUEST_RAM_MB, sb.AUDIO))
        self.assertEqual(run["harness_args"], {"before_apps": ["--app-set", "b"], "after_apps": ["--fb-mb", sb.FB_MB]})
        self.assertTrue(run["data_iso"])
        table = load_by_path(ROOT / run["rows_file"], "kf_pc_table")
        self.assertTrue(set(table.CHECKS))

    def test_matlab_run_names_the_rows_module_and_the_pins(self):
        run = self.bundles["imgc"]["run"]
        rows = load_by_path(ROOT / run["rows_file"], "kf_pc_rows")
        self.assertTrue(len(rows.ROWS) > 0)
        self.assertEqual(run["isolation"], "row")
        self.assertTrue((ROOT / run["pins_file"]).is_file())

    def test_resolving_each_bundles_apps_gives_that_bundle(self):
        for bid, b in self.bundles.items():
            self.assertEqual(pc.bundles_for(self.cfg, b["apps"]), [bid], bid)

    def test_a_mixed_request_over_the_committed_entries(self):
        self.assertEqual(pc.bundles_for(self.cfg, ["solidworks", "premiere", "matlab", "maya"]), ["imga", "imgb", "imgc", "imge"])
        self.assertEqual(pc.download_bytes(self.cfg, ["imga", "imgc"]), 50496618964 + 8128391922)

    def test_the_committed_file_is_not_mutated_by_resolving(self):
        before = copy.deepcopy(self.cfg)
        pc.bundles_for(self.cfg, ["solidworks", "matlab"])
        self.assertEqual(self.cfg, before)


PARITY = ROOT / "ci/vast-broker/test/fixtures/bundles/parity-cases.json"


class Parity(unittest.TestCase):
    """The cases shared with the Worker's JavaScript port (ci/vast-broker/src/bundles.js, test/bundles.test.js): this module is the reference, so
    every case's expected answer, refusal reason included, must be what it computes."""

    @classmethod
    def setUpClass(cls):
        cls.cases = json.loads(PARITY.read_text())

    def test_every_validation_case_has_the_recorded_reason(self):
        for c in self.cases["validation"]:
            self.assertEqual(reason(pc.validate, copy.deepcopy(c["doc"])), c["reason"], c["name"])

    def test_every_resolver_case_has_the_recorded_answer(self):
        for c in self.cases["resolver"]:
            try:
                got = {"bundles": pc.bundles_for(copy.deepcopy(c["doc"]), list(c["apps"]), c.get("os"))}
            except pc.ConfigError as exc:
                got = {"error": exc.reason}
            self.assertEqual(got, c["expect"], c["name"])

    def test_the_cases_cover_both_outcomes_and_every_refusal_of_the_matrix(self):
        reasons = {c["reason"] for c in self.cases["validation"]}
        self.assertIn(None, reasons)
        self.assertTrue({want for _, want in CLI_REFUSALS.values()} <= reasons)
        self.assertTrue(any("error" in c["expect"] for c in self.cases["resolver"]))
        self.assertTrue(any("bundles" in c["expect"] for c in self.cases["resolver"]))

    def test_the_committed_table_in_the_cases_is_the_committed_table(self):
        embedded = next(c["doc"] for c in self.cases["validation"] if c["name"] == "valid committed file")
        self.assertEqual(embedded, pc.load())


if __name__ == "__main__":
    unittest.main()
