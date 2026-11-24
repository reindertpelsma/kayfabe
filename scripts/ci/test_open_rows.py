#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later
"""GPU-free tests of the open FFmpeg NVENC/NVDEC matrix row and the DCGM + NVML sweep row (2026-11-17).

STATUS: LIVE, 2026-11-17 (docs/design/V3_OPEN_ROWS_FFMPEG_DCGM.md section 9). Fixtures are written to the formats the tools document, with the fake GPU
UUID prefix fa4efa4e-fa4e-4fa4-8fa4-fa4efa4e....; native outputs of the Vast boxes are added under scripts/apps/open_rows/fixtures/ when they exist.
Mutants of the code under test: scripts/ci/open_rows_mutants.py (two named killers each).
"""
import ctypes
import json
import os
from pathlib import Path
import re
import shutil
import stat
import subprocess
import sys
import tempfile
import unittest

ROOT = Path(__file__).resolve().parents[2]
OR = ROOT / "scripts/apps/open_rows"
sys.path.insert(0, str(OR))
import census  # noqa: E402
import dcgm_nvml  # noqa: E402
import nvcodec_lib as L  # noqa: E402
import nvcodec_matrix  # noqa: E402
import nvml_lib as N  # noqa: E402
import nvml_sweep  # noqa: E402
import rowio  # noqa: E402

FAKE_UUID = "GPU-fa4efa4e-fa4e-4fa4-8fa4-fa4efa4e0001"


# ------------------------------------------------------------------------------------------------ fixtures (probe output)
def probe_text(av1=False, yuv444=True, bits10=True, wmax=8192, hmax=8192, h264_wmax=4096, sessions=8, limit_hit=True, dec=None, nvenc_error=False):
    """What nvcodec_probe prints for a made-up die. av1=False is an Ampere-like die, av1=True an Ada-like one."""
    out = ["meta\tdevice_name\tFake Die FA4E", "meta\tcompute_capability\t8.6", "meta\tdriver_api_max\t12.2", "meta\theader_api\t12.2"]
    if nvenc_error:
        out.append("error\tnvenc\tlibnvidia-encode not loadable")
    else:
        for codec, w in (("h264", h264_wmax), ("hevc", wmax), ("av1", wmax)):
            has = av1 if codec == "av1" else True
            out.append("enc_codec\t%s\t%d" % (codec, has))
            if has:
                for name, v in (("WIDTH_MAX", w), ("HEIGHT_MAX", hmax if codec != "h264" else 4096), ("WIDTH_MIN", 129), ("HEIGHT_MIN", 33),
                                ("SUPPORT_YUV444_ENCODE", int(yuv444)), ("SUPPORT_10BIT_ENCODE", int(bits10 and codec != "h264")),
                                ("NUM_MAX_BFRAMES", 4), ("SUPPORT_LOOKAHEAD", 1), ("SUPPORT_TEMPORAL_AQ", 1), ("SUPPORT_BFRAME_REF_MODE", 2)):
                    out.append("enc_cap\t%s.%s\t%d" % (codec, name, v))
                out.append("enc_cap\t%s.cap60\tERR:NV_ENC_ERR_INVALID_PARAM" % codec)
                out.append("enc_profile\t%s\tMAIN" % codec)
        out.append("sessions\topened\t%d" % sessions)
        out.append("sessions\tlimit_hit\t%d" % limit_hit)
        out.append("sessions\tstopped_by\tNV_ENC_ERR_OUT_OF_MEMORY")
    dec = dec if dec is not None else {("h264", "420", 8): (4096, 4096), ("hevc", "420", 8): (8192, 8192), ("hevc", "420", 10): (8192, 8192),
                                       ("hevc", "444", 8): (8192, 8192), ("vp9", "420", 8): (8192, 8192), ("vp9", "420", 10): (8192, 8192),
                                       ("av1", "420", 8): (8192, 8192), ("av1", "420", 10): (8192, 8192)}
    for codec in ("h264", "hevc", "vp9", "av1"):
        for chroma in ("420", "422", "444"):
            for depth in (8, 10, 12):
                lim = dec.get((codec, chroma, depth))
                if lim:
                    out.append("dec_cap\t%s.%s.%d\tsupported=1 max=%dx%d min=48x16 mbmax=262144 nvdecs=1 fmtmask=0x3" % (codec, chroma, depth, lim[0], lim[1]))
                else:
                    out.append("dec_cap\t%s.%s.%d\tsupported=0 max=0x0 min=0x0 mbmax=0 nvdecs=0 fmtmask=0x0" % (codec, chroma, depth))
    return "\n".join(out) + "\n"


CASES_TEXT = Path(OR / "nvcodec_cases.tsv").read_text()


class CaseTable(unittest.TestCase):
    def test_the_committed_table_is_valid(self):
        cases = L.load_cases(CASES_TEXT)
        self.assertGreaterEqual(len(cases), 40)
        self.assertEqual(len({c["id"] for c in cases}), len(cases))

    def test_the_table_covers_the_matrix_the_owner_listed(self):
        ids = {c["id"] for c in L.load_cases(CASES_TEXT)}
        for needed in ("enc_av1_420_8_1080p", "enc_hevc_main10_1080p", "enc_hevc_444_8_1080p", "enc_h264_444_8_1080p", "enc_hevc_8k", "enc_h264_above4096",
                       "enc_hevc_bframes_lookahead", "dec_vp9_420_8_hw", "dec_av1_420_8_hw", "dec_hevc_420_10_hw", "dec_hevc_420_10_cuvid", "dec_hevc_8k_hw"):
            self.assertIn(needed, ids)
        sess = sorted(c["sessions"] for c in L.load_cases(CASES_TEXT) if c["op"] == "sessions")
        self.assertTrue({1, 3, 5, 8, 9}.issubset(sess), sess)  # around the GeForce cap whichever it is (3, 5 or 8)
        modes = {c["mode"] for c in L.load_cases(CASES_TEXT)}
        self.assertEqual(modes, {"nvenc", "hwaccel_cuda", "cuvid"})

    def test_every_decode_and_encode_case_names_a_comparison_format(self):
        for c in L.load_cases(CASES_TEXT):
            self.assertIn((c["chroma"], c["bitdepth"]), L.COMPARE_FMT, c["id"])
            if c["op"] == "dec":
                self.assertIn((c["chroma"], c["bitdepth"]), L.HW_SW_FMT, c["id"])

    def test_the_table_holds_no_per_die_data(self):
        text = CASES_TEXT.lower()
        for word in ("rtx", "geforce", "ada", "ampere", "turing", "blackwell", "ga106", "ad104"):
            self.assertNotRegex(re.sub(r"^#.*$", "", text, flags=re.M), r"\b%s\b" % word)

    def _bad(self, old, new, count=1):
        text = CASES_TEXT.replace(old, new, count)
        self.assertNotEqual(text, CASES_TEXT)
        with self.assertRaises(L.TableError):
            L.load_cases(text)

    def test_a_duplicate_id_is_refused(self):
        lines = CASES_TEXT.splitlines()
        first = next(l for l in lines if l.startswith("enc_h264_420_8_1080p"))
        with self.assertRaises(L.TableError):
            L.load_cases(CASES_TEXT + first + "\n")

    def test_bad_cells_are_refused(self):
        self._bad("\tenc\th264\thigh\tyuv420p\t420\t8\t1920", "\tencx\th264\thigh\tyuv420p\t420\t8\t1920")
        self._bad("\tnvenc\t-\tenc:h264 & enc:h264:dim", "\tcuvid\t-\tenc:h264 & enc:h264:dim")
        self._bad("enc:h264 & enc:h264:dim", "enc:h264 & enc:h264:diim")
        self._bad("enc:hevc & enc:hevc:SUPPORT_10BIT_ENCODE>=1", "enc:hevc & enc:hevc:SUPPORT_10BIT_ENCODE>>1")
        self._bad("\t-rc vbr -b:v 0 -cq 24 -bf 3 -rc-lookahead 20\t", "\t-rc vbr;rm -b:v 0 -cq 24\t")
        self._bad("\tyuv420p\t420\t8\t1920\t1080\t30", "\tyuv999p\t420\t8\t1920\t1080\t30")

    def test_sessions_above_one_only_for_the_sessions_op(self):
        c = next(c for c in L.load_cases(CASES_TEXT) if c["id"] == "enc_hevc_420_8_1080p")
        row = "\t".join([c["id"] + "z", "encode", "enc", "hevc", "main", "yuv420p", "420", "8", "1920", "1080", "30", "4", "nvenc", "-", "enc:hevc", "28"])
        with self.assertRaises(L.TableError):
            L.load_cases(CASES_TEXT + row + "\n")

    def test_the_header_must_match(self):
        with self.assertRaises(L.TableError):
            L.load_cases(CASES_TEXT.replace("id\tgroup", "name\tgroup", 1))


class ProbeParsing(unittest.TestCase):
    def test_parse(self):
        p = L.parse_probe(probe_text(av1=True))
        self.assertEqual(p.meta["device_name"], "Fake Die FA4E")
        self.assertTrue(p.enc_codec["av1"])
        self.assertEqual(p.enc_caps["hevc"]["WIDTH_MAX"], 8192)
        self.assertEqual(p.enc_caps["hevc"]["cap60"], "ERR:NV_ENC_ERR_INVALID_PARAM")
        self.assertEqual(p.dec[("av1", "420", 10)]["maxw"], 8192)
        self.assertFalse(p.dec[("h264", "420", 10)]["supported"])
        self.assertEqual(p.sessions["opened"], "8")
        self.assertTrue(p.enc_available() and p.dec_available())

    def test_garbage_lines_are_ignored(self):
        p = L.parse_probe("hello\n\tonly\n" + probe_text() + "dec_cap\tnot a key\tsupported=1\n")
        self.assertEqual(p.sessions["opened"], "8")

    def test_an_nvenc_error_makes_the_encoder_unavailable(self):
        p = L.parse_probe(probe_text(nvenc_error=True))
        self.assertFalse(p.enc_available())
        self.assertTrue(p.dec_available())

    def test_flat_view_has_every_cap(self):
        flat = L.parse_probe(probe_text()).flat()
        self.assertIn("enc_cap.hevc.cap60", flat)
        self.assertIn("dec_cap.vp9.420.10", flat)
        self.assertEqual(flat["sessions.opened"], "8")


class CapsComparison(unittest.TestCase):
    """The expectation is derived from the probe of the same machine; two probes make two expectations for one table row."""

    def setUp(self):
        self.cases = {c["id"]: c for c in L.load_cases(CASES_TEXT)}
        self.ampere = L.parse_probe(probe_text(av1=False))
        self.ada = L.parse_probe(probe_text(av1=True))

    def expect(self, cid, probe):
        return L.evaluate_needs(self.cases[cid], probe)[0]

    def test_av1_encode_follows_the_die(self):
        self.assertEqual(self.expect("enc_av1_420_8_1080p", self.ampere), "UNSUPPORTED")
        self.assertEqual(self.expect("enc_av1_420_8_1080p", self.ada), "SUPPORTED")

    def test_a_die_without_av1_is_not_supported_not_a_failure(self):
        exp, why = L.evaluate_needs(self.cases["enc_av1_420_8_1080p"], self.ampere)
        self.assertEqual(L.verdict(exp, False, "unknown_encoder_param")[0], "NOT_SUPPORTED")
        self.assertIn("not offered", why[0])

    def test_dimension_limits_come_from_the_probe(self):
        self.assertEqual(self.expect("enc_h264_above4096", self.ada), "UNSUPPORTED")  # h264 caps say 4096
        self.assertEqual(self.expect("enc_hevc_8k", self.ada), "SUPPORTED")
        small = L.parse_probe(probe_text(wmax=4096, hmax=4096))
        self.assertEqual(self.expect("enc_hevc_8k", small), "UNSUPPORTED")
        self.assertEqual(self.expect("enc_hevc_5k", small), "UNSUPPORTED")

    def test_444_and_10bit_follow_their_caps(self):
        p = L.parse_probe(probe_text(yuv444=False, bits10=False))
        self.assertEqual(self.expect("enc_hevc_444_8_1080p", p), "UNSUPPORTED")
        self.assertEqual(self.expect("enc_hevc_main10_1080p", p), "UNSUPPORTED")
        self.assertEqual(self.expect("enc_hevc_444_8_1080p", self.ampere), "SUPPORTED")
        self.assertEqual(self.expect("enc_h264_10bit_1080p", self.ampere), "UNSUPPORTED")  # the h264 cap is 0 in the fixture

    def test_sessions_follow_the_discovered_cap(self):
        for n, want in ((1, "SUPPORTED"), (8, "SUPPORTED"), (9, "UNSUPPORTED"), (12, "UNSUPPORTED")):
            self.assertEqual(self.expect("sess_h264_n%02d" % n, self.ampere), want, n)
        unlimited = L.parse_probe(probe_text(sessions=16, limit_hit=False))
        self.assertEqual(self.expect("sess_h264_n12", unlimited), "SUPPORTED")

    def test_decode_follows_cuvid_caps(self):
        self.assertEqual(self.expect("dec_h264_420_10_hw", self.ampere), "UNSUPPORTED")
        self.assertEqual(self.expect("dec_hevc_420_10_hw", self.ampere), "SUPPORTED")
        self.assertEqual(self.expect("dec_hevc_8k_hw", self.ampere), "SUPPORTED")
        self.assertEqual(self.expect("dec_h264_above4096_hw", self.ampere), "UNSUPPORTED")  # 4224 > 4096
        none = L.parse_probe(probe_text(dec={}))
        self.assertEqual(self.expect("dec_vp9_420_8_hw", none), "UNSUPPORTED")

    def test_a_probe_that_could_not_ask_is_unknown_never_a_guess(self):
        dead = L.parse_probe(probe_text(nvenc_error=True))
        self.assertEqual(self.expect("enc_hevc_420_8_1080p", dead), "UNKNOWN")
        self.assertEqual(self.expect("sess_h264_n03", dead), "UNKNOWN")
        self.assertEqual(L.verdict("UNKNOWN", None, "")[0], "NOT_RUN")
        self.assertEqual(self.expect("enc_hevc_420_8_1080p", L.parse_probe("")), "UNKNOWN")


class UnavailableEngines(unittest.TestCase):
    """A Vast container whose NVENC/NVDEC cannot be reached (measured 2026-11-17: OpenEncodeSessionEx unsupported device, cuvid NO_DEVICE) is a
    named NOT_RUN, never an `unsupported` expectation."""

    def test_nvenc_unavailable_names_the_probe_error(self):
        p = L.parse_probe("meta\tdevice_name\tX\nerror\tnvenc\topen session NV_ENC_ERR_UNSUPPORTED_DEVICE\n")
        c = next(c for c in L.load_cases(CASES_TEXT) if c["id"] == "enc_hevc_420_8_1080p")
        self.assertEqual(L.unavailable_reason(c, p), ("nvenc_unavailable", "open session NV_ENC_ERR_UNSUPPORTED_DEVICE"))

    def test_decoder_rows_that_are_errors_are_unknown_not_unsupported(self):
        p = L.parse_probe("dec_cap\thevc.420.8\tERR:cuda100\ndec_cap\th264.420.8\tERR:cuda100\n")
        c = next(c for c in L.load_cases(CASES_TEXT) if c["id"] == "dec_hevc_420_8_hw")
        self.assertEqual(L.evaluate_needs(c, p)[0], "UNKNOWN")
        self.assertEqual(L.unavailable_reason(c, p)[0], "nvdec_unavailable")
        self.assertFalse(p.dec_available())

    def test_a_working_probe_has_no_unavailable_reason(self):
        p = L.parse_probe(probe_text())
        for c in L.load_cases(CASES_TEXT):
            self.assertEqual(L.unavailable_reason(c, p), (None, ""), c["id"])

    def test_the_reasons_are_configuration_errors(self):
        for r in ("nvenc_unavailable", "nvdec_unavailable", "no_encode_device", "no_decode_device"):
            self.assertEqual(L.verdict("SUPPORTED", False, r)[0], "NOT_RUN")


