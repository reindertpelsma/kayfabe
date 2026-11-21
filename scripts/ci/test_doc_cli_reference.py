"""The command-line reference (docs/howto/cli-reference.md) holds the device's properties and their defaults.

STATUS: LIVE, 2026-11-20 (scripts/ci/doc_cli_reference.py; mutants: scripts/ci/doc_cli_reference_mutants.py).

FALSIFIER (written before the code): a property added to the `kf3_properties` table of qemu/hw/misc/kf3/kf3.c with no row on
the page, a default changed in the source (or on the page) without the other, or a generated table edited by hand or left
behind by a change of the source or of the meaning text, passes this file. Each repository test below reads the source with
its own small regular expression, not with the generator's parser, so a parser that loses a property cannot hide itself.
"""

import json
import re
import shutil
import sys
import tempfile
import unittest
from pathlib import Path

HERE = Path(__file__).resolve().parent
sys.path.insert(0, str(HERE))
import doc_cli_reference as ref  # noqa: E402

ROOT = HERE.parents[1]
SOURCE = ROOT / ref.SOURCE


def independent_properties():
    """name -> the default as the source writes it, found with a plain regex over the table's lines (no generator code)."""
    text = SOURCE.read_text()
    table = text[text.index("kf3_properties[]"):]
    table = table[:table.index("};")]
    found = {}
    for line in table.splitlines():
        m = re.match(r'\s*DEFINE_PROP_([A-Z0-9]+)\("([^"]+)",\s*Kf3State,\s*\w+(?:,\s*(.*?))?\),\s*$', line)
        if m and m[1] != "END_OF_LIST":
            found[m[2]] = (m[1], m[3])
    return found


def page_names(page_text):
    return set(re.findall(r"^\|\s*`([a-z0-9-]+)`", page_text, re.M))


class Repository(unittest.TestCase):
    def test_the_page_matches_the_source_the_text_file_and_itself(self):
        self.assertEqual(ref.check(ROOT), [])

    def test_the_generator_finds_every_property_line_of_the_source(self):
        independent = independent_properties()
        self.assertGreaterEqual(len(independent), 27)
        self.assertEqual([r["name"] for r in ref.parse_properties(ROOT)], list(independent))

    def test_every_property_of_the_source_is_a_row_of_the_page(self):
        names = page_names((ROOT / ref.PAGE).read_text())
        self.assertEqual(set(independent_properties()) - names, set())

    def test_the_page_lists_no_property_the_source_does_not_declare(self):
        names = page_names((ROOT / ref.PAGE).read_text())
        self.assertEqual(names - set(independent_properties()), set())

    def test_plain_defaults_of_the_source_are_the_defaults_of_the_page(self):
        shown = ref.page_rows(ref.page_block((ROOT / ref.PAGE).read_text()))
        checked = 0
        for name, (kind, default) in independent_properties().items():
            if kind == "BOOL":
                expected = {"true": "on", "false": "off"}[default]
            elif kind == "STRING":
                expected = "unset"
            elif re.fullmatch(r"-?\d+", default):
                expected = default
            else:
                continue
            self.assertEqual(shown.get(name), expected, name)
            checked += 1
        self.assertGreaterEqual(checked, 22)

    def test_a_size_default_written_with_mib_is_shown_in_bytes(self):
        shown = ref.page_rows(ref.page_block((ROOT / ref.PAGE).read_text()))
        self.assertEqual(shown["bar1-size"], str(256 * 1024 * 1024))
        self.assertEqual(shown["bar2-size"], str(32 * 1024 * 1024))

    def test_the_text_file_names_exactly_the_properties_of_the_source(self):
        text = json.loads((ROOT / ref.TEXT).read_text())
        self.assertEqual(set(text), set(independent_properties()))

    def test_every_property_has_a_status_and_a_meaning_in_plain_words(self):
        for name, entry in json.loads((ROOT / ref.TEXT).read_text()).items():
            self.assertTrue(entry["status"].strip(), name)
            self.assertGreater(len(entry["meaning"]), 20, name)
            self.assertNotRegex(entry["meaning"] + entry["status"], r"CODE READ|MEASURED|INFERRED|\[measured", name)


