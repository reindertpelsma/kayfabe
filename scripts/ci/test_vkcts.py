#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later
"""GPU-free tests of the open Vulkan CTS row (scripts/apps/vkcts, docs/design/V3_VULKAN_CTS_ROW.md).

STATUS: LIVE, 2026-11-17. No GPU, no network and no CTS source is needed: the deqp-vk binary is a small Python stand-in that
writes QPA logs the way qpTestLog.c does (formats read from the pinned CTS source, framework/qphelper/qpTestLog.c and
framework/common/tcuTestSessionExecutor.cpp); git is exercised against local repositories. Mutants of the code under test:
scripts/ci/vkcts_mutants.py (every mutant needs two failing tests of this file)."""
import collections
import hashlib
import json
import os
import re
import shutil
import stat
import subprocess
import sys
import tempfile
import textwrap
import unittest
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
TOOL = ROOT / "scripts/apps/vkcts"
sys.path.insert(0, str(TOOL))
import vkcts_lib as L  # noqa: E402
import vkcts as V  # noqa: E402

PINS = json.loads((TOOL / "pins.json").read_text())
CASELIST_TEXT = (TOOL / "caselist.tsv").read_text()
SPEC = json.loads((TOOL / "caselist_spec.json").read_text())
FAKE_ENV = {"PATH": os.environ.get("PATH", "/usr/bin:/bin"), "LANG": "C"}

FAKE_DEQP = r'''#!/usr/bin/env python3
import json, os, sys, time, signal
args = dict(a[2:].split("=", 1) for a in sys.argv[1:] if a.startswith("--") and "=" in a)
plan = json.loads(os.environ.get("FAKE_PLAN", "{}"))
cases = [c for c in open(args["deqp-caselist-file"]).read().split() if c]
out = open(args["deqp-log-filename"], "a")
counts = dict(Pass=0, Fail=0, NotSupported=0, Warn=0)
body = {
    "dEQP-VK.info.device_properties": ["deviceType = VK_PHYSICAL_DEVICE_TYPE_%s" % plan.get("type", "DISCRETE_GPU"),
                                       "deviceName = %s" % plan.get("device", "NVIDIA GeForce RTX 3060"), "vendorID = 0x10de",
                                       "apiVersion = 0x00404138", "driverVersion = 0x9127c0c0"],
    "dEQP-VK.info.device_extensions": ["extensionName = %s, specVersion = 1" % e for e in plan.get("exts", ["VK_KHR_swapchain", "VK_KHR_ray_query"])],
}
sys.stdout.write("dEQP Core fake starting..\n")
for c in cases:
    mode = plan.get("cases", {}).get(c, plan.get("default", "Pass"))
    if mode == "UNKNOWN":
        continue
    sys.stdout.write("\nTest case '%s'..\n" % c); sys.stdout.flush()
    out.write("\n#beginTestCaseResult %s\n<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<TestCaseResult Version=\"0.3.4\" CasePath=\"%s\" CaseType=\"SelfValidate\">\n" % (c, c))
    for line in body.get(c, ["<Message>checking</Message>"]):
        out.write("<Text>%s</Text>\n" % line)
    out.flush()
    if mode == "CRASH":
        os.kill(os.getpid(), signal.SIGKILL)
    if mode == "TERMINATE":
        out.write("\n#terminateTestCaseResult Crash\n"); out.flush(); sys.exit(1)
    if mode == "TERMINATE_TIMEOUT":
        out.write("\n#terminateTestCaseResult Timeout\n"); out.flush(); sys.exit(1)
    if mode == "HANG":
        time.sleep(600)
    if mode == "BADEND":
        out.write("\n#endTestCaseResult\n"); out.flush(); continue
    status = {"Pass": "Pass", "Warn": "QualityWarning"}.get(mode, mode)
    desc = {"Fail": "Fail: image mismatch <&>", "NotSupported": "Extension VK_KHR_x not supported"}.get(mode, "Passed")
    out.write("<Result StatusCode=\"%s\">%s</Result>\n</TestCaseResult>\n\n#endTestCaseResult\n" % (status, desc)); out.flush()
    sys.stdout.write("  %s (%s)\n" % (status, desc))
    counts[{"Pass": "Pass", "QualityWarning": "Warn", "NotSupported": "NotSupported"}.get(status, "Fail")] += 1
n = len(cases)
sys.stdout.write("\nDONE!\n\nTest run totals:\n  Passed:        %d/%d (x%%)\n  Failed:        %d/%d (x%%)\n  Not supported: %d/%d (x%%)\n  Warnings:      %d/%d (x%%)\n  Waived:        0/%d (x%%)\n"
                 % (counts["Pass"], n, counts["Fail"], n, counts["NotSupported"], n, counts["Warn"], n, n))
'''

FAKE_SMI = r'''#!/usr/bin/env python3
import os, sys, time
mode = os.environ.get("FAKE_SMI", "busy")
if "dmon" in sys.argv:
    print("# gpu    sm   mem   enc   dec\n# Idx     %     %     %     %")
    for i in range(6):
        print("    0   %d    10     0     0" % (40 if mode == "busy" else 0))
    sys.exit(0)
for i in range(8):
    util = 55 if mode == "busy" else 0
    mem = 1000 + (900 if (mode == "busy" and i >= 2) else 0)
    print("2026/11/17 10:00:%02d.000, %d, 10, %d, 0, 0" % (i, util, mem))
sys.stdout.flush()
'''


def entries(text=CASELIST_TEXT):
    return L.read_caselist(text)


def write_exec(path, text):
    Path(path).parent.mkdir(parents=True, exist_ok=True)
    Path(path).write_text(text)
    Path(path).chmod(0o755)


def make_artifact(tmp, with_binary=True):
    art = Path(tmp) / "artifact"
    (art / "data/vulkan").mkdir(parents=True)
    if with_binary:
        write_exec(art / "bin/deqp-vk", FAKE_DEQP)
    return art


def small_pins(tmp, **run):
    pins = json.loads(json.dumps(PINS))
    pins["run"].update(batch_cases=3, case_timeout_s=2, budget_s=60, max_idle_restarts=2)
    pins["run"].update(run)
    p = Path(tmp) / "pins.json"
    p.write_text(json.dumps(pins))
    return p


def caselist_file(tmp, cases, extra=()):
    info = PINS["run"]["info_cases"]
    p = Path(tmp) / "list.tsv"
    p.write_text("".join("info\t%s\n" % c for c in info) + "".join("mini\t%s\n" % c for c in list(cases) + list(extra)))
    return p


def run_tool(args, env=None, timeout=120):
    e = dict(FAKE_ENV)
    e.update(env or {})
    return subprocess.run([sys.executable, "-I", str(TOOL / "vkcts.py")] + args, env=e, capture_output=True, text=True, timeout=timeout)


def qpa(*blocks):
    return "\n".join(blocks)


def block(name, status="Pass", desc="Passed", body=""):
    return ('\n#beginTestCaseResult %s\n<?xml version="1.0" encoding="UTF-8"?>\n<TestCaseResult Version="0.3.4" CasePath="%s" CaseType="SelfValidate">\n%s'
            '<Result StatusCode="%s">%s</Result>\n</TestCaseResult>\n\n#endTestCaseResult\n' % (name, name, body, status, desc))


