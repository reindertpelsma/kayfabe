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

    def test_short_cuda_vectoradd_readback_proves_work_between_samples(self):
        log = ("[Vector addition of 50000 elements]\n"
               "Copy input data from the host memory to the CUDA device\n"
               "CUDA kernel launch with 196 blocks of 256 threads\n"
               "Copy output data from the CUDA device to the host memory\n"
               "Test PASSED\nDone\n")
        app = dict(APP, rx="Test PASSED", proof=["cuda-vectoradd-readback"])
        f = facts(pdh=dict(nv={}, other={}), gpu_devices=["NVIDIA GeForce RTX 4070 status=OK code=0"])
        self.assertEqual(V.decide(app, f, log)["verdict"], "PASS")
        for devices in ([], ["Microsoft Basic Render Driver status=OK code=0"],
                        ["NVIDIA GeForce RTX 4070 status=OK code=43"]):
            self.assertEqual(V.decide(app, dict(f, gpu_devices=devices), log)["verdict"], "FAIL")
        for omitted in ("CUDA kernel launch with 196 blocks of 256 threads\n",
                        "Copy output data from the CUDA device to the host memory\n"):
            self.assertEqual(V.decide(app, f, log.replace(omitted, ""))["verdict"], "FAIL")
        self.assertEqual(V.decide(app, dict(f, rc=1), log)["verdict"], "FAIL")
        self.assertEqual(V.decide(app, f, log + "CHECK vectorAdd FAIL\n")["verdict"], "FAIL")
        self.assertEqual(V.decide(app, dict(f, pdh=dict(nv={}, other={"3d": 10})), log)["verdict"], "FAIL")

    def test_cup8_exact_cuda_readback_without_periodic_samples(self):
        app = dict(APP, rx=r"CUP8 RESULT N=\d+ bad=0 maxerr=0 ", proof=["cuda-cup8-readback"])
        f = facts(pdh=dict(nv={}, other={}), gpu_devices=["NVIDIA GeForce RTX 4070 status=OK code=0"])
        log = ("LAUNCH OK\nSYNC OK\nok   cuMemcpyDtoH(hC,dC,sz)\n"
               "CUP8 RESULT N=2048 bad=0 maxerr=0 C[0]=2048(exp 2048) -> PASS\n")
        self.assertEqual(V.decide(app, f, log)["verdict"], "PASS")
        for omitted in ("LAUNCH OK\n", "SYNC OK\n", "ok   cuMemcpyDtoH(hC,dC,sz)\n"):
            self.assertEqual(V.decide(app, f, log.replace(omitted, ""))["verdict"], "FAIL")
        self.assertEqual(V.decide(app, dict(f, gpu_devices=[]), log)["verdict"], "FAIL")
        self.assertEqual(V.decide(app, f, log.replace('bad=0', 'bad=1'))["verdict"], "FAIL")

    def test_cuda_demo_readbacks_require_math_and_original_gui(self):
        device = 'GPU Device 0: "Ampere" with compute capability 8.9\n'
        ocean = (device +
                 '> compareBin2Bin <float> nelements=65536, epsilon=0.10, threshold=0.15\n'
                 '   src_file <spatialDomain.bin>, size=262144 bytes\n'
                 '   ref_file <./ref_spatialDomain.bin>, size=262144 bytes\n  OK\n'
                 '> compareBin2Bin <float> nelements=131072, epsilon=0.10, threshold=0.15\n'
                 '   src_file <slopeShading.bin>, size=524288 bytes\n'
                 '   ref_file <./ref_slopeShading.bin>, size=524288 bytes\n  OK\n')
        fog = (device + 'CURAND initialized\n'
               '> compareBin2Bin <float> nelements=1200000, epsilon=0.25, threshold=0.35\n'
               '   src_file <randomFog.bin>\n   ref_file <./ref_randomFog.bin>\n  OK\n')
        f = facts(pdh=dict(nv={}, other={}), gpu_devices=['NVIDIA GeForce RTX 4070 status=OK code=0'])
        for kind, qa in [('oceanfft', ocean), ('randomfog', fog)]:
            with self.subTest(kind=kind):
                app = dict(APP, rx=r'^KFSURVIVE demo ALIVE_AT 25', proof=['cuda-' + kind + '-readback'])
                log = qa + 'KFSURVIVE demo ALIVE_AT 25\n'
                self.assertEqual(V.decide(app, f, log)['verdict'], 'PASS')
                self.assertEqual(V.decide(app, f, log.replace('\n', '\r\n'))['verdict'], 'PASS')
                # Each missing comparison, wrong count or failed result rejects
                # even if a healthy GPU is busy during the GUI phase.
                busy = dict(f, pdh=dict(nv={'3D': 90}, other={}))
                mutations = [log.replace(device, ''), log.replace('ALIVE_AT', 'EXITED_EARLY'),
                             log.replace('nelements=', 'nelements=9'), log.replace('ref_file', 'missing_ref'),
                             log.replace('  OK', '  FAIL', 1), log.rsplit('  OK', 1)[0] + 'KFSURVIVE demo ALIVE_AT 25\n']
                if kind == 'randomfog':
                    mutations.append(log.replace('CURAND initialized', ''))
                else:
                    mutations.append(log.replace('size=524288', 'size=4'))
                for bad in mutations:
                    self.assertEqual(V.decide(app, busy, bad)['verdict'], 'FAIL')
                for devices in ([], ['NVIDIA GeForce RTX 4070 status=OK code=43'],
                                ['Microsoft Basic Render Driver status=OK code=0']):
                    self.assertEqual(V.decide(app, dict(f, gpu_devices=devices), log)['verdict'], 'FAIL')
                self.assertEqual(V.decide(app, dict(f, rc=1), log)['verdict'], 'FAIL')
                self.assertEqual(V.decide(app, f, log + 'CHECK demo FAIL\n')['verdict'], 'FAIL')
                self.assertEqual(V.decide(app, dict(f, pdh=dict(nv={}, other={'3D': 20})), log)['verdict'], 'FAIL')

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
