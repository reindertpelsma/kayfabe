# SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later
"""The committed VK-GL-CTS caselists, their selection spec and hash (cts_caselist.py). GPU-free; no CTS tree needed."""
import contextlib
import io
import json
import os
import shutil
import tempfile
import unittest

import helpers as H
import cts_caselist as C
from kfopen_common import OpenRowError, load_json, sha256_text

SPEC = load_json(C.SPEC)


class Committed(unittest.TestCase):
    def test_committed_lists_match_the_spec_and_their_recorded_hash(self):
        self.assertEqual([], C.check(SPEC))

    def test_each_list_is_a_few_hundred_cases(self):
        for name, lst in SPEC["lists"].items():
            self.assertTrue(C.MIN_CASES <= lst["count"] <= C.MAX_CASES, name)
            text = open(os.path.join(H.OR, lst["out"]), encoding="utf-8").read()
            self.assertEqual(lst["count"], len(C.parse(text)), name)

    def test_gl_list_covers_the_groups_the_row_is_about(self):
        cases = C.parse(open(os.path.join(H.OR, "caselists", "gl46_core.txt")).read())
        for group in ("compute_shader", "tessellation_shader", "gl_spirv", "internalformat", "copy_image", "shader_image_load_store", "sparse_buffer_tests"):
            self.assertTrue([c for c in cases if c.startswith("KHR-GL46.%s." % group)], group)
        self.assertTrue(all(c.startswith(("KHR-GL46.", "GTF-GL46.")) for c in cases))

    def test_vulkan_list_covers_breadth_and_leaves_video_out(self):
        cases = C.parse(open(os.path.join(H.OR, "caselists", "vk_breadth.txt")).read())
        for group in ("ray_query", "ray_tracing_pipeline", "mesh_shader", "sparse_resources", "synchronization2", "dgc", "shader_object", "compute", "api"):
            self.assertTrue([c for c in cases if c.startswith("dEQP-VK.%s." % group)], group)
        self.assertFalse([c for c in cases if c.startswith("dEQP-VK.video.")], "video is built off (DEQP_DISABLE_VK_VIDEO_TESTS=ON)")

    def test_lists_are_unique_and_have_no_wildcards(self):
        for lst in SPEC["lists"].values():
            cases = C.parse(open(os.path.join(H.OR, lst["out"])).read())
            self.assertEqual(len(cases), len(set(cases)))
            self.assertFalse([c for c in cases if "*" in c or " " in c])

    def test_spec_names_the_pinned_commit(self):
        import kfopen_pins as P
        self.assertEqual(P.load_pins()["sources"]["cts"]["commit"], SPEC["cts_commit"])

    def test_every_row_points_at_a_list_of_the_spec(self):
        import kfopen_pins as P
        outs = {l["out"] for l in SPEC["lists"].values()}
        for rid, row in P.load_pins()["rows"].items():
            if row["kind"] in ("cts-gl", "cts-vk"):
                self.assertIn(row["caselist"], outs, rid)

    def test_deqp_reads_the_files_as_plain_lists(self):
        for lst in SPEC["lists"].values():
            raw = open(os.path.join(H.OR, lst["out"]), "rb").read()
            self.assertNotIn(b"\r", raw)
            self.assertFalse(raw.lstrip().startswith(b"{"), "a trie list starts with '{'; ours is one case per line")


class Select(unittest.TestCase):
    LINES = ["a.g.%03d" % i for i in range(100)] + ["b.g.x"]

    def test_select_takes_evenly_spaced(self):
        got = C.select(self.LINES, "a.g.", 4)
        self.assertEqual(["a.g.000", "a.g.025", "a.g.050", "a.g.075"], got)

    def test_select_all_and_oversized_take_return_the_whole_group(self):
        self.assertEqual(100, len(C.select(self.LINES, "a.g.", "all")))
        self.assertEqual(100, len(C.select(self.LINES, "a.g.", 500)))

    def test_select_keeps_order_and_has_no_repeats(self):
        got = C.select(self.LINES, "a.g.", 7)
        self.assertEqual(got, sorted(got))
        self.assertEqual(len(got), len(set(got)))

    def test_empty_group_and_bad_take_refused(self):
        with self.assertRaisesRegex(OpenRowError, "CASELIST_EMPTY_GROUP"):
            C.select(self.LINES, "zzz.", 3)
        with self.assertRaisesRegex(OpenRowError, "CASELIST_BAD_TAKE"):
            C.select(self.LINES, "a.g.", 0)

    def test_generate_on_synthetic_tree(self):
        with tempfile.TemporaryDirectory() as t:
            rel = "m/main.txt"
            os.makedirs(os.path.join(t, "m"))
            open(os.path.join(t, "m", "main.txt"), "w").write("\n".join("KHR-GL46.grp.case%03d" % i for i in range(60)) + "\n# c\n\n")
            spec = {"lists": {"gl46_core": {"groups": [{"source": rel, "prefix": "KHR-GL46.grp.", "take": 6}, {"source": rel, "prefix": "KHR-GL46.grp.case00", "take": "all"}]}}}
            out = C.generate(spec, t)["gl46_core"]
            self.assertEqual(6 + 10 - 1, len(out), "the overlap of the two groups is not repeated")
            self.assertEqual("KHR-GL46.grp.case000", out[0])
            self.assertEqual(len(out), len(set(out)))

    def test_render_names_commit_and_groups(self):
        text = C.render("gl46_core", "c" * 40, ["KHR-GL46.a.b.c"], [{"prefix": "KHR-GL46.a.", "take": "all"}])
        self.assertIn("# kfopen caselist gl46_core: 1 cases", text.splitlines()[0])
        self.assertIn("c" * 40, text.splitlines()[0])
        self.assertEqual(["KHR-GL46.a.b.c"], C.parse(text))