class Verdicts(unittest.TestCase):
    def test_supported_and_worked_is_pass(self):
        self.assertEqual(L.verdict("SUPPORTED", True, "ok"), ("PASS", "ok"))

    def test_supported_and_failed_is_fail(self):
        self.assertEqual(L.verdict("SUPPORTED", False, "encoder_params_rejected"), ("FAIL", "encoder_params_rejected"))

    def test_unsupported_and_failed_is_not_supported(self):
        self.assertEqual(L.verdict("UNSUPPORTED", False, "encoder_params_rejected"), ("NOT_SUPPORTED", "encoder_params_rejected"))

    def test_unsupported_and_worked_is_a_mismatch(self):
        self.assertEqual(L.verdict("UNSUPPORTED", True, "ok"), ("MISMATCH", "unexpected_success"))

    def test_no_attempt_is_not_run(self):
        self.assertEqual(L.verdict("SUPPORTED", None, "no_sw_encoder"), ("NOT_RUN", "no_sw_encoder"))

    def test_our_own_configuration_errors_are_not_run_not_fail(self):
        for reason in L.CONFIG_REASONS:
            self.assertEqual(L.verdict("SUPPORTED", False, reason), ("NOT_RUN", reason))
            self.assertEqual(L.verdict("UNSUPPORTED", False, reason), ("NOT_RUN", reason))
        self.assertEqual(L.verdict("SUPPORTED", True, "ok", forced="no_ffmpeg"), ("NOT_RUN", "no_ffmpeg"))

    def test_summary_is_green_only_without_fail_and_mismatch(self):
        rows = [{"result": r} for r in ("PASS", "NOT_SUPPORTED", "NOT_RUN")]
        self.assertTrue(L.summarize(rows)[1])
        self.assertFalse(L.summarize(rows + [{"result": "FAIL"}])[1])
        self.assertFalse(L.summarize(rows + [{"result": "MISMATCH"}])[1])
        self.assertEqual(L.summarize(rows)[0]["NOT_SUPPORTED"], 1)

    def test_sessions_around_the_cap(self):
        self.assertEqual(L.session_outcome(3, 3, 8), (True, ""))
        self.assertEqual(L.session_outcome(7, 8, 8)[0], False)  # under the cap but one failed: a FAIL
        self.assertEqual(L.session_outcome(8, 9, 8), (False, ""))  # exactly the cap worked: NOT_SUPPORTED
        self.assertIn("n_ok=5 but cap 8", L.session_outcome(5, 9, 8)[1])
        self.assertIn("not enforced", L.session_outcome(9, 9, 8)[1])
        self.assertEqual(L.sessions_result("NOT_SUPPORTED", "x", "n_ok=5 but cap 8"), ("MISMATCH", "partial_sessions"))
        self.assertEqual(L.sessions_result("NOT_SUPPORTED", "x", ""), ("NOT_SUPPORTED", "x"))
        self.assertEqual(L.sessions_result("FAIL", "x", "n_ok=7 of 8 under cap 8"), ("FAIL", "x"))


class Psnr(unittest.TestCase):
    def test_parse_average(self):
        txt = "[Parsed_psnr_2 @ 0x1] PSNR y:40.1 u:42.0 v:43.0 average:40.9 min:39.0 max:45.0\n"
        self.assertAlmostEqual(L.parse_psnr(txt), 40.9)

    def test_parse_inf_is_identical_frames(self):
        self.assertEqual(L.parse_psnr("PSNR y:inf u:inf v:inf average:inf min:inf max:inf"), float("inf"))

    def test_parse_the_last_summary_wins(self):
        two = "PSNR y:1 u:1 v:1 average:10.0\nPSNR y:1 u:1 v:1 average:33.5\n"
        self.assertEqual(L.parse_psnr(two), 33.5)

    def test_parse_without_a_summary_is_none(self):
        self.assertIsNone(L.parse_psnr("no summary here"))

    def test_threshold_boundary(self):
        self.assertTrue(L.psnr_ok(28.0, 28.0))
        self.assertFalse(L.psnr_ok(27.99, 28.0))
        self.assertTrue(L.psnr_ok(float("inf"), 45.0))
        self.assertFalse(L.psnr_ok(None, 0.0))

    def test_every_case_has_a_threshold_in_range(self):
        for c in L.load_cases(CASES_TEXT):
            self.assertGreater(c["psnr_min"] if c["op"] != "sessions" else 1, 0, c["id"])
            if c["op"] == "dec":
                self.assertGreaterEqual(c["psnr_min"], 40, c["id"])  # hardware and software decode of one stream must match


class Commands(unittest.TestCase):
    def test_encode_argv(self):
        c = next(c for c in L.load_cases(CASES_TEXT) if c["id"] == "enc_hevc_444_8_1080p")
        a = L.encode_argv("/x/ffmpeg", c, "/o/out.mkv")
        self.assertIsInstance(a, list)
        self.assertEqual(a[0], "/x/ffmpeg")
        for tok in ("-pix_fmt", "yuv444p", "hevc_nvenc", "-profile:v", "rext", "constqp"):
            self.assertIn(tok, a)
        self.assertEqual(a[-1], "/o/out.mkv")
        self.assertIn("frames:v", " ".join(a))
        self.assertIn("all_seed=%d" % L.SEED, " ".join(a))

    def test_flags_replace_the_default_rate_control(self):
        c = next(c for c in L.load_cases(CASES_TEXT) if c["id"] == "enc_h264_bframes_lookahead")
        a = L.encode_argv("ffmpeg", c, "o.mkv")
        self.assertNotIn("constqp", a)
        self.assertIn("-rc-lookahead", a)
        self.assertEqual(a[a.index("-bf") + 1], "3")

    def test_the_source_is_generated_and_seeded(self):
        c = L.load_cases(CASES_TEXT)[0]
        self.assertIn("lavfi", L.encode_argv("f", c, "o"))
        self.assertTrue(L.source_spec(c).startswith("testsrc2=size="))
        self.assertNotIn("http", " ".join(L.encode_argv("f", c, "o")))

    def test_decode_compare_modes(self):
        hw = next(c for c in L.load_cases(CASES_TEXT) if c["id"] == "dec_hevc_420_10_hw")
        cu = next(c for c in L.load_cases(CASES_TEXT) if c["id"] == "dec_hevc_420_10_cuvid")
        a = L.decode_compare_argv("ffmpeg", hw, "s.mkv")
        self.assertEqual(a.count("s.mkv"), 2)
        self.assertIn("-hwaccel_output_format", a)  # no silent software fallback
        self.assertIn("hwdownload,format=p010le,format=yuv420p10le", " ".join(a))
        b = L.decode_compare_argv("ffmpeg", cu, "s.mkv")
        self.assertIn("hevc_cuvid", b)
        self.assertNotIn("-hwaccel", b)

    def test_av1_hwaccel_forces_the_native_decoder_and_only_there(self):
        av1 = next(c for c in L.load_cases(CASES_TEXT) if c["id"] == "dec_av1_420_8_hw")
        a = L.decode_compare_argv("ffmpeg", av1, "s.mkv")
        i = a.index("-hwaccel")
        self.assertEqual(a[i - 2:i], ["-c:v", "av1"])  # applies to the hardware input, not the software one
        self.assertNotIn("-c:v", a[:a.index("s.mkv", a.index("s.mkv") + 1)][:2])
        hevc = next(c for c in L.load_cases(CASES_TEXT) if c["id"] == "dec_hevc_420_8_hw")
        self.assertNotIn("-c:v", L.decode_compare_argv("ffmpeg", hevc, "s.mkv"))

    def test_the_444_10bit_surface_is_named_correctly(self):
        c = next(c for c in L.load_cases(CASES_TEXT) if c["id"] == "dec_hevc_444_10_hw")
        self.assertIn("yuv444p16le", " ".join(L.decode_compare_argv("ffmpeg", c, "s")))

    def test_software_encoder_choice(self):
        self.assertEqual(L.pick_sw_encoder("av1", {"libaom-av1", "libsvtav1"}), "libsvtav1")
        self.assertEqual(L.pick_sw_encoder("av1", {"libaom-av1"}), "libaom-av1")
        self.assertIsNone(L.pick_sw_encoder("av1", set()))
        self.assertIsNone(L.pick_sw_encoder("h264", {"h264_nvenc"}))

    def test_session_argv_is_a_sink(self):
        c = next(c for c in L.load_cases(CASES_TEXT) if c["id"] == "sess_h264_n04")
        a = L.session_argv("ffmpeg", c)
        self.assertEqual(a[-3:], ["-f", "null", "-"])
        self.assertIn("h264_nvenc", a)


class ReasonsAndCompare(unittest.TestCase):
    def test_reason_driver_api(self):
        self.assertEqual(L.classify_reason("[h264_nvenc @ 0x5] Driver does not support the required nvenc API version. Required: 13.0 Found: 12.2"), "driver_api_too_old")

    def test_reason_session_limit(self):
        self.assertEqual(L.classify_reason("[hevc_nvenc @ 0x5] OpenEncodeSessionEx failed: out of memory (10)"), "session_limit")

    def test_reason_session_limit_as_a_580_driver_words_it(self):
        # measured on a native RTX 3060 (traces/open_rows_ffdcgm_20261117): the cap is reported as an incompatible client key, then ffmpeg adds a second line
        text = "[h264_nvenc @ 0x1] OpenEncodeSessionEx failed: incompatible client key (21): (no details)\n[h264_nvenc @ 0x1] No capable devices found\n"
        self.assertEqual(L.classify_reason(text), "session_limit")

    def test_reason_caps_as_ffmpeg_words_it(self):
        # ffmpeg's own capability checks (measured on a native RTX 3060, traces/open_rows_ffdcgm_20261117), each followed by the generic line
        for first in ("[h264_nvenc @ 0x1] 10 bit encode not supported", "[h264_nvenc @ 0x1] Width 4224 exceeds 4096"):
            self.assertEqual(L.classify_reason(first + "\n[h264_nvenc @ 0x1] No capable devices found\n"), "caps_not_supported", first)

    def test_reason_a_bare_no_capable_devices_stays_a_device_error(self):
        self.assertEqual(L.classify_reason("[h264_nvenc @ 0x1] No capable devices found\n"), "no_encode_device")

    def test_reason_params(self):
        self.assertEqual(L.classify_reason("[av1_nvenc @ 0x5] InitializeEncoder failed: invalid param (8)"), "encoder_params_rejected")

    def test_reason_missing_encoder(self):
        self.assertEqual(L.classify_reason("Unknown encoder 'av1_nvenc'"), "unknown_encoder")

    def test_reason_hw_decode_as_ffmpeg_words_it_for_a_stream_nvdec_cannot_take(self):
        text = "decoding for stream 0 failed\nImpossible to convert between the formats supported by the filter 'graph 0 input from stream 1:0'\n"
        self.assertEqual(L.classify_reason(text), "hw_decode_unsupported")

    def test_reason_hw_decode(self):
        self.assertEqual(L.classify_reason("[hevc @ 0x1] Hardware is lacking required capabilities"), "hw_decode_unsupported")
        self.assertEqual(L.classify_reason("Failed setup for format cuda: hwaccel initialisation returned error."), "hw_decode_unsupported")

    def test_reason_no_device_as_a_vast_container_reports_it(self):
        self.assertEqual(L.classify_reason("[h264_nvenc @ 0x1] OpenEncodeSessionEx failed: unsupported device (2): (no details)"), "no_encode_device")
        self.assertEqual(L.classify_reason("cuvidGetDecoderCaps(&caps) failed -> CUDA_ERROR_NO_DEVICE: no CUDA-capable device is detected"), "no_decode_device")

    def test_reason_other(self):
        self.assertEqual(L.classify_reason("something else entirely"), "other")

    def _row(self, cid, result, reason="-"):
        return {"case_id": cid, "result": result, "reason": reason}

    def test_agreement_and_buckets(self):
        P, F, NS, NR = "PASS", "FAIL", "NOT_SUPPORTED", "NOT_RUN"
        c = lambda h, g, limits=(): L.classify_pair(self._row("x", h) if h else None, self._row("x", g) if g else None, limits)
        self.assertEqual(c(P, P), ("AGREE", ""))
        self.assertEqual(c(NS, NS), ("AGREE", ""))
        self.assertEqual(c(F, F), ("AGREE", L.BUCKET_HARNESS))  # the host control fails too: not kayfabe
        self.assertEqual(c(P, F), ("DISAGREE", L.BUCKET_CANDIDATE))  # works on the host, fails in the guest
        self.assertEqual(c(NS, P), ("DISAGREE", L.BUCKET_CANDIDATE))
        self.assertEqual(c(F, P), ("DISAGREE", L.BUCKET_HARNESS))
        self.assertEqual(c(P, NR), ("DISAGREE", L.BUCKET_CONFIG))
        self.assertEqual(c(NR, NR), ("AGREE", L.BUCKET_CONFIG))
        self.assertEqual(c(P, None), ("MISSING", L.BUCKET_CONFIG))

    LIMITS = "case_glob\tresult\treason\nenc_*_8k\tFAIL\tV3_VIDEO_ENGINES.md section 6\n"

    def test_a_documented_limit_moves_a_disagreement_out_of_candidates(self):
        h, g = self._row("enc_hevc_8k", "PASS"), self._row("enc_hevc_8k", "FAIL")
        self.assertEqual(L.classify_pair(h, g, L.load_known_limits(self.LIMITS)), ("DISAGREE", L.BUCKET_KNOWN))

    def test_a_limit_explains_only_the_result_it_names(self):
        h, g = self._row("enc_hevc_8k", "PASS"), self._row("enc_hevc_8k", "MISMATCH")
        self.assertEqual(L.classify_pair(h, g, L.load_known_limits(self.LIMITS)), ("DISAGREE", L.BUCKET_CANDIDATE))

    def test_a_limit_for_fail_does_not_explain_not_supported(self):
        h, g = self._row("enc_hevc_8k", "PASS"), self._row("enc_hevc_8k", "NOT_SUPPORTED")
        self.assertEqual(L.classify_pair(h, g, L.load_known_limits(self.LIMITS)), ("DISAGREE", L.BUCKET_CANDIDATE))

    def test_a_failing_host_control_is_not_a_kayfabe_candidate(self):
        h, g = self._row("enc_x", "FAIL"), self._row("enc_x", "PASS")
        self.assertEqual(L.classify_pair(h, g), ("DISAGREE", L.BUCKET_HARNESS))

    def test_a_limit_names_its_cases_by_glob(self):
        limits = L.load_known_limits("case_glob\tresult\treason\ndec_av1_*\tFAIL\tsome document\n")
        self.assertEqual(L.classify_pair(self._row("dec_av1_420_8_hw", "PASS"), self._row("dec_av1_420_8_hw", "FAIL"), limits), ("DISAGREE", L.BUCKET_KNOWN))
        self.assertEqual(L.classify_pair(self._row("dec_vp9_420_8_hw", "PASS"), self._row("dec_vp9_420_8_hw", "FAIL"), limits), ("DISAGREE", L.BUCKET_CANDIDATE))

    def test_a_limit_explains_only_the_cases_it_names(self):
        h2, g2 = self._row("enc_hevc_5k", "PASS"), self._row("enc_hevc_5k", "FAIL")
        self.assertEqual(L.classify_pair(h2, g2, L.load_known_limits(self.LIMITS)), ("DISAGREE", L.BUCKET_CANDIDATE))

    def test_limits_need_a_reason(self):
        with self.assertRaises(L.TableError):
            L.load_known_limits("enc_x\tFAIL\t \n")
        with self.assertRaises(L.TableError):
            L.load_known_limits("enc_x\tBROKEN\tbecause\n")

    def test_the_committed_limits_are_the_per_vm_encode_session_cap_and_nothing_else(self):
        # MEASURED 2026-11-17 (first guest run, RTX 3090): the host opens 8 NVENC sessions, the kf3 guest 3 (kf-qemu encquota ENC_SESSIONS_PER_VM = 3,
        # security review S1-42, docs/design/V3_SECURITY_MODEL.md): a documented limit, so sessions above 3 that the host serves are bucket 1, not a candidate.
        limits = L.load_known_limits((OR / "nvcodec_known_limits.tsv").read_text())
        self.assertTrue(limits)
        for glob, result, reason in limits:
            self.assertEqual(result, "NOT_SUPPORTED")
            self.assertIn("encquota", reason)
            self.assertIn("S1-42", reason)
        P, NS = "PASS", "NOT_SUPPORTED"
        for n in ("04", "05", "06", "08", "09", "12"):
            cid = "sess_h264_n" + n
            self.assertEqual(L.classify_pair(self._row(cid, P), self._row(cid, NS), limits), ("DISAGREE", L.BUCKET_KNOWN), cid)
        for n in ("01", "02", "03"):  # up to the cap the guest must serve what the host serves
            cid = "sess_h264_n" + n
            self.assertEqual(L.classify_pair(self._row(cid, P), self._row(cid, NS), limits), ("DISAGREE", L.BUCKET_CANDIDATE), cid)
        self.assertEqual(L.classify_pair(self._row("sess_h264_n05", P), self._row("sess_h264_n05", "FAIL"), limits), ("DISAGREE", L.BUCKET_CANDIDATE))
        self.assertEqual(L.classify_pair(self._row("enc_hevc_8k", P), self._row("enc_hevc_8k", NS), limits), ("DISAGREE", L.BUCKET_CANDIDATE))

    def test_probe_differences_are_reported_per_key(self):
        a, b = L.parse_probe(probe_text(av1=True)), L.parse_probe(probe_text(av1=True, sessions=5))
        d = L.compare_probes(a, b)
        self.assertEqual([k for k, _, _ in d], ["sessions.opened"])
        self.assertEqual(L.compare_probes(a, a), [])

    def test_results_round_trip(self):
        r = {"case_id": "enc_x", "group": "encode", "expect": "SUPPORTED", "result": "PASS", "reason": "ok", "rc": 0, "secs": "1.5", "psnr": "40.12",
             "frames": 30, "detail": "a\tb\nc"}
        back = L.parse_results(L.row_to_tsv(r) + "\n")
        self.assertEqual(back[0]["result"], "PASS")
        self.assertEqual(back[0]["detail"], "a b c")
        self.assertEqual(L.parse_results("case_id\tx\nbroken line\n"), [])

    def test_compare_results(self):
        h = [self._row("a", "PASS"), self._row("b", "NOT_SUPPORTED"), self._row("c", "PASS")]
        g = [self._row("a", "PASS"), self._row("b", "NOT_SUPPORTED"), self._row("c", "FAIL", "hw_decode_unsupported")]
        out = {r["case_id"]: r for r in L.compare_results(h, g)}
        self.assertEqual(out["a"]["agreement"], "AGREE")
        self.assertEqual(out["c"]["bucket"], L.BUCKET_CANDIDATE)
        self.assertEqual(out["c"]["guest_reason"], "hw_decode_unsupported")