# ----------------------------------------------------------------------------------------------------------------------
class PinsTests(unittest.TestCase):
    def test_committed_pins_are_valid_and_pin_by_commit(self):
        L.validate_pins(PINS)
        self.assertRegex(PINS["cts"]["commit"], r"^[0-9a-f]{40}$")
        for name, row in PINS["externals_git"].items():
            self.assertRegex(row["commit"], r"^[0-9a-f]{40}$", name)

    def test_no_pin_names_a_binary_or_an_artifact_digest(self):
        text = (TOOL / "pins.json").read_text()
        self.assertNotIn("deqp-vk\": {", text)
        self.assertNotIn("binary_sha256", text)
        self.assertFalse(any("deqp-vk" in k for k in PINS.get("externals_files", {})))

    def bad(self, mutate):
        p = json.loads(json.dumps(PINS))
        mutate(p)
        with self.assertRaises(L.PinRefused):
            L.validate_pins(p)

    def test_a_cts_pin_that_is_a_tag_or_short_or_empty_is_refused(self):
        for value in ("vulkan-cts-1.4.7.1", "0d18b34", "", "0D18B347943E5E843529A05C95220E27FE30C538", "0d18b347943e5e843529a05c95220e27fe30c53"):
            with self.subTest(value):
                self.bad(lambda p: p["cts"].__setitem__("commit", value))

    def test_an_external_without_a_commit_is_refused(self):
        self.bad(lambda p: p["externals_git"]["glslang"].__setitem__("commit", "main"))
        self.bad(lambda p: p["externals_git"]["glslang"].pop("commit"))

    def test_a_file_external_without_sha256_and_a_plain_http_origin_are_refused(self):
        self.bad(lambda p: p["externals_files"]["zlib"].__setitem__("sha256", ""))
        self.bad(lambda p: p["externals_git"]["amber"].__setitem__("url", "http://github.com/google/amber.git"))
        self.bad(lambda p: p["cts"].__setitem__("url", "https://example.org/cts.git"))

    def test_wrong_schema_and_missing_build_arguments_are_refused(self):
        self.bad(lambda p: p.__setitem__("schema", "other"))
        self.bad(lambda p: p["build"].__setitem__("cmake_args", []))

    def fetch_text(self, pins=None, **change):
        """A fetch_sources.py in the shape of the upstream one, generated from the pins and then altered."""
        pins = pins or PINS
        out = []
        for name, row in pins["externals_git"].items():
            rev = row.get("upstream_ref") or row["commit"]
            rev = change.get("rev_" + name, rev)
            url = change.get("url_" + name, row["url"])
            out.append('    GitRepo(\n        "%s",\n        "git@github.com:x/y.git",\n        "%s",\n        "%s"),\n' % (url, rev, name))
        for name, row in pins["externals_files"].items():
            digest = change.get("sha_" + name, row["sha256"])
            out.append('    SourcePackage(\n        "%s",\n        "%s",\n        "%s"),\n' % (row["url"], digest, name))
        if change.get("extra"):
            out.append('    GitRepo(\n        "https://github.com/x/new.git",\n        "git@github.com:x/new.git",\n        "%s",\n        "newthing"),\n' % ("a" * 40))
        return "PACKAGES = [\n" + "".join(out) + "]\n"

    def test_a_fetch_script_that_agrees_with_the_pins_is_accepted(self):
        self.assertTrue(L.check_fetch_sources(self.fetch_text(), PINS))

    def test_the_real_shape_is_parsed_by_the_same_reader(self):
        git, files = L.parse_fetch_sources(self.fetch_text())
        self.assertEqual(set(git), set(PINS["externals_git"]))
        self.assertEqual(files["zlib"][1], PINS["externals_files"]["zlib"]["sha256"])

    def test_a_moved_revision_url_or_digest_in_the_fetch_script_is_refused(self):
        for change in ({"rev_glslang": "f" * 40}, {"url_amber": "https://github.com/evil/amber.git"}, {"sha_zlib": "0" * 64}):
            with self.subTest(change), self.assertRaises(L.PinRefused):
                L.check_fetch_sources(self.fetch_text(**change), PINS)

    def test_a_changed_origin_in_the_fetch_script_is_refused(self):
        with self.assertRaises(L.PinRefused):
            L.check_fetch_sources(self.fetch_text(url_jsoncpp="https://github.com/evil/jsoncpp.git"), PINS)

    def test_a_changed_digest_of_a_file_external_is_refused(self):
        with self.assertRaises(L.PinRefused):
            L.check_fetch_sources(self.fetch_text(sha_libpng="1" * 64), PINS)

    def test_an_unpinned_external_with_a_valid_commit_is_still_refused(self):
        text = self.fetch_text(extra=True)
        self.assertIn("newthing", text)
        with self.assertRaisesRegex(L.PinRefused, "does not pin"):
            L.check_fetch_sources(text, PINS)

    def test_a_git_external_this_repository_does_not_pin_is_refused(self):
        with self.assertRaises(L.PinRefused):
            L.check_fetch_sources(self.fetch_text(extra=True), PINS)

    def test_a_pinned_external_missing_from_the_script_is_refused(self):
        text = self.fetch_text().replace('"glslang"', '"glslang2"')
        with self.assertRaises(L.PinRefused):
            L.check_fetch_sources(text, PINS)

    def test_a_ref_in_the_script_needs_an_upstream_ref_and_a_pinned_commit(self):
        with self.assertRaises(L.PinRefused):
            L.check_fetch_sources(self.fetch_text(rev_video_generator="v9.9.9"), PINS)
        with self.assertRaises(L.PinRefused):
            L.check_fetch_sources(self.fetch_text(), PINS, {"video_generator": "e" * 40})
        self.assertTrue(L.check_fetch_sources(self.fetch_text(), PINS, {"video_generator": PINS["externals_git"]["video_generator"]["commit"]}))

    def test_a_short_pin_is_refused_even_when_the_head_equals_it(self):
        for short in ("abcd", "a" * 39, "A" * 40, "g" * 40):
            with self.subTest(short), self.assertRaises(L.PinRefused):
                L.head_matches(short, short)

    def test_head_must_equal_the_pinned_commit(self):
        self.assertTrue(L.head_matches("a" * 40, "a" * 40))
        with self.assertRaises(L.PinRefused):
            L.head_matches("b" * 40, "a" * 40)
        with self.assertRaises(L.PinRefused):
            L.head_matches("a" * 40, "")

    def test_a_changed_cmake_argument_changes_the_key(self):
        p = json.loads(json.dumps(PINS))
        p["build"]["cmake_args"][0] = "-DDEQP_TARGET=default"
        self.assertNotEqual(L.content_key(p), L.content_key(PINS))

    def test_a_changed_external_commit_changes_the_key(self):
        p = json.loads(json.dumps(PINS))
        p["externals_git"]["glslang"]["commit"] = "2" * 40
        self.assertNotEqual(L.content_key(p), L.content_key(PINS))

    def test_the_content_key_follows_the_pins_and_the_build_configuration_only(self):
        k = L.content_key(PINS)
        self.assertEqual(k, L.content_key(json.loads(json.dumps(PINS))))
        p = json.loads(json.dumps(PINS))
        p["cts"]["commit"] = "1" * 40
        self.assertNotEqual(k, L.content_key(p))
        p = json.loads(json.dumps(PINS))
        p["build"]["cmake_args"] = p["build"]["cmake_args"] + ["-DX=1"]
        self.assertNotEqual(k, L.content_key(p))
        p = json.loads(json.dumps(PINS))
        p["externals_git"]["amber"]["commit"] = "3" * 40
        self.assertNotEqual(k, L.content_key(p))
        p = json.loads(json.dumps(PINS))
        p["run"]["budget_s"] = 1
        self.assertEqual(k, L.content_key(p))

    def test_the_committed_pins_agree_with_the_spec_and_the_build_is_headless_deqp_vk_only(self):
        args = PINS["build"]["cmake_args"]
        self.assertIn("-DDEQP_TARGET=vulkan_headless", args)
        self.assertIn("-DSELECTED_BUILD_TARGETS=deqp-vk", args)
        self.assertEqual(PINS["build"]["target"], "deqp-vk")
        self.assertFalse([a for a in args if "x11" in a.lower() or "wayland" in a.lower()])


class GitFetchTests(unittest.TestCase):
    """fetch_cts and verify_externals against local git repositories (no network)."""

    def git(self, cwd, *a):
        return subprocess.run(["git", "-c", "user.name=t", "-c", "user.email=t@t", "-c", "init.defaultBranch=main"] + list(a), cwd=cwd, capture_output=True, text=True, check=True).stdout.strip()

    def remote(self, tmp, content="a"):
        r = Path(tmp) / "remote"
        r.mkdir(exist_ok=True)
        if not (r / ".git").exists():
            self.git(r, "init", "-q")
        (r / "f.txt").write_text(content)
        self.git(r, "add", "-A")
        self.git(r, "commit", "-q", "-m", content)
        self.git(r, "config", "uploadpack.allowAnySHA1InWant", "true")
        return r, self.git(r, "rev-parse", "HEAD")

    def pins_for(self, remote, commit):
        p = json.loads(json.dumps(PINS))
        p["cts"].update(url="file://%s" % remote, commit=commit, tag="vulkan-cts-test")
        return p

    def test_the_checkout_ends_at_exactly_the_pinned_commit(self):
        with tempfile.TemporaryDirectory() as t:
            r, c1 = self.remote(t, "one")
            _, c2 = self.remote(t, "two")
            V.fetch_cts(Path(t) / "src", self.pins_for(r, c1), sleep=lambda s: None)
            self.assertEqual(self.git(Path(t) / "src", "rev-parse", "HEAD"), c1)
            self.assertEqual((Path(t) / "src/f.txt").read_text(), "one")

    def test_a_server_that_does_not_have_the_pinned_commit_is_refused(self):
        with tempfile.TemporaryDirectory() as t:
            r, _ = self.remote(t, "one")
            with self.assertRaises(L.PinRefused):
                V.fetch_cts(Path(t) / "src", self.pins_for(r, "c" * 40), sleep=lambda s: None)

    def test_a_tag_moved_to_another_commit_is_refused_when_the_pinned_commit_cannot_be_fetched(self):
        with tempfile.TemporaryDirectory() as t:
            r, c1 = self.remote(t, "one")
            self.git(r, "tag", "vulkan-cts-test", c1)
            with self.assertRaises(L.PinRefused):
                V.fetch_cts(Path(t) / "src", self.pins_for(r, "d" * 40), sleep=lambda s: None)

    def test_a_second_moved_tag_is_refused_the_same_way(self):
        with tempfile.TemporaryDirectory() as t:
            r, _ = self.remote(t, "one")
            _, c2 = self.remote(t, "two")
            self.git(r, "tag", "-a", "-m", "annotated", "vulkan-cts-test", c2)
            with self.assertRaises(L.PinRefused):
                V.fetch_cts(Path(t) / "src", self.pins_for(r, "e" * 40), sleep=lambda s: None)

    def test_a_tree_left_at_another_commit_is_replaced_not_trusted(self):
        with tempfile.TemporaryDirectory() as t:
            r, c1 = self.remote(t, "one")
            V.fetch_cts(Path(t) / "src", self.pins_for(r, c1), sleep=lambda s: None)
            (Path(t) / "src/f.txt").write_text("tampered")
            self.git(Path(t) / "src", "commit", "-q", "-am", "local change")
            V.fetch_cts(Path(t) / "src", self.pins_for(r, c1), sleep=lambda s: None)
            self.assertEqual((Path(t) / "src/f.txt").read_text(), "one")
            self.assertEqual(self.git(Path(t) / "src", "rev-parse", "HEAD"), c1)

    def test_externals_are_verified_by_head_and_by_file_digest(self):
        with tempfile.TemporaryDirectory() as t:
            src = Path(t)
            pins = json.loads(json.dumps(PINS))
            for name, row in pins["externals_git"].items():
                d = src / "external" / name / "src"
                d.mkdir(parents=True)
                self.git(d, "init", "-q")
                (d / "x").write_text(name)
                self.git(d, "add", "-A")
                self.git(d, "commit", "-q", "-m", name)
                row["commit"] = self.git(d, "rev-parse", "HEAD")
            data = b"zlib bytes"
            for name, row in pins["externals_files"].items():
                f = src / "external" / name / "packages"
                f.mkdir(parents=True)
                (f / row["url"].rsplit("/", 1)[-1]).write_bytes(data)
                row["sha256"] = hashlib.sha256(data).hexdigest()
            V.verify_externals(src, pins)
            bad = json.loads(json.dumps(pins))
            bad["externals_git"]["jsoncpp"]["commit"] = "9" * 40
            with self.assertRaises(L.PinRefused):
                V.verify_externals(src, bad)
            bad = json.loads(json.dumps(pins))
            bad["externals_files"]["libpng"]["sha256"] = "7" * 64
            with self.assertRaises(L.PinRefused):
                V.verify_externals(src, bad)
            shutil.rmtree(src / "external/amber/src/.git")
            with self.assertRaises(L.PinRefused):
                V.verify_externals(src, pins)

    def test_a_fetched_file_external_with_another_digest_is_refused_on_its_own(self):
        with tempfile.TemporaryDirectory() as t:
            src = Path(t)
            pins = json.loads(json.dumps(PINS))
            pins["externals_git"] = {}
            pins["externals_files"] = {"zlib": dict(pins["externals_files"]["zlib"])}
            f = src / "external/zlib/packages"
            f.mkdir(parents=True)
            (f / "zlib-1.2.13.tar.gz").write_bytes(b"x")
            pins["externals_files"]["zlib"]["sha256"] = hashlib.sha256(b"x").hexdigest()
            V.verify_externals(src, pins)
            (f / "zlib-1.2.13.tar.gz").write_bytes(b"y")
            with self.assertRaises(L.PinRefused):
                V.verify_externals(src, pins)

    def test_the_job_count_follows_cpu_and_memory(self):
        self.assertEqual(V.plan_jobs(16, 64.0, 2.0), 16)
        self.assertEqual(V.plan_jobs(16, 10.0, 2.0), 5)
        self.assertEqual(V.plan_jobs(4, 1.0, 2.0), 1)
        self.assertEqual(V.plan_jobs(16, 64.0, 2.0, override=3), 3)


