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


class Verdicts(unittest.TestCase):
    def test_the_five_outcomes(self):
        self.assertEqual(L.verdict("SUPPORTED", True, "ok"), ("PASS", "ok"))
        self.assertEqual(L.verdict("SUPPORTED", False, "encoder_params_rejected"), ("FAIL", "encoder_params_rejected"))
        self.assertEqual(L.verdict("UNSUPPORTED", False, "encoder_params_rejected"), ("NOT_SUPPORTED", "encoder_params_rejected"))
        self.assertEqual(L.verdict("UNSUPPORTED", True, "ok"), ("MISMATCH", "unexpected_success"))
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
    def test_parse(self):
        txt = "[Parsed_psnr_2 @ 0x1] PSNR y:40.1 u:42.0 v:43.0 average:40.9 min:39.0 max:45.0\n"
        self.assertAlmostEqual(L.parse_psnr(txt), 40.9)
        self.assertEqual(L.parse_psnr("PSNR y:inf u:inf v:inf average:inf min:inf max:inf"), float("inf"))
        self.assertIsNone(L.parse_psnr("no summary here"))
        two = "PSNR y:1 u:1 v:1 average:10.0\nPSNR y:1 u:1 v:1 average:33.5\n"
        self.assertEqual(L.parse_psnr(two), 33.5)

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
    def test_reasons(self):
        cases = {
            "[h264_nvenc @ 0x5] Driver does not support the required nvenc API version. Required: 13.0 Found: 12.2": "driver_api_too_old",
            "[hevc_nvenc @ 0x5] OpenEncodeSessionEx failed: out of memory (10)": "session_limit",
            "[av1_nvenc @ 0x5] InitializeEncoder failed: invalid param (8)": "encoder_params_rejected",
            "Unknown encoder 'av1_nvenc'": "unknown_encoder",
            "[hevc @ 0x1] Hardware is lacking required capabilities": "hw_decode_unsupported",
            "Failed setup for format cuda: hwaccel initialisation returned error.": "hw_decode_unsupported",
            "Cannot load libnvidia-encode.so.1": "no_encode_device",
            "something else entirely": "other",
        }
        for text, want in cases.items():
            self.assertEqual(L.classify_reason(text), want, text)

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

    def test_a_documented_limit_moves_a_disagreement_out_of_candidates(self):
        limits = L.load_known_limits("case_glob\tresult\treason\nenc_*_8k\tFAIL\tV3_VIDEO_ENGINES.md section 6\n")
        h, g = self._row("enc_hevc_8k", "PASS"), self._row("enc_hevc_8k", "FAIL")
        self.assertEqual(L.classify_pair(h, g, limits), ("DISAGREE", L.BUCKET_KNOWN))
        self.assertEqual(L.classify_pair(h, dict(g, result="MISMATCH"), limits), ("DISAGREE", L.BUCKET_CANDIDATE))  # the limit explains FAIL only
        h2, g2 = self._row("enc_hevc_5k", "PASS"), self._row("enc_hevc_5k", "FAIL")
        self.assertEqual(L.classify_pair(h2, g2, limits), ("DISAGREE", L.BUCKET_CANDIDATE))

    def test_limits_need_a_reason(self):
        with self.assertRaises(L.TableError):
            L.load_known_limits("enc_x\tFAIL\t \n")
        with self.assertRaises(L.TableError):
            L.load_known_limits("enc_x\tBROKEN\tbecause\n")
        self.assertEqual(L.load_known_limits((OR / "nvcodec_known_limits.tsv").read_text()), [])

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

    def test_live_counters_use_a_tolerance_and_static_values_do_not(self):
        h = rows_of(R("query", "utilization.rates", "OK", "gpu=3;memory=1"))[("query", "utilization.rates")]
        near = rows_of(R("query", "utilization.rates", "OK", "gpu=20;memory=1"))[("query", "utilization.rates")]
        far = rows_of(R("query", "utilization.rates", "OK", "gpu=90;memory=1"))[("query", "utilization.rates")]
        self.assertEqual(self.cmp(h, near)[0], "OK")
        self.assertEqual(self.cmp(h, far)[0], "FINDING_DIFF_LIVE")
        hs = rows_of(R("query", "static.num_cores", "OK", "3584"))[("query", "static.num_cores")]
        gs = rows_of(R("query", "static.num_cores", "OK", "3583"))[("query", "static.num_cores")]
        self.assertEqual(self.cmp(hs, gs)[0], "FINDING_DIFF_STATIC")
        self.assertEqual(self.cmp(hs, hs)[0], "OK")

    def test_numeric_class_only_asks_for_a_number(self):
        h = rows_of(R("query", "memory.info", "OK", "total=12884901888;free=1;used=1"))[("query", "memory.info")]
        g = rows_of(R("query", "memory.info", "OK", "total=4294967296;free=1;used=1"))[("query", "memory.info")]
        self.assertEqual(self.cmp(h, g)[0], "OK")  # kayfabe's frame-buffer policy: the number is what is owed

    def test_a_missing_part_and_a_missing_row(self):
        h = rows_of(R("query", "memory.info", "OK", "total=1;free=1;used=1"))[("query", "memory.info")]
        g = rows_of(R("query", "memory.info", "OK", "total=1"))[("query", "memory.info")]
        self.assertEqual(self.cmp(h, g)[0], "FINDING_PART_MISSING")
        self.assertEqual(self.cmp(h, None)[0], "FINDING_GUEST_MISSING")

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
        self.assertTrue(N.within(10, 14, "abs:5"))
        self.assertFalse(N.within(10, 16, "abs:5"))
        self.assertTrue(N.within(100, 130, "rel:0.3"))