class Evaluation(unittest.TestCase):
    def test_mib_products_and_literals(self):
        self.assertEqual(ref.evaluate("256 * MiB", {}), 256 * 1024 * 1024)
        self.assertEqual(ref.evaluate("-1", {}), -1)
        self.assertEqual(ref.evaluate("8192", {}), 8192)

    def test_a_define_is_resolved_and_an_unknown_name_is_refused(self):
        self.assertEqual(ref.evaluate("LIMIT", {"LIMIT": "64"}), 64)
        with self.assertRaises(ref.RefError):
            ref.evaluate("SOMETHING_ELSE", {})

    def test_a_define_loop_is_refused(self):
        with self.assertRaises(ref.RefError):
            ref.evaluate("A", {"A": "B", "B": "A"})


class Fixture(unittest.TestCase):
    """A private copy of the four files; each test breaks one thing and expects the named finding."""

    def setUp(self):
        self.tmp = Path(tempfile.mkdtemp(prefix="kf-cli-ref-"))
        self.addCleanup(shutil.rmtree, self.tmp, True)
        for rel in (ref.SOURCE, "qemu/hw/misc/kf3/kf3.h", ref.TEXT, ref.PAGE):
            target = self.tmp / rel
            target.parent.mkdir(parents=True, exist_ok=True)
            shutil.copy(ROOT / rel, target)

    def edit(self, rel, old, new, count=1):
        path = self.tmp / rel
        text = path.read_text()
        self.assertEqual(text.count(old), count, old)
        path.write_text(text.replace(old, new))

    def findings(self):
        return "\n".join(ref.check(self.tmp))

    def test_the_copy_is_clean(self):
        self.assertEqual(ref.check(self.tmp), [])

    # missing property
    def test_a_property_added_to_the_source_without_a_row_fails(self):
        self.edit(ref.SOURCE, '    DEFINE_PROP_UINT32("x-gsp-observer-seconds"',
                  '    DEFINE_PROP_UINT32("new-knob", Kf3State, new_knob, 5),\n    DEFINE_PROP_UINT32("x-gsp-observer-seconds"')
        self.assertIn("property new-knob of", self.findings())

    def test_a_property_added_as_the_last_entry_fails(self):
        self.edit(ref.SOURCE, "};\n\nstatic void kf3_class_init", '    DEFINE_PROP_BOOL("last-knob", Kf3State, last_knob, true),\n};\n\nstatic void kf3_class_init')
        self.assertIn("last-knob", self.findings())

    def test_a_property_added_with_its_text_but_no_page_row_is_missing_from_the_page(self):
        self.edit(ref.SOURCE, '    DEFINE_PROP_UINT32("x-gsp-observer-seconds"',
                  '    DEFINE_PROP_UINT32("new-knob", Kf3State, new_knob, 5),\n    DEFINE_PROP_UINT32("x-gsp-observer-seconds"')
        text = json.loads((self.tmp / ref.TEXT).read_text())
        text["new-knob"] = {"status": "Experimental", "meaning": "A knob that was added to the device."}
        (self.tmp / ref.TEXT).write_text(json.dumps(text))
        self.assertIn("new-knob is missing from the page", self.findings())

    def test_a_row_removed_from_the_page_fails(self):
        lines = (self.tmp / ref.PAGE).read_text().splitlines(keepends=True)
        kept = [line for line in lines if not line.startswith("| `msix-vectors`")]
        self.assertEqual(len(kept), len(lines) - 1)
        (self.tmp / ref.PAGE).write_text("".join(kept))
        self.assertIn("property msix-vectors is missing from the page", self.findings())

    def test_a_row_for_a_property_the_source_does_not_declare_fails(self):
        self.edit(ref.PAGE, "| `msix-vectors` |", "| `msix-vectors` | uint32 | `32` | x | x |\n| `ghost-knob` | uint32 | `1` | x | x |\n| `msix-vectors-old` |")
        self.assertIn("the page lists ghost-knob", self.findings())

    # wrong default
    def test_a_default_changed_in_the_source_fails(self):
        self.edit(ref.SOURCE, 'fb_mb, 8192)', "fb_mb, 4096)")
        self.assertIn("property fb-mb shows default 8192, the source says 4096", self.findings())

    def test_a_default_changed_on_the_page_fails(self):
        self.edit(ref.PAGE, "| `fb-mb` | uint64 | `8192` |", "| `fb-mb` | uint64 | `4096` |")
        self.assertIn("property fb-mb shows default 4096, the source says 8192", self.findings())

    def test_a_boolean_default_flipped_in_the_source_fails(self):
        self.edit(ref.SOURCE, 'windows_support, true)', "windows_support, false)")
        self.assertIn("property windows-support shows default on, the source says off", self.findings())

    def test_a_default_given_by_a_define_follows_the_define(self):
        self.edit(ref.SOURCE, "#define KF3_BAR1_OVERLAYS_DEFAULT 64", "#define KF3_BAR1_OVERLAYS_DEFAULT 96")
        self.assertIn("property bar1-overlays shows default 64, the source says 96", self.findings())

    # stale table
    def test_a_meaning_changed_in_the_text_file_without_regenerating_fails(self):
        text = json.loads((self.tmp / ref.TEXT).read_text())
        text["gpu-minor"]["meaning"] = "Which host GPU to use, said in another way than the page says it."
        (self.tmp / ref.TEXT).write_text(json.dumps(text))
        self.assertIn("the generated table is stale", self.findings())

    def test_a_row_edited_by_hand_fails_as_stale(self):
        self.edit(ref.PAGE, "Number of MSI-X vectors the device offers.", "Number of MSI-X vectors (edited by hand).")
        self.assertIn("the generated table is stale", self.findings())

    def test_a_status_changed_in_the_text_file_without_regenerating_fails(self):
        text = json.loads((self.tmp / ref.TEXT).read_text())
        text["dummy-bar"]["status"] = "Experimental"
        (self.tmp / ref.TEXT).write_text(json.dumps(text))
        self.assertIn("the generated table is stale", self.findings())

    def test_write_repairs_a_stale_table_and_check_then_passes(self):
        self.edit(ref.PAGE, "Number of MSI-X vectors the device offers.", "Number of MSI-X vectors (edited by hand).")
        self.assertNotEqual(ref.check(self.tmp), [])
        ref.write(self.tmp)
        self.assertEqual(ref.check(self.tmp), [])

    def test_write_puts_exactly_the_generated_block_on_the_page(self):
        text = json.loads((self.tmp / ref.TEXT).read_text())
        text["gpu-minor"]["meaning"] = "Which host GPU to use, said in another way than the page says it."
        (self.tmp / ref.TEXT).write_text(json.dumps(text))
        ref.write(self.tmp)
        self.assertEqual(ref.page_block((self.tmp / ref.PAGE).read_text()), ref.generate(self.tmp))
        self.assertIn("said in another way", (self.tmp / ref.PAGE).read_text())

    def test_hand_written_text_around_the_block_is_kept_by_write(self):
        self.edit(ref.PAGE, "## The display broker", "## The display broker\n\nHand written line.")
        ref.write(self.tmp)
        self.assertIn("Hand written line.", (self.tmp / ref.PAGE).read_text())

    # text file
    def test_a_property_missing_from_the_text_file_fails(self):
        text = json.loads((self.tmp / ref.TEXT).read_text())
        del text["gop"]
        (self.tmp / ref.TEXT).write_text(json.dumps(text))
        self.assertIn("property gop of", self.findings())

    def test_a_text_entry_for_a_property_the_source_does_not_declare_fails(self):
        text = json.loads((self.tmp / ref.TEXT).read_text())
        text["ghost"] = {"status": "Experimental", "meaning": "A property that the device does not have."}
        (self.tmp / ref.TEXT).write_text(json.dumps(text))
        self.assertIn("names property ghost", self.findings())

    def test_a_missing_page_or_missing_markers_fail(self):
        (self.tmp / ref.PAGE).write_text("# no block here\n")
        self.assertIn("lacks the generated block markers", self.findings())
        (self.tmp / ref.PAGE).unlink()
        self.assertIn("does not exist", self.findings())

    def test_a_property_removed_from_the_source_but_left_on_the_page_and_in_the_text_fails(self):
        self.edit(ref.SOURCE, '    DEFINE_PROP_BOOL("dummy-bar", Kf3State, dummy_bar, false),\n', "")
        findings = self.findings()
        self.assertIn("names property dummy-bar", findings)
        self.assertIn("the page lists dummy-bar", findings)

    def test_a_property_removed_from_the_source_and_the_text_but_left_on_the_page_fails(self):
        self.edit(ref.SOURCE, '    DEFINE_PROP_BOOL("dummy-bar", Kf3State, dummy_bar, false),\n', "")
        text = json.loads((self.tmp / ref.TEXT).read_text())
        del text["dummy-bar"]
        (self.tmp / ref.TEXT).write_text(json.dumps(text))
        self.assertIn("the page lists dummy-bar", self.findings())

    # the parser
    def test_a_declaration_in_a_block_comment_is_ignored(self):
        self.edit(ref.SOURCE, '    DEFINE_PROP_UINT32("x-gsp-observer-seconds"',
                  '    /* DEFINE_PROP_UINT32("old-knob", Kf3State, old_knob, 5), */\n    DEFINE_PROP_UINT32("x-gsp-observer-seconds"')
        self.assertEqual(ref.check(self.tmp), [])

    def test_a_declaration_in_a_line_comment_is_ignored(self):
        self.edit(ref.SOURCE, '    DEFINE_PROP_UINT32("x-gsp-observer-seconds"',
                  '    // DEFINE_PROP_UINT32("old-knob", Kf3State, old_knob, 5),\n    DEFINE_PROP_UINT32("x-gsp-observer-seconds"')
        self.assertEqual(ref.check(self.tmp), [])

    def test_a_declaration_the_generator_cannot_read_is_an_error_not_a_skip(self):
        self.edit(ref.SOURCE, 'bar2_size, 32 * MiB)', "bar2_size, SOME_UNDEFINED_NAME)")
        self.assertIn("cannot read the property table", self.findings())

    def test_a_declaration_of_an_unknown_kind_is_an_error_not_a_skip(self):
        self.edit(ref.SOURCE, '    DEFINE_PROP_UINT32("x-gsp-observer-seconds"',
                  '    DEFINE_PROP_DOUBLE("odd-knob", Kf3State, odd_knob, 5),\n    DEFINE_PROP_UINT32("x-gsp-observer-seconds"')
        self.assertIn("unrecognised property declaration", self.findings())

    def test_a_declaration_without_a_default_is_an_error_not_a_skip(self):
        self.edit(ref.SOURCE, '    DEFINE_PROP_UINT32("x-gsp-observer-seconds"',
                  '    DEFINE_PROP_UINT32("short-knob", Kf3State),\n    DEFINE_PROP_UINT32("x-gsp-observer-seconds"')
        self.assertIn("unrecognised property declaration", self.findings())

    def test_a_duplicate_property_of_another_type_is_an_error(self):
        self.edit(ref.SOURCE, '    DEFINE_PROP_UINT32("x-gsp-observer-seconds"',
                  '    DEFINE_PROP_BOOL("gpu-minor", Kf3State, gpu_minor2, false),\n    DEFINE_PROP_UINT32("x-gsp-observer-seconds"')
        self.assertIn("declared twice", self.findings())

    def test_a_duplicate_property_is_an_error(self):
        self.edit(ref.SOURCE, '    DEFINE_PROP_UINT32("x-gsp-observer-seconds"',
                  '    DEFINE_PROP_UINT32("gpu-minor", Kf3State, gpu_minor2, 0),\n    DEFINE_PROP_UINT32("x-gsp-observer-seconds"')
        self.assertIn("declared twice", self.findings())


if __name__ == "__main__":
    unittest.main()