# ----------------------------------------------------------------------------------------------------------------------
class CaselistTests(unittest.TestCase):
    def test_the_committed_list_parses_has_no_duplicate_and_every_name_is_well_formed(self):
        ents = entries()
        L.validate_caselist(ents, PINS["run"]["info_cases"], SPEC["required_areas"])
        names = [c for _, c in ents]
        self.assertEqual(len(names), len(set(names)))
        for n in names:
            self.assertRegex(n, r"^dEQP-VK(\.[A-Za-z0-9_-]+)+$")
            self.assertNotIn("..", n)

    def test_the_committed_list_digest_is_the_one_in_pins(self):
        self.assertEqual(L.caselist_sha256(CASELIST_TEXT), PINS["run"]["caselist_sha256"])
        self.assertRegex(PINS["run"]["caselist_sha256"], r"^[0-9a-f]{64}$")

    def test_a_changed_list_is_refused_unless_it_is_the_users_own(self):
        with tempfile.TemporaryDirectory() as t:
            p = Path(t) / "l.tsv"
            p.write_text(CASELIST_TEXT + "xx\tdEQP-VK.api.smoke.triangle\n")
            with self.assertRaises(L.PinRefused):
                V.read_caselist(p, PINS, trusted=False)
            got, sha = V.read_caselist(p, PINS, trusted=True)
            self.assertEqual(len(got), len(entries()) + 1)
            self.assertEqual(sha, L.caselist_sha256(p.read_text()))

    def test_the_list_is_a_few_hundred_cases_at_most_and_starts_with_the_info_cases(self):
        ents = entries()
        self.assertLessEqual(len(ents), L.MAX_CASES)
        self.assertGreaterEqual(len(ents), 200)
        self.assertEqual([c for _, c in ents[:2]], PINS["run"]["info_cases"])

    def test_every_gap_area_has_cases_and_none_exceeds_its_quota(self):
        count = collections.Counter(a for a, _ in entries())
        for area in SPEC["required_areas"]:
            self.assertGreaterEqual(count[area], 6, area)
        quota = collections.Counter()
        for b in SPEC["buckets"]:
            quota[b["area"]] += b["count"]
        for area, n in count.items():
            if area != "info":
                self.assertLessEqual(n, quota[area], area)

    def test_the_list_covers_the_named_driver_surfaces(self):
        names = [c for _, c in entries()]
        for needle in ("dEQP-VK.ray_tracing_pipeline.", "dEQP-VK.ray_query.", "acceleration_structures", "serialization", "compaction", "dEQP-VK.mesh_shader.ext.",
                       "task", "dEQP-VK.sparse_resources.", "dEQP-VK.video.decode.", "dEQP-VK.video.encode.", "timeline_semaphore", "dEQP-VK.synchronization2.",
                       "descriptor_buffer", "dEQP-VK.dgc.", "cooperative_matrix"):
            self.assertTrue(any(needle in n for n in names), needle)

    def test_the_header_names_the_pinned_commit(self):
        self.assertIn(PINS["cts"]["commit"], CASELIST_TEXT.splitlines()[1])

    def test_malformed_lines_are_refused(self):
        good = "aa_b\tdEQP-VK.api.smoke.triangle"
        self.assertEqual(L.read_caselist(good), [("aa_b", "dEQP-VK.api.smoke.triangle")])
        for bad in ("dEQP-VK.api.smoke.triangle", "aa\tdEQP-VK.api smoke", "aa\tdEQP-VK.api.smoke;rm -rf", "aa\tdEQP-VK..x", "aa\tdEQP-GLES2.x.y", "aa\tdEQP-VK.",
                    "aa\t$(id)", "AA\tdEQP-VK.a.b", "aa\tdEQP-VK.a.b\textra", "aa\t../dEQP-VK.a", "aa\tdEQP-VK.a.b`id`"):
            with self.subTest(bad), self.assertRaises(L.CaselistError):
                L.read_caselist(bad)

    def test_comments_and_blank_lines_are_ignored(self):
        self.assertEqual(L.read_caselist("# x\n\n  \n" + "aa\tdEQP-VK.a.b\n"), [("aa", "dEQP-VK.a.b")])

    def test_duplicates_oversize_missing_info_and_missing_area_are_refused(self):
        e = [("aa", "dEQP-VK.a.b"), ("aa", "dEQP-VK.a.b")]
        with self.assertRaises(L.CaselistError):
            L.validate_caselist(e)
        with self.assertRaises(L.CaselistError):
            L.validate_caselist([("aa", "dEQP-VK.a.c%d" % i) for i in range(L.MAX_CASES + 1)])
        with self.assertRaises(L.CaselistError):
            L.validate_caselist([("aa", "dEQP-VK.a.b")], info_cases=["dEQP-VK.info.x"])
        with self.assertRaises(L.CaselistError):
            L.validate_caselist([("aa", "dEQP-VK.a.b")], required_areas=["bb"])
        with self.assertRaises(L.CaselistError):
            L.validate_caselist([])

    def test_the_generator_is_deterministic_and_stratified(self):
        sys.path.insert(0, str(TOOL))
        import gen_caselist as G  # noqa: E402
        names = sorted("dEQP-VK.grp.%s.case%02d" % (g, i) for g in "abcde" for i in range(10))
        b = {"area": "x", "include": ["dEQP-VK.grp."], "depth": 1, "count": 5}
        got = G.select(names, b)
        self.assertEqual(got, G.select(list(reversed(names)), b))
        self.assertEqual(len({n.split(".")[2] for n in got}), 5)
        b2 = {"area": "x", "include": ["dEQP-VK.grp."], "depth": 1, "count": 12}
        got2 = G.select(names, b2)
        self.assertEqual(len(got2), 12)
        self.assertEqual(len(set(got2)), 12)
        self.assertEqual(G.select(names, {"area": "x", "include": ["dEQP-VK.grp."], "exclude": ["dEQP-VK.grp.a."], "depth": 1, "count": 4}),
                         G.select([n for n in names if ".a." not in n], {"area": "x", "include": ["dEQP-VK.grp."], "depth": 1, "count": 4}))
        self.assertEqual(G.select(names, {"area": "x", "include": ["dEQP-VK.nothing."], "depth": 1, "count": 3}), [])


# ----------------------------------------------------------------------------------------------------------------------
class QpaTests(unittest.TestCase):
    def parse(self, text, **kw):
        return L.parse_qpa(text.splitlines(True), **kw)

    def test_pass_fail_not_supported_and_warning_blocks(self):
        text = qpa(block("dEQP-VK.a.p"), block("dEQP-VK.a.f", "Fail", "image mismatch"), block("dEQP-VK.a.n", "NotSupported", "no ext"),
                   block("dEQP-VK.a.w", "QualityWarning", "meh"))
        res, open_case = self.parse(text)
        self.assertIsNone(open_case)
        self.assertEqual([(r.case, r.status) for r in res], [("dEQP-VK.a.p", "Pass"), ("dEQP-VK.a.f", "Fail"), ("dEQP-VK.a.n", "NotSupported"), ("dEQP-VK.a.w", "QualityWarning")])
        self.assertTrue(all(r.complete for r in res))
        self.assertEqual(res[1].detail, "image mismatch")

    def test_xml_escapes_in_the_detail_are_decoded(self):
        res, _ = self.parse(block("dEQP-VK.a.f", "Fail", "a &lt;b&gt; &amp; &quot;c&quot;"))
        self.assertEqual(res[0].detail, 'a <b> & "c"')

    def test_a_case_that_is_never_closed_is_a_crash_and_is_the_open_case(self):
        text = block("dEQP-VK.a.p") + "\n#beginTestCaseResult dEQP-VK.a.dies\n<TestCaseResult>\n<Text>working</Text>\n"
        res, open_case = self.parse(text)
        self.assertEqual(open_case, "dEQP-VK.a.dies")
        self.assertEqual((res[-1].case, res[-1].status, res[-1].complete), ("dEQP-VK.a.dies", "Crash", False))

    def test_a_terminate_marker_carries_its_status(self):
        for word, want in (("Crash", "Crash"), ("Timeout", "Timeout"), ("Pass", "Crash"), ("", "Crash")):
            with self.subTest(word):
                res, open_case = self.parse("\n#beginTestCaseResult dEQP-VK.a.t\n<x/>\n\n#terminateTestCaseResult %s\n" % word)
                self.assertEqual(res[0].status, want)
                self.assertIsNone(open_case)
                self.assertFalse(res[0].complete)

    def test_a_new_case_while_one_is_open_closes_the_open_one_as_a_crash(self):
        text = "\n#beginTestCaseResult dEQP-VK.a.one\n<x/>\n" + block("dEQP-VK.a.two")
        res, open_case = self.parse(text)
        self.assertEqual([(r.case, r.status) for r in res], [("dEQP-VK.a.one", "Crash"), ("dEQP-VK.a.two", "Pass")])
        self.assertIsNone(open_case)

    def test_a_block_without_a_result_element_is_a_crash_not_a_pass(self):
        res, _ = self.parse("\n#beginTestCaseResult dEQP-VK.a.x\n<TestCaseResult>\n</TestCaseResult>\n\n#endTestCaseResult\n")
        self.assertEqual(res[0].status, "Crash")

    def test_session_noise_outside_blocks_is_ignored(self):
        text = "#sessionInfo releaseName x\n#beginSession\n" + block("dEQP-VK.a.p") + "\n#endSession\n"
        res, _ = self.parse(text)
        self.assertEqual(len(res), 1)

    def test_bodies_are_kept_for_the_info_cases_and_capped_for_the_rest(self):
        big = "".join("<Text>line %d %s</Text>\n" % (i, "x" * 200) for i in range(200))
        text = block("dEQP-VK.info.device_properties", body="<Text>deviceName = X</Text>\n" + big) + block("dEQP-VK.a.f", "Fail", "bad", body=big)
        res, _ = self.parse(text, keep_bodies={"dEQP-VK.info.device_properties"}, max_body=1 << 20, default_body=2000)
        self.assertGreater(len(res[0].body), 150)
        self.assertLess(sum(len(x) for x in res[1].body), 2500)

    def test_overlong_lines_and_binary_junk_do_not_break_the_parser(self):
        text = block("dEQP-VK.a.p", body="<Text>" + "A" * 300000 + "</Text>\n") + "\x00\x01\xff junk\n" + block("dEQP-VK.a.q")
        res, _ = self.parse(text)
        self.assertEqual([r.case for r in res], ["dEQP-VK.a.p", "dEQP-VK.a.q"])

    def test_a_forged_result_in_the_body_of_another_case_is_only_data(self):
        forged = block("dEQP-VK.a.victim", "Pass", "forged")
        res, _ = self.parse("\n#beginTestCaseResult dEQP-VK.a.real\n" + forged)
        self.assertEqual([r.case for r in res], ["dEQP-VK.a.real", "dEQP-VK.a.victim"])

    def test_console_totals_are_parsed_and_cross_checked(self):
        out = "\nDONE!\n\nTest run totals:\n  Passed:        3/6 (50.0%)\n  Failed:        1/6 (16.7%)\n  Not supported: 1/6 (16.7%)\n  Warnings:      1/6 (16.7%)\n  Waived:        0/6 (0.0%)\n"
        t = L.parse_totals(out)
        self.assertEqual(t, {"passed": 3, "failed": 1, "not_supported": 1, "warnings": 1, "waived": 0, "executed": 6})
        res = [L.QpaResult("c%d" % i, s, "", True) for i, s in enumerate(["Pass", "Pass", "Pass", "Fail", "NotSupported", "QualityWarning"])]
        self.assertTrue(L.totals_agree(res, t))
        self.assertFalse(L.totals_agree(res[:-1], t))
        res[0] = L.QpaResult("c0", "Fail", "", True)
        self.assertFalse(L.totals_agree(res, t))
        self.assertIsNone(L.parse_totals("ABORTED!"))
        self.assertIsNone(L.totals_agree(res, None))