DISCOVERY = "1 GPU found.\n+--------+----------------------------------------------------------------------+\n| GPU ID | Device Information                                                   |\n"
DMON_LIST = """Long Name                                   Short Name   Field ID
GPU Temperature                             TMPTR        150
SM Clock                                    SMCLK        100
SM Utilization                              GPUTL        203
"""
DMON = """#Entity   ID   TMPTR   SMCLK   GPUTL
GPU       0    41      210     N/A
GPU       0    42      210     N/A
"""
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
        self.assertEqual(N.parse_dcgm_dmon_list(DMON_LIST), [(150, "TMPTR", "GPU Temperature"), (100, "SMCLK", "SM Clock"), (203, "GPUTL", "SM Utilization")])

    def test_dmon_values_and_na(self):
        series = N.parse_dcgm_dmon(DMON, ["TMPTR", "SMCLK", "GPUTL"])
        self.assertEqual(series["TMPTR"], [41.0, 42.0])
        self.assertEqual(series["GPUTL"], [None, None])
        rows = N.dmon_rows(series)
        self.assertIn("dcgm\tTMPTR\t-\tOK\t42", rows)
        self.assertIn("dcgm\tGPUTL\t-\tNA\tN/A", rows)
        self.assertEqual(N.dmon_rows({"X": []})[0], "dcgm\tX\t-\tNA\tno sample")

    def test_diag_json_both_layouts(self):
        t4 = N.parse_diag_json(DIAG_V4)
        self.assertEqual([(c, n, s) for c, n, g, s, i in t4], [("Deployment", "software", "Pass"), ("Hardware", "memory", "Skip"), ("Hardware", "pcie", "Fail")])
        t3 = N.parse_diag_json("log line before\n" + DIAG_V3)
        self.assertEqual(t3[0][3], "Pass")
        self.assertEqual(t3[0][2], "0")
        self.assertEqual(N.parse_diag_json("not json"), [])
        self.assertEqual(N.parse_diag_json(""), [])

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

    def test_smi_lines(self):
        self.assertEqual(census.parse_smi_line("2, 40, 0, 55, 1200"), {"sess": 2.0, "enc": 40.0, "dec": 0.0, "gpu": 55.0, "mem": 1200.0})
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
        self.assertEqual(census.gpu_use_verdict(census.peaks([]), "gpu")[0], "UNPROVEN")  # no data is never a pass

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
    def test_round_trip_and_corruption(self):
        log = "noise\n" + rowio.emit("a.tsv", "x\ty\n") + "more noise\n" + rowio.emit("b.tsv", "z\n")
        self.assertEqual(rowio.extract(log), {"a.tsv": "x\ty\n", "b.tsv": "z\n"})
        self.assertEqual(rowio.extract(log.replace("x\ty", "x\tq")), {"b.tsv": "z\n"})
        self.assertEqual(rowio.extract(log.replace("@@END b.tsv\n", "")), {"a.tsv": "x\ty\n"})  # truncated: dropped, never repaired

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

    def test_field_ids_come_from_the_driver_answer(self):
        self.assertEqual(self.parsed[("field", "fi7")]["status"], "OK")
        self.assertEqual(self.parsed[("field", "fi7")]["value"], "1234")
        self.assertEqual(self.parsed[("field", "fi9")]["value"], "2.5")
        self.assertEqual(self.parsed[("field", "fi11")]["status"], "NOT_SUPPORTED")
        self.assertNotIn(("field", "fi8"), self.parsed)  # INVALID_ARGUMENT = the library does not know the id: not listed
        self.assertIn("known_ids=3 ", self.meta[0])

    def test_typed_queries(self):
        self.assertEqual(self.parsed[("query", "utilization.rates")]["value"], "gpu=42;memory=7")
        self.assertEqual(self.parsed[("query", "utilization.encoder")]["value"], "v0=5;v1=167000")
        self.assertEqual(self.parsed[("query", "utilization.decoder")]["status"], "NOT_SUPPORTED")
        self.assertEqual(self.parsed[("query", "clock.current.sm")]["value"], "1001")
        self.assertEqual(self.parsed[("query", "name")]["value"], "Fake Die FA4E")
        self.assertEqual(self.parsed[("query", "processes.compute")]["value"], "count=2")
        self.assertEqual(self.parsed[("query", "static.cuda_cc")]["value"], "v0=8;v1=6")
        self.assertEqual(self.parsed[("query", "system.driver_version")]["value"], "580.126.20")
        self.assertEqual(self.parsed[("query", "system.cuda_driver_version")]["value"], "13000")

    def test_a_missing_symbol_is_a_row_not_a_crash(self):
        self.assertEqual(self.parsed[("query", "power.usage_mw")]["status"], "NO_SYMBOL")

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
    codec_fail = mode == "psnr_low"
    say("[Parsed_psnr_2 @ 0x1] PSNR y:%s u:1 v:1 average:%s min:1 max:1" % (("20.0", "20.0") if codec_fail else ("41.0", "41.0")))
    sys.exit(0)