class Validate(unittest.TestCase):
    def cases(self, n, prefix="KHR-GL46.grp.sub."):
        return "".join("%scase%04d\n" % (prefix, i) for i in range(n))

    def test_good_list_passes(self):
        self.assertEqual([], C.validate("gl46_core", self.cases(200)))

    def test_too_few_or_too_many_cases_refused(self):
        self.assertTrue(C.validate("gl46_core", self.cases(10)))
        self.assertTrue(C.validate("gl46_core", self.cases(1500)))

    def test_duplicate_case_refused(self):
        text = self.cases(200) + "KHR-GL46.grp.sub.case0001\n"
        self.assertTrue([p for p in C.validate("gl46_core", text) if "CASELIST_DUPLICATE" in p])

    def test_duplicate_makes_check_fail(self):
        with tempfile.TemporaryDirectory() as t:
            text = "# kfopen caselist gl46_core: x %s\n" % SPEC["cts_commit"] + self.cases(200) + "KHR-GL46.grp.sub.case0003\n"
            os.makedirs(os.path.join(t, "caselists"))
            open(os.path.join(t, "caselists", "gl46_core.txt"), "w", newline="\n").write(text)
            spec = {"cts_commit": SPEC["cts_commit"], "lists": {"gl46_core": {"out": "caselists/gl46_core.txt", "sha256": sha256_text(text)}}}
            problems = C.check(spec, t)
            self.assertTrue([p for p in problems if "CASELIST_DUPLICATE" in p], problems)

    def test_validate_refuses_foreign_root_in_the_gl_list(self):
        self.assertTrue([p for p in C.validate("gl46_core", self.cases(200, "dEQP-VK.api.x.")) if "CASELIST_BAD_ROOT" in p])

    def test_gl_case_cannot_hide_in_the_vulkan_list(self):
        self.assertTrue([p for p in C.validate("vk_breadth", self.cases(200, "KHR-GL46.a.b.")) if "CASELIST_BAD_ROOT" in p])

    def test_wildcards_spaces_and_odd_names_refused(self):
        for bad in ("KHR-GL46.a.*", "KHR-GL46.a b.c", "KHR-GL46.a", "KHR-GL46..a.b"):
            self.assertTrue([p for p in C.validate("gl46_core", self.cases(200) + bad + "\n") if "CASELIST_BAD_NAME" in p], bad)


class HashAndCommit(unittest.TestCase):
    def tree(self):
        t = tempfile.mkdtemp(prefix="kfopen-cl-")
        self.addCleanup(shutil.rmtree, t, True)
        os.makedirs(os.path.join(t, "caselists"))
        for lst in SPEC["lists"].values():
            shutil.copy(os.path.join(H.OR, lst["out"]), os.path.join(t, lst["out"]))
        return t

    def test_hand_edited_caselist_fails_hash(self):
        t = self.tree()
        p = os.path.join(t, "caselists", "gl46_core.txt")
        open(p, "a", newline="\n").write("KHR-GL46.extra.group.case\n")
        problems = C.check(SPEC, t)
        self.assertTrue([x for x in problems if "CASELIST_HASH" in x], problems)

    def test_check_cli_exits_nonzero_on_edited_list(self):
        t = self.tree()
        open(os.path.join(t, "caselists", "vk_breadth.txt"), "a", newline="\n").write("dEQP-VK.extra.group.case\n")
        old = (C.HERE, C.SPEC)
        C.HERE, C.SPEC = t, os.path.join(t, "caselist_spec.json")
        shutil.copy(old[1], C.SPEC)
        try:
            with contextlib.redirect_stdout(io.StringIO()):
                self.assertEqual(1, C.main(["check"]))
        finally:
            C.HERE, C.SPEC = old

    def test_crlf_caselist_refused(self):
        t = self.tree()
        p = os.path.join(t, "caselists", "gl46_core.txt")
        data = open(p, "rb").read().replace(b"\n", b"\r\n")
        open(p, "wb").write(data)
        self.assertTrue([x for x in C.check(SPEC, t) if "CASELIST_CRLF" in x])

    def test_list_that_does_not_name_the_pinned_commit_refused(self):
        t = self.tree()
        spec = json.loads(json.dumps(SPEC))
        spec["cts_commit"] = "9" * 40
        self.assertTrue([x for x in C.check(spec, t) if "CASELIST_COMMIT" in x])

    def test_missing_list_refused(self):
        t = self.tree()
        os.unlink(os.path.join(t, "caselists", "vk_breadth.txt"))
        self.assertTrue([x for x in C.check(SPEC, t) if "CASELIST_MISSING" in x])

    def test_committed_check_exits_zero(self):
        self.assertEqual(0, C.main(["check"]))


if __name__ == "__main__":
    unittest.main()