class RealOutputTests(unittest.TestCase):
    """The QPA log and the console of a real deqp-vk (vulkan-cts-1.4.7.1, RTX 3060, driver 580.159.03, 2026-11-17, traces/vkcts_host_20261117/),
    cut down: two dgc cases in full, 12 of 250 extensions and the head of the properties dump of the info cases. No GPU, no UUID."""

    QPA = (ROOT / "scripts/ci/fixtures/vkcts_real_qpa.txt").read_text()
    CONSOLE = (ROOT / "scripts/ci/fixtures/vkcts_real_console.txt").read_text()
    INFO = {"dEQP-VK.info.device_properties", "dEQP-VK.info.device_extensions"}

    def test_the_real_log_parses_to_the_real_statuses(self):
        res, open_case = L.parse_qpa(self.QPA.splitlines(True), keep_bodies=self.INFO)
        self.assertIsNone(open_case)
        got = {r.case: (r.status, r.complete) for r in res}
        self.assertEqual(got["dEQP-VK.dgc.ext.compute.layout.complementary_push_index_dispatch_shader_objects_descriptor_heap"], ("NotSupported", True))
        self.assertEqual(got["dEQP-VK.dgc.ext.compute.misc.execute_many_1024_compute_queue"], ("Pass", True))
        self.assertEqual(len(res), 4)
        self.assertIn("VK_EXT_descriptor_heap is not supported", res[0].detail)

    def test_the_real_device_properties_and_extensions_give_the_device_identity(self):
        res, _ = L.parse_qpa(self.QPA.splitlines(True), keep_bodies=self.INFO)
        body = {r.case: r.body for r in res}
        info = L.device_info(body["dEQP-VK.info.device_properties"], body["dEQP-VK.info.device_extensions"])
        self.assertEqual((info["name"], info["type"], info["vendor"], info["api"]), ("NVIDIA GeForce RTX 3060", "DISCRETE_GPU", "4318", "4211000"))
        self.assertEqual(len(info["extensions"]), 12)
        self.assertIn("VK_KHR_acceleration_structure", info["extensions"])
        self.assertTrue(L.device_is_gpu(info))

    def test_the_real_console_totals_agree_with_the_real_log(self):
        res, _ = L.parse_qpa(self.QPA.splitlines(True))
        totals = L.parse_totals(self.CONSOLE)
        self.assertEqual((totals["passed"], totals["not_supported"], totals["executed"]), (3, 1, 4))
        self.assertTrue(L.totals_agree(res, totals))
        self.assertFalse(L.totals_agree(res[:3], totals))

    def test_cases_run_in_the_trees_order_not_the_case_list_order_and_the_results_follow_the_list(self):
        order = [l.split("'")[1] for l in self.CONSOLE.splitlines() if l.startswith("Test case '")]
        self.assertEqual(order[0].split(".")[1], "dgc")  # the real binary runs the info cases last although the list names them first
        res, _ = L.parse_qpa(self.QPA.splitlines(True))
        listed = ["dEQP-VK.info.device_properties", "dEQP-VK.info.device_extensions"] + [r.case for r in res if ".dgc." in r.case]
        results, _, _ = L.execute(listed, lambda b: L.BatchOutcome(res, None, 0, False), 10, 100, lambda: 0.0, 3)
        self.assertEqual(list(results), listed)


class ClassifyTests(unittest.TestCase):
    TABLE = {"Pass": "pass", "QualityWarning": "warn", "CompatibilityWarning": "warn", "Waiver": "warn", "NotSupported": "not_supported", "Fail": "fail",
             "ResourceError": "fail", "InternalError": "fail", "DeviceLost": "fail", "Pending": "fail", "EnforceDefaultContext": "fail",
             "EnforceDefaultInstance": "fail", "Crash": "crash", "Timeout": "timeout", "NotRun": "not_run"}

    def test_every_cts_status_has_its_class(self):
        for status, cls in self.TABLE.items():
            self.assertEqual(L.classify(status), cls, status)

    def test_an_unknown_status_is_a_failure_never_a_pass(self):
        for status in ("Passed", "pass", "", "Unknown", "NotSupported ", "OK"):
            self.assertEqual(L.classify(status), "fail", status)

    def test_an_unknown_status_counts_as_fail_in_the_summary(self):
        rows = L.case_rows({"dEQP-VK.a.x": ("Weird", "d"), "dEQP-VK.a.y": ("Pass", "d")}, {"dEQP-VK.a.x": "aa", "dEQP-VK.a.y": "aa"})
        self.assertEqual(L.summarize(rows)["total"]["fail"], 1)
        self.assertEqual(L.summarize(rows)["total"]["pass"], 1)

    def test_not_supported_is_not_a_failure_and_a_pass_is_not_a_not_supported(self):
        self.assertNotIn(L.classify("NotSupported"), L.BAD_CLASSES)
        self.assertNotIn(L.classify("NotSupported"), L.OK_CLASSES)
        self.assertIn(L.classify("Pass"), L.OK_CLASSES)
        self.assertIn(L.classify("Crash"), L.BAD_CLASSES)
        self.assertIn(L.classify("Timeout"), L.BAD_CLASSES)

    def test_the_committed_table_is_the_cts_status_list(self):
        self.assertEqual(L.STATUS_CLASS, self.TABLE)

    def test_sanitize_removes_everything_that_could_forge_a_line(self):
        for evil in ("a\tb", "a\nVKCTS_RESULTS_END n=0", "a\rb", "\x1b[31m", "x" * 500, "‮", "`id`", "$(id)", "a|b"):
            out = L.sanitize(evil)
            self.assertNotRegex(out, r"[\t\n\r\x1b`$|‮]")
            self.assertLessEqual(len(out), L.DETAIL_MAX)
        self.assertEqual(L.sanitize(""), "-")


# ----------------------------------------------------------------------------------------------------------------------
def outcome(results=(), open_case=None, rc=0, killed=False, totals_ok=None):
    return L.BatchOutcome([L.QpaResult(c, s, d, comp) for c, s, d, comp in results], open_case, rc, killed, totals_ok)


