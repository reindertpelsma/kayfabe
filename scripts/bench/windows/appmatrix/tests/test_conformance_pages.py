# SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later
"""CPU-only checks of the offline browser verdict and unavailable-API paths; no browser is launched."""
import json
from pathlib import Path
import re
import shutil
import subprocess
import unittest

import helpers as H


@unittest.skipUnless(shutil.which("node"), "node required")
class Pages(unittest.TestCase):
    def scripts(self):
        for api in ("webgl", "webgpu"):
            page = (Path(H.AM) / "guest/web" / (api + "_conformance.html")).read_text()
            yield api, re.search(r"<script>(.*)</script>", page, re.S)[1]

    def evaluate(self, js):
        p = subprocess.run(["node", "-e", js], capture_output=True, text=True, timeout=15)
        self.assertEqual(p.returncode, 0, p.stderr)
        return json.loads(p.stdout)

    def test_exactly_four_checked_results_nvidia_and_nonzero_frames_are_required(self):
        for api, script in self.scripts():
            func = re.search(r"function conformanceOK\(checks, gpu, frames\) \{[^}]+\}", script)[0]
            values = self.evaluate(func + "\nconsole.log(JSON.stringify([conformanceOK([true,true,true,true],true,8),conformanceOK([],true,8),conformanceOK([true],true,8),conformanceOK([true,true,true,true],false,8),conformanceOK([true,true,true,true],true,0),conformanceOK([true,false,true,true],true,8)]));")
            self.assertEqual(values, [True, False, False, False, False, False], api)

    def test_a_failed_check_or_short_run_cannot_pass(self):
        for api, script in self.scripts():
            func = re.search(r"function conformanceOK\(checks, gpu, frames\) \{[^}]+\}", script)[0]
            values = self.evaluate(func + "\nconsole.log(JSON.stringify([conformanceOK([true,false,true,true],true,100),conformanceOK([true,true,true,true],true,7),conformanceOK([true,true,true,1],true,8),conformanceOK([],true,0)]));")
            self.assertEqual(values, [False] * 4, api)

    def test_missing_api_posts_a_failure_with_no_completed_checks(self):
        for api, script in self.scripts():
            harness = "const vm=require('node:vm');const context={navigator:{},document:{getElementById:()=>({getContext:()=>null})},fetch:async(url,opts)=>console.log(opts.body)};vm.runInNewContext(" + json.dumps(script) + ",context);"
            result = self.evaluate(harness)
            self.assertIs(result["ok"], False, api)
            self.assertEqual(result["checks"], [], api)
            self.assertTrue(result["why"], api)

    def test_webgpu_accepts_explicit_hardware_info_in_current_and_older_api(self):
        script = dict(self.scripts())["webgpu"]
        func = re.search(r"function hardwareAdapter\(info, adapter\) \{[^}]+\}", script)[0]
        values = self.evaluate(func + "\nconsole.log(JSON.stringify([hardwareAdapter({vendor:'nvidia',isFallbackAdapter:false},{}),hardwareAdapter({vendor:'nvidia'},{isFallbackAdapter:false}),hardwareAdapter({vendor:'nvidia'},{}),hardwareAdapter({vendor:'nvidia',isFallbackAdapter:true},{}),hardwareAdapter({vendor:'intel',isFallbackAdapter:false},{})]));")
        self.assertEqual(values, [True, True, False, False, False])

    def test_unknown_fallback_and_non_nvidia_adapters_never_pass(self):
        script = dict(self.scripts())["webgpu"]
        func = re.search(r"function hardwareAdapter\(info, adapter\) \{[^}]+\}", script)[0]
        values = self.evaluate(func + "\nconsole.log(JSON.stringify([hardwareAdapter({vendor:'nvidia'},{}),hardwareAdapter({vendor:'intel'},{isFallbackAdapter:false}),hardwareAdapter({vendor:'nvidia'},{isFallbackAdapter:true})]));")
        self.assertEqual(values, [False] * 3)


if __name__ == "__main__":
    unittest.main()
