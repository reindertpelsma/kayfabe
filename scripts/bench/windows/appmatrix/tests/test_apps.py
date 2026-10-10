# SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later
import json
import os
import re
import subprocess
import sys
import tempfile
import unittest

import helpers as H
import appdisk
import gen_apps
import mock_guest
import verdict as V
import winapps


class AppsJson(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        cls.doc = H.load_apps()
        cls.man = H.load_manifest()
        cls.ids = {p["id"] for p in cls.man["packages"]}

    def test_apps_json_is_generated_from_gen_apps(self):
        want = json.dumps(gen_apps.build(), indent=1, sort_keys=False) + "\n"
        self.assertEqual(open(os.path.join(H.AM, "apps.json")).read(), want, "apps.json is stale: run gen_apps.py")

    def test_unique_ids_and_required_fields(self):
        ids = [a["id"] for a in self.doc["apps"]]
        self.assertEqual(len(ids), len(set(ids)))
        for a in self.doc["apps"]:
            for k in ("id", "category", "ps", "rx", "timeout_s", "expect_s", "proof", "difficulty", "why", "session", "tier", "pkgs", "linux"):
                self.assertIn(k, a, f"{a['id']} lacks {k}")
            self.assertIn(a["session"], ("service", "interactive"))
            self.assertIn(a["tier"], (1, 2, 3))
            self.assertIn(a["difficulty"], ("low", "medium", "high"))
            self.assertGreater(a["timeout_s"], a["expect_s"], a["id"])
            self.assertTrue(a["proof"] or a["id"] in ("dxdiag_report",), f"{a['id']} has no GPU-use proof")

    def test_packages_exist_in_the_manifest(self):
        for a in self.doc["apps"]:
            for p in a["pkgs"]:
                self.assertIn(p, self.ids, f"{a['id']} needs unknown package {p}")

    def test_regexes_compile_and_proofs_parse(self):
        for a in self.doc["apps"]:
            re.compile(a["rx"])
            if a.get("fail_rx"):
                re.compile(a["fail_rx"])
            for p in a["proof"]:
                k, arg = V.parse_proof(p)
                if arg:
                    re.compile(arg)

    def test_a_fabricated_success_log_passes_its_own_predicate(self):
        for a in self.doc["apps"]:
            s = mock_guest.sample_from_regex(a["rx"])
            self.assertRegex(s, a["rx"], f"{a['id']}: sample {s!r} does not match {a['rx']!r}")
            if a.get("fail_rx"):
                self.assertNotRegex(s, a["fail_rx"], a["id"])

    def test_every_linux_row_is_mapped_or_explained(self):
        txt = open(os.path.join(H.REPO, "scripts", "apps", "run_apps.sh")).read()
        block = txt.split("APPS=$(cat <<'EOF'\n", 1)[1].split("\nEOF\n", 1)[0]
        linux = [l.split("|", 1)[0] for l in block.splitlines() if l and "|" in l]
        self.assertGreaterEqual(len(linux), 65)
        mapped = set(self.doc["linux_map"]) | set(self.doc["no_equivalent"])
        missing = [l for l in linux if l not in mapped]
        self.assertEqual(missing, [], f"Linux rows with neither a Windows app nor a reason: {missing}")
        for l in self.doc["no_equivalent"]:
            self.assertIn(l, linux)
            self.assertNotIn(l, self.doc["linux_map"], f"{l} is both mapped and 'no equivalent'")
        for l, ws in self.doc["linux_map"].items():
            self.assertIn(l, linux, f"{l} is not a Linux row")
            for w in ws:
                self.assertIn(w, {a['id'] for a in self.doc['apps']})

    def test_rendered_scripts_are_ascii_and_fully_substituted(self):
        for a in self.doc["apps"]:
            s = winapps.render_app_script(self.doc["prelude"], a, "E:")
            self.assertNotIn("@@", s, a["id"])
            self.assertIn("C:\\kf\\out\\" + a["id"], s)
            s.encode("ascii")

    def test_rendered_scripts_parse_in_powershell(self):
        pw = H.find_pwsh()
        if not pw:
            self.skipTest("no pwsh (set KF_PWSH)")
        with tempfile.TemporaryDirectory() as d:
            bad = []
            for a in self.doc["apps"]:
                p = os.path.join(d, a["id"] + ".ps1")
                open(p, "w", newline="").write(winapps.render_app_script(self.doc["prelude"], a, "E:"))
            out = H.pwsh_parse(sorted(os.path.join(d, f) for f in os.listdir(d)))
            self.assertIn("PARSED_BAD=0", out, out)

    def test_counts_by_category_are_stable_enough_to_quote(self):
        n = len(self.doc["apps"])
        self.assertGreaterEqual(n, 90)
        self.assertGreaterEqual(sum(1 for a in self.doc["apps"] if a["tier"] == 1), 70)


class Selection(unittest.TestCase):
    def test_select_by_tier_name_category(self):
        doc = H.load_apps()["apps"]
        t1 = winapps.select_apps(doc, ["all"], 1, [], set())
        t3 = winapps.select_apps(doc, ["all"], 3, [], set())
        self.assertLess(len(t1), len(t3))
        one = winapps.select_apps(doc, ["vkcube"], 1, [], set())          # a tier-2 app named explicitly is selected
        self.assertEqual([a["id"] for a in one], ["vkcube"])
        self.assertTrue(all(a["category"] == "video" for a in winapps.select_apps(doc, ["all"], 3, ["video"], set())))
        with self.assertRaises(SystemExit):
            winapps.select_apps(doc, ["nope"], 1, [], set())


if __name__ == "__main__":
    unittest.main()