class ExecuteTests(unittest.TestCase):
    CASES = ["dEQP-VK.t.c%02d" % i for i in range(10)]

    def clock(self, step=0.0):
        t = [0.0]

        def now():
            t[0] += step
            return t[0]
        return now

    def scripted(self, plan):
        """plan: {case: 'crash'|'hang'|'skip'|status}. Behaves like deqp-vk: runs the batch in order, stops at a crash or a hang."""
        calls = []

        def run_batch(batch):
            calls.append(list(batch))
            res = []
            for c in batch:
                what = plan.get(c, "Pass")
                if what == "crash":
                    res.append((c, "Crash", "x", False))
                    return outcome(res, c, rc=-11)
                if what == "hang":
                    res.append((c, "Crash", "x", False))
                    return outcome(res, c, rc=-9, killed=True)
                if what == "skip":
                    continue
                res.append((c, what, "d", True))
            return outcome(res, None, rc=0)
        run_batch.calls = calls
        return run_batch

    def test_all_pass_in_case_order_and_batches_of_the_given_size(self):
        rb = self.scripted({})
        res, _, notes = L.execute(self.CASES, rb, 4, 100, self.clock(), 3)
        self.assertEqual(list(res), self.CASES)
        self.assertTrue(all(v[0] == "Pass" for v in res.values()))
        self.assertEqual([len(b) for b in rb.calls], [4, 4, 2])
        self.assertIn("batches=3", notes)

    def test_a_crash_is_attributed_to_the_running_case_and_the_run_continues_after_it(self):
        rb = self.scripted({"dEQP-VK.t.c02": "crash"})
        res, _, _ = L.execute(self.CASES, rb, 4, 100, self.clock(), 3)
        self.assertEqual(res["dEQP-VK.t.c02"][0], "Crash")
        self.assertEqual(res["dEQP-VK.t.c01"][0], "Pass")
        self.assertEqual(res["dEQP-VK.t.c03"][0], "Pass")
        self.assertEqual(res["dEQP-VK.t.c09"][0], "Pass")
        self.assertNotIn("dEQP-VK.t.c02", rb.calls[1])
        self.assertEqual(rb.calls[1][0], "dEQP-VK.t.c03")

    def test_a_hang_killed_by_the_harness_is_a_timeout_not_a_crash(self):
        rb = self.scripted({"dEQP-VK.t.c05": "hang"})
        res, _, _ = L.execute(self.CASES, rb, 4, 100, self.clock(), 3)
        self.assertEqual(res["dEQP-VK.t.c05"][0], "Timeout")
        self.assertEqual(res["dEQP-VK.t.c06"][0], "Pass")

    def test_every_crash_is_counted_and_many_crashes_do_not_stop_a_run_that_progresses(self):
        rb = self.scripted({c: "crash" for c in self.CASES[::2]})
        res, _, _ = L.execute(self.CASES, rb, 3, 100, self.clock(), 1)
        self.assertEqual(sum(1 for v in res.values() if v[0] == "Crash"), 5)
        self.assertEqual(sum(1 for v in res.values() if v[0] == "Pass"), 5)

    def test_a_wedged_device_that_gives_no_result_stops_after_the_idle_limit(self):
        n = [0]

        def run_batch(batch):
            n[0] += 1
            return outcome([], None, rc=-9, killed=True)
        res, _, notes = L.execute(self.CASES, run_batch, 4, 1000, self.clock(), 2)
        self.assertEqual(n[0], 3)
        self.assertTrue(all(v[0] == "NotRun" for v in res.values()))
        self.assertIn("stopped=idle", notes)

    def test_the_budget_is_checked_before_every_batch_and_the_rest_is_not_run(self):
        rb = self.scripted({})
        res, _, notes = L.execute(self.CASES, rb, 3, 10, self.clock(step=4.0), 3)
        self.assertLess(len(rb.calls), 4)
        self.assertEqual(res[self.CASES[-1]][0], "NotRun")
        self.assertIn("stopped=budget", notes)
        self.assertIn("budget", res[self.CASES[-1]][1])

    def test_a_clean_exit_that_leaves_a_case_without_a_result_marks_it_not_run_and_ends(self):
        rb = self.scripted({"dEQP-VK.t.c01": "skip"})
        res, _, _ = L.execute(self.CASES, rb, 4, 100, self.clock(), 3)
        self.assertEqual(res["dEQP-VK.t.c01"][0], "NotRun")
        self.assertEqual(res["dEQP-VK.t.c02"][0], "Pass")
        self.assertLessEqual(len(rb.calls), 3)

    def test_results_for_cases_that_were_not_asked_for_are_ignored(self):
        def run_batch(batch):
            return outcome([(c, "Pass", "", True) for c in batch] + [("dEQP-VK.other.x", "Fail", "", True)], None)
        res, _, notes = L.execute(self.CASES, run_batch, 10, 100, self.clock(), 3)
        self.assertEqual(list(res), self.CASES)
        self.assertIn("unexpected case in log", notes)

    def test_a_spent_budget_runs_nothing_and_marks_every_case_not_run(self):
        rb = self.scripted({})
        res, _, notes = L.execute(self.CASES, rb, 4, 0, self.clock(), 3)
        self.assertEqual(rb.calls, [])
        self.assertTrue(all(v[0] == "NotRun" for v in res.values()))
        self.assertIn("stopped=budget", notes)

    def test_the_idle_limit_is_the_argument_given(self):
        n = [0]

        def run_batch(batch):
            n[0] += 1
            return outcome([], None, rc=-9, killed=True)
        L.execute(self.CASES, run_batch, 4, 1000, self.clock(), 0)
        self.assertEqual(n[0], 1)

    def test_progress_resets_the_idle_counter_so_a_flaky_start_does_not_end_the_run(self):
        state = {"n": 0}

        def run_batch(batch):
            state["n"] += 1
            if state["n"] % 2 == 1:
                return outcome([], None, rc=-9, killed=True)
            return outcome([(c, "Pass", "", True) for c in batch[:2]], None, rc=-11)
        res, _, _ = L.execute(self.CASES, run_batch, 3, 1000, self.clock(), 1)
        self.assertTrue(all(v[0] == "Pass" for v in res.values()), res)

    def test_two_wedged_starts_then_progress_is_still_within_a_limit_of_two(self):
        seq = iter([False, False, True, False, False, True, True, True, True, True, True, True])

        def run_batch(batch):
            if next(seq, True):
                return outcome([(c, "Pass", "", True) for c in batch[:1]], None, rc=-11)
            return outcome([], None, rc=-9, killed=True)
        res, _, _ = L.execute(self.CASES[:4], run_batch, 2, 1000, self.clock(), 2)
        self.assertTrue(all(v[0] == "Pass" for v in res.values()), res)

    def test_an_open_case_without_a_parsed_result_is_still_attributed(self):
        def run_batch(batch):
            return L.BatchOutcome([], batch[0], -11, False) if batch[0] == self.CASES[0] else outcome([(c, "Pass", "", True) for c in batch], None)
        res, _, _ = L.execute(self.CASES[:3], run_batch, 3, 100, self.clock(), 3)
        self.assertEqual(res[self.CASES[0]][0], "Crash")
        killed = L.execute(self.CASES[:1], lambda b: L.BatchOutcome([], b[0], -9, True), 3, 100, self.clock(), 3)[0]
        self.assertEqual(killed[self.CASES[0]][0], "Timeout")

    def test_an_open_case_without_a_parsed_result_that_was_killed_is_a_timeout(self):
        res, _, _ = L.execute(self.CASES[:2], lambda b: L.BatchOutcome([], b[0], -9, True), 3, 100, self.clock(), 0)
        self.assertEqual(res[self.CASES[0]][0], "Timeout")

    def test_the_first_result_of_a_case_wins(self):
        def run_batch(batch):
            return outcome([(batch[0], "Pass", "", True), (batch[0], "Fail", "", True)] + [(c, "Pass", "", True) for c in batch[1:]], None)
        res, _, _ = L.execute(self.CASES[:3], run_batch, 3, 100, self.clock(), 3)
        self.assertEqual(res[self.CASES[0]][0], "Pass")

    def test_console_totals_that_disagree_are_noted(self):
        def run_batch(batch):
            return outcome([(c, "Pass", "", True) for c in batch], None, totals_ok=False)
        _, _, notes = L.execute(self.CASES[:2], run_batch, 2, 100, self.clock(), 3)
        self.assertIn("console_totals_disagree", notes)

    def test_info_and_failure_bodies_are_returned_passing_bodies_are_not(self):
        def run_batch(batch):
            return L.BatchOutcome([L.QpaResult("dEQP-VK.i.x", "Pass", "", True, ["deviceName = A"]), L.QpaResult("dEQP-VK.t.c00", "Fail", "bad", True, ["why"]),
                                                                  L.QpaResult("dEQP-VK.t.c01", "Pass", "", True, ["noise"])], None, 0, False)
        _, bodies, _ = L.execute(["dEQP-VK.i.x", "dEQP-VK.t.c00", "dEQP-VK.t.c01"], run_batch, 3, 100, self.clock(), 3, keep_bodies={"dEQP-VK.i.x"})
        self.assertEqual(set(bodies), {"dEQP-VK.i.x", "dEQP-VK.t.c00"})


class DeviceTests(unittest.TestCase):
    PROPS = ["VkPhysicalDeviceProperties = {", "  apiVersion = 0x00404138", "  driverVersion = 0x9127c0c0", "  vendorID = 0x10de",
             "  deviceType = VK_PHYSICAL_DEVICE_TYPE_DISCRETE_GPU", "  deviceName = NVIDIA GeForce RTX 3060"]
    EXTS = ["extensionName = VK_KHR_ray_query, specVersion = 1", "extensionName = VK_KHR_swapchain, specVersion = 70", "extensionName = VK_KHR_ray_query, specVersion = 1"]

    def test_device_identity_and_extension_set(self):
        info = L.device_info(self.PROPS, self.EXTS)
        self.assertEqual((info["name"], info["type"], info["vendor"]), ("NVIDIA GeForce RTX 3060", "DISCRETE_GPU", "0x10de"))
        self.assertEqual(info["extensions"], ["VK_KHR_ray_query", "VK_KHR_swapchain"])
        self.assertTrue(L.device_is_gpu(info))

    def test_a_software_rasteriser_or_a_cpu_device_is_not_a_gpu(self):
        for typ, name in (("CPU", "llvmpipe (LLVM 15.0.7, 256 bits)"), ("DISCRETE_GPU", "llvmpipe"), ("DISCRETE_GPU", "SwiftShader Device"), ("OTHER", "x"), ("-", "-")):
            self.assertFalse(L.device_is_gpu({"type": typ, "name": name}), (typ, name))
        self.assertTrue(L.device_is_gpu({"type": "INTEGRATED_GPU", "name": "Intel(R) Graphics"}))

    def test_missing_info_gives_dashes_and_not_a_gpu(self):
        info = L.device_info(None, None)
        self.assertEqual(info["name"], "-")
        self.assertFalse(L.device_is_gpu(info))


# ----------------------------------------------------------------------------------------------------------------------
def rows_of(spec):
    """spec: {case: status} -> result rows with area 'a'."""
    return [(c, s, L.classify(s), "a", "d") for c, s in spec.items()]


