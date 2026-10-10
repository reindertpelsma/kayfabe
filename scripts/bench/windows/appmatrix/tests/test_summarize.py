# SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later
import os
import tempfile
import unittest

import helpers as H
import summarize


class Summarize(unittest.TestCase):
    def test_table_in_the_linux_shape_with_alone_override_and_linux_reference(self):
        with tempfile.TemporaryDirectory() as d:
            run, lin = os.path.join(d, "run"), os.path.join(d, "lin")
            os.makedirs(run); os.makedirs(lin)
            open(os.path.join(run, "win.res"), "w").write(
                "APPRES side=win app=nvidia_smi verdict=PASS rc=0 secs=1 quiet=0 note=- boot=g1 guest_tdr=0 wer=0 gsp_cycles=0 proof=out tier=1 cat=smi\n"
                "APPRES side=win app=vkpeak verdict=FAIL rc=1 secs=9 quiet=2 note=rc:1,no-gpu-proof boot=g1 guest_tdr=1 wer=1 gsp_cycles=3 proof=- tier=1 cat=vulkan\n"
                "APPRES side=win app=llama_cuda_gen verdict=PASS rc=0 secs=60 quiet=0 note=- boot=g2 guest_tdr=0 wer=0 gsp_cycles=0 proof=out tier=1 cat=llm\n"
                "APPDIG side=win app=llama_cuda_gen OUTSHA llama_cpp aaaa\n")
            open(os.path.join(run, "win_isolated.res"), "w").write(
                "APPRES side=win app=vkpeak verdict=PASS rc=0 secs=8 quiet=0 note=- boot=i1 guest_tdr=0 wer=0 gsp_cycles=0 proof=out tier=1 cat=vulkan\n")
            open(os.path.join(lin, "guest.res"), "w").write(
                "APPRES side=guest app=nvidia_smi verdict=PASS rc=0 secs=1 quiet=0 note=-\nAPPRES side=guest app=vkpeak verdict=PASS rc=0 secs=1 quiet=0 note=-\n"
                "APPRES side=guest app=llama_cpp_gen verdict=FAIL rc=1 secs=1 quiet=0 note=-\n")
            open(os.path.join(d, "ref.res"), "w").write("APPDIG side=host app=llama_cuda_gen OUTSHA llama_cpp bbbb\n")
            doc = H.load_apps()
            t = summarize.summarize(run, doc, lin, os.path.join(d, "ref.res"))
            self.assertIn("| app | linux guest | win (batched) | win (alone) | detail |", t)
            self.assertIn("| nvidia_smi | PASS | PASS | - |", t)
            self.assertRegex(t, r"\| vkpeak \| PASS \| FAIL \| PASS \|")             # batched FAIL, alone PASS, shown beside it
            self.assertIn("digest != ref", t)
            self.assertIn("PASS*: 1", t)
            self.assertIn("Linux rows with no Windows equivalent", t)
            self.assertIn("guest TDR events", t)


if __name__ == "__main__":
    unittest.main()