enc = next((x for x in a if x.endswith("_nvenc")), "")
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

    def run_row(self, probe, mode="all_ok", only="enc_*,sess_h264_n01,dec_hevc_420_8_hw,dec_hevc_420_10_hw"):
        t = Path(self.tmp.name)
        (t / "probe.txt").write_text(probe)
        self.probe.write_text("#!/bin/sh\ncat %s\n" % (t / "probe.txt"))
        self.probe.chmod(0o755)
        env = dict(os.environ, FAKE_MODE=mode, OR=str(OR))
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

    def test_psnr_below_the_threshold_fails_the_case(self):
        p, rows = self.run_row(probe_text(av1=True), mode="psnr_low")
        r = rows["enc_h264_420_8_1080p"]
        self.assertEqual((r["result"], r["reason"]), ("FAIL", "psnr_below_threshold"))

    def test_an_unexpected_success_is_a_mismatch(self):
        p, rows = self.run_row(probe_text(av1=False))  # the fake encodes AV1 fine although the probe says the die cannot
        self.assertEqual(rows["enc_av1_420_8_1080p"]["result"], "MISMATCH")

    def test_a_probe_that_cannot_ask_makes_everything_not_run(self):
        p, rows = self.run_row(probe_text(nvenc_error=True))
        self.assertTrue(all(r["result"] == "NOT_RUN" for k, r in rows.items() if k.startswith("enc_")), p.stdout[-400:])
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
        for frag in ("open_rows/census.py", "--before \"$q0\"", "cextra", "nvcodec_matrix) echo enc", "nvml_dcgm) echo gpu"):
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


if __name__ == "__main__":
    unittest.main()