class BlockTests(unittest.TestCase):
    def test_a_block_round_trips_with_count_and_digest(self):
        rows = rows_of({"dEQP-VK.a.p": "Pass", "dEQP-VK.a.f": "Fail", "dEQP-VK.a.n": "NotSupported"})
        side, back, dg = L.parse_block("noise\n" + L.render_block("host", rows) + "tail\n")
        self.assertEqual((side, back, dg), ("host", rows, L.digest(rows)))

    def test_the_digest_ignores_details_and_order_but_not_classes(self):
        a = rows_of({"dEQP-VK.a.p": "Pass", "dEQP-VK.a.f": "Fail"})
        b = [(c, s, k, ar, "other detail") for c, s, k, ar, _ in reversed(a)]
        self.assertEqual(L.digest(a), L.digest(b))
        self.assertNotEqual(L.digest(a), L.digest(rows_of({"dEQP-VK.a.p": "Pass", "dEQP-VK.a.f": "NotSupported"})))

    def render_lines(self):
        return L.render_block("guest", rows_of({"dEQP-VK.a.p": "Pass", "dEQP-VK.a.f": "Fail"})).splitlines()

    def test_a_block_with_a_wrong_count_is_refused(self):
        with self.assertRaises(L.CaselistError):
            L.parse_block("\n".join(self.render_lines()).replace("n=2", "n=1") + "\n")

    def test_a_block_with_a_wrong_digest_is_refused(self):
        with self.assertRaises(L.CaselistError):
            L.parse_block(re.sub(r"digest=[0-9a-f]{16}", "digest=" + "f" * 16, "\n".join(self.render_lines())) + "\n")

    def test_the_digest_does_not_depend_on_areas_or_details(self):
        a = [("dEQP-VK.a.p", "Pass", "pass", "xx", "one")]
        b = [("dEQP-VK.a.p", "Pass", "pass", "yy", "two")]
        self.assertEqual(L.digest(a), L.digest(b))

    def test_every_tampering_of_a_block_is_refused(self):
        lines = self.render_lines()
        cases = {
            "no end": lines[:-1],
            "row removed": lines[:2] + lines[3:],
            "status changed": [l.replace("\tFail\tfail\t", "\tPass\tfail\t") for l in lines],
            "class changed": [l.replace("\tFail\tfail\t", "\tFail\tpass\t") for l in lines],
            "count changed": [l.replace("n=2", "n=3") for l in lines],
            "digest changed": [re.sub(r"digest=[0-9a-f]{16}", "digest=" + "0" * 16, l) for l in lines],
            "unknown status": [l.replace("\tFail\tfail\t", "\tBogus\tfail\t") for l in lines],
            "bad name": [l.replace("dEQP-VK.a.f", "dEQP-VK.a f") for l in lines],
            "short row": [l if "dEQP" not in l else l.rsplit("\t", 1)[0] for l in lines],
            "duplicate": lines[:3] + [lines[3]] + lines[3:],
        }
        for what, ls in cases.items():
            with self.subTest(what), self.assertRaises(L.CaselistError):
                L.parse_block("\n".join(ls) + "\n")

    def test_a_case_outside_the_expected_list_is_refused_and_no_block_is_refused(self):
        text = L.render_block("host", rows_of({"dEQP-VK.a.p": "Pass"}))
        with self.assertRaises(L.CaselistError):
            L.parse_block(text, expected_cases={"dEQP-VK.a.q"})
        self.assertEqual(L.parse_block(text, expected_cases={"dEQP-VK.a.p"})[0], "host")
        with self.assertRaises(L.CaselistError):
            L.parse_block("nothing here\n")

    def test_details_cannot_forge_markers_or_columns(self):
        rows = [("dEQP-VK.a.f", "Fail", "fail", "a", "x\nVKCTS_RESULTS_END n=0 digest=" + "0" * 16 + "\tmore")]
        text = L.render_block("host", rows)
        self.assertEqual(text.count("VKCTS_RESULTS_END"), 1)
        L.parse_block(text)

    def test_render_block_never_emits_a_tab_or_newline_or_markup_inside_a_detail(self):
        rows = [("dEQP-VK.a.f", "Fail", "fail", "aa", "one\ttwo\nthree <b> `x` $(y) |z")]
        line = [l for l in L.render_block("host", rows).splitlines() if l.startswith("dEQP")][0]
        self.assertEqual(len(line.split("\t")), 5)
        for ch in "<>`$|":
            self.assertNotIn(ch, line)

    def test_the_summary_counts_by_class_and_area(self):
        rows = [("dEQP-VK.a.%d" % i, s, L.classify(s), "x" if i < 2 else "y", "-") for i, s in enumerate(["Pass", "Fail", "NotSupported", "Crash"])]
        s = L.summarize(rows)
        self.assertEqual(s["total"]["pass"], 1)
        self.assertEqual(s["total"]["fail"], 1)
        self.assertEqual(s["total"]["not_supported"], 1)
        self.assertEqual(s["total"]["crash"], 1)
        self.assertEqual(s["areas"]["x"]["fail"], 1)
        self.assertEqual(s["areas"]["y"]["crash"], 1)
        line = L.summary_line("host", rows, {"name": "NVIDIA GeForce RTX 3060", "type": "DISCRETE_GPU"}, "PASS")
        self.assertIn("cases=4 pass=1 warn=0 not_supported=1 fail=1 crash=1 timeout=0 not_run=0", line)
        self.assertIn("device=NVIDIA_GeForce_RTX_3060", line)
        self.assertIn("gpu_use=PASS", line)


class DiffTests(unittest.TestCase):
    EXPECT = {
        ("pass", "pass"): "SAME_OK", ("pass", "warn"): "SAME_OK", ("warn", "pass"): "SAME_OK",
        ("not_supported", "not_supported"): "SAME_NOT_SUPPORTED",
        ("fail", "fail"): "BOTH_FAIL", ("crash", "crash"): "BOTH_FAIL", ("timeout", "timeout"): "BOTH_FAIL",
        ("fail", "crash"): "BOTH_FAIL_DIFFERENT", ("crash", "timeout"): "BOTH_FAIL_DIFFERENT",
        ("pass", "fail"): "GUEST_ONLY_FAIL", ("pass", "crash"): "GUEST_ONLY_FAIL", ("warn", "timeout"): "GUEST_ONLY_FAIL",
        ("pass", "not_supported"): "GUEST_LOSES_SUPPORT", ("warn", "not_supported"): "GUEST_LOSES_SUPPORT",
        ("not_supported", "pass"): "GUEST_GAINS_SUPPORT", ("not_supported", "warn"): "GUEST_GAINS_SUPPORT",
        ("not_supported", "fail"): "GUEST_FAIL_HOST_NS", ("not_supported", "crash"): "GUEST_FAIL_HOST_NS",
        ("fail", "pass"): "HOST_ONLY_FAIL", ("crash", "warn"): "HOST_ONLY_FAIL",
        ("fail", "not_supported"): "HOST_FAIL_GUEST_NOT_SUPPORTED", ("timeout", "not_supported"): "HOST_FAIL_GUEST_NOT_SUPPORTED",
        ("not_run", "pass"): "INCOMPLETE", ("pass", "not_run"): "INCOMPLETE", ("not_run", "not_run"): "INCOMPLETE", ("fail", "not_run"): "INCOMPLETE",
    }

    def test_the_comparison_table(self):
        for (h, g), want in self.EXPECT.items():
            self.assertEqual(L.categorize(h, g), want, (h, g))

    def test_every_pair_of_classes_has_a_category(self):
        for h in L.CLASSES:
            for g in L.CLASSES:
                self.assertIn(L.categorize(h, g), L.CATEGORIES, (h, g))

    def test_a_case_that_fails_on_both_sides_is_never_a_candidate(self):
        for h in L.BAD_CLASSES:
            for g in L.BAD_CLASSES:
                self.assertNotIn(L.categorize(h, g), L.CANDIDATES)

    def test_a_host_pass_guest_fail_is_a_candidate_and_so_is_a_lost_feature(self):
        self.assertIn("GUEST_ONLY_FAIL", L.CANDIDATES)
        self.assertIn("GUEST_LOSES_SUPPORT", L.CANDIDATES)
        self.assertNotIn("HOST_ONLY_FAIL", L.CANDIDATES)
        self.assertNotIn("INCOMPLETE", L.CANDIDATES)

    def test_diff_rows_counts_candidates_and_areas(self):
        host = rows_of({"dEQP-VK.a.1": "Pass", "dEQP-VK.a.2": "Pass", "dEQP-VK.a.3": "Fail", "dEQP-VK.a.4": "NotSupported", "dEQP-VK.a.5": "Pass"})
        guest = rows_of({"dEQP-VK.a.1": "Pass", "dEQP-VK.a.2": "Fail", "dEQP-VK.a.3": "Fail", "dEQP-VK.a.4": "NotSupported", "dEQP-VK.a.5": "NotSupported"})
        rows, summary = L.diff_rows(host, guest)
        self.assertEqual([r[4] for r in rows], ["SAME_OK", "GUEST_ONLY_FAIL", "BOTH_FAIL", "SAME_NOT_SUPPORTED", "GUEST_LOSES_SUPPORT"])
        self.assertEqual(summary["candidates"], 2)
        self.assertEqual(summary["categories"]["BOTH_FAIL"], 1)
        self.assertEqual(summary["areas"]["a"]["GUEST_ONLY_FAIL"], 1)

    def test_a_case_on_one_side_only_is_incomplete_never_a_pass(self):
        rows, summary = L.diff_rows(rows_of({"dEQP-VK.a.1": "Pass"}), rows_of({"dEQP-VK.a.2": "Pass"}))
        self.assertEqual({r[0]: r[4] for r in rows}, {"dEQP-VK.a.1": "INCOMPLETE", "dEQP-VK.a.2": "INCOMPLETE"})
        self.assertEqual(summary["categories"]["SAME_OK"], 0)

    def test_a_case_missing_in_the_guest_is_incomplete_not_same_and_not_a_candidate(self):
        rows, summary = L.diff_rows(rows_of({"dEQP-VK.a.1": "Pass", "dEQP-VK.a.2": "Pass"}), rows_of({"dEQP-VK.a.1": "Pass"}))
        self.assertEqual(summary["categories"]["SAME_OK"], 1)
        self.assertEqual(summary["categories"]["INCOMPLETE"], 1)
        self.assertEqual(summary["candidates"], 0)

    def test_the_diff_text_lists_only_the_differences_and_has_one_summary_line(self):
        rows, summary = L.diff_rows(rows_of({"dEQP-VK.a.1": "Pass", "dEQP-VK.a.2": "Pass"}), rows_of({"dEQP-VK.a.1": "Pass", "dEQP-VK.a.2": "Crash"}))
        lines = L.diff_lines(rows, summary, "h" * 16, "g" * 16)
        self.assertEqual(len([l for l in lines if l.startswith("dEQP")]), 1)
        self.assertEqual(len([l for l in lines if l.startswith("VKCTS_DIFF ")]), 1)
        self.assertIn("candidates=1", lines[-1])
        self.assertIn("guest_only_fail=1", lines[-1])

    def test_the_extension_difference_names_what_the_guest_lacks(self):
        self.assertEqual(L.extension_diff({"extensions": ["A", "B"]}, {"extensions": ["B", "C"]}), (["A"], ["C"]))