# ---------------------------------------------------------------------------------------------------- NVML rows and diff
SMI_XML = """<?xml version="1.0" ?>
<!DOCTYPE nvidia_smi_log SYSTEM "nvsmi_device_v12.dtd">
<nvidia_smi_log>
  <timestamp>Mon Nov 17 15:00:00 2026</timestamp>
  <driver_version>580.126.20</driver_version>
  <attached_gpus>1</attached_gpus>
  <gpu id="00000000:01:00.0">
    <product_name>Fake Die FA4E</product_name>
    <uuid>%s</uuid>
    <minor_number>0</minor_number>
    <pci><pci_bus_id>00000000:01:00.0</pci_bus_id><tx_util>0 KB/s</tx_util></pci>
    <fb_memory_usage><total>12288 MiB</total><used>12 MiB</used></fb_memory_usage>
    <utilization><gpu_util>35 %%</gpu_util><memory_util>N/A</memory_util><encoder_util>0 %%</encoder_util></utilization>
    <temperature><gpu_temp>41 C</gpu_temp></temperature>
    <gpu_power_readings><power_draw>N/A</power_draw><current_power_limit>170.00 W</current_power_limit></gpu_power_readings>
    <clocks><graphics_clock>210 MHz</graphics_clock></clocks>
    <processes><process_info><pid>123</pid></process_info><process_info><pid>456</pid></process_info></processes>
  </gpu>
</nvidia_smi_log>
""" % FAKE_UUID


def rows_of(*lines):
    return N.parse_rows("\n".join(lines) + "\n")


def R(kind, key, status, value, label="-"):
    return N.row_line(kind, key, label, status, value)


class NvmlValues(unittest.TestCase):
    def test_numbers(self):
        self.assertEqual(N.number_of("43 %"), 43.0)
        self.assertEqual(N.number_of("250.00 W"), 250.0)
        self.assertEqual(N.number_of("0x1f"), 31.0)
        self.assertEqual(N.number_of("-5"), -5.0)
        for bad in ("N/A", "[N/A]", "Not Supported", "", "-", "abc", "1 2 3 4"):
            self.assertIsNone(N.number_of(bad), bad)
        self.assertTrue(N.is_na_text("[Not Supported]"))
        self.assertFalse(N.is_na_text("0"))

    def test_multi_part_values(self):
        self.assertEqual(N.numeric_parts("gpu=3;memory=1"), {"gpu": 3.0, "memory": 1.0})
        self.assertIsNone(N.numeric_parts("gpu=3;memory=N/A"))
        self.assertEqual(N.numeric_parts("7"), {"v": 7.0})
        self.assertIsNone(N.numeric_parts("-"))
        self.assertIsNone(N.numeric_parts(None))

    def test_smi_xml_is_flattened_and_identity_is_scrubbed(self):
        flat = N.flatten_smi_xml(SMI_XML)
        self.assertEqual(flat["nvidia_smi_log/gpu/utilization/gpu_util"], "35 %")
        self.assertEqual(flat["nvidia_smi_log/gpu/uuid"], "<scrubbed>")
        self.assertEqual(flat["nvidia_smi_log/gpu/pci/pci_bus_id"], "<scrubbed>")
        self.assertIn("nvidia_smi_log/gpu/processes/process_info[1]/pid", flat)
        text = "\n".join(N.smi_rows(flat))
        self.assertNotIn("fa4efa4e", text)
        self.assertIn("smi\tnvidia_smi_log/gpu/utilization/memory_util\t-\tNA\tN/A", text)

    def test_the_identity_of_a_smi_xml_never_reaches_a_row(self):
        text = "\n".join(N.smi_rows(N.flatten_smi_xml(SMI_XML)))
        for needle in (FAKE_UUID, "fa4efa4e", "00000000:01:00.0"):
            self.assertNotIn(needle, text)

    def test_the_identity_of_a_query_property_never_reaches_a_row(self):
        rows = N.smiq_rows(["uuid", "serial", "pci.bus_id", "name"], "%s, 123456, 00000000:01:00.0, Fake Die" % FAKE_UUID)
        text = "\n".join(rows)
        for needle in ("fa4efa4e", "123456", "00000000:01:00.0"):
            self.assertNotIn(needle, text)
        self.assertIn("name\t-\tOK\tFake Die", text)

    def test_help_query_gpu_and_the_csv_line(self):
        help_text = 'Section about timestamp.\n"timestamp"\nThe timestamp.\n"utilization.gpu"\nPercent.\n"uuid"\n"memory.total"\n'
        names = N.parse_help_query_gpu(help_text)
        self.assertEqual(names, ["timestamp", "utilization.gpu", "uuid", "memory.total"])
        rows = N.smiq_rows(names, "2026/11/17 15:00:00.1, [N/A], %s, 12288\n" % FAKE_UUID)
        self.assertIn("smiq\tutilization.gpu\t-\tNA\t[N/A]", rows)
        self.assertIn("smiq\tuuid\t-\tOK\t<scrubbed>", rows)
        self.assertNotIn("fa4efa4e", "\n".join(rows))
        with self.assertRaises(ValueError):
            N.smiq_rows(names, "1, 2")


CLASSES = N.load_classes((OR / "nvml_field_classes.tsv").read_text())
EXPECTED = N.load_expected_ns((OR / "nvml_expected_not_supported.tsv").read_text())


class NvmlDiff(unittest.TestCase):
    """The owner's rule: a number on the host must be a number in the guest, or a documented NOT_SUPPORTED."""

    def cmp(self, h, g, classes=CLASSES, table=EXPECTED):
        return N.compare_row(h, g, classes, table)

    def test_na_in_guest_where_host_has_a_number_is_a_finding(self):
        h = rows_of(R("query", "utilization.rates", "OK", "gpu=3;memory=1"))[("query", "utilization.rates")]
        g = rows_of(R("query", "utilization.rates", "NOT_SUPPORTED", "-"))[("query", "utilization.rates")]
        self.assertEqual(self.cmp(h, g)[0], "FINDING_NA_IN_GUEST")
        g2 = rows_of(R("query", "utilization.rates", "OK", "gpu=N/A;memory=1"))[("query", "utilization.rates")]
        self.assertEqual(self.cmp(h, g2)[0], "FINDING_NA_IN_GUEST")

    def test_smi_na_text_in_guest_is_a_finding(self):
        k = ("smi", "nvidia_smi_log/gpu/utilization/gpu_util")
        h = rows_of(R("smi", k[1], "OK", "35 %"))[k]
        g = rows_of(R("smi", k[1], "NA", "N/A"))[k]
        self.assertEqual(self.cmp(h, g)[0], "FINDING_NA_IN_GUEST")

    def test_a_documented_not_supported_is_expected(self):
        h = rows_of(R("query", "ecc.total_errors.corrected_volatile", "OK", "0"))[("query", "ecc.total_errors.corrected_volatile")]
        g = rows_of(R("query", "ecc.total_errors.corrected_volatile", "NOT_SUPPORTED", "-"))[("query", "ecc.total_errors.corrected_volatile")]
        st, detail = self.cmp(h, g)
        self.assertEqual(st, "EXPECTED_NS")
        self.assertIn("owner_ruling_H", detail)

    def test_a_listed_value_the_guest_serves_is_reported_so_the_line_can_go(self):
        h = rows_of(R("query", "ecc.mode", "OK", "v0=0;v1=0"))[("query", "ecc.mode")]
        self.assertEqual(self.cmp(h, h)[0], "OK_SERVED_THOUGH_LISTED")

    def test_an_unlisted_value_is_never_hidden_by_the_table(self):
        h = rows_of(R("query", "power.usage_mw", "OK", "25000"))[("query", "power.usage_mw")]
        g = rows_of(R("query", "power.usage_mw", "NOT_SUPPORTED", "-"))[("query", "power.usage_mw")]
        self.assertEqual(self.cmp(h, g)[0], "FINDING_NA_IN_GUEST")  # power is deliberately not in the table

    def test_a_live_counter_within_its_tolerance_is_ok(self):
        h = rows_of(R("query", "utilization.rates", "OK", "gpu=3;memory=1"))[("query", "utilization.rates")]
        near = rows_of(R("query", "utilization.rates", "OK", "gpu=20;memory=1"))[("query", "utilization.rates")]
        self.assertEqual(self.cmp(h, near)[0], "OK")

    def test_a_live_counter_beyond_its_tolerance_is_a_finding(self):
        h = rows_of(R("query", "utilization.rates", "OK", "gpu=3;memory=1"))[("query", "utilization.rates")]
        far = rows_of(R("query", "utilization.rates", "OK", "gpu=90;memory=1"))[("query", "utilization.rates")]
        self.assertEqual(self.cmp(h, far)[0], "FINDING_DIFF_LIVE")

    def test_a_static_value_must_be_equal(self):
        hs = rows_of(R("query", "static.num_cores", "OK", "3584"))[("query", "static.num_cores")]
        gs = rows_of(R("query", "static.num_cores", "OK", "3583"))[("query", "static.num_cores")]
        self.assertEqual(self.cmp(hs, gs)[0], "FINDING_DIFF_STATIC")
        self.assertEqual(self.cmp(hs, hs)[0], "OK")

    def test_a_static_smi_property_must_be_equal(self):
        k = ("smiq", "compute_cap")
        h, g = rows_of(R("smiq", "compute_cap", "OK", "8.6"))[k], rows_of(R("smiq", "compute_cap", "OK", "8.9"))[k]
        self.assertEqual(self.cmp(h, g)[0], "FINDING_DIFF_STATIC")

    def test_clocks_follow_the_pstate_so_only_the_number_is_owed(self):
        # 210 MHz idle on one native box, 1777 MHz boosted on another (traces/open_rows_ffdcgm_20261117)
        h = rows_of(R("query", "clock.current.sm", "OK", "210"))[("query", "clock.current.sm")]
        g = rows_of(R("query", "clock.current.sm", "OK", "1777"))[("query", "clock.current.sm")]
        self.assertEqual(self.cmp(h, g)[0], "OK")
        self.assertEqual(self.cmp(h, rows_of(R("query", "clock.current.sm", "NOT_SUPPORTED", "-"))[("query", "clock.current.sm")])[0], "FINDING_NA_IN_GUEST")

    def test_the_max_clock_of_one_die_is_static(self):
        h = rows_of(R("query", "clock.max.sm", "OK", "2130"))[("query", "clock.max.sm")]
        g = rows_of(R("query", "clock.max.sm", "OK", "2100"))[("query", "clock.max.sm")]
        self.assertEqual(self.cmp(h, g)[0], "FINDING_DIFF_STATIC")

    def test_the_supported_clock_lists_are_not_compared(self):
        k = ("smi", "nvidia_smi_log/gpu/supported_clocks/supported_mem_clock[0]/supported_graphics_clock[3]")
        self.assertEqual(self.cmp(rows_of(R("smi", k[1], "OK", "427 MHz"))[k], None)[0], "IGNORED")

    def test_the_device_identifier_and_part_number_never_reach_a_row(self):
        flat = N.flatten_smi_xml("<a><gpu><pdi>0x1f10be42c354148e</pdi><gpu_part_number>2503-300-A1</gpu_part_number><product_name>X</product_name></gpu></a>")
        self.assertEqual((flat["a/gpu/pdi"], flat["a/gpu/gpu_part_number"], flat["a/gpu/product_name"]), ("<scrubbed>", "<scrubbed>", "X"))

    def test_a_leaf_no_table_line_names_is_only_asked_to_be_a_number(self):
        k = ("smi", "nvidia_smi_log/gpu/unlisted_leaf")
        h, g = rows_of(R("smi", k[1], "OK", "5"))[k], rows_of(R("smi", k[1], "OK", "6"))[k]
        self.assertEqual(self.cmp(h, g)[0], "OK")

    def test_numeric_class_only_asks_for_a_number(self):
        h = rows_of(R("query", "memory.info", "OK", "total=12884901888;free=1;used=1"))[("query", "memory.info")]
        g = rows_of(R("query", "memory.info", "OK", "total=4294967296;free=1;used=1"))[("query", "memory.info")]
        self.assertEqual(self.cmp(h, g)[0], "OK")  # kayfabe's frame-buffer policy: the number is what is owed

    def test_a_missing_part_is_a_finding(self):
        h = rows_of(R("query", "memory.info", "OK", "total=1;free=1;used=1"))[("query", "memory.info")]
        g = rows_of(R("query", "memory.info", "OK", "total=1"))[("query", "memory.info")]
        self.assertEqual(self.cmp(h, g)[0], "FINDING_PART_MISSING")

    def test_a_missing_encoder_stat_part_is_a_finding(self):
        h = rows_of(R("query", "encoder.stats", "OK", "v0=0;v1=0;v2=0"))[("query", "encoder.stats")]
        g = rows_of(R("query", "encoder.stats", "OK", "v0=0;v1=0"))[("query", "encoder.stats")]
        self.assertEqual(self.cmp(h, g)[0], "FINDING_PART_MISSING")

    def test_a_missing_row_is_a_finding(self):
        h = rows_of(R("query", "memory.info", "OK", "total=1;free=1;used=1"))[("query", "memory.info")]
        self.assertEqual(self.cmp(h, None)[0], "FINDING_GUEST_MISSING")

    def test_a_field_missing_from_the_guest_is_a_finding_in_the_sweep(self):
        host = rows_of(R("field", "fi1", "OK", "5"))
        res, counts = N.compare_sweeps(host, {}, CLASSES, EXPECTED)
        self.assertEqual(counts, {"FINDING_GUEST_MISSING": 1})

    def test_the_sweep_marks_documented_not_supported_rows_expected(self):
        host = rows_of(R("query", "nvlink.state.0", "OK", "1"))
        guest = rows_of(R("query", "nvlink.state.0", "NOT_SUPPORTED", "-"))
        res, counts = N.compare_sweeps(host, guest, CLASSES, EXPECTED)
        self.assertEqual(counts, {"EXPECTED_NS": 1})
        self.assertEqual(N.findings(counts), 0)

    def test_a_live_value_exactly_at_the_tolerance_is_ok(self):
        h = rows_of(R("query", "utilization.rates", "OK", "gpu=3;memory=1"))[("query", "utilization.rates")]
        g = rows_of(R("query", "utilization.rates", "OK", "gpu=38;memory=1"))[("query", "utilization.rates")]  # abs:35
        self.assertEqual(self.cmp(h, g)[0], "OK")

    def test_a_live_smi_value_beyond_its_tolerance_is_a_finding(self):
        k = ("smi", "nvidia_smi_log/gpu/utilization/gpu_util")
        h, g = rows_of(R("smi", k[1], "OK", "3 %"))[k], rows_of(R("smi", k[1], "OK", "90 %"))[k]
        self.assertEqual(self.cmp(h, g)[0], "FINDING_DIFF_LIVE")

    def test_process_ids_are_not_compared(self):
        k = ("smi", "nvidia_smi_log/gpu/processes/process_info[0]/pid")
        h, g = rows_of(R("smi", k[1], "OK", "123"))[k], rows_of(R("smi", k[1], "OK", "999"))[k]
        self.assertEqual(self.cmp(h, g)[0], "IGNORED")

    def test_the_guest_serving_more_than_the_host_is_listed_not_failed(self):
        h = rows_of(R("query", "power.usage_mw", "NOT_SUPPORTED", "-"))[("query", "power.usage_mw")]
        g = rows_of(R("query", "power.usage_mw", "OK", "20000"))[("query", "power.usage_mw")]
        self.assertEqual(self.cmp(h, g)[0], "GUEST_ONLY_NUMBER")
        self.assertEqual(self.cmp(h, h)[0], "BOTH_NOT_NUMBER")

    def test_ignored_keys(self):
        k = ("smi", "nvidia_smi_log/timestamp")
        h = rows_of(R("smi", k[1], "OK", "1"))[k]
        self.assertEqual(self.cmp(h, None)[0], "IGNORED")

    def test_unknown_field_ids_default_to_numeric(self):
        h = rows_of(R("field", "fi99", "OK", "12", "NVML_FI_FAKE"))[("field", "fi99")]
        g = rows_of(R("field", "fi99", "NOT_SUPPORTED", "-"))[("field", "fi99")]
        self.assertEqual(self.cmp(h, g)[0], "FINDING_NA_IN_GUEST")
        g2 = rows_of(R("field", "fi99", "OK", "13"))[("field", "fi99")]
        self.assertEqual(self.cmp(h, g2)[0], "OK")

    def test_sweep_counts_findings(self):
        host = rows_of(R("field", "fi1", "OK", "5"), R("field", "fi2", "OK", "6"), R("field", "fi3", "NOT_SUPPORTED", "-"))
        guest = rows_of(R("field", "fi1", "OK", "5"), R("field", "fi2", "NOT_SUPPORTED", "-"), R("field", "fi9", "OK", "1"))
        res, counts = N.compare_sweeps(host, guest, CLASSES, EXPECTED)
        self.assertEqual(counts["FINDING_NA_IN_GUEST"], 1)
        self.assertEqual(counts["GUEST_EXTRA"], 1)
        self.assertEqual(N.findings(counts), 1)

    def test_tables_are_validated(self):
        with self.assertRaises(N.TableError):
            N.load_classes("query:x\tbogus\t-\n")
        with self.assertRaises(N.TableError):
            N.load_classes("query:x\tlive\t0.5\n")
        with self.assertRaises(N.TableError):
            N.load_expected_ns("query:x\tgroup\t \n")
        for glob, group, reason in EXPECTED:
            self.assertGreater(len(reason), 20, glob)

    def test_tolerance_boundaries(self):
        self.assertTrue(N.within(10, 15, "abs:5"))
        self.assertFalse(N.within(10, 16, "abs:5"))
        self.assertTrue(N.within(100, 130, "rel:0.3"))
        self.assertFalse(N.within(100, 170, "rel:0.3"))


