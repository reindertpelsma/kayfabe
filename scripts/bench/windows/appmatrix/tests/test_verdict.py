# SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later
import unittest

import helpers  # noqa: F401
import verdict as V

APP = dict(id="x", rx=r"^DONE", fail_rx=r"^CHECK .*FAIL", proof=["out:NVIDIA", "pdh:Compute|3D"], nvidia_only=True)


def facts(**kw):
    f = dict(rc=0, secs=3, quiet=0, timed_out=False, pdh=dict(nv={"Compute_0": 12.0}, other={}), smi=dict(util_max=0, mem_used_max_mb=0, mem_used_base_mb=0),
             events=dict(nvlddmkm_153=0, display_4101=0, wer_1001=0))
    f.update(kw)
    return f


class Verdicts(unittest.TestCase):
    def test_pass(self):
        d = V.decide(APP, facts(), "NVIDIA GeForce RTX 4070\nDONE\n")
        self.assertEqual(d["verdict"], "PASS")
        self.assertIn("out", d["proof"])
        self.assertIn("pdh:Compute_0", d["proof"])

    def test_rc_nonzero_is_fail(self):
        d = V.decide(APP, facts(rc=1), "NVIDIA\nDONE\n")
        self.assertEqual(d["verdict"], "FAIL")
        self.assertIn("rc=1", d["note"])

    def test_missing_success_string(self):
        d = V.decide(APP, facts(), "NVIDIA\nnothing\n")
        self.assertEqual(d["verdict"], "FAIL")
        self.assertIn("success-string-missing", d["note"])

    def test_check_fail_line(self):
        d = V.decide(APP, facts(), "NVIDIA\nCHECK foo FAIL maxerr=3\nDONE\n")
        self.assertEqual(d["verdict"], "FAIL")
        self.assertIn("fail-pattern", d["note"])

    def test_timeout(self):
        self.assertEqual(V.decide(APP, facts(timed_out=True, rc=124), "NVIDIA\n")["verdict"], "TIMEOUT")
        self.assertEqual(V.decide(APP, facts(rc=137), "")["verdict"], "TIMEOUT")

    def test_notrun_when_binary_missing(self):
        d = V.decide(APP, facts(rc=1), "The term 'x.exe' is not recognized as the name of a cmdlet, function\n")
        self.assertEqual(d["verdict"], "NOTRUN")
        self.assertEqual(V.decide(APP, dict(notrun="stage-failed:ffmpeg"), "")["verdict"], "NOTRUN")

    def test_no_gpu_proof_is_fail(self):
        d = V.decide(APP, facts(pdh=dict(nv={}, other={})), "DONE\n")
        self.assertEqual(d["verdict"], "FAIL")
        self.assertIn("no-gpu-proof", d["note"])

    def test_software_adapter_is_fail_even_with_proof(self):
        d = V.decide(APP, facts(pdh=dict(nv={"3D": 9.0}, other={"3D": 40.0})), "NVIDIA\nDONE\n")
        self.assertEqual(d["verdict"], "FAIL")
        self.assertIn("non-NVIDIA-adapter", d["note"])
        # ... unless the app opts out
        app2 = dict(APP, nvidia_only=False)
        self.assertEqual(V.decide(app2, facts(pdh=dict(nv={"3D": 9.0}, other={"3D": 40.0})), "NVIDIA\nDONE\n")["verdict"], "PASS")

    def test_pdh_instance_names_are_case_insensitive(self):
        # Get-Counter lower-cases instance names: "engtype_3D" reaches the host as "3d" (found on the native-NVIDIA baseline)
        app = dict(APP, proof=["pdh:3D|VideoDecode"])
        self.assertEqual(V.decide(app, facts(pdh=dict(nv={"3d": 9.0}, other={})), "DONE\n")["verdict"], "PASS")
        self.assertEqual(V.decide(app, facts(pdh=dict(nv={"videodecode": 4.0}, other={})), "DONE\n")["verdict"], "PASS")
        self.assertEqual(V.decide(app, facts(pdh=dict(nv={"copy": 4.0}, other={})), "DONE\n")["verdict"], "FAIL")

    def test_engine_noise_below_threshold_is_no_proof(self):
        app = dict(APP, proof=["pdh:VideoEncode"])
        self.assertEqual(V.decide(app, facts(pdh=dict(nv={"VideoEncode": 0.0}, other={})), "DONE\n")["verdict"], "FAIL")
        self.assertEqual(V.decide(app, facts(pdh=dict(nv={"VideoEncode": 7.0}, other={})), "DONE\n")["verdict"], "PASS")

    def test_smi_proof(self):
        app = dict(APP, proof=["smi"])
        ok = facts(smi=dict(util_max=0, mem_used_max_mb=900, mem_used_base_mb=400), pdh=dict(nv={}, other={}))
        self.assertEqual(V.decide(app, ok, "DONE\n")["verdict"], "PASS")
        no = facts(smi=dict(util_max=0, mem_used_max_mb=410, mem_used_base_mb=400), pdh=dict(nv={}, other={}))
        self.assertEqual(V.decide(app, no, "DONE\n")["verdict"], "FAIL")

    def test_crash_facts(self):
        d = V.decide(APP, dict(crash="guest-rebooted (bugcheck?)", rc=None, secs=40), "")
        self.assertEqual(d["verdict"], "FAIL")
        self.assertIn("guest-rebooted", d["note"])

    def test_tdr_and_wer_counted_without_changing_a_pass(self):
        d = V.decide(APP, facts(events=dict(nvlddmkm_153=2, display_4101=1, wer_1001=1)), "NVIDIA\nDONE\n")
        self.assertEqual((d["verdict"], d["tdr"], d["wer"]), ("PASS", 3, 1))

    def test_appres_line_is_parseable_like_the_linux_one(self):
        import re
        d = V.decide(APP, facts(), "NVIDIA\nDONE\n")
        d["note"] = "KFSURVIVE x EXITED_EARLY rc=1 after=2 s"
        line = V.appres_line("win", "x", d, boot="g1", gsp_cycles=2, extra="tier=1 cat=probe")
        kv = dict(re.findall(r"(\w+)=((?:(?! \w+=).)*)", line[7:].strip()))
        self.assertEqual(kv["verdict"], "PASS")
        self.assertEqual(kv["rc"], "0")                        # the '=' of the note did not split the line
        self.assertEqual(kv["boot"], "g1")
        self.assertIn("rc:1", kv["note"])

    def test_digest_lines(self):
        self.assertEqual(V.digest_lines("a\nOUTSHA llama_cpp abc123\nDIGEST x 1\n"), ["OUTSHA llama_cpp abc123", "DIGEST x 1"])

    def test_bad_proof_kind(self):
        with self.assertRaises(ValueError):
            V.parse_proof("magic:x")


if __name__ == "__main__":
    unittest.main()