# ----------------------------------------------------------------------------------------------------------------------
class RunIntegrationTests(unittest.TestCase):
    """vkcts.py run against a stand-in deqp-vk that writes QPA the way the real one does."""

    CASES = ["dEQP-VK.m.c%02d" % i for i in range(8)]

    def run_row(self, plan, tmp, side="guest", census="none", cases=None, pins=None, env=None, extra=()):
        art = make_artifact(tmp)
        cl = caselist_file(tmp, cases or self.CASES)
        out = Path(tmp) / "out"
        e = {"FAKE_PLAN": json.dumps(plan)}
        e.update(env or {})
        p = run_tool(["run", "--side", side, "--artifact", str(art), "--out", str(out), "--caselist", str(cl), "--census", census,
                      "--pins", str(pins or small_pins(tmp))] + list(extra), env=e)
        return p, out

    def test_a_clean_run_prints_the_summary_the_block_and_the_digest(self):
        with tempfile.TemporaryDirectory() as t:
            p, out = self.run_row({"cases": {self.CASES[1]: "Fail", self.CASES[2]: "NotSupported", self.CASES[3]: "Warn"}}, t)
            self.assertEqual(p.returncode, 0, p.stdout + p.stderr)
            self.assertRegex(p.stdout, r"VKCTS_SUMMARY side=guest cases=10 pass=7 warn=1 not_supported=1 fail=1 crash=0 timeout=0 not_run=0 device=NVIDIA_GeForce_RTX_3060 type=DISCRETE_GPU gpu_use=EXTERNAL")
            side, rows, dg = L.parse_block(p.stdout)
            self.assertEqual(side, "guest")
            self.assertEqual(len(rows), 10)
            self.assertIn("DIGEST vkcts_cases %s" % dg, p.stdout)
            self.assertEqual({r[0]: r[1] for r in rows}[self.CASES[1]], "Fail")
            self.assertIn("VKCTS_DEVICE side=guest name=NVIDIA_GeForce_RTX_3060 type=DISCRETE_GPU vendor=0x10de", p.stdout)
            self.assertIn("VK_KHR_ray_query", p.stdout)
            self.assertTrue((out / "results.tsv").is_file())
            self.assertNotIn("console_totals_disagree", p.stdout)
            self.assertRegex(p.stdout, r"VKCTS_DETAIL case=%s .*image mismatch" % re.escape(self.CASES[1]))

    def test_a_crash_and_a_hang_are_attributed_and_the_rest_still_runs(self):
        with tempfile.TemporaryDirectory() as t:
            p, _ = self.run_row({"cases": {self.CASES[1]: "CRASH", self.CASES[4]: "HANG", self.CASES[6]: "TERMINATE"}}, t)
            self.assertEqual(p.returncode, 0, p.stdout + p.stderr)
            _, rows, _ = L.parse_block(p.stdout)
            st = {r[0]: r[1] for r in rows}
            self.assertEqual(st[self.CASES[1]], "Crash")
            self.assertEqual(st[self.CASES[4]], "Timeout")
            self.assertEqual(st[self.CASES[6]], "Crash")
            self.assertEqual(st[self.CASES[0]], "Pass")
            self.assertEqual(st[self.CASES[7]], "Pass")
            self.assertRegex(p.stdout, r"crash=2 timeout=1")

    def test_a_terminated_timeout_a_block_without_result_and_an_unknown_case_are_classified(self):
        with tempfile.TemporaryDirectory() as t:
            p, _ = self.run_row({"cases": {self.CASES[0]: "TERMINATE_TIMEOUT", self.CASES[3]: "BADEND", self.CASES[5]: "UNKNOWN"}}, t)
            self.assertEqual(p.returncode, 0, p.stdout + p.stderr)
            _, rows, _ = L.parse_block(p.stdout)
            st = {r[0]: r[1] for r in rows}
            self.assertEqual(st[self.CASES[0]], "Timeout")
            self.assertEqual(st[self.CASES[3]], "Crash")
            self.assertEqual(st[self.CASES[5]], "NotRun")
            self.assertIn("no result from a clean run", {r[0]: r[4] for r in rows}[self.CASES[5]])
            self.assertEqual(st[self.CASES[6]], "Pass")

    def test_a_forged_detail_never_reaches_the_block_unsanitised(self):
        with tempfile.TemporaryDirectory() as t:
            p, _ = self.run_row({"cases": {self.CASES[1]: "Fail"}}, t)
            _, rows, _ = L.parse_block(p.stdout)
            detail = {r[0]: r[4] for r in rows}[self.CASES[1]]
            self.assertNotIn("<", detail)
            self.assertNotIn("&", detail)

    def test_a_cpu_device_type_fails_even_with_a_gpu_like_name(self):
        with tempfile.TemporaryDirectory() as t:
            p, _ = self.run_row({"device": "NVIDIA GeForce RTX 3060", "type": "CPU"}, t)
            self.assertEqual(p.returncode, V.EXIT_DEVICE)

    def test_a_software_name_fails_even_with_a_gpu_type(self):
        with tempfile.TemporaryDirectory() as t:
            p, _ = self.run_row({"device": "llvmpipe (LLVM 15)", "type": "DISCRETE_GPU"}, t)
            self.assertEqual(p.returncode, V.EXIT_DEVICE)

    def test_a_missing_artifact_is_not_run_not_a_failure_of_the_gpu(self):
        with tempfile.TemporaryDirectory() as t:
            p = run_tool(["run", "--side", "guest", "--artifact", str(Path(t) / "nope"), "--out", str(Path(t) / "o"), "--caselist", str(caselist_file(t, self.CASES)),
                          "--census", "none"])
            self.assertEqual(p.returncode, V.EXIT_NOTRUN)
            self.assertIn("No such file or directory", p.stdout)
            self.assertNotIn("VKCTS_SUMMARY", p.stdout)

    def test_a_software_rasteriser_fails_the_identity_assertion(self):
        with tempfile.TemporaryDirectory() as t:
            p, _ = self.run_row({"device": "llvmpipe (LLVM 15.0.7, 256 bits)", "type": "CPU"}, t)
            self.assertEqual(p.returncode, V.EXIT_DEVICE)
            self.assertIn("VKCTS_CONFIG device is not a real GPU", p.stdout)
            self.assertIn("VKCTS_SUMMARY", p.stdout)

    def test_a_user_caselist_is_trusted_and_the_committed_one_is_checked(self):
        with tempfile.TemporaryDirectory() as t:
            art = make_artifact(t)
            pins = json.loads((TOOL / "pins.json").read_text())
            pins["run"]["caselist_sha256"] = "0" * 64
            pp = Path(t) / "bad.json"
            pp.write_text(json.dumps(pins))
            p = run_tool(["run", "--side", "guest", "--artifact", str(art), "--out", str(Path(t) / "o"), "--census", "none", "--pins", str(pp)])
            self.assertNotEqual(p.returncode, 0)
            self.assertIn("PinRefused", p.stderr)

    def test_the_host_run_asserts_gpu_use_from_the_host_counters(self):
        with tempfile.TemporaryDirectory() as t:
            write_exec(Path(t) / "bin/nvidia-smi", FAKE_SMI)
            env = {"PATH": "%s:%s" % (Path(t) / "bin", FAKE_ENV["PATH"]), "FAKE_SMI": "busy"}
            p, out = self.run_row({}, t, side="host", census="host", env=env)
            self.assertEqual(p.returncode, 0, p.stdout + p.stderr)
            self.assertRegex(p.stdout, r"VKCTS_GPU side=host source=host verdict=PASS")
            self.assertIn("gpu_use=PASS", p.stdout)
            self.assertEqual(json.loads((out / "census/gpu.json").read_text())["verdict"], "PASS")

    def test_an_idle_gpu_fails_the_row_and_names_the_reason(self):
        with tempfile.TemporaryDirectory() as t:
            write_exec(Path(t) / "bin/nvidia-smi", FAKE_SMI)
            env = {"PATH": "%s:%s" % (Path(t) / "bin", FAKE_ENV["PATH"]), "FAKE_SMI": "idle"}
            p, _ = self.run_row({}, t, side="host", census="host", env=env)
            self.assertEqual(p.returncode, V.EXIT_GPU_UNUSED)
            self.assertRegex(p.stdout, r"VKCTS_GPU side=host source=host verdict=FAIL")
            self.assertIn("gpu engine idle", p.stdout.replace("_", " "))
            self.assertIn("VKCTS_CONFIG GPU use was not shown", p.stdout)

    def test_a_host_run_without_nvidia_smi_cannot_claim_gpu_use(self):
        with tempfile.TemporaryDirectory() as t:
            p, _ = self.run_row({}, t, side="host", census="host", env={"PATH": "/usr/bin:/bin"})
            if shutil.which("nvidia-smi", path="/usr/bin:/bin"):
                self.skipTest("a real nvidia-smi is on PATH")
            self.assertEqual(p.returncode, V.EXIT_GPU_UNUSED)
            self.assertIn("no_nvidia-smi", p.stdout)

    def test_census_stop_judges_sampled_files_from_a_directory(self):
        with tempfile.TemporaryDirectory() as t:
            d = Path(t)
            (d / "host_query.csv").write_text("".join("2026/11/17 10:00:%02d.000, 60, 10, %d, 0, 0\n" % (i, 1000 + 100 * i) for i in range(6)))
            (d / "host_dmon.txt").write_text("# gpu sm mem enc dec\n# Idx % % % %\n 0 50 10 0 0\n 0 50 10 0 0\n")
            rec = V.census_stop(d, "guest")
            self.assertEqual(rec["verdict"], "PASS")
            self.assertEqual(rec["source"], "host")
            (d / "host_query.csv").write_text("".join("2026/11/17 10:00:%02d.000, 0, 0, 1000, 0, 0\n" % i for i in range(6)))
            (d / "host_dmon.txt").write_text("# gpu sm mem enc dec\n# Idx % % % %\n 0 0 0 0 0\n")
            rec = V.census_stop(d, "guest")
            self.assertEqual(rec["verdict"], "FAIL")
            line = V.gpu_line("guest", rec)
            self.assertIn("verdict=FAIL", line)
            self.assertIn("source=host", line)
            (d / "host_query.csv").write_text("")
            (d / "host_dmon.txt").write_text("")
            self.assertEqual(V.census_stop(d, "guest")["verdict"], "FAIL")