DISCOVERY = """1 GPU found (Active).
+--------+----------------------------------------------------------------------+
| GPU ID | Device Information                                                   |
+--------+----------------------------------------------------------------------+
| 0      | Name: Fake Die FA4E                                                  |
|        | PCI Bus ID: 00000000:01:00.0                                         |
|        | Device UUID: GPU-fa4efa4e-fa4e-4fa4-8fa4-fa4efa4e0001                |
+--------+----------------------------------------------------------------------+
0 NvSwitches found.
"""
DMON_LIST = """________________________________________________________________________________
Long Name                                                             Field ID
________________________________________________________________________________
driver_version                                                        1
nvml_version                                                          2
gpu_temp                                                              150
gpu_utilization                                                       203
"""
DMON = """#Entity         150            203
ID              C
GPU 0           41             N/A
GPU 0           42             N/A
GPU 1           99             9
"""
DMON_TEXT = """#Entity         1              2              52
ID              #              #
GPU 0           580.126.20     13.580.126.20  0
"""
DMON_REFUSED = "Error setting watches. Result: -36: Profiling is not supported for this group of GPUs or GPU\n"
DIAG_V4 = json.dumps({"DCGM Diagnostic": {"test_categories": [
    {"category": "Deployment", "tests": [{"name": "software", "results": [{"gpu_id": "0", "status": "Pass"}]}]},
    {"category": "Hardware", "tests": [{"name": "memory", "results": [{"gpu_id": "0", "status": "Skip", "info": "not supported on this GPU"}]},
                                       {"name": "pcie", "results": [{"gpu_id": "0", "status": "Fail", "warnings": [{"warning": "x"}]}]}]}]}})
DIAG_V3 = json.dumps({"DCGM GPU Diagnostic": {"test_categories": [{"category": "Integration", "tests": [{"name": "Diagnostic", "results": [
    {"entity_group": "GPU", "entity_id": 0, "status": "Pass"}]}]}]}})


class DcgmParsers(unittest.TestCase):
    def test_discovery_and_lists(self):
        self.assertEqual(N.parse_dcgm_discovery(DISCOVERY), 1)
        self.assertIsNone(N.parse_dcgm_discovery("Error: unable to connect"))
        self.assertEqual(N.parse_dcgm_dmon_list(DMON_LIST), [(1, "driver_version"), (2, "nvml_version"), (150, "gpu_temp"), (203, "gpu_utilization")])

    def test_dmon_values_and_na(self):
        series = N.parse_dcgm_dmon(DMON, ["gpu_temp", "gpu_utilization"])
        self.assertEqual(series["gpu_temp"], ["41", "42"])
        self.assertEqual(series["gpu_utilization"], ["N/A", "N/A"])
        rows = N.dmon_rows(series)
        self.assertIn("dcgm\tgpu_temp\t-\tOK\t42", rows)
        self.assertIn("dcgm\tgpu_utilization\t-\tNA\tN/A", rows)
        self.assertEqual(N.dmon_rows({"X": []})[0], "dcgm\tX\t-\tNA\tno sample")

    def test_a_text_field_is_kept_as_text(self):
        rows = N.dmon_rows(N.parse_dcgm_dmon(DMON_TEXT, ["driver_version", "nvml_version", "nvml_index"]))
        self.assertEqual(rows, ["dcgm\tdriver_version\t-\tOK\t580.126.20", "dcgm\tnvml_version\t-\tOK\t13.580.126.20", "dcgm\tnvml_index\t-\tOK\t0"])

    def test_dmon_blank_and_not_found_cells_are_none_not_zero(self):
        text = "#Entity A B\nGPU 0 Blank 7\n"
        rows = N.dmon_rows(N.parse_dcgm_dmon(text, ["A", "B"]))
        self.assertEqual(rows, ["dcgm\tA\t-\tNA\tN/A", "dcgm\tB\t-\tOK\t7"])

    def test_a_dmon_field_with_only_na_cells_is_listed_na(self):
        rows = N.dmon_rows(N.parse_dcgm_dmon("GPU 0 N/A 5\nGPU 0 N/A 6\n", ["A", "B"]))
        self.assertEqual(rows, ["dcgm\tA\t-\tNA\tN/A", "dcgm\tB\t-\tOK\t6"])

    def test_a_refused_watch_is_a_not_supported_row(self):
        self.assertEqual(N.dmon_error(DMON_REFUSED), (-36, "Profiling is not supported for this group of GPUs or GPU"))
        self.assertEqual(N.dmon_error_row("PROF_X", DMON_REFUSED), "dcgm\tPROF_X\t-\tNOT_SUPPORTED\t-36: Profiling is not supported for this group of GPUs or GPU")

    def test_another_error_is_an_error_row_and_no_output_is_named(self):
        self.assertEqual(N.dmon_error_row("A", "Error setting watches. Result: -7: Something else\n").split("\t")[3], "ERR_-7")
        self.assertEqual(N.dmon_error_row("A", "").split("\t")[3], "ERR_NO_TABLE")

    def test_a_cell_with_spaces_stays_in_its_column(self):
        text = ("#Entity         50                        51          52\n"
                "ID                                                    \n"
                "GPU 0           NVIDIA GeForce RTX 3060   GeForce     0\n")
        series = N.parse_dcgm_dmon(text, ["name", "brand", "nvml_index"])
        self.assertEqual(series, {"name": ["NVIDIA GeForce RTX 3060"], "brand": ["GeForce"], "nvml_index": ["0"]})

    def test_a_real_dmon_row_of_a_geforce(self):
        # the output of `dcgmi dmon -e 50,51,52,54,150,203 -c 1` of DCGM 4.8.0 on an RTX 3060 (traces/open_rows_ffdcgm_20261117), the UUID replaced by the fake one
        text = ("#Entity         50                          51                52           54                                              150          203               \n"
                "ID                                                                                                                          C                             \n"
                "GPU 0           NVIDIA GeForce RTX 3060     GeForce           0            GPU-fa4efa4e-fa4e-4fa4-8fa4-fa4efa4e0001        38           0                 \n")
        series = N.parse_dcgm_dmon(text, ["name", "brand", "nvml_index", "uuid", "gpu_temp", "gpu_utilization"])
        self.assertEqual(series["name"], ["NVIDIA GeForce RTX 3060"])
        self.assertEqual((series["brand"], series["nvml_index"], series["gpu_temp"], series["gpu_utilization"]), (["GeForce"], ["0"], ["38"], ["0"]))
        rows = N.dmon_rows(series)
        self.assertIn("dcgm\tname\t-\tOK\tNVIDIA GeForce RTX 3060", rows)
        self.assertNotIn("fa4efa4e", "\n".join(rows))

    def test_identity_fields_are_never_recorded(self):
        rows = N.dmon_rows({"uuid": ["GPU-fa4efa4e-fa4e-4fa4-8fa4-fa4efa4e0001"], "serial_number": ["12345"], "pci_busid": ["00000000:01:00.0"], "gpu_temp": ["41"]})
        text = "\n".join(rows)
        for needle in ("fa4efa4e", "12345", "00000000:01:00.0"):
            self.assertNotIn(needle, text)
        self.assertIn("dcgm\tgpu_temp\t-\tOK\t41", rows)

    def test_dmon_ignores_other_gpus(self):
        self.assertEqual(N.parse_dcgm_dmon("GPU 1 99\nGPU 0 5\n", ["A"]), {"A": ["5"]})

    def test_a_diag_warning_is_read_as_its_message(self):
        doc = {"DCGM Diagnostic": {"test_categories": [{"category": "Deployment", "tests": [{"name": "software", "results": [{"entity_id": 0, "status": "Fail", "warnings": [
            {"error_category": 8, "error_id": 29, "error_severity": 5, "warning": "Persistence Mode: Persistence mode for GPU 0 is disabled."}]}]}]}]}}
        # the software diagnostic of a native run on a VM whose persistence mode was off (traces/open_rows_ffdcgm_20261117)
        tests = N.parse_diag_json(json.dumps(doc))
        self.assertEqual(tests[0][3:], ("Fail", "Persistence Mode: Persistence mode for GPU 0 is disabled."))

    def test_diag_json_both_layouts(self):
        t4 = N.parse_diag_json(DIAG_V4)
        self.assertEqual([(c, n, s) for c, n, g, s, i in t4], [("Deployment", "software", "Pass"), ("Hardware", "memory", "Skip"), ("Hardware", "pcie", "Fail")])
        self.assertEqual((t4[1][4], t4[2][4]), ("not supported on this GPU", "x"))  # a result's `info`, and a `warnings` list read as its messages
        t3 = N.parse_diag_json("log line before\n" + DIAG_V3)
        self.assertEqual(t3[0][3], "Pass")
        self.assertEqual(t3[0][2], "0")
        self.assertEqual(N.parse_diag_json("not json"), [])
        self.assertEqual(N.parse_diag_json(""), [])

    def test_a_test_the_host_passes_that_fails_in_the_guest_is_a_finding(self):
        host = N.parse_rows("\n".join(N.diag_rows(N.parse_diag_json(DIAG_V4))) + "\n")
        guest_doc = json.loads(DIAG_V4)
        guest_doc["DCGM Diagnostic"]["test_categories"][0]["tests"][0]["results"][0]["status"] = "Fail"
        guest = N.parse_rows("\n".join(N.diag_rows(N.parse_diag_json(json.dumps(guest_doc)))) + "\n")
        out = {k: v for k, _, _, v in N.compare_diag(host, guest)}
        self.assertEqual(out["Deployment/software/gpu0"], "FINDING_DIAG_FAIL_IN_GUEST")
        self.assertEqual(out["Hardware/pcie/gpu0"], "AGREE")

    NVVS_LOG = ("2026-11-17 15:05:42.357 INFO  [7038:7038] Test memory: The Memory test is skipped. Check DCGM and system configuration. This error may be "
                "eliminated with an updated configuration. [./nvvs/src/PluginTest.cpp:221] [PluginTest::AddInfo]\n"
                "2026-11-17 15:05:42.358 DEBUG [7038:7038] Added info for test 0:diagnostic [./nvvs/src/PluginLib.cpp:327] [PluginLib::GetPluginInfo]\n"
                "2026-11-17 15:05:42.359 INFO  [7038:7038] Test pcie: Not enough bandwidth. [./nvvs/src/PluginTest.cpp:221] [PluginTest::AddInfo]\n")

    def test_the_skip_reason_comes_from_nvvs_log(self):
        info = N.parse_nvvs_test_info(self.NVVS_LOG)
        self.assertEqual(sorted(info), ["memory", "pcie"])
        self.assertTrue(info["memory"].startswith("The Memory test is skipped."))

    def test_diag_rows_carry_the_skip_reason(self):
        doc = json.loads(DIAG_V4)
        for c in doc["DCGM Diagnostic"]["test_categories"]:
            for t in c["tests"]:
                for r in t["results"]:
                    r.pop("info", None)  # the real JSON of a skipped test carries no reason
        rows = N.diag_rows(N.parse_diag_json(json.dumps(doc)), N.parse_nvvs_test_info(self.NVVS_LOG))
        skip = next(r for r in rows if "Hardware/memory" in r)
        self.assertIn("SKIP\tThe Memory test is skipped.", skip)

    def test_gpu_work_ran_only_for_tests_outside_deployment_that_gave_a_verdict(self):
        self.assertTrue(N.gpu_work_ran(N.parse_diag_json(DIAG_V4)))  # pcie failed: it worked the GPU
        only_deploy = json.dumps({"DCGM Diagnostic": {"test_categories": [{"category": "Deployment", "tests": [{"name": "software", "results": [{"entity_id": 0, "status": "Pass"}]}]}]}})
        self.assertFalse(N.gpu_work_ran(N.parse_diag_json(only_deploy)))
        all_skipped = json.dumps({"DCGM Diagnostic": {"test_categories": [{"category": "Stress", "tests": [{"name": "targeted_stress", "results": [{"entity_id": 0, "status": "Skip"}]}]}]}})
        self.assertFalse(N.gpu_work_ran(N.parse_diag_json(all_skipped)))

    def test_diag_rows_and_comparison(self):
        host = N.parse_rows("\n".join(N.diag_rows(N.parse_diag_json(DIAG_V4))) + "\n")
        guest_doc = json.loads(DIAG_V4)
        guest_doc["DCGM Diagnostic"]["test_categories"][0]["tests"][0]["results"][0]["status"] = "Fail"
        guest = N.parse_rows("\n".join(N.diag_rows(N.parse_diag_json(json.dumps(guest_doc)))) + "\n")
        out = {k: (h, g, v) for k, h, g, v in N.compare_diag(host, guest)}
        self.assertEqual(out["Deployment/software/gpu0"], ("PASS", "FAIL", "FINDING_DIAG_FAIL_IN_GUEST"))
        self.assertEqual(out["Hardware/memory/gpu0"][2], "AGREE")
        self.assertEqual(out["Hardware/pcie/gpu0"][2], "AGREE")  # the host control fails it too: not a finding
        guest.pop(("diag", "Hardware/memory/gpu0"))
        self.assertEqual({k: v for k, _, _, v in N.compare_diag(host, guest)}["Hardware/memory/gpu0"], "DIFFER")


class Census(unittest.TestCase):
    def test_scrub(self):
        s = census.scrub("uuid %s ip 10.20.30.40 mac 52:54:00:12:34:56 pci 0000:01:00.0 driver 580.159.04 v 12.2.72" % FAKE_UUID)
        self.assertNotIn("fa4efa4e", s)
        self.assertNotIn("10.20.30.40", s)
        self.assertNotIn("52:54", s)
        self.assertNotIn("0000:01:00.0", s)
        self.assertIn("580.159.04", s)

    def test_smi_numbers(self):
        self.assertEqual(census.parse_smi_line("2, 40, 0, 55, 1200"), {"sess": 2.0, "enc": 40.0, "dec": 0.0, "gpu": 55.0, "mem": 1200.0})

    def test_smi_na_is_not_zero(self):
        self.assertEqual(census.parse_smi_line("0, [N/A], [N/A], [N/A], 12")["enc"], None)
        self.assertIsNone(census.parse_smi_line("garbage"))

    def S(self, *rows):
        return [(float(i), dict(zip(("sess", "enc", "dec", "gpu", "mem"), r))) for i, r in enumerate(rows)]

    def test_gpu_use_is_proven_only_by_a_readable_counter(self):
        proven = census.gpu_use_verdict(census.peaks(self.S((0, 0, 0, 0, 100), (1, 30, 0, 50, 300), (1, 40, 0, 60, 300))), "enc")
        self.assertEqual(proven[0], "PROVEN")
        idle = census.gpu_use_verdict(census.peaks(self.S((0, 0, 0, 0, 100), (0, 0, 0, 0, 100))), "enc")
        self.assertEqual(idle[0], "FAILED")
        na = census.gpu_use_verdict(census.peaks(self.S((None, None, None, None, 100), (None, None, None, None, 101))), "enc")
        self.assertEqual(na[0], "UNPROVEN")
        self.assertIn("N/A", na[1])

    def test_no_data_is_never_a_pass(self):
        self.assertEqual(census.gpu_use_verdict(census.peaks([]), "gpu")[0], "UNPROVEN")
        self.assertEqual(census.gpu_use_verdict(census.peaks([]), "enc")[0], "UNPROVEN")

    def test_na_counters_are_unproven_not_failed(self):
        p = census.peaks(self.S((None, None, None, None, 100), (None, None, None, None, 100)))
        self.assertEqual(census.gpu_use_verdict(p, "gpu")[0], "UNPROVEN")

    def test_a_single_busy_sample_is_not_enough_for_the_encoder(self):
        p = census.peaks(self.S((None, 0, None, None, 1), (None, 60, None, None, 1), (None, 0, None, None, 1)))
        self.assertEqual(census.gpu_use_verdict(p, "enc")[0], "FAILED")

    def test_cli_verdict_without_samples_is_not_proven(self):
        with tempfile.TemporaryDirectory() as d:
            (Path(d) / "s.csv").write_text("")
            p = subprocess.run([sys.executable, str(OR / "census.py"), "verdict", "--csv", str(Path(d) / "s.csv"), "--need", "gpu"], capture_output=True, text=True)
            self.assertEqual(p.returncode, 1)
            self.assertIn("host_gpu_use=UNPROVEN", p.stdout)

    def test_cli_verdict_accepts_the_session_counter_alone(self):
        with tempfile.TemporaryDirectory() as d:
            (Path(d) / "s.csv").write_text(census.samples_to_csv(self.S((0, None, None, None, 1), (4, None, None, None, 1))))
            p = subprocess.run([sys.executable, str(OR / "census.py"), "verdict", "--csv", str(Path(d) / "s.csv"), "--need", "enc"], capture_output=True, text=True)
            self.assertEqual(p.returncode, 0, p.stdout)

    def test_cli_kf3_counts_only_the_row_in_the_row(self):
        with tempfile.TemporaryDirectory() as d:
            log = Path(d) / "q.log"
            log.write_text("kf3: GSP rpc UNSERVICED Unserviced { code: 76, sequence: 1 }\nboot line\nkf3: GSP rpc UNSERVICED Unserviced { code: 76, sequence: 2 }\n")
            p = subprocess.run([sys.executable, str(OR / "census.py"), "kf3", "--log", str(log), "--before", "2"], capture_output=True, text=True)
            self.assertIn("unserviced_before=1 unserviced_after=2 unserviced_in_row=1", p.stdout)

    def test_an_informational_need_records_the_peaks_and_never_fails(self):
        p = census.peaks(self.S((0, 0, 0, 0, 100), (0, 0, 0, 0, 100)))
        verdict, why = census.gpu_use_verdict(p, "info")
        self.assertEqual(verdict, "INFO")
        self.assertIn("gpu=0", why)
        self.assertEqual(census.gpu_use_verdict(census.peaks([]), "info")[0], "UNPROVEN")  # no data is still never a pass

    def test_cli_info_need_exits_zero_on_an_idle_gpu(self):
        with tempfile.TemporaryDirectory() as d:
            (Path(d) / "s.csv").write_text(census.samples_to_csv(self.S((0, 0, 0, 0, 100), (0, 0, 0, 0, 100))))
            p = subprocess.run([sys.executable, str(OR / "census.py"), "verdict", "--csv", str(Path(d) / "s.csv"), "--need", "info"], capture_output=True, text=True)
            self.assertEqual(p.returncode, 0, p.stdout)
            self.assertIn("host_gpu_use=INFO", p.stdout)

    def test_session_counter_alone_proves_an_encode(self):
        p = census.peaks(self.S((0, None, None, None, 100), (3, None, None, None, 100)))
        self.assertEqual(census.gpu_use_verdict(p, "enc")[0], "PROVEN")

    def test_gpu_need_needs_two_samples(self):
        one = census.gpu_use_verdict(census.peaks(self.S((0, 0, 0, 0, 1), (0, 0, 0, 90, 1), (0, 0, 0, 0, 1))), "gpu")
        self.assertEqual(one[0], "FAILED")
        two = census.gpu_use_verdict(census.peaks(self.S((0, 0, 0, 90, 1), (0, 0, 0, 80, 1))), "gpu")
        self.assertEqual(two[0], "PROVEN")

    def test_csv_round_trip(self):
        s = self.S((1, None, 0, 5, 100), (2, 3, 4, 5, 101))
        back = census.csv_to_samples(census.samples_to_csv(s))
        self.assertEqual(back[0][1]["enc"], None)
        self.assertEqual(back[1][1]["dec"], 4.0)

    def test_unserviced_census_counts_before_and_after_and_has_a_known_positive(self):
        before = ["kf3: GSP rpc UNSERVICED Unserviced { code: 76, sequence: %d }" % i for i in range(3)] + ["kf3: family=Ada phase=Running trapped=5 unserviced=7"]
        row = ["kf3: GSP rpc UNSERVICED Unserviced { code: 76, sequence: 9 }", "kf3: GSP rpc UNSERVICED Fn(0x1f) uuid %s" % FAKE_UUID, "other line"]
        c = census.unserviced_counts(before + row, len(before))
        self.assertEqual((c["before"], c["after"], c["in_row"]), (3, 5, 2))  # the status counter `unserviced=7` is not an event
        self.assertIn("code: 76", c["ids"])
        text = census.format_unserviced(c)
        self.assertIn("unserviced_before=3 unserviced_after=5 unserviced_in_row=2", text)
        self.assertNotIn("fa4efa4e", text)
        zero = census.unserviced_counts(["nothing here"], 0)
        self.assertEqual((zero["before"], zero["after"], zero["in_row"]), (0, 0, 0))

    def test_cli_verdict_and_kf3(self):
        with tempfile.TemporaryDirectory() as d:
            csv = Path(d) / "s.csv"
            csv.write_text(census.samples_to_csv(self.S((2, 30, 0, 50, 10), (2, 30, 0, 50, 10))))
            log = Path(d) / "q.log"
            log.write_text("a\nkf3: GSP rpc UNSERVICED Unserviced { code: 76, sequence: 1 }\n")
            p = subprocess.run([sys.executable, str(OR / "census.py"), "verdict", "--csv", str(csv), "--need", "enc"], capture_output=True, text=True)
            self.assertEqual(p.returncode, 0)
            self.assertIn("host_gpu_use=PROVEN", p.stdout)
            p = subprocess.run([sys.executable, str(OR / "census.py"), "kf3", "--log", str(log), "--before", "1"], capture_output=True, text=True)
            self.assertIn("unserviced_before=0 unserviced_after=1 unserviced_in_row=1", p.stdout)


class RowIo(unittest.TestCase):
    LOG = "noise\n" + rowio.emit("a.tsv", "x\ty\n") + "more noise\n" + rowio.emit("b.tsv", "z\n")

    def test_round_trip(self):
        self.assertEqual(rowio.extract(self.LOG), {"a.tsv": "x\ty\n", "b.tsv": "z\n"})

    def test_an_edited_block_is_dropped(self):
        self.assertEqual(rowio.extract(self.LOG.replace("x\ty", "x\tq")), {"b.tsv": "z\n"})

    def test_a_same_length_edit_is_still_dropped_by_the_digest(self):
        self.assertEqual(rowio.extract(self.LOG.replace("z\n@@END", "y\n@@END")), {"a.tsv": "x\ty\n"})

    def test_a_truncated_block_is_dropped_never_repaired(self):
        self.assertEqual(rowio.extract(self.LOG.replace("@@END b.tsv\n", "")), {"a.tsv": "x\ty\n"})

    def test_a_block_with_a_wrong_declared_length_is_dropped(self):
        good = rowio.emit("a.tsv", "x\ty\n")
        bad = good.replace("bytes=4", "bytes=5")
        self.assertNotEqual(good, bad)
        self.assertEqual(rowio.extract(bad), {})

    def test_load_refuses_a_log_with_an_edited_block(self):
        with tempfile.TemporaryDirectory() as d:
            lg = Path(d) / "row.log"
            lg.write_text(rowio.emit("a.tsv", "aaaa\n").replace("aaaa", "aaab"))
            self.assertIsNone(rowio.load(str(lg), "a.tsv"))

    def test_a_missing_end_marker_drops_the_block_even_when_the_next_block_follows(self):
        text = rowio.emit("a.tsv", "x\n").replace("@@END a.tsv\n", "") + rowio.emit("b.tsv", "y\n")
        self.assertNotIn("a.tsv", rowio.extract(text))

    def test_load_refuses_a_block_with_a_wrong_declared_length(self):
        with tempfile.TemporaryDirectory() as d:
            lg = Path(d) / "row.log"
            lg.write_text(rowio.emit("a.tsv", "x\ty\n").replace("bytes=4", "bytes=5"))
            self.assertIsNone(rowio.load(str(lg), "a.tsv"))

    def test_load_drops_a_block_whose_end_marker_is_missing(self):
        with tempfile.TemporaryDirectory() as d:
            lg = Path(d) / "row.log"
            lg.write_text(rowio.emit("a.tsv", "x\n").replace("@@END a.tsv\n", "") + rowio.emit("b.tsv", "y\n"))
            self.assertIsNone(rowio.load(str(lg), "a.tsv"))
            self.assertEqual(rowio.load(str(lg), "b.tsv"), "y\n")

    def test_load_from_directory_or_log(self):
        with tempfile.TemporaryDirectory() as d:
            (Path(d) / "a.tsv").write_text("q\n")
            self.assertEqual(rowio.load(d, "a.tsv"), "q\n")
            self.assertIsNone(rowio.load(d, "missing.tsv"))
            lg = Path(d) / "row.log"
            lg.write_text(rowio.emit("a.tsv", "w\n"))
            self.assertEqual(rowio.load(str(lg), "a.tsv"), "w\n")
            self.assertIsNone(rowio.load(str(Path(d) / "nope"), "a.tsv"))


# ----------------------------------------------------------------------- the sweep against a fake libnvidia-ml (built here)
FAKE_NVML_C = r"""
#include <string.h>
typedef struct { unsigned fieldId, scopeId; long long timestamp, latencyUsec; int valueType, nvmlReturn; unsigned char value[8]; } FV;
int nvmlInit_v2(void) { return 0; }
int nvmlShutdown(void) { return 0; }
int nvmlDeviceGetHandleByIndex_v2(unsigned i, void **h) { *h = (void *)0x1000; return i == 0 ? 0 : 2; }
int nvmlDeviceGetFieldValues(void *h, int n, FV *v) {
  for (int i = 0; i < n; i++) {
    unsigned id = v[i].fieldId;
    if (id == 7) { v[i].nvmlReturn = 0; v[i].valueType = 1; unsigned x = 1234; memcpy(v[i].value, &x, 4); }
    else if (id == 9) { v[i].nvmlReturn = 0; v[i].valueType = 0; double d = 2.5; memcpy(v[i].value, &d, 8); }
    else if (id == 11) { v[i].nvmlReturn = 3; }
    else v[i].nvmlReturn = 2;
  }
  return 0;
}
typedef struct { unsigned gpu, memory; } Util;
int nvmlDeviceGetUtilizationRates(void *h, Util *u) { u->gpu = 42; u->memory = 7; return 0; }
int nvmlDeviceGetEncoderUtilization(void *h, unsigned *a, unsigned *b) { *a = 5; *b = 167000; return 0; }
int nvmlDeviceGetDecoderUtilization(void *h, unsigned *a, unsigned *b) { return 3; }
int nvmlDeviceGetClockInfo(void *h, unsigned t, unsigned *c) { *c = 1000 + t; return 0; }
int nvmlDeviceGetName(void *h, char *b, unsigned n) { strcpy(b, "Fake Die FA4E"); return 0; }
int nvmlDeviceGetComputeRunningProcesses_v3(void *h, unsigned *n, void *p) { *n = 2; return 7; }
int nvmlDeviceGetTotalEccErrors(void *h, unsigned e, unsigned c, unsigned long long *o) { *o = 0; return 0; }
int nvmlDeviceGetCudaComputeCapability(void *h, int *a, int *b) { *a = 8; *b = 6; return 0; }
int nvmlSystemGetDriverVersion(char *b, unsigned n) { strcpy(b, "580.126.20"); return 0; }
int nvmlSystemGetNVMLVersion(char *b, unsigned n) { strcpy(b, "13.0.19.2"); return 0; }
int nvmlSystemGetCudaDriverVersion(int *v) { *v = 13000; return 0; }
"""