class DiffCommandTests(unittest.TestCase):
    CASES = ["dEQP-VK.m.c%02d" % i for i in range(6)]

    def log(self, tmp, name, side, plan):
        art = make_artifact(Path(tmp) / name)
        cl = caselist_file(Path(tmp) / name, self.CASES)
        p = run_tool(["run", "--side", side, "--artifact", str(art), "--out", str(Path(tmp) / name / "o"), "--caselist", str(cl), "--census", "none",
                      "--pins", str(small_pins(Path(tmp) / name))], env={"FAKE_PLAN": json.dumps(plan)})
        self.assertEqual(p.returncode, 0, p.stdout + p.stderr)
        f = Path(tmp) / ("vkcts.%s.log" % side)
        f.write_text("=== vkcts side=%s start\n" % side + p.stdout + "=== end rc=0\n")
        return f

    def test_the_results_directory_form_writes_the_diff_and_the_summary(self):
        with tempfile.TemporaryDirectory() as t:
            for n in ("h", "g"):
                (Path(t) / n).mkdir()
            self.log(t, "h", "host", {"exts": ["VK_KHR_ray_query", "VK_EXT_mesh_shader"]})
            self.log(t, "g", "guest", {"cases": {self.CASES[1]: "Fail", self.CASES[2]: "NotSupported"}, "exts": ["VK_KHR_ray_query"]})
            (Path(t) / "vkcts.guest.gpu.txt").write_text("VKCTS_GPU side=guest source=host verdict=PASS samples=9 busy_samples=5 peak_gpu_pct=40 mem_rise_mb=300 reasons=-\n")
            p = run_tool(["diff", t])
            self.assertEqual(p.returncode, 0, p.stdout + p.stderr)
            self.assertRegex(p.stdout, r"VKCTS_DIFF cases=8 candidates=2 .*guest_only_fail=1 guest_loses_support=1")
            self.assertIn("%s\tmini\tPass\tFail\tGUEST_ONLY_FAIL" % self.CASES[1], p.stdout)
            self.assertIn("VKCTS_EXT_DIFF host_only=VK_EXT_mesh_shader guest_only=-", p.stdout)
            self.assertIn("VKCTS_GPU side=guest source=host verdict=PASS", p.stdout)
            self.assertIn("VKCTS_AREA mini ", p.stdout)
            self.assertTrue((Path(t) / "vkcts_diff.tsv").is_file())
            self.assertEqual(run_tool(["diff", t, "--strict"]).returncode, 1)

    def test_strict_exits_one_when_there_is_a_candidate(self):
        with tempfile.TemporaryDirectory() as t:
            for n in ("h", "g"):
                (Path(t) / n).mkdir()
            self.log(t, "h", "host", {})
            self.log(t, "g", "guest", {"cases": {self.CASES[0]: "Crash"}})
            self.assertEqual(run_tool(["diff", t, "--strict"]).returncode, 1)
            self.assertEqual(run_tool(["diff", t]).returncode, 0)

    def test_identical_runs_have_no_candidates_and_strict_exits_zero(self):
        with tempfile.TemporaryDirectory() as t:
            for n in ("h", "g"):
                (Path(t) / n).mkdir()
            self.log(t, "h", "host", {"cases": {self.CASES[0]: "Fail"}})
            self.log(t, "g", "guest", {"cases": {self.CASES[0]: "Fail"}})
            p = run_tool(["diff", t, "--strict"])
            self.assertEqual(p.returncode, 0, p.stdout)
            self.assertIn("candidates=0", p.stdout)
            self.assertIn("both_fail=1", p.stdout)

    def test_a_log_without_a_valid_block_is_a_bad_input_exit(self):
        with tempfile.TemporaryDirectory() as t:
            (Path(t) / "vkcts.host.log").write_text("nothing\n")
            (Path(t) / "vkcts.guest.log").write_text("nothing\n")
            p = run_tool(["diff", t])
            self.assertEqual(p.returncode, V.EXIT_BAD_INPUT)
            self.assertIn("VKCTS_DIFF_ERROR", p.stdout)
            self.assertEqual(run_tool(["diff", str(Path(t) / "missing")]).returncode, V.EXIT_BAD_INPUT)


# ----------------------------------------------------------------------------------------------------------------------
class MatrixRowTests(unittest.TestCase):
    """The row inside scripts/apps/run_apps.sh: a discovery row, never in `all`, never in a gate list."""

    def bash(self, args, root, env=None):
        e = dict(FAKE_ENV)
        e["APPS_ROOT"] = str(root)
        e.update(env or {})
        return subprocess.run(["bash", str(ROOT / "scripts/apps/run_apps.sh")] + args, env=e, capture_output=True, text=True, timeout=180)

    def test_the_row_is_listed_but_not_in_all_and_not_in_any_gate_list(self):
        with tempfile.TemporaryDirectory() as t:
            self.assertIn("vkcts", self.bash(["x", "list"], t).stdout.split())
            self.assertNotIn("vkcts", self.bash(["x", "listall"], t).stdout.split())
            self.assertIn("nvidia_smi", self.bash(["x", "listall"], t).stdout.split())
        for f in ("gate_list.txt", "gate_list_middle.txt"):
            self.assertNotIn("vkcts", (ROOT / "scripts/apps" / f).read_text().split())

    def test_apps_matrix_expands_all_from_listall_which_is_list_minus_the_discovery_row(self):
        with tempfile.TemporaryDirectory() as t:
            full = self.bash(["x", "list"], t).stdout.split()
            allnames = self.bash(["x", "listall"], t).stdout.split()
            self.assertEqual([n for n in full if n != "vkcts"], allnames)

    def test_the_row_is_not_in_the_expected_provisioning_rows(self):
        self.assertNotIn("vkcts", (ROOT / "scripts/apps/expected_rows.txt").read_text())

    def stage(self, root, plan=None):
        b = Path(root) / "bundle"
        tool = b / "vkcts_tool"
        tool.mkdir(parents=True)
        for f in ("vkcts.py", "vkcts_lib.py", "pins.json", "caselist.tsv"):
            shutil.copy(TOOL / f, tool / f)
        pins = json.loads((tool / "pins.json").read_text())
        pins["run"].update(batch_cases=100, case_timeout_s=5, budget_s=120)
        (tool / "pins.json").write_text(json.dumps(pins))
        art = b / "vkcts"
        (art / "data").mkdir(parents=True)
        write_exec(art / "bin/deqp-vk", FAKE_DEQP)
        return b

    def test_the_row_runs_on_the_committed_list_and_reports_a_pass_row_with_digest_and_note(self):
        with tempfile.TemporaryDirectory() as t:
            self.stage(t)
            cases = [c for _, c in entries()]
            plan = {"cases": {cases[5]: "Fail", cases[7]: "NotSupported"}}
            p = self.bash(["host", "vkcts"], t, {"FAKE_PLAN": json.dumps(plan)})
            res = [l for l in p.stdout.splitlines() if l.startswith("APPRES")][0]
            self.assertRegex(res, r"app=vkcts verdict=NOTRUN|app=vkcts verdict=FAIL|app=vkcts verdict=PASS")
            # side=host asks for the host census; with no nvidia-smi on PATH the row cannot claim GPU use
            self.assertIn("app=vkcts", res)
            guest = self.bash(["guest", "vkcts"], t, {"FAKE_PLAN": json.dumps(plan)})
            res = [l for l in guest.stdout.splitlines() if l.startswith("APPRES")][0]
            self.assertRegex(res, r"side=guest app=vkcts verdict=PASS rc=0 ")
            self.assertRegex(res, r"note=ok cases=%d pass=\d+ " % len(cases))
            self.assertRegex(guest.stdout, r"APPDIG side=guest app=vkcts DIGEST vkcts_cases [0-9a-f]{16}")
            log = (Path(t) / "out/guest/vkcts.log").read_text()
            side, rows, _ = L.parse_block(log)
            self.assertEqual(len(rows), len(cases))

    def test_a_missing_artifact_is_a_norun_row(self):
        with tempfile.TemporaryDirectory() as t:
            b = self.stage(t)
            shutil.rmtree(b / "vkcts")
            res = [l for l in self.bash(["guest", "vkcts"], t).stdout.splitlines() if l.startswith("APPRES")][0]
            self.assertIn("verdict=NOTRUN", res)

    def test_a_row_whose_device_is_not_a_gpu_is_a_fail_row(self):
        with tempfile.TemporaryDirectory() as t:
            self.stage(t)
            res = [l for l in self.bash(["guest", "vkcts"], t, {"FAKE_PLAN": json.dumps({"type": "CPU", "device": "llvmpipe"})}).stdout.splitlines() if l.startswith("APPRES")][0]
            self.assertIn("verdict=FAIL", res)
            self.assertIn("rc=3", res)
            self.assertIn("note=CONFIG", res)

    def test_matrix_table_calls_it_discovery(self):
        sys.path.insert(0, str(ROOT / "scripts/apps"))
        import matrix_table as mt
        cls, final, reason = mt.classify("vkcts", {"verdict": "PASS"}, {"verdict": "PASS"}, None)
        self.assertEqual(cls, "DISCOVERY")
        self.assertIn("docs/design/V3_VULKAN_CTS_ROW.md", reason)
        self.assertEqual(mt.classify("vkcts", {"verdict": "FAIL"}, {"verdict": "FAIL"}, None)[0], "DISCOVERY")
        self.assertEqual(mt.classify("deviceQuery", {"verdict": "PASS"}, {"verdict": "PASS"}, None)[0], "GATE")

    def test_the_discovery_row_stays_out_of_the_generated_gate_list_even_when_it_passes_everywhere(self):
        sys.path.insert(0, str(ROOT / "scripts/apps"))
        import matrix_table as mt
        with tempfile.TemporaryDirectory() as t:
            Path(t, "host.res").write_text("APPRES side=host app=vkcts verdict=PASS rc=0 secs=1 quiet=0 note=-\nAPPRES side=host app=deviceQuery verdict=PASS rc=0 secs=1 quiet=0 note=-\n")
            Path(t, "guest.res").write_text("APPRES side=guest app=vkcts verdict=PASS rc=0 secs=1 quiet=0 note=-\nAPPRES side=guest app=deviceQuery verdict=PASS rc=0 secs=1 quiet=0 note=-\n")
            out = Path(t) / "gate.txt"
            mt.main([t, "--out", str(Path(t) / "t.tsv"), "--gate-list", str(out)])
            gate = [l for l in out.read_text().splitlines() if l and not l.startswith("#")]
            self.assertEqual(gate, ["deviceQuery"])


class ShellGlueTests(unittest.TestCase):
    def test_the_glue_scripts_parse_and_stay_small(self):
        for f in ("run_apps.sh", "apps_matrix.sh", "apps_hook.sh"):
            self.assertEqual(subprocess.run(["bash", "-n", str(ROOT / "scripts/apps" / f)]).returncode, 0, f)
        self.assertFalse(list(TOOL.glob("*.sh")))  # logic is Python

    def test_the_hook_and_the_matrix_driver_call_the_tool_around_the_row(self):
        hook = (ROOT / "scripts/apps/apps_hook.sh").read_text()
        self.assertIn("vkcts.py\" census start", hook)
        self.assertIn("vkcts.py\" census stop", hook)
        matrix = (ROOT / "scripts/apps/apps_matrix.sh").read_text()
        self.assertIn("vkcts.py\" diff", matrix)
        self.assertIn("listall", matrix)


if __name__ == "__main__":
    unittest.main()