@unittest.skipUnless(shutil.which("gcc"), "needs gcc to build a fake libnvidia-ml")
class SweepAgainstFakeLibrary(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        cls.tmp = tempfile.TemporaryDirectory()
        c = Path(cls.tmp.name) / "fake.c"
        c.write_text(FAKE_NVML_C)
        cls.lib = str(Path(cls.tmp.name) / "libfakenvml.so")
        subprocess.run(["gcc", "-shared", "-fPIC", "-o", cls.lib, str(c)], check=True)
        cls.rows, cls.meta = nvml_sweep.sweep(cls.lib, 127, None)
        cls.parsed = N.parse_rows("\n".join(cls.rows) + "\n")

    @classmethod
    def tearDownClass(cls):
        cls.tmp.cleanup()

    def test_an_unsigned_field_is_decoded_by_its_value_type(self):
        self.assertEqual((self.parsed[("field", "fi7")]["status"], self.parsed[("field", "fi7")]["value"]), ("OK", "1234"))

    def test_a_double_field_is_decoded_by_its_value_type(self):
        self.assertEqual(self.parsed[("field", "fi9")]["value"], "2.5")

    def test_a_field_the_driver_calls_not_supported_is_listed_so(self):
        self.assertEqual(self.parsed[("field", "fi11")]["status"], "NOT_SUPPORTED")

    def test_an_id_the_library_does_not_know_is_not_listed(self):
        self.assertNotIn(("field", "fi8"), self.parsed)  # INVALID_ARGUMENT = the library does not know the id

    def test_unknown_and_known_ids_are_counted(self):
        self.assertIn("known_ids=3 ", self.meta[0])
        self.assertIn("unknown_ids=125 ", self.meta[0])

    def test_typed_queries(self):
        self.assertEqual(self.parsed[("query", "utilization.rates")]["value"], "gpu=42;memory=7")
        self.assertEqual(self.parsed[("query", "utilization.encoder")]["value"], "v0=5;v1=167000")
        self.assertEqual(self.parsed[("query", "utilization.decoder")]["status"], "NOT_SUPPORTED")
        self.assertEqual(self.parsed[("query", "clock.current.sm")]["value"], "1001")
        self.assertEqual(self.parsed[("query", "name")]["value"], "Fake Die FA4E")
        self.assertEqual(self.parsed[("query", "processes.compute")]["value"], "count=2")
        self.assertEqual(self.parsed[("query", "static.cuda_cc")]["value"], "v0=8;v1=6")
        self.assertEqual(self.parsed[("query", "system.driver_version")]["value"], "580.126.20")
        self.assertEqual(self.parsed[("query", "system.nvml_version")]["value"], "13.0.19.2")  # four numbers that look like an address stay

    def test_a_version_string_that_looks_like_an_address_is_kept(self):
        self.assertEqual(self.parsed[("query", "system.nvml_version")]["value"], "13.0.19.2")
        self.assertEqual(self.parsed[("query", "system.cuda_driver_version")]["value"], "13000")

    def test_a_missing_symbol_is_a_row_not_a_crash(self):
        self.assertEqual(self.parsed[("query", "power.usage_mw")]["status"], "NO_SYMBOL")

    def test_every_query_the_library_lacks_is_listed_by_name(self):
        missing = [k for (kind, k), r in self.parsed.items() if kind == "query" and r["status"] == "NO_SYMBOL"]
        self.assertIn("clock.app.graphics", missing)
        self.assertGreater(len(missing), 30)

    def test_the_sweep_compared_with_itself_has_no_findings(self):
        _, counts = N.compare_sweeps(self.parsed, self.parsed, CLASSES, EXPECTED)
        self.assertEqual(N.findings(counts), 0)


# -------------------------------------------------------- the runner end to end with a fake ffmpeg and a fake probe (no GPU)
FAKE_FFMPEG = r'''#!/usr/bin/env python3
import os, sys
a = sys.argv[1:]
mode = os.environ.get("FAKE_MODE", "all_ok")
def say(t): sys.stderr.write(t + "\n")
if "-encoders" in a:
    print(" V....D h264_nvenc           NVIDIA NVENC H.264 encoder\n V....D hevc_nvenc           NVIDIA NVENC hevc encoder\n V....D av1_nvenc            NVIDIA NVENC av1 encoder\n V....D libx264 x\n V....D libx265 x\n V....D libvpx-vp9 x\n V....D libsvtav1 x")
    sys.exit(0)
if "-decoders" in a:
    print(" V....D h264_cuvid x\n V....D hevc_cuvid x\n V....D vp9_cuvid x\n V....D av1_cuvid x")
    sys.exit(0)
if "-hwaccels" in a:
    print("Hardware acceleration methods:\ncuda\nvaapi")
    sys.exit(0)
if "-filter_complex" in a and any("psnr" in x for x in a):
    if mode == "psnr_missing":
        sys.exit(0)
    decode = "-hwaccel" in a or any(x.endswith("_cuvid") for x in a)
    v = {"psnr_low": "20.0", "psnr_edge": "24.00", "psnr_just_under": "23.99", "psnr_inf": "inf"}.get(mode, "inf" if decode else "41.0")
    if mode == "psnr_two":
        say("[Parsed_psnr_2 @ 0x1] PSNR y:1 u:1 v:1 average:20.0 min:1 max:1")
    say("[Parsed_psnr_2 @ 0x1] PSNR y:%s u:1 v:1 average:%s min:1 max:1" % (v, v))
    sys.exit(0)
if "-f" in a and a[-3:] == ["-f", "null", "-"] and os.environ.get("FAKE_SESS_OK") is not None:
    d = os.environ["FAKE_SESS_DIR"]
    nth = 0
    while True:  # claim the next free slot atomically: the session processes start together
        nth += 1
        try:
            os.close(os.open(os.path.join(d, str(nth)), os.O_CREAT | os.O_EXCL))
            break
        except FileExistsError:
            continue
    if nth > int(os.environ["FAKE_SESS_OK"]):
        say("[h264_nvenc @ 0x1] OpenEncodeSessionEx failed: out of memory (10)")
        sys.exit(1)
    sys.exit(0)
enc = next((x for x in a if x.endswith("_nvenc")), "")
if mode == "leaky":
    say("[x @ 0x1] device GPU-fa4efa4e-fa4e-4fa4-8fa4-fa4efa4e0001 at 10.20.30.40 failed")
    sys.exit(1)
if mode == "no_device" and enc:
    say("[%s @ 0x1] OpenEncodeSessionEx failed: unsupported device (2): (no details)" % enc)
    sys.exit(1)
if mode == "no_av1" and enc == "av1_nvenc":
    say("[av1_nvenc @ 0x1] InitializeEncoder failed: invalid param (8)")
    sys.exit(1)
if mode == "hevc_fails" and enc == "hevc_nvenc":
    say("[hevc_nvenc @ 0x1] InitializeEncoder failed: invalid param (8)")
    sys.exit(1)
out = a[-1]
if out not in ("-",):
    open(out, "wb").write(b"x")
sys.exit(0)
'''
FAKE_FFPROBE = r'''#!/usr/bin/env python3
import json, os, sys
a = sys.argv[1:]
codec = os.environ.get("FAKE_CODEC")
# the runner passes the case through the file name: <case id>.mkv ; frames and size are read from the table the same way the test does
name = os.path.basename(a[-1])
import re
sys.path.insert(0, os.environ["OR"])
import nvcodec_lib as L
cases = {c["id"]: c for c in L.load_cases(open(os.path.join(os.environ["OR"], "nvcodec_cases.tsv")).read())}
c = cases[name[:-4]]
print(json.dumps({"streams": [{"codec_name": c["codec"], "pix_fmt": c["pix_fmt"], "width": c["width"], "height": c["height"], "nb_read_frames": str(c["frames"])}]}))
'''


class RunnerEndToEnd(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory()
        t = Path(self.tmp.name)
        for name, body in (("ffmpeg", FAKE_FFMPEG), ("ffprobe", FAKE_FFPROBE)):
            (t / name).write_text(body)
            (t / name).chmod(0o755)
        self.probe = t / "probe.sh"

    def tearDown(self):
        self.tmp.cleanup()

    def run_row(self, probe, mode="all_ok", only="enc_*,sess_h264_n01,dec_hevc_420_8_hw,dec_hevc_420_10_hw", extra_env=None):
        t = Path(self.tmp.name)
        (t / "probe.txt").write_text(probe)
        self.probe.write_text("#!/bin/sh\ncat %s\n" % (t / "probe.txt"))
        self.probe.chmod(0o755)
        env = dict(os.environ, FAKE_MODE=mode, OR=str(OR), **(extra_env or {}))
        p = subprocess.run([sys.executable, str(OR / "nvcodec_matrix.py"), "run", "--out", str(t / "out"), "--ffmpeg", str(t / "ffmpeg"),
                            "--ffprobe", str(t / "ffprobe"), "--probe", str(self.probe), "--only", only, "--emit"], capture_output=True, text=True, env=env, timeout=240)
        rows = {r["case_id"]: r for r in L.parse_results((t / "out/results.tsv").read_text())}
        return p, rows

    def test_a_die_with_av1_passes_and_one_without_is_not_supported(self):
        p, rows = self.run_row(probe_text(av1=True))
        self.assertEqual(rows["enc_av1_420_8_1080p"]["result"], "PASS", p.stdout[-600:])
        p, rows = self.run_row(probe_text(av1=False), mode="no_av1")
        self.assertEqual(rows["enc_av1_420_8_1080p"]["result"], "NOT_SUPPORTED")
        self.assertEqual(rows["enc_hevc_420_8_1080p"]["result"], "PASS")

    def test_a_supported_case_that_fails_is_a_fail_and_the_exit_status_says_so(self):
        p, rows = self.run_row(probe_text(av1=True), mode="hevc_fails")
        self.assertEqual(rows["enc_hevc_420_8_1080p"]["result"], "FAIL")
        self.assertEqual(rows["enc_hevc_420_8_1080p"]["reason"], "encoder_params_rejected")
        self.assertEqual(p.returncode, 1)
        self.assertRegex(p.stdout, r"NVCODEC_MATRIX_SUMMARY pass=\d+ not_supported=\d+ fail=[1-9]")
        pattern = next(l for l in (ROOT / "scripts/apps/run_apps.sh").read_text().splitlines() if l.startswith("nvcodec_matrix|")).split("|")[2]
        self.assertNotRegex([l for l in p.stdout.splitlines() if l.startswith("NVCODEC_MATRIX_SUMMARY")][-1], pattern)  # run_apps.sh must grade it FAIL

    def test_psnr_below_the_threshold_fails_the_case(self):
        p, rows = self.run_row(probe_text(av1=True), mode="psnr_low")
        r = rows["enc_h264_420_8_1080p"]
        self.assertEqual((r["result"], r["reason"]), ("FAIL", "psnr_below_threshold"))

    def test_psnr_exactly_at_the_threshold_passes_and_just_under_fails(self):
        p, rows = self.run_row(probe_text(av1=True), mode="psnr_edge", only="enc_h264_420_8_1080p")
        self.assertEqual(rows["enc_h264_420_8_1080p"]["result"], "PASS")
        p, rows = self.run_row(probe_text(av1=True), mode="psnr_just_under", only="enc_h264_420_8_1080p")
        self.assertEqual(rows["enc_h264_420_8_1080p"]["result"], "FAIL")

    def test_the_last_psnr_summary_wins_end_to_end(self):
        p, rows = self.run_row(probe_text(av1=True), mode="psnr_two", only="enc_h264_420_8_1080p")
        self.assertEqual(rows["enc_h264_420_8_1080p"]["result"], "PASS")  # the first summary line says 20 dB, the last says 41

    def test_identical_frames_pass_end_to_end(self):
        p, rows = self.run_row(probe_text(av1=True), mode="psnr_inf", only="enc_h264_420_8_1080p")
        self.assertEqual(rows["enc_h264_420_8_1080p"]["result"], "PASS")

    def test_a_missing_psnr_summary_fails_the_case(self):
        p, rows = self.run_row(probe_text(av1=True), mode="psnr_missing", only="enc_h264_420_8_1080p")
        r = rows["enc_h264_420_8_1080p"]
        self.assertEqual((r["result"], r["reason"]), ("FAIL", "output_not_decodable"))

    def test_an_unreachable_encoder_is_not_run_with_its_reason(self):
        p, rows = self.run_row(probe_text(av1=True), mode="no_device", only="enc_h264_420_8_1080p")
        r = rows["enc_h264_420_8_1080p"]
        self.assertEqual((r["result"], r["reason"]), ("NOT_RUN", "no_encode_device"))

    def test_a_size_the_probe_forbids_that_encodes_anyway_is_a_mismatch(self):
        p, rows = self.run_row(probe_text(av1=True, wmax=4096, hmax=4096), only="enc_hevc_8k")
        self.assertEqual(rows["enc_hevc_8k"]["result"], "MISMATCH")

    def test_a_decode_size_the_probe_forbids_that_decodes_anyway_is_a_mismatch(self):
        p, rows = self.run_row(probe_text(av1=True), only="dec_h264_above4096_hw")
        self.assertEqual(rows["dec_h264_above4096_hw"]["result"], "MISMATCH")

    def test_a_hardware_decode_that_differs_from_software_fails_the_case(self):
        p, rows = self.run_row(probe_text(av1=True), mode="psnr_low", only="dec_hevc_420_8_hw")
        r = rows["dec_hevc_420_8_hw"]
        self.assertEqual((r["result"], r["reason"]), ("FAIL", "psnr_below_threshold"))

    def test_a_supported_decode_passes_in_both_modes(self):
        p, rows = self.run_row(probe_text(av1=True), only="dec_hevc_420_8_hw,dec_hevc_420_8_cuvid")
        self.assertEqual({k: r["result"] for k, r in rows.items()}, {"dec_hevc_420_8_hw": "PASS", "dec_hevc_420_8_cuvid": "PASS"})

    def test_decoder_rows_that_are_errors_make_decode_cases_not_run(self):
        dead = probe_text(av1=True, dec={}).replace("supported=0 max=0x0 min=0x0 mbmax=0 nvdecs=0 fmtmask=0x0", "ERR:cuda100")
        p, rows = self.run_row(dead, only="dec_hevc_420_8_hw")
        r = rows["dec_hevc_420_8_hw"]
        self.assertEqual((r["result"], r["reason"]), ("NOT_RUN", "nvdec_unavailable"))

    def test_logs_never_keep_a_uuid_or_an_address(self):
        p, rows = self.run_row(probe_text(av1=True), mode="leaky", only="enc_h264_420_8_1080p")
        log = (Path(self.tmp.name) / "out/logs/enc_h264_420_8_1080p.log").read_text()
        self.assertIn("failed", log)
        self.assertNotIn("fa4efa4e", log)
        self.assertNotIn("10.20.30.40", log)

    def test_two_runs_that_fail_alike_blame_the_harness_not_kayfabe(self):
        t = Path(self.tmp.name)
        ph, _ = self.run_row(probe_text(av1=True), mode="hevc_fails")
        (t / "host.log").write_text(ph.stdout)
        shutil.rmtree(t / "out")
        pg, _ = self.run_row(probe_text(av1=True), mode="hevc_fails")
        (t / "guest.log").write_text(pg.stdout)
        c = subprocess.run([sys.executable, str(OR / "compare_logs.py"), "nvcodec_matrix", str(t / "host.log"), str(t / "guest.log")], capture_output=True, text=True)
        self.assertEqual(c.returncode, 0)
        self.assertRegex(c.stdout, r"CMP enc_hevc_420_8_1080p host=FAIL guest=FAIL AGREE bucket=harness_or_app_bug_confirmed_by_host_control")

    def test_a_guest_that_says_not_supported_where_the_host_passes_is_a_candidate(self):
        t = Path(self.tmp.name)
        ph, _ = self.run_row(probe_text(av1=True), only="enc_av1_420_8_1080p")
        (t / "host.log").write_text(ph.stdout)
        shutil.rmtree(t / "out")
        pg, _ = self.run_row(probe_text(av1=False), mode="no_av1", only="enc_av1_420_8_1080p")
        (t / "guest.log").write_text(pg.stdout)
        c = subprocess.run([sys.executable, str(OR / "compare_logs.py"), "nvcodec_matrix", str(t / "host.log"), str(t / "guest.log")], capture_output=True, text=True)
        self.assertEqual(c.returncode, 1)
        self.assertRegex(c.stdout, r"host=PASS guest=NOT_SUPPORTED DISAGREE bucket=kayfabe_candidate")
        self.assertIn("CAPDIFF enc_codec.av1 host=1 guest=0", c.stdout)

    def sessions(self, n, ok, cap=3):
        d = Path(self.tmp.name) / ("sess%d_%d" % (n, ok))
        d.mkdir(exist_ok=True)
        p, rows = self.run_row(probe_text(sessions=cap), only="sess_h264_n%02d" % n, extra_env={"FAKE_SESS_OK": str(ok), "FAKE_SESS_DIR": str(d)})
        return rows["sess_h264_n%02d" % n]

    def test_sessions_up_to_the_cap_pass(self):
        self.assertEqual(self.sessions(3, 3)["result"], "PASS")

    def test_sessions_above_the_cap_are_not_supported_when_exactly_the_cap_worked(self):
        r = self.sessions(5, 3)
        self.assertEqual((r["result"], r["reason"]), ("NOT_SUPPORTED", "session_limit"))
        self.assertEqual(r["n_ok"], "3/5")

    def test_sessions_above_the_cap_with_another_count_are_a_mismatch(self):
        r = self.sessions(5, 2)
        self.assertEqual((r["result"], r["reason"]), ("MISMATCH", "partial_sessions"))
        self.assertIn("n_ok=2 but cap 3", r["detail"])  # the cap is the one the probe discovered

    def test_a_session_failing_under_the_cap_is_a_fail(self):
        self.assertEqual(self.sessions(3, 2)["result"], "FAIL")

    def test_an_unexpected_success_is_a_mismatch(self):
        p, rows = self.run_row(probe_text(av1=False))  # the fake encodes AV1 fine although the probe says the die cannot
        self.assertEqual(rows["enc_av1_420_8_1080p"]["result"], "MISMATCH")

    def test_a_probe_that_cannot_ask_makes_everything_not_run(self):
        p, rows = self.run_row(probe_text(nvenc_error=True), only="enc_*,sess_h264_n01")
        self.assertTrue(all(r["result"] == "NOT_RUN" and r["reason"] == "nvenc_unavailable" for r in rows.values()), p.stdout[-400:])
        self.assertEqual(p.returncode, 1)  # nothing passed

    def test_the_summary_line_matches_the_row_pattern_of_run_apps(self):
        p, rows = self.run_row(probe_text(av1=True))
        line = [l for l in p.stdout.splitlines() if l.startswith("NVCODEC_MATRIX_SUMMARY")][-1]
        pattern = next(l for l in (ROOT / "scripts/apps/run_apps.sh").read_text().splitlines() if l.startswith("nvcodec_matrix|")).split("|")[2]
        if "fail=0" in line and "mismatch=0" in line:
            self.assertRegex(line, pattern)
        self.assertIn("gpu_use=", line)
        files = rowio.extract(p.stdout)
        self.assertEqual(set(files), {"probe.tsv", "results.tsv", "summary.txt"})

    def test_compare_of_a_run_with_itself_through_its_log(self):
        p, rows = self.run_row(probe_text(av1=True))
        log = Path(self.tmp.name) / "row.log"
        log.write_text(p.stdout)
        c = subprocess.run([sys.executable, str(OR / "compare_logs.py"), "nvcodec_matrix", str(log), str(log)], capture_output=True, text=True)
        self.assertEqual(c.returncode, 0, c.stdout)
        self.assertIn("candidates=0 capdiffs=0", c.stdout)

    def test_a_guest_that_fails_where_the_host_passes_is_a_candidate(self):
        t = Path(self.tmp.name)
        ph, _ = self.run_row(probe_text(av1=True))
        (t / "host.log").write_text(ph.stdout)
        shutil.rmtree(t / "out")
        pg, _ = self.run_row(probe_text(av1=True), mode="hevc_fails")
        (t / "guest.log").write_text(pg.stdout)
        c = subprocess.run([sys.executable, str(OR / "compare_logs.py"), "nvcodec_matrix", str(t / "host.log"), str(t / "guest.log")], capture_output=True, text=True)
        self.assertEqual(c.returncode, 1)
        self.assertRegex(c.stdout, r"CMP enc_hevc_420_8_1080p host=PASS guest=FAIL DISAGREE bucket=kayfabe_candidate_bug_undocumented")


FAKE_DCGMI = r"""#!/usr/bin/env python3
import os, sys
a = sys.argv[1:]
mode = os.environ.get("FAKE_DCGM", "ok")
if a[:2] == ["discovery", "-l"]:
    print("1 GPU found.")
elif a == ["-v"]:
    print("dcgmi  version: 4.8.0")
elif a[:2] == ["dmon", "-l"]:
    print("Long Name                                                             Field ID")
    print("gpu_temp                                                              150")
    print("gpu_utilization                                                       203")
    if mode == "refused":
        print("prof_sm_active                                                        1002")
elif a[:1] == ["dmon"]:
    ids = a[a.index("-e") + 1].split(",")
    if "1002" in ids:
        print("Error setting watches. Result: -36: Profiling is not supported for this group of GPUs or GPU")
        sys.exit(1)
    vals = {"150": "41", "203": "N/A" if mode == "na" else "12"}
    print("#Entity         " + "  ".join(ids))
    print("GPU 0           " + "  ".join(vals[i] for i in ids))
elif a[:1] == ["diag"]:
    st = "Fail" if mode == "diagfail" else "Skip" if mode == "diagskip" else "Pass"
    print('{"DCGM Diagnostic": {"test_categories": [{"category": "Hardware", "tests": [{"name": "memory", "results": [{"gpu_id": "0", "status": "%s"}]}]}]}}' % st)
"""
FAKE_HOSTENGINE = "#!/usr/bin/env python3\nimport time\ntime.sleep(300)\n"


@unittest.skipUnless(shutil.which("gcc"), "needs gcc to build a fake libnvidia-ml")
class DcgmRowEndToEnd(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        cls.tmp = tempfile.TemporaryDirectory()
        t = Path(cls.tmp.name)
        (t / "fake.c").write_text(FAKE_NVML_C)
        cls.lib = str(t / "libfakenvml.so")
        subprocess.run(["gcc", "-shared", "-fPIC", "-o", cls.lib, str(t / "fake.c")], check=True)
        cls.bundle = t / "bundle"
        (cls.bundle / "dcgm/usr/bin").mkdir(parents=True)
        for name, body in (("dcgmi", FAKE_DCGMI), ("nv-hostengine", FAKE_HOSTENGINE)):
            (cls.bundle / "dcgm/usr/bin" / name).write_text(body)
            (cls.bundle / "dcgm/usr/bin" / name).chmod(0o755)

    @classmethod
    def tearDownClass(cls):
        cls.tmp.cleanup()

    def run_row(self, out, bundle=None, mode="ok", extra=()):
        env = dict(os.environ, FAKE_DCGM=mode, PATH="/usr/bin:/bin")
        p = subprocess.run([sys.executable, str(OR / "dcgm_nvml.py"), "run", "--out", str(out), "--nvml-lib", self.lib, "--diag-level", "1",
                            "--diag-timeout", "30", "--emit"] + (["--bundle", str(bundle)] if bundle else []) + list(extra), capture_output=True, text=True, env=env, timeout=120)
        return p

    def test_a_run_with_dcgm_records_everything_and_matches_the_run_apps_pattern(self):
        out = Path(self.tmp.name) / "o1"
        p = self.run_row(out, self.bundle)
        self.assertEqual(p.returncode, 0, p.stdout[-500:] + p.stderr[-500:])
        line = [l for l in p.stdout.splitlines() if l.startswith("NVML_DCGM_SUMMARY")][-1]
        pattern = next(l for l in (ROOT / "scripts/apps/run_apps.sh").read_text().splitlines() if l.startswith("nvml_dcgm|")).split("|")[2]
        self.assertRegex(line, pattern)
        self.assertIn("dcgm=RAN", line)
        self.assertIn("diag_pass=1", line)
        self.assertIn("dcgm_fields_ok=2/2", line)
        self.assertIn("dcgm_fields_ok=2/2", line)
        self.assertEqual(set(rowio.extract(p.stdout)), {"nvml_sweep.tsv", "dcgm.tsv", "dcgm_status.txt", "summary.txt"})

    def test_a_refused_field_gets_its_own_not_supported_row(self):
        out = Path(self.tmp.name) / "o9"
        self.run_row(out, self.bundle, mode="refused")
        rows = N.parse_rows((out / "dcgm.tsv").read_text())
        self.assertEqual(rows[("dcgm", "prof_sm_active")]["status"], "NOT_SUPPORTED")
        self.assertEqual([k[1] for k, r in rows.items() if k[0] == "dcgm" and r["status"] == "NOT_SUPPORTED"], ["prof_sm_active"])  # and only that one

    def test_a_refused_field_does_not_hide_the_fields_beside_it(self):
        out = Path(self.tmp.name) / "o10"
        self.run_row(out, self.bundle, mode="refused")
        rows = N.parse_rows((out / "dcgm.tsv").read_text())
        self.assertEqual((rows[("dcgm", "gpu_temp")]["status"], rows[("dcgm", "gpu_temp")]["value"]), ("OK", "41"))
        self.assertEqual(rows[("dcgm", "gpu_utilization")]["status"], "OK")

    def test_a_diagnostic_that_skips_everything_claims_no_gpu_work(self):
        out = Path(self.tmp.name) / "o11"
        p = self.run_row(out, self.bundle, mode="diagskip")
        self.assertIn("gpu_use=NA", [l for l in p.stdout.splitlines() if l.startswith("NVML_DCGM_SUMMARY")][-1])
        self.assertIn("no diagnostic that works the GPU ran", (out / "summary.txt").read_text())

    def test_a_machine_without_dcgm_says_so_by_name(self):
        out = Path(self.tmp.name) / "o2"
        p = self.run_row(out)
        self.assertEqual(p.returncode, 0)
        self.assertIn("dcgm=NOT_RUN reason=dcgmi_not_installed", (out / "dcgm_status.txt").read_text())
        self.assertIn("dcgm=NOT_RUN", p.stdout)

    def test_the_flag_disables_dcgm_by_name(self):
        out = Path(self.tmp.name) / "o3"
        self.run_row(out, self.bundle, extra=["--no-dcgm"])
        self.assertIn("reason=disabled_by_flag", (out / "dcgm_status.txt").read_text())

    def test_the_nvml_part_runs_without_dcgm(self):
        out = Path(self.tmp.name) / "o4"
        self.run_row(out)
        rows = N.parse_rows((out / "nvml_sweep.tsv").read_text())
        self.assertEqual(rows[("query", "utilization.rates")]["value"], "gpu=42;memory=7")

    def test_compare_of_the_run_with_itself_has_no_finding(self):
        out = Path(self.tmp.name) / "o5"
        self.run_row(out, self.bundle)
        c = subprocess.run([sys.executable, str(OR / "compare_logs.py"), "nvml_dcgm", str(out), str(out)], capture_output=True, text=True)
        self.assertEqual(c.returncode, 0, c.stdout[-500:])
        self.assertIn("NVML_COMPARE findings=0 dcgm_findings=0", c.stdout)

    def test_na_in_the_guest_where_the_host_has_a_number_fails_the_comparison(self):
        host, guest = Path(self.tmp.name) / "o6h", Path(self.tmp.name) / "o6g"
        self.run_row(host, self.bundle)
        self.run_row(guest, self.bundle, mode="na")
        c = subprocess.run([sys.executable, str(OR / "compare_logs.py"), "nvml_dcgm", str(host), str(guest)], capture_output=True, text=True)
        self.assertEqual(c.returncode, 1)
        self.assertRegex(c.stdout, r"DCGM_ROW FINDING_NA_IN_GUEST gpu_utilization")

    def test_a_diag_test_that_passes_on_the_host_and_fails_in_the_guest_fails_the_comparison(self):
        host, guest = Path(self.tmp.name) / "o7h", Path(self.tmp.name) / "o7g"
        self.run_row(host, self.bundle)
        self.run_row(guest, self.bundle, mode="diagfail")
        c = subprocess.run([sys.executable, str(OR / "compare_logs.py"), "nvml_dcgm", str(host), str(guest)], capture_output=True, text=True)
        self.assertEqual(c.returncode, 1)
        self.assertIn("FINDING_DIAG_FAIL_IN_GUEST", c.stdout)

    def test_the_host_engine_is_stopped_at_the_end(self):
        out = Path(self.tmp.name) / "o8"
        self.run_row(out, self.bundle)
        ps = subprocess.run(["pgrep", "-f", str(self.bundle / "dcgm/usr/bin/nv-hostengine")], capture_output=True, text=True)
        self.assertEqual(ps.stdout.strip(), "")


FIX = Path(__file__).resolve().parent / "fixtures/open_rows"


class RealOutputs(unittest.TestCase):
    """Outputs of the native runs on two Vast boxes (traces/open_rows_ffdcgm_20261117), read by the parsers as they came from the tools."""

    def test_the_probe_of_a_geforce_rtx_3060_vm(self):
        p = L.parse_probe((FIX / "probe_geforce_rtx3060_ga104_575.tsv").read_text())
        self.assertEqual((p.meta["device_name"], p.meta["driver_api_max"]), ("NVIDIA GeForce RTX 3060", "13.0"))
        self.assertEqual(p.enc_codec, {"h264": True, "hevc": True, "av1": False})  # no AV1 encode on Ampere
        self.assertEqual((p.enc_caps["h264"]["WIDTH_MAX"], p.enc_caps["hevc"]["WIDTH_MAX"], p.enc_caps["hevc"]["SUPPORT_YUV444_ENCODE"]), (4096, 8192, 1))
        self.assertEqual(p.sessions, {"opened": "8", "limit_hit": "1", "stopped_by": "NV_ENC_ERR_INCOMPATIBLE_CLIENT_KEY"})
        self.assertTrue(p.dec[("av1", "420", 10)]["supported"] and not p.dec[("h264", "420", 10)]["supported"])
        self.assertEqual(p.dec[("hevc", "420", 8)]["maxw"], 8192)
        self.assertTrue(p.enc_available() and p.dec_available())

    def test_the_table_against_that_probe_reproduces_the_logged_expectations(self):
        p = L.parse_probe((FIX / "probe_geforce_rtx3060_ga104_575.tsv").read_text())
        logged = {r["case_id"]: r for r in L.parse_results((FIX / "matrix_results_geforce_rtx3060_ga104.tsv").read_text())}
        cases = L.load_cases(CASES_TEXT)
        self.assertEqual({c["id"] for c in cases}, set(logged))
        for c in cases:
            self.assertEqual(L.evaluate_needs(c, p)[0], logged[c["id"]]["expect"], c["id"])

    def test_the_logged_results_are_green_and_count_as_logged(self):
        rows = L.parse_results((FIX / "matrix_results_geforce_rtx3060_ga104.tsv").read_text())
        counts, ok = L.summarize(rows)
        self.assertEqual((counts["PASS"], counts["NOT_SUPPORTED"], counts["FAIL"], counts["MISMATCH"], counts["NOT_RUN"]), (35, 7, 0, 0, 5))
        self.assertTrue(ok)
        by = {r["case_id"]: r for r in rows}
        self.assertEqual(by["sess_h264_n09"]["reason"], "session_limit")
        self.assertEqual(by["enc_av1_420_8_1080p"]["reason"], "unknown_encoder")  # ffmpeg 4.4.2 of Ubuntu 22.04 has no av1_nvenc

    def test_a_logged_run_compared_with_itself_has_no_candidate(self):
        rows = L.parse_results((FIX / "matrix_results_geforce_rtx3060_ga104.tsv").read_text())
        self.assertTrue(all(r["agreement"] == "AGREE" for r in L.compare_results(rows, rows)))

    def test_a_vast_container_without_video_is_not_run_by_name(self):
        rows = L.parse_results((FIX / "matrix_in_a_vast_container_without_video.txt").read_text())
        self.assertTrue(rows and all(r["result"] == "NOT_RUN" for r in rows))
        self.assertEqual({r["reason"] for r in rows}, {"nvenc_unavailable", "nvdec_unavailable"})

    def test_the_real_diag_json_parses_into_one_pass_and_seven_skips(self):
        tests = N.parse_diag_json((FIX / "dcgm_diag_r3_geforce_rtx3060.json").read_text())
        self.assertEqual([t[3] for t in tests], ["Pass"] + ["Skip"] * 7)
        self.assertEqual(tests[0][:3], ("Deployment", "software", "0"))
        self.assertFalse(N.gpu_work_ran(tests))  # every diagnostic that works the GPU skipped on a GeForce

    def test_the_real_nvvs_log_says_why_the_tests_were_skipped(self):
        info = N.parse_nvvs_test_info((FIX / "nvvs_test_lines_geforce_rtx3060.txt").read_text())
        self.assertIn("targeted_stress", info)
        self.assertTrue(info["memory"].startswith("The Memory test is skipped. Check DCGM and system configuration."))

    def test_the_real_dmon_field_list(self):
        fields = N.parse_dcgm_dmon_list((FIX / "dcgmi_dmon_list_head.txt").read_text())
        self.assertEqual(fields[:3], [(1, "driver_version"), (2, "nvml_version"), (3, "process_name")])
        self.assertIn((57, "pci_busid"), fields)

    def test_the_real_sweep_rows_are_well_formed_and_equal_to_themselves(self):
        rows = N.parse_rows((FIX / "nvml_sweep_fields_and_queries_geforce_rtx3060.tsv").read_text())
        self.assertEqual(sum(1 for k, r in rows.items() if k[0] == "field" and r["status"] == "OK"), 56)
        self.assertEqual(rows[("query", "utilization.rates")]["status"], "OK")
        self.assertEqual(rows[("query", "ecc.mode")]["status"], "NOT_SUPPORTED")  # a GeForce has no ECC: the host answers no number, so none is owed
        res, counts = N.compare_sweeps(rows, rows, CLASSES, EXPECTED)
        self.assertEqual(N.findings(counts), 0)
        for r in rows.values():
            self.assertNotRegex(r["value"], r"[0-9a-f]{8}-[0-9a-f]{4}-")

    def test_a_guest_that_loses_the_real_utilization_rows_is_caught(self):
        rows = N.parse_rows((FIX / "nvml_sweep_fields_and_queries_geforce_rtx3060.tsv").read_text())
        guest = dict(rows)
        for key in ("utilization.rates", "utilization.encoder", "utilization.decoder"):
            guest[("query", key)] = dict(rows[("query", key)], status="NOT_SUPPORTED", value="-")
        res, counts = N.compare_sweeps(rows, guest, CLASSES, EXPECTED)
        self.assertEqual(counts.get("FINDING_NA_IN_GUEST"), 3)  # the known gap of PR #459: guest utilization.* was [N/A]


class StaticContracts(unittest.TestCase):
    """The trust rules of the owner (2026-11-17) and the wiring, pinned as text. These are text pins: they prove the line is there, not that it works."""

    def test_no_sha256_of_a_binary_is_pinned_in_the_open_rows(self):
        for p in OR.iterdir():
            if p.suffix in (".env", ".sh", ".tsv", ".c"):
                text = p.read_text()
                self.assertNotRegex(text, r"\b[0-9a-f]{64}\b", p.name)
        pins = (OR / "pins.env").read_text()
        self.assertRegex(pins, r"OPENROWS_NVCH_COMMIT=[0-9a-f]{40}\n")
        self.assertIn("OPENROWS_DCGM_VERSION=", pins)
        self.assertIn("https://developer.download.nvidia.com/compute/cuda/repos", pins)

    def test_pins_env_names_a_commit_and_a_version_never_a_hash(self):
        keys = {l.split("=", 1)[0] for l in (OR / "pins.env").read_text().splitlines() if "=" in l and not l.startswith("#")}
        self.assertEqual(keys, {"OPENROWS_NVCH_URL", "OPENROWS_NVCH_TAG", "OPENROWS_NVCH_COMMIT", "OPENROWS_NVIDIA_REPO_BASE", "OPENROWS_NVIDIA_KEY_FILE",
                                "OPENROWS_NVIDIA_KEY_FPR", "OPENROWS_DCGM_VERSION", "OPENROWS_DCGM_PACKAGE_STEMS"})

    def test_provision_waits_for_a_busy_package_manager_before_every_apt_call(self):
        text = (OR / "provision.sh").read_text()
        self.assertIn("lock-frontend", text)  # a fresh Ubuntu machine holds the dpkg lock for a while (it cost a native run its DCGM step); waiting is by the lock file, never by process name
        self.assertEqual(len(re.findall(r"wait_apt; apt-get ", text)), 2)  # the DCGM update and the DCGM install
        self.assertEqual(len(re.findall(r"\bwait_apt\b", text)), 4)  # its definition, the helper's install and those two

    def test_the_dcgm_install_itself_waits_for_the_lock(self):
        self.assertRegex((OR / "provision.sh").read_text(), r"wait_apt; apt-get [^\n]* install -y")

    def test_the_helper_install_waits_for_the_lock_too(self):
        text = (OR / "provision.sh").read_text()
        self.assertRegex(text, r"apt_install\(\) \{\n  wait_apt\n")

    def test_the_nvidia_repository_key_is_checked_against_its_pinned_fingerprint(self):
        """2026-11-19 (standalone trust, #483): the apt key was trusted on first use. Its full fingerprint is pinned, its low 64 bits are the
        key id NVIDIA itself publishes (A4B469963BF863CC, developer.nvidia.com/blog/updating-the-cuda-linux-gpg-repository-key), and the key is
        refused before gpg --dearmor when the fetched file is another key."""
        pins = dict(l.split("=", 1) for l in (OR / "pins.env").read_text().splitlines() if "=" in l and not l.startswith("#"))
        self.assertRegex(pins["OPENROWS_NVIDIA_KEY_FPR"], r"^[0-9A-F]{40}$")
        self.assertTrue(pins["OPENROWS_NVIDIA_KEY_FPR"].endswith("A4B469963BF863CC"))
        text = (OR / "provision.sh").read_text()
        check = text.index('key_fpr_ok "$ROOT/nvidia.key" "$OPENROWS_NVIDIA_KEY_FPR"')
        self.assertLess(text.index('-o "$ROOT/nvidia.key"'), check)
        self.assertLess(check, text.index("gpg --dearmor"))

    @unittest.skipUnless(shutil.which("gpg"), "gpg is needed to make a test key")
    def test_key_fpr_ok_accepts_only_the_pinned_key(self):
        text = (OR / "provision.sh").read_text()
        fn = re.search(r"^key_fpr_ok\(\) \{.*?\n\}\n", text, flags=re.S | re.M)
        self.assertIsNotNone(fn)
        with tempfile.TemporaryDirectory() as d:
            env = dict(os.environ, GNUPGHOME=d)
            subprocess.run(["gpg", "--batch", "--passphrase", "", "--quick-gen-key", "kayfabe test <t@example.invalid>", "ed25519", "sign", "1d"],
                           env=env, check=True, capture_output=True)
            out = subprocess.run(["gpg", "--batch", "--with-colons", "--list-keys"], env=env, check=True, capture_output=True, text=True).stdout
            fpr = next(l.split(":")[9] for l in out.splitlines() if l.startswith("fpr:"))
            key = Path(d) / "k.pub"
            key.write_bytes(subprocess.run(["gpg", "--batch", "--export", "--armor"], env=env, check=True, capture_output=True).stdout)
            junk = Path(d) / "junk.pub"
            junk.write_text("not a key\n")
            def ok(path, want):
                return subprocess.run(["bash", "-c", fn.group(0) + 'key_fpr_ok "$1" "$2"', "x", str(path), want], env=env).returncode == 0
            self.assertTrue(ok(key, fpr))
            self.assertFalse(ok(key, "0" * 40))
            self.assertFalse(ok(junk, fpr))

    def test_provision_never_trusts_an_unsigned_repository(self):
        text = (OR / "provision.sh").read_text()
        for bad in ("trusted=yes", "allow-insecure", "--allow-unauthenticated", "AllowUnauthenticated"):
            self.assertNotIn(bad, text)

    def test_the_probe_headers_come_only_through_the_pin_helper(self):
        text = (OR / "provision.sh").read_text()
        self.assertNotIn("git clone", text)
        self.assertNotIn("wget", text)

    def test_the_nv_codec_headers_are_pinned_by_a_full_upstream_commit(self):
        pins = dict(l.split("=", 1) for l in (OR / "pins.env").read_text().splitlines() if "=" in l and not l.startswith("#"))
        self.assertRegex(pins["OPENROWS_NVCH_COMMIT"], r"^[0-9a-f]{40}$")
        self.assertEqual(pins["OPENROWS_NVCH_URL"], "https://github.com/FFmpeg/nv-codec-headers.git")
        self.assertRegex(pins["OPENROWS_NVCH_TAG"], r"^n\d+\.\d+\.\d+\.\d+$")

    def test_provision_names_the_apt_shim_instead_of_bypassing_it(self):
        text = (OR / "provision.sh").read_text()
        self.assertIsNone(re.search(r"command\s+apt-get", text))
        self.assertIn("apt_shim_cache_only", text)
        self.assertIn('type -t apt-get', text)

    def test_no_apt_script_but_pin_lib_bypasses_the_shim(self):
        found = {}
        for path in sorted((ROOT / "scripts/apps").rglob("*.sh")):
            n = len(re.findall(r"command\s+apt-get", path.read_text()))
            if n:
                found[path.name] = n
        self.assertEqual(found, {"pin_lib.sh": 2})

    def test_gate_lists_do_not_name_discovery_rows(self):
        for name in ("gate_list.txt", "gate_list_middle.txt"):
            lines = [l.strip() for l in (ROOT / "scripts/apps" / name).read_text().splitlines() if l.strip() and not l.startswith("#")]
            self.assertFalse({"nvcodec_matrix", "nvml_dcgm"} & set(lines), name)

    def test_the_hook_takes_the_before_count_before_the_row_runs(self):
        hook = (ROOT / "scripts/apps/apps_hook.sh").read_text()
        self.assertLess(hook.index('q0=$(wc -l < "$QLOG"'), hook.index("census.py\" sample"))
        self.assertRegex(hook, r'census\.py" kf3 --log "\$QLOG" --before "\$q0"')

    def test_a_failed_open_rows_step_only_prints_a_line_in_the_bundle_build(self):
        b = (ROOT / "scripts/apps/build_bundle.sh").read_text()
        self.assertRegex(b, r'provision\.sh" "\$B" \|\| say "open rows')
        self.assertLess(b.index("open_rows/provision.sh"), b.index("bundle_fail()"))

    def test_the_apt_shim_is_not_bypassed_here(self):
        for p in OR.iterdir():
            if p.is_file():
                self.assertIsNone(re.search(r"command\s+apt-get", p.read_text()), p.name)

    def test_provision_is_glue(self):
        text = (OR / "provision.sh").read_text()
        self.assertLess(len(text.splitlines()), 100)
        self.assertIn("set -euo pipefail", text)
        self.assertIn("pin_git_checkout", text)  # the one source input is fetched through the fail-closed helper
        self.assertIn("signed-by", text)  # the package manager's own signed-repo mechanism
        self.assertNotRegex(text, r"curl[^\n]*\|\s*(ba)?sh")
        self.assertNotIn("r2.cloudflarestorage", text)

    def test_the_probe_never_prints_identity(self):
        c = (OR / "nvcodec_probe.c").read_text()
        for word in ("cuDeviceGetUuid", "cuDeviceGetPCIBusId", "serial"):
            self.assertNotIn(word, c.replace("the serial and the PCI address never are", ""))

    def test_the_rows_are_discovery_rows_not_gate_rows(self):
        run = (ROOT / "scripts/apps/run_apps.sh").read_text()
        self.assertRegex(run, r"(?m)^nvcodec_matrix\|\d+\|")
        self.assertRegex(run, r"(?m)^nvml_dcgm\|\d+\|")
        for name in ("gate_list.txt", "gate_list_middle.txt"):
            text = (ROOT / "scripts/apps" / name).read_text()
            self.assertNotIn("nvcodec_matrix", text)
            self.assertNotIn("nvml_dcgm", text)
        self.assertNotIn("OPENROWS", (ROOT / "scripts/apps/expected_rows.txt").read_text())

    def test_the_hook_reads_the_host_census(self):
        hook = (ROOT / "scripts/apps/apps_hook.sh").read_text()
        for frag in ("open_rows/census.py", "--before \"$q0\"", "cextra", "nvcodec_matrix) echo enc", "nvml_dcgm) echo info"):
            self.assertIn(frag, hook)
        self.assertIn("compare_logs.py", (ROOT / "scripts/apps/apps_matrix.sh").read_text())

    def test_the_bundle_provisions_the_rows_without_being_able_to_fail(self):
        b = (ROOT / "scripts/apps/build_bundle.sh").read_text()
        self.assertIn('open_rows/provision.sh" "$B" ||', b)
        self.assertNotIn("BUILD_open_rows", b)

    def test_the_recipe_pins_the_current_runner(self):
        import hashlib
        recipe = json.loads((ROOT / "scripts/apps/image/recipe.json").read_text())
        pin = next(f for f in recipe["local_files"] if f["path"] == "scripts/apps/run_apps.sh")
        self.assertEqual(pin["sha256"], hashlib.sha256((ROOT / "scripts/apps/run_apps.sh").read_bytes()).hexdigest())

    def test_the_run_apps_regex_of_the_dcgm_row_matches_its_summary(self):
        pattern = next(l for l in (ROOT / "scripts/apps/run_apps.sh").read_text().splitlines() if l.startswith("nvml_dcgm|")).split("|")[2]
        self.assertRegex("NVML_DCGM_SUMMARY nvml_fields_ok=12 nvml_fields_other=3", pattern)

    def test_the_run_apps_regex_of_the_matrix_row_demands_no_failures(self):
        pattern = next(l for l in (ROOT / "scripts/apps/run_apps.sh").read_text().splitlines() if l.startswith("nvcodec_matrix|")).split("|")[2]
        self.assertRegex("NVCODEC_MATRIX_SUMMARY pass=3 not_supported=2 fail=0 mismatch=0 not_run=1", pattern)
        self.assertNotRegex("NVCODEC_MATRIX_SUMMARY pass=3 not_supported=2 fail=1 mismatch=0 not_run=1", pattern)
        self.assertNotRegex("NVCODEC_MATRIX_SUMMARY pass=3 not_supported=2 fail=0 mismatch=2 not_run=1", pattern)


if __name__ == "__main__":
    unittest.main()
