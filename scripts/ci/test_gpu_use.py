#!/usr/bin/env python3
"""GPU-free tests of the GPU-use judge (scripts/bench/gpu_use.py) and the Linux matrix wiring (scripts/apps/gpu_use_rows.py, run_apps.sh, apps_hook.sh).

FALSIFIER (written before the code, 2026-11-17): a row run with the GPU hidden (the application's own CPU switch, a software device, a software encoder, a
silent fallback) still passes. Every class (b) row therefore has a GPU fixture that must be judged PASS and a CPU fixture that must be judged FAIL, and
the same CPU run goes through the real row command (run_apps.sh with fake binaries) so the proof lines the judge reads are the ones the row prints.

Fixtures: scripts/ci/fixtures/gpu_use/ (README.md there says which lines are MEASURED on a real box and which are SYNTHETIC). No real UUIDs.
"""
import importlib.util
import json
import os
from pathlib import Path
import re
import stat
import subprocess
import sys
import tempfile
import unittest

HERE = Path(__file__).resolve().parent
ROOT = HERE.parents[1]
FIX = HERE / "fixtures" / "gpu_use"


def load(name, rel):
    spec = importlib.util.spec_from_file_location(name, ROOT / rel)
    mod = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(mod)
    return mod


gu = load("kf_gpu_use_t", "scripts/bench/gpu_use.py")
rows = load("kf_gpu_use_rows_t", "scripts/apps/gpu_use_rows.py")


def fx(name):
    return (FIX / name).read_text()


def status(births, subs=0, doorbells=None, nsi="", live="", parks=10):
    tokens = live
    db = "" if doorbells is None else " dbfast[ON regs=1 placed=1/256 sites=1 doorbells=%d wakes=0]" % doorbells
    return ("kf3: family=Ampere phase=Running trapped=1 chan[births=%d pt_births=0 acts=1/0refused nsi=[%s] served=1 parks=%d host_rings=0 contended=0 "
            "poisoned=0 tokens=[%s]]%s unserviced=[]" % (births, nsi, parks, tokens, db))


def tok(t, subs, fwd=0, serves=1):
    return "%s:fwd=%d,subs=%d,serves=%d,put=Some(0),gp_get=None" % (t, fwd, subs, serves)


def retired(t, subs, fwd=0, serves=1, extra=""):
    return "kf3: chan token %s (host 0x1001d) RETIRED, forwarded=%d submissions=%d splits=0/0 serves=%d last_put=Some(1) gp_get=Some(1)%s dead=None" % (
        t, fwd, subs, serves, extra)


class Census(unittest.TestCase):
    def test_a_real_window_counts_the_retired_channels(self):
        c = gu.window_census(fx("kf3_window_gpu.log").splitlines(), fx("kf3_base.txt").strip())
        self.assertFalse(c["base_missing"])
        self.assertEqual((c["births"], c["submissions"], c["retired"]), (6, 411, 6))
        self.assertIn("GR0", c["wakes"])

    def test_a_window_where_nothing_moved_has_no_work(self):
        c = gu.window_census(fx("kf3_window_idle.log").splitlines(), fx("kf3_base_end.txt").strip())
        self.assertEqual((c["births"], c["submissions"]), (0, 0))

    def test_other_counters_moving_is_not_work(self):
        c = gu.window_census(fx("kf3_window_cpu_app.log").splitlines(), fx("kf3_base_end.txt").strip())
        self.assertEqual(c["submissions"], 0)
        self.assertEqual(gu.judge_census(c)[0], gu.FAIL)

    def test_live_channels_growth_between_two_status_lines(self):
        base = status(2, live=tok("0x801", 10))
        win = [status(5, live=tok("0x801", 70) + " " + tok("0x802", 5))]
        c = gu.window_census(win, base)
        self.assertEqual((c["births"], c["submissions"]), (3, 65))

    def test_a_channel_retired_in_the_window_counts_only_its_window_part(self):
        base = status(2, live=tok("0x801", 10))
        self.assertEqual(gu.window_census([retired("0x801", 50)], base)["submissions"], 40)
        self.assertEqual(gu.window_census([retired("0x801", 50), status(2, live="")], base)["submissions"], 40)

    def test_a_live_channel_the_window_did_not_touch_does_not_reduce_the_work(self):
        base = status(2, live=tok("0x801", 100))
        self.assertEqual(gu.window_census([retired("0x900", 40)], base)["submissions"], 40)

    def test_one_of_two_live_channels_retiring_leaves_the_other_out_of_the_work(self):
        base = status(2, live=tok("0x801", 100) + " " + tok("0x802", 5))
        self.assertEqual(gu.window_census([retired("0x802", 25)], base)["submissions"], 20)

    def test_token_reuse_after_a_retirement_is_not_double_counted(self):
        base = status(2, live=tok("0x801", 10))
        win = [retired("0x801", 50), status(3, live=tok("0x801", 5))]
        self.assertEqual(gu.window_census(win, base)["submissions"], 45)

    def test_a_channel_retired_after_the_last_status_line_is_not_counted_twice(self):
        base = status(2, live="")
        win = [status(3, live=tok("0x801", 20)), retired("0x801", 30)]
        self.assertEqual(gu.window_census(win, base)["submissions"], 30)

    def test_a_counter_that_went_down_counts_from_zero(self):
        base = status(50, doorbells=900)
        win = [status(4, doorbells=7)]
        c = gu.window_census(win, base)
        self.assertEqual((c["births"], c["doorbells"]), (4, 7))

    def test_without_a_baseline_the_window_is_a_lower_bound_and_says_so(self):
        c = gu.window_census([status(5, live=tok("0x801", 100)), status(6, live=tok("0x801", 150))], None)
        self.assertTrue(c["base_missing"])
        self.assertEqual(c["submissions"], 50)

    def test_engine_wakes_are_deltas(self):
        base = status(1, nsi="GR0:100/0raised CE2:5/0raised")
        win = [status(1, nsi="GR0:180/3raised(nolive=0,novec=0) CE2:5/0raised CE3:2/1raised")]
        self.assertEqual(gu.window_census(win, base)["wakes"], {"GR0": 80, "CE3": 2})

    def test_garbage_and_hostile_lines_do_not_crash_or_count(self):
        junk = ["kf3: family=" + "x" * 100000, "kf3: family=A phase=B tokens=[" + "0x1:fwd=1,subs=1,serves=1 " * 10, "\x00\xff" * 100, "RETIRED" * 1000,
                "kf3: chan token 0xZZ RETIRED, forwarded=a submissions=b serves=c"]
        c = gu.window_census(junk, None)
        self.assertEqual(c["submissions"], 0)

    def test_xid_lines_are_counted_not_judged(self):
        c = gu.window_census(["NVRM: Xid (PCI:0000:00:03): 31, pid=1"], None)
        self.assertEqual(c["xid"], 1)


class JudgeCensus(unittest.TestCase):
    def test_work_passes(self):
        c = gu.window_census(fx("kf3_window_gpu.log").splitlines(), fx("kf3_base.txt").strip())
        self.assertEqual(gu.judge_census(c)[0], gu.PASS)

    def test_no_work_with_a_baseline_is_a_fail(self):
        c = gu.window_census(fx("kf3_window_idle.log").splitlines(), fx("kf3_base_end.txt").strip())
        self.assertEqual(gu.judge_census(c)[0], gu.FAIL)

    def test_no_work_without_a_baseline_cannot_be_told_and_is_not_run(self):
        c = gu.window_census(fx("kf3_window_idle.log").splitlines(), None)
        self.assertEqual(gu.judge_census(c)[0], gu.NOT_RUN)

    def test_an_empty_slice_is_not_run_never_a_pass(self):
        self.assertEqual(gu.judge_census(gu.window_census([], None))[0], gu.NOT_RUN)
        self.assertEqual(gu.judge_census(gu.window_census(fx("kf3_window_no_status.log").splitlines(), fx("kf3_base.txt").strip()))[0], gu.NOT_RUN)
        self.assertEqual(gu.judge_census(None)[0], gu.NOT_RUN)

    def test_the_floor_is_between_opening_the_device_and_a_cuda_context(self):
        base = status(1)
        open_only = gu.window_census([retired("0x1", 2), status(1)], base)
        context = gu.window_census([retired("0x1", 411), status(1)], base)
        self.assertEqual(gu.judge_census(open_only)[0], gu.FAIL)
        self.assertEqual(gu.judge_census(context)[0], gu.PASS)


class AppProofs(unittest.TestCase):
    def proof(self, kind, name, **params):
        return gu.PROOFS[kind](fx(name), **params)

    def test_every_row_has_a_gpu_log_that_passes_and_a_cpu_log_that_fails(self):
        cases = [("blender", "blender_gpu.log", "blender_cpu.log", {}), ("llama", "llama_gen_gpu.log", "llama_gen_cpu_ngl0.log", {"mode": "gen"}),
                 ("llama", "llama_gen_gpu.log", "llama_gen_cpu_nocuda.log", {"mode": "gen"}), ("llama", "llama_bench_gpu.log", "llama_bench_cpu_ngl0.log", {"mode": "bench"}),
                 ("hashcat", "hashcat_gpu.log", "hashcat_cpu.log", {}), ("opencl_bench", "clpeak_gpu.log", "clpeak_cpu.log", {}),
                 ("vkpeak", "vkpeak_gpu.log", "vkpeak_cpu.log", {}), ("egl", "egl_gpu.log", "egl_cpu.log", {}),
                 ("nvenc", "nvenc_gpu.log", "nvenc_cpu.log", {"codec": "h264"}), ("nvdec", "nvdec_gpu.log", "nvdec_cpu.log", {}),
                 ("framework", "torch_gpu.log", "torch_cpu.log", {"framework": "torch", "min_mib": 1000})]
        for kind, good, bad, params in cases:
            with self.subTest(kind=kind, bad=bad):
                self.assertIs(self.proof(kind, good, **params)["ok"], True, self.proof(kind, good, **params))
                self.assertIs(self.proof(kind, bad, **params)["ok"], False, self.proof(kind, bad, **params))

    def test_blender_silent_fallback_is_caught_even_when_the_row_printed_ok(self):
        p = self.proof("blender", "blender_fallback.log")
        self.assertIs(p["ok"], False)
        self.assertTrue(any("fallback" in r or "not an NVIDIA" in r for r in p["reasons"]))

    def test_blender_needs_both_device_types(self):
        text = fx("blender_gpu.log").replace("dev=OPTIX", "dev=XX").replace("BLENDER_OK OPTIX", "BLENDER_OK XX")
        self.assertIs(gu.proof_blender(text)["ok"], False)

    def test_blender_proof_line_missing_is_a_dropped_proof_not_a_pass(self):
        text = "\n".join(l for l in fx("blender_gpu.log").splitlines() if not l.startswith("BLENDER_DEVICES"))
        p = gu.proof_blender(text)
        self.assertIs(p["ok"], False)
        self.assertIn("dropped", " ".join(p["reasons"]))

    def test_blender_cpu_enabled_next_to_the_gpu_fails(self):
        text = fx("blender_gpu.log").replace("cpu_enabled=False", "cpu_enabled=True")
        self.assertIs(gu.proof_blender(text)["ok"], False)

    def test_llama_partial_offload_is_not_a_pass(self):
        text = fx("llama_gen_gpu.log").replace("offloaded 29/29", "offloaded 20/29")
        self.assertIs(gu.proof_llama(text, mode="gen")["ok"], False)

    def test_llama_zero_of_n_is_the_cpu_message_that_still_matches_the_old_regexes(self):
        self.assertIsNotNone(re.search(r"offloaded [0-9]+/[0-9]+ layers to GPU", "offloaded 0/29 layers to GPU"))
        self.assertIs(gu.proof_llama("ggml_cuda_init: found 1 CUDA devices\n  Device 0: NVIDIA GeForce RTX 4090, compute capability 8.9\noffloaded 0/29 layers to GPU", mode="gen")["ok"], False)

    def test_clpeak_passes_on_the_nvidia_block_even_with_a_cpu_block_present_and_fails_on_the_cpu_block_alone(self):
        p = self.proof("opencl_bench", "clpeak_gpu.log")
        self.assertIs(p["ok"], True)
        self.assertEqual(len(p["facts"]["devices"]), 2)      # the real log: the NVIDIA device AND pocl
        self.assertIs(self.proof("opencl_bench", "clpeak_cpu.log")["ok"], False)

    def test_clpeak_skipped_tests_on_the_nvidia_block_fail(self):
        text = fx("clpeak_gpu.log").replace("Single-precision compute (GFLOPS)", "Tests skipped\n    Single-precision compute (GFLOPS)", 1)
        self.assertIs(gu.proof_opencl_bench(text)["ok"], False)

    def test_hashcat_skipped_cpu_device_is_not_a_used_device(self):
        p = self.proof("hashcat", "hashcat_gpu.log")
        self.assertIs(p["ok"], True)
        self.assertEqual((p["facts"]["devices"], p["facts"]["used"]), (3, 1))

    def test_hashcat_without_device_lines_is_unproven_none(self):
        self.assertIsNone(gu.proof_hashcat("Session..........: hashcat\nSpeed.#1.........:  5296.5 MH/s")["ok"])

    def test_nvdec_unmeasured_marker_missing_is_none_and_fallbacks_are_false(self):
        self.assertIsNone(gu.proof_nvdec("frame=  600 fps=0.0")["ok"])
        self.assertIs(self.proof("nvdec", "nvdec_failed_setup.log")["ok"], False)

    def test_the_harness_header_line_is_not_application_output(self):
        # the real header of the nvdec row contains the words `Failed setup for format cuda` (they are in the command)
        text = "=== nvdec_h264 side=guest cmd: grep -q \"Failed setup for format cuda\" x\n[h264 @ 0x1] Reinit context to 1280x720, pix_fmt: cuda\n=== end rc=0"
        self.assertIs(gu.proof_nvdec(text)["ok"], True)

    def test_framework_proof_needs_cuda_nvidia_and_device_memory(self):
        self.assertIsNone(self.proof("framework", "torch_noproof.log", framework="torch", min_mib=1000)["ok"])
        self.assertIs(gu.proof_framework("GPU_USE_PROOF torch device=cuda:0 name=NVIDIA GeForce RTX 4090 peak_mib=3", framework="torch", min_mib=1000)["ok"], False)
        self.assertIs(gu.proof_framework("GPU_USE_PROOF torch device=cuda:0 name=Software Rasterizer peak_mib=2000", framework="torch", min_mib=1000)["ok"], False)
        self.assertIs(gu.proof_framework(fx("torch_gpu.log"), framework="cupy", min_mib=1)["ok"], None)   # another framework's line does not count

    def test_a_hostile_log_is_bounded(self):
        big = "GPU_USE_PROOF torch device=cuda:0 name=NVIDIA peak_mib=" + "9" * (3 << 20)
        self.assertIsNone(gu.proof_framework(big, framework="torch", min_mib=1)["ok"])
        self.assertLessEqual(len(gu._text("a\n" * (3 << 20))), gu.MAX_TEXT)


def smi_busy():
    q = "\n".join("2026/11/17 00:00:%02d.000, %s, 0, %s, 0, 0" % (i, "[N/A]" if i < 2 else 80, 300 if i < 2 else 3000) for i in range(8))
    return gu.smi_record(q, "", ("gpu", "mem"))


def smi_idle():
    q = "\n".join("2026/11/17 00:00:%02d.000, 0, 0, 300, 0, 0" % i for i in range(8))
    return gu.smi_record(q, "", ("gpu", "mem"))


class Judge(unittest.TestCase):
    census_ok = staticmethod(lambda: gu.window_census(fx("kf3_window_gpu.log").splitlines(), fx("kf3_base.txt").strip()))
    census_none = staticmethod(lambda: gu.window_census(fx("kf3_window_idle.log").splitlines(), fx("kf3_base_end.txt").strip()))

    def good_app(self):
        return gu.proof_blender(fx("blender_gpu.log"))

    def test_everything_agrees_is_a_pass_with_host_authority(self):
        v = gu.judge("blender_cycles", census=self.census_ok(), smi=smi_busy(), app=self.good_app(), need_engines=("gpu", "mem"))
        self.assertEqual((v["verdict"], v["authority"]), (gu.PASS, "host"))

    def test_the_host_saying_no_work_beats_a_guest_that_says_gpu(self):
        v = gu.judge("blender_cycles", census=self.census_none(), smi=smi_busy(), app=self.good_app(), need_engines=("gpu", "mem"))
        self.assertEqual(v["verdict"], gu.FAIL)
        self.assertTrue(any("host census is the authority" in r for r in v["reasons"]))

    def test_an_idle_host_engine_fails_whatever_the_guest_printed(self):
        v = gu.judge("blender_cycles", census=self.census_ok(), smi=smi_idle(), app=self.good_app(), need_engines=("gpu", "mem"))
        self.assertEqual(v["verdict"], gu.FAIL)

    def test_a_cpu_report_fails_even_when_the_host_saw_work(self):
        v = gu.judge("blender_cycles", census=self.census_ok(), smi=smi_busy(), app=gu.proof_blender(fx("blender_cpu.log")), need_engines=("gpu", "mem"))
        self.assertEqual(v["verdict"], gu.FAIL)

    def test_a_dropped_proof_fails_and_an_unmeasured_marker_is_not_run(self):
        v = gu.judge("x", census=self.census_ok(), app=gu.proof_framework("no proof line", framework="torch"), need_engines=())
        self.assertEqual(v["verdict"], gu.FAIL)
        v = gu.judge("x", census=self.census_ok(), app=gu.proof_nvdec("frame=1"), need_engines=(), app_missing=gu.NOT_RUN)
        self.assertEqual(v["verdict"], gu.NOT_RUN)

    def test_a_dropped_proof_fails_when_the_host_is_happy(self):
        v = gu.judge("x", census=self.census_ok(), smi=smi_busy(), app=gu.proof_framework("CHECK ok", framework="torch"), need_engines=("gpu", "mem"))
        self.assertEqual(v["verdict"], gu.FAIL)
        self.assertEqual(v["authority"], "host")

    def test_an_unreadable_sampler_is_not_run_not_idle(self):
        rec = gu.smi_record("", "", ("gpu",))
        self.assertTrue(rec["unreadable"])
        v = gu.judge("x", census=self.census_ok(), smi=rec, app=self.good_app(), need_engines=("gpu",))
        self.assertEqual(v["verdict"], gu.NOT_RUN)
        self.assertTrue(any("not an idle one" in r for r in v["reasons"]))

    def test_no_sampler_record_when_engines_are_required_is_not_run(self):
        self.assertEqual(gu.judge("x", census=self.census_ok(), smi=None, app=self.good_app(), need_engines=("gpu",))["verdict"], gu.NOT_RUN)

    def test_a_fail_wins_over_a_not_run(self):
        v = gu.judge("x", census=None, smi=smi_idle(), app=self.good_app(), need_engines=("gpu",))
        self.assertEqual(v["verdict"], gu.FAIL)

    def test_the_bare_metal_baseline_has_guest_authority_and_needs_no_census(self):
        v = gu.judge("blender_cycles", census=None, app=self.good_app(), need_census=False, side="host")
        self.assertEqual((v["verdict"], v["authority"]), (gu.PASS, "guest"))

    def test_control_verdict(self):
        self.assertEqual(gu.control_verdict({"verdict": gu.FAIL}), "CONTROL_OK")
        for v in (gu.PASS, gu.NOT_RUN):
            self.assertEqual(gu.control_verdict({"verdict": v}), "CONTROL_BROKEN")


# row -> (fixture of the GPU run, fixture of the CPU run) for the rows that have both; the kf3 window and the host engines are the same two cases for all
GPU_CPU = {
    "blender_cycles": ("blender_gpu.log", "blender_cpu.log"), "llama_cpp_gen": ("llama_gen_gpu.log", "llama_gen_cpu_ngl0.log"),
    "llama_bench": ("llama_bench_gpu.log", "llama_bench_cpu_ngl0.log"), "hashcat": ("hashcat_gpu.log", "hashcat_cpu.log"),
    "clpeak": ("clpeak_gpu.log", "clpeak_cpu.log"), "vkpeak": ("vkpeak_gpu.log", "vkpeak_cpu.log"), "egl_offscreen": ("egl_gpu.log", "egl_cpu.log"),
    "nvdec_h264": ("nvdec_gpu.log", "nvdec_cpu.log"), "nvenc_h264": ("nvenc_gpu.log", "nvenc_cpu.log"), "torch_correct": ("torch_gpu.log", "torch_cpu.log"),
}


def smi_for(app, busy=True):
    eng = rows.policy(app)["engines"]
    if not eng:
        return None
    q = "\n".join("2026/11/17 00:00:%02d.000, %s, 0, %s, 0, 0" % (i, ("[N/A]" if i < 2 else (60 if busy else 0)), (300 if i < 2 or not busy else 3000)) for i in range(8))
    d = "# gpu sm mem enc dec\n" + "\n".join("0 %d 0 %d 0" % (50 if busy else 0, 40 if busy else 0) for _ in range(4))
    q2 = q.replace(", 0, 0\n", ", 0, %d\n" % (1 if busy else 0))
    # the encoder session counter is the last column of the query: 0 before, 1 while busy
    q2 = "\n".join(l.rsplit(",", 2)[0] + ", %d, 0" % (0 if (i < 2 or not busy) else 1) for i, l in enumerate(q.splitlines()))
    return gu.smi_record(q2, d, eng, min_samples=rows.policy(app)["min_samples"])


class NegativeControls(unittest.TestCase):
    """The same row with the GPU hidden must be judged FAIL, in the full judge (host census and engines in the picture), for every row with a CPU form."""

    def run_row(self, app, fixture, work, busy):
        kf3 = fx("kf3_window_gpu.log").splitlines() if work else fx("kf3_window_idle.log").splitlines()
        base = fx("kf3_base.txt").strip() if work else fx("kf3_base_end.txt").strip()
        return rows.evaluate(app, "guest", fx(fixture), kf3, base, smi_for(app, busy))

    def test_the_gpu_run_of_every_row_passes(self):
        for app, (good, _) in GPU_CPU.items():
            with self.subTest(app=app):
                self.assertEqual(self.run_row(app, good, True, True)["verdict"], gu.PASS, self.run_row(app, good, True, True)["reasons"])

    def test_the_cpu_run_of_every_row_is_judged_fail_by_the_app_proof_alone(self):
        for app, (_, bad) in GPU_CPU.items():
            with self.subTest(app=app):
                rec = self.run_row(app, bad, True, True)        # even with host work and busy engines (an unrelated process): the app says CPU
                self.assertEqual(rec["verdict"], gu.FAIL, rec["reasons"])
                self.assertEqual(gu.control_verdict(rec), "CONTROL_OK")

    def test_the_cpu_run_is_judged_fail_by_the_host_alone_when_the_guest_lies(self):
        for app, (good, _) in GPU_CPU.items():
            with self.subTest(app=app):
                rec = self.run_row(app, good, False, False)     # the guest printed a perfect GPU log; the host saw nothing
                self.assertEqual(rec["verdict"], gu.FAIL, rec["reasons"])
                self.assertTrue(any(r.startswith("[host-census] FAIL") for r in rec["reasons"]))

    def test_rows_with_engines_fail_on_an_idle_host_gpu_alone(self):
        for app, (good, _) in GPU_CPU.items():
            if not rows.policy(app)["engines"]:
                continue
            with self.subTest(app=app):
                rec = self.run_row(app, good, True, False)
                self.assertEqual(rec["verdict"], gu.FAIL, rec["reasons"])
                self.assertTrue(any(r.startswith("[host-smi] FAIL") for r in rec["reasons"]))

    def test_llama_partial_offload_through_the_full_judge(self):
        text = fx("llama_gen_gpu.log").replace("offloaded 29/29", "offloaded 12/29")
        self.assertEqual(rows.evaluate("llama_cpp_gen", "guest", text, fx("kf3_window_gpu.log").splitlines(), fx("kf3_base.txt").strip(), smi_for("llama_cpp_gen"))["verdict"], gu.FAIL)

    def test_other_cpu_forms(self):
        for app, fixture in (("llama_cpp_gen", "llama_gen_cpu_nocuda.log"), ("blender_cycles", "blender_fallback.log"), ("nvdec_h264", "nvdec_failed_setup.log")):
            with self.subTest(app=app):
                self.assertEqual(self.run_row(app, fixture, True, True)["verdict"], gu.FAIL)

    def test_a_row_without_a_kf3_log_is_not_run_never_a_pass(self):
        rec = rows.evaluate("torch_correct", "guest", fx("torch_gpu.log"), None, None, smi_for("torch_correct"))
        self.assertEqual(rec["verdict"], gu.NOT_RUN)


class Rows(unittest.TestCase):
    def test_every_row_of_run_apps_is_classified_and_the_counts_are_pinned(self):
        names = rows.run_apps_names()
        self.assertEqual(len(names), 71)
        counts = {}
        for n in names:
            counts[rows.row_class(n)] = counts.get(rows.row_class(n), 0) + 1
        self.assertEqual(counts, {"a": 52, "a-info": 4, "a+proof": 6, "b": 8, "b-skip": 1})
        for n in rows.ROWS:
            self.assertIn(n, names, "a classified row that run_apps.sh does not have: " + n)

    def test_every_wired_row_has_a_proof_kind_and_a_negative_control_fixture_pair(self):
        for n in rows.WIRED:
            self.assertIn(n, rows.ROWS)
            self.assertIn(rows.ROWS[n][1], gu.PROOFS)
        # the rows with a CPU form are exactly those with a fixture pair (nvenc_hevc, hf_generate, torch_ai_bench, cupy share the proof of their sibling)
        for n in GPU_CPU:
            self.assertIn(n, rows.WIRED)

    def line(self, app, verdict="PASS", note="-"):
        return "APPRES side=guest app=%s verdict=%s rc=0 secs=3 quiet=0 note=%s" % (app, verdict, note)

    def annotate(self, app, fixture, kf3_good=True, verdict="PASS", note="-"):
        kf3 = fx("kf3_window_gpu.log" if kf3_good else "kf3_window_idle.log").splitlines()
        base = fx("kf3_base.txt").strip() if kf3_good else fx("kf3_base_end.txt").strip()
        return rows.annotate_line(self.line(app, verdict, note), "guest", fx(fixture), kf3, base, smi_for(app, kf3_good))

    def test_a_good_run_keeps_its_verdict_and_gets_the_fields(self):
        new, rec = self.annotate("blender_cycles", "blender_gpu.log")
        self.assertIn(" verdict=PASS ", new)
        self.assertRegex(new, r" gpu_use=PASS gpu_auth=host gpu_subs=411 gpu_births=6$")

    def test_a_pass_that_fell_back_becomes_a_fail_with_the_reason_in_the_note(self):
        new, _ = self.annotate("llama_cpp_gen", "llama_gen_cpu_ngl0.log", note="warn:x")
        self.assertIn(" verdict=FAIL ", new)
        self.assertIn("note=gpu-use:FAIL:", new)
        self.assertIn("/ warn:x", new)
        kv = dict(re.findall(r"(\w+)=((?:(?! \w+=).)*)", new[len("APPRES "):]))
        self.assertEqual((kv["verdict"], kv["gpu_use"]), ("FAIL", "FAIL"))
        self.assertNotIn("|", kv["note"])

    def test_a_pass_that_cannot_be_judged_becomes_notrun(self):
        new, _ = rows.annotate_line(self.line("blender_cycles"), "guest", fx("blender_gpu.log"), [], None, None)
        self.assertIn(" verdict=NOTRUN ", new)
        self.assertIn("gpu-use:NOT_RUN", new)
        new, _ = rows.annotate_line(self.line("egl_offscreen"), "guest", fx("egl_gpu.log"), [], None, None)       # a row with no engine requirement: the empty window alone
        self.assertIn(" verdict=NOTRUN ", new)

    def test_a_row_whose_script_dropped_its_proof_is_rewritten_to_fail(self):
        new, _ = self.annotate("torch_correct", "torch_noproof.log")
        self.assertIn(" verdict=FAIL ", new)
        self.assertIn("dropped its device proof", new)

    def test_a_row_that_was_not_a_pass_is_left_alone(self):
        for verdict in ("FAIL", "TIMEOUT", "NOTRUN", "HANG"):
            new, _ = self.annotate("blender_cycles", "blender_cpu.log", verdict=verdict)
            self.assertIn(" verdict=%s " % verdict, new)

    def test_class_a_rows_are_recorded_never_judged(self):
        new, rec = rows.annotate_line(self.line("vectorAdd"), "guest", "anything", fx("kf3_window_idle.log").splitlines(), fx("kf3_base_end.txt").strip(), None)
        self.assertIn(" verdict=PASS ", new)
        self.assertIn(" gpu_use=NA ", new)

    def test_the_bare_metal_baseline_is_judged_by_the_app_proof_only(self):
        new, _ = rows.annotate_line(self.line("hashcat"), "host", fx("hashcat_cpu.log"))
        self.assertIn(" verdict=FAIL ", new)
        new, _ = rows.annotate_line(self.line("hashcat"), "host", fx("hashcat_gpu.log"))
        self.assertRegex(new, r" verdict=PASS .* gpu_use=PASS gpu_auth=guest")

    def test_geekbench_policy_skip_is_never_rewritten(self):
        new, _ = rows.annotate_line(self.line("geekbench_gpu"), "guest", "x", [], None, None)
        self.assertIn(" verdict=PASS ", new)


class Cli(unittest.TestCase):
    def run_cli(self, *args, **kw):
        return subprocess.run([sys.executable, "-B", "-I", str(ROOT / "scripts/apps/gpu_use_rows.py")] + list(args), capture_output=True, text=True, stdin=subprocess.DEVNULL, timeout=60, **kw)

    def test_annotate_through_the_cli_with_files(self):
        with tempfile.TemporaryDirectory() as d:
            d = Path(d)
            (d / "a.log").write_text(fx("llama_gen_cpu_ngl0.log"))
            (d / "k.log").write_text(fx("kf3_window_gpu.log"))
            (d / "b.txt").write_text(fx("kf3_base.txt"))
            line = "APPRES side=guest app=llama_cpp_gen verdict=PASS rc=0 secs=3 quiet=0 note=-"
            p = self.run_cli("annotate", "--side", "guest", "--app", "llama_cpp_gen", "--line", line, "--app-log", str(d / "a.log"), "--kf3-log", str(d / "k.log"),
                             "--kf3-base", str(d / "b.txt"), "--record-dir", str(d))
            self.assertEqual(p.returncode, 0, p.stderr)
            self.assertIn("verdict=FAIL", p.stdout)
            rec = json.loads((d / "llama_cpp_gen.gpu_use.json").read_text())
            self.assertEqual(rec["verdict"], "FAIL")

    def test_negctl_prints_a_control_line(self):
        with tempfile.TemporaryDirectory() as d:
            (Path(d) / "a.log").write_text(fx("llama_gen_cpu_ngl0.log"))
            line = "APPRES side=guest app=llama_cpp_gen verdict=PASS rc=0 secs=3 quiet=0 note=-"
            p = self.run_cli("annotate", "--side", "host", "--app", "llama_cpp_gen", "--line", line, "--app-log", str(Path(d) / "a.log"), "--negctl")
            self.assertRegex(p.stdout, r"^NEGCTL app=llama_cpp_gen control=CONTROL_OK judged=FAIL why=")
            (Path(d) / "a.log").write_text(fx("llama_gen_gpu.log"))
            p = self.run_cli("annotate", "--side", "host", "--app", "llama_cpp_gen", "--line", line, "--app-log", str(Path(d) / "a.log"), "--negctl")
            self.assertIn("control=CONTROL_BROKEN judged=PASS", p.stdout)

    def test_negctl_of_a_row_that_is_not_judged_is_a_broken_control(self):
        line = "APPRES side=guest app=vectorAdd verdict=PASS rc=0 secs=1 quiet=0 note=-"
        p = self.run_cli("annotate", "--side", "host", "--app", "vectorAdd", "--line", line, "--app-log", str(FIX / "torch_cpu.log"), "--negctl")
        self.assertIn("control=CONTROL_BROKEN", p.stdout)

    def test_a_crash_inside_the_census_fails_closed_too(self):
        import io
        from contextlib import redirect_stdout
        saved = rows.gu.window_census
        rows.gu.window_census = lambda *a, **k: (_ for _ in ()).throw(RuntimeError("boom"))
        try:
            ns = type("A", (), {"app_log": str(FIX / "egl_gpu.log"), "kf3_log": str(FIX / "kf3_window_gpu.log"), "kf3_base": str(FIX / "kf3_base.txt"), "smi_dir": None,
                                "side": "guest", "negctl": False, "record_dir": None, "app": "egl_offscreen",
                                "line": "APPRES side=guest app=egl_offscreen verdict=PASS rc=0 secs=3 quiet=0 note=-"})()
            buf = io.StringIO()
            with redirect_stdout(buf):
                rows.cmd_annotate(ns)
        finally:
            rows.gu.window_census = saved
        self.assertIn("verdict=NOTRUN", buf.getvalue())

    def test_a_missing_log_file_is_not_run(self):
        line = "APPRES side=host app=hashcat verdict=PASS rc=0 secs=3 quiet=0 note=-"
        p = self.run_cli("annotate", "--side", "host", "--app", "hashcat", "--line", line, "--app-log", "/nonexistent/x.log")
        self.assertIn("verdict=NOTRUN", p.stdout)

    def test_a_crashing_judge_fails_closed_for_a_wired_pass(self):
        # a directory where the log should be: read_text returns None for an OSError, so crash another way: an unknown proof kind
        saved = rows.ROWS["hashcat"]
        try:
            rows.ROWS["hashcat"] = ("b", "no_such_kind", {}, (), {"interval_ms": 100, "min_samples": 1}, "FAIL")
            import io
            from contextlib import redirect_stdout
            ns = type("A", (), {"app_log": None, "kf3_log": None, "kf3_base": None, "smi_dir": None, "side": "host", "negctl": False, "record_dir": None, "app": "hashcat",
                                "line": "APPRES side=host app=hashcat verdict=PASS rc=0 secs=3 quiet=0 note=-"})()
            ns.app_log = str(FIX / "hashcat_gpu.log")
            buf = io.StringIO()
            with redirect_stdout(buf):
                rows.cmd_annotate(ns)
            self.assertIn("verdict=NOTRUN", buf.getvalue())
            self.assertIn("judge-error", buf.getvalue())
        finally:
            rows.ROWS["hashcat"] = saved

    def test_annotate_res_rewrites_a_res_file_once(self):
        with tempfile.TemporaryDirectory() as d:
            d = Path(d)
            (d / "hashcat.log").write_text(fx("hashcat_cpu.log"))
            (d / "vectorAdd.log").write_text("Test PASSED")
            (d / "x.res").write_text("APPRES side=host app=hashcat verdict=PASS rc=0 secs=3 quiet=0 note=-\nAPPRES side=host app=vectorAdd verdict=PASS rc=0 secs=1 quiet=0 note=-\nsome other line\n")
            for _ in range(2):
                self.assertEqual(self.run_cli("annotate-res", "--side", "host", "--res", str(d / "x.res"), "--logs", str(d)).returncode, 0)
            text = (d / "x.res").read_text()
            self.assertEqual(text.count("gpu_use="), 2)
            self.assertIn("app=hashcat verdict=FAIL", text)
            self.assertIn("app=vectorAdd verdict=PASS", text)
            self.assertIn("some other line", text)

    def test_sample_passes_the_exit_code_and_writes_the_probe_files(self):
        with tempfile.TemporaryDirectory() as d:
            d = Path(d)
            smi = d / "bin" / "nvidia-smi"
            smi.parent.mkdir()
            smi.write_text("#!/bin/sh\ni=0; while [ $i -lt 300 ]; do echo '2026/11/17 00:00:00.000, 5, 0, 300, 0, 0'; i=$((i+1)); sleep 0.05; done\n")
            smi.chmod(smi.stat().st_mode | stat.S_IXUSR)
            env = dict(os.environ, PATH=str(smi.parent) + os.pathsep + os.environ["PATH"])
            p = self.run_cli("sample", "--out", str(d), "--app", "demo", "--interval-ms", "100", "--settle-s", "0.2", "--", "sh", "-c", "echo hi; exit 7", env=env)
            self.assertEqual((p.returncode, p.stdout.strip()), (7, "hi"))
            self.assertTrue((d / "demo.smi" / "host_query.csv").exists())
            self.assertIn("5", (d / "demo.smi" / "host_query.csv").read_text())

    def test_sample_without_nvidia_smi_just_runs_the_command(self):
        with tempfile.TemporaryDirectory() as d:
            env = dict(os.environ, PATH="/nonexistent-bin" + os.pathsep + "/usr/bin" + os.pathsep + "/bin")
            p = self.run_cli("sample", "--out", d, "--app", "demo", "--settle-s", "0", "--", "sh", "-c", "exit 3", env=env)
            self.assertEqual(p.returncode, 3)


# ---------------------------------------------------------------------------------- the real row commands, with fake binaries
FAKE = r'''#!/bin/bash
echo "$(basename "$0") $*" >> "$APPS_ROOT/calls.log"
%s
'''


class RunAppsRows(unittest.TestCase):
    """run_apps.sh with fake tools: the default commands still PASS and print the proof lines; KF_NEGCTL=1 runs the CPU form of the row, and the judge says FAIL."""

    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory()
        self.root = Path(self.tmp.name)
        for sub in ("bundle/share", "bundle/bin", "bundle/llama", "bundle/samples", "data/blender", "venv/bin", "bin", "out"):
            (self.root / sub).mkdir(parents=True)
        (self.root / "data" / "qwen2.5-1.5b-instruct-q4_k_m.gguf").write_text("x")

    def tearDown(self):
        self.tmp.cleanup()

    def tool(self, rel, body):
        p = self.root / rel
        p.write_text(FAKE % body)
        p.chmod(0o755)

    def run_row(self, app, negctl=False):
        env = dict(os.environ, APPS_ROOT=str(self.root), PATH=str(self.root / "bin") + os.pathsep + os.environ["PATH"], KF_NEGCTL="1" if negctl else "0")
        for k in [k for k in env if k.startswith("KF_") and k != "KF_NEGCTL"]:
            del env[k]
        (self.root / "calls.log").unlink(missing_ok=True)
        p = subprocess.run(["bash", str(ROOT / "scripts/apps/run_apps.sh"), "guest", app], capture_output=True, text=True, stdin=subprocess.DEVNULL, timeout=120, env=env)
        log = (self.root / "out" / "guest" / (app + ".log")).read_text()
        res = [l for l in p.stdout.splitlines() if l.startswith("APPRES ")][-1]
        return res, log, (self.root / "calls.log").read_text() if (self.root / "calls.log").exists() else ""

    def judge(self, app, log, kind_params=None):
        rec = rows.evaluate(app, "host", log)
        return rec

    def test_blender(self):
        self.tool("data/blender/blender", r'''dev="${@: -2:1}"
if [ "$dev" = CPU ]; then echo "BLENDER_DEVICES dev=CPU scene=CPU active=False cpu_enabled=True enabled=1 names=AMD EPYC"; echo "BLENDER_OK CPU cpu=AMD EPYC mean=0.6405"
else echo "BLENDER_DEVICES dev=$dev scene=GPU active=True cpu_enabled=False enabled=1 names=NVIDIA GeForce RTX 4090"; echo "BLENDER_OK $dev gpu=NVIDIA GeForce RTX 4090 mean=0.6405"; fi''')
        res, log, calls = self.run_row("blender_cycles")
        self.assertIn("verdict=PASS", res)
        self.assertEqual(calls.count("-- CUDA "), 1)
        self.assertEqual(rows.evaluate("blender_cycles", "host", log)["verdict"], gu.PASS)
        res, log, calls = self.run_row("blender_cycles", negctl=True)
        self.assertIn("-- CPU ", calls)
        self.assertNotIn("-- CUDA ", calls)
        rec = rows.evaluate("blender_cycles", "host", log)
        self.assertEqual((rec["verdict"], gu.control_verdict(rec)), (gu.FAIL, "CONTROL_OK"))

    def test_llama(self):
        self.tool("bundle/llama/llama-simple", r'''n=29; a=$(echo "$@" | sed -n 's/.*-ngl \([0-9]*\).*/\1/p'); [ "$a" -gt 29 ] && a=29
echo "ggml_cuda_init: found 1 CUDA devices"; echo "  Device 0: NVIDIA GeForce RTX 4090, compute capability 8.9"; echo "load_tensors: offloaded $a/$n layers to GPU"
echo "The sky is blue. main: decoded 64 tokens"''')
        res, log, calls = self.run_row("llama_cpp_gen")
        self.assertIn("-ngl 99", calls)
        self.assertEqual(rows.evaluate("llama_cpp_gen", "host", log)["verdict"], gu.PASS)
        res, log, calls = self.run_row("llama_cpp_gen", negctl=True)
        self.assertIn("-ngl 0", calls)
        self.assertEqual(rows.evaluate("llama_cpp_gen", "host", log)["verdict"], gu.FAIL)

    def test_llama_bench(self):
        self.tool("bundle/llama/llama-bench", r'''a=$(echo "$@" | sed -n 's/.*-ngl \([0-9]*\).*/\1/p')
echo "ggml_cuda_init: found 1 CUDA devices"; echo "  Device 0: NVIDIA GeForce RTX 4090, compute capability 8.9"
echo "| model | size | params | backend | ngl | test | t/s |"; echo "| qwen2 1.5B | 1.04 GiB | 1.78 B | CUDA | $a | pp512 | 24902.09 |"; echo "| qwen2 1.5B | 1.04 GiB | 1.78 B | CUDA | $a | tg64 | 417.12 |"''')
        self.assertEqual(rows.evaluate("llama_bench", "host", self.run_row("llama_bench")[1])["verdict"], gu.PASS)
        self.assertEqual(rows.evaluate("llama_bench", "host", self.run_row("llama_bench", True)[1])["verdict"], gu.FAIL)

    def test_hashcat_keeps_its_device_header_beyond_the_tail(self):
        self.tool("bin/hashcat", r'''gpu=1; echo "$@" | grep -q -- "-D 1" && gpu=0
if [ $gpu = 1 ]; then echo "CUDA API (CUDA 12.2)"; echo "* Device #1: NVIDIA GeForce RTX 4090, 22786/24217 MB, 128MCU"; else echo "OpenCL API (PoCL)"; echo "* Device #1: cpu-haswell-AMD EPYC 7B13, 1024/2048 MB, 6MCU"; fi
for i in $(seq 1 60); do echo "progress line $i"; done
echo "e4726719b68b205913167f0975d977ee:kayfab"; echo "Status...........: Cracked"''')
        res, log, calls = self.run_row("hashcat")
        self.assertIn("verdict=PASS", res)
        self.assertNotIn("-D 1", calls)
        self.assertIn("* Device #1: NVIDIA", log)
        self.assertEqual(rows.evaluate("hashcat", "host", log)["verdict"], gu.PASS)
        res, log, calls = self.run_row("hashcat", True)
        self.assertIn("-D 1", calls)
        self.assertEqual(rows.evaluate("hashcat", "host", log)["verdict"], gu.FAIL)

    def ffmpeg(self):
        self.tool("bin/ffmpeg", r'''out="${@: -1}"; enc=$(echo "$@" | sed -n 's/.*-c:v \([a-z0-9_]*\).*/\1/p')
case "$*" in *"-f null"*) echo "[h264 @ 0x1] Reinit context to 1280x720, pix_fmt: $(echo "$*" | grep -q 'hwaccel none' && echo yuv420p || echo cuda)"; echo "frame=  600 fps=0.0"; exit 0;; esac
echo "$enc" > "$out"; echo "frame=  600 fps=186 q=17.0"''')
        self.tool("bin/ffprobe", r'''f="${@: -1}"
case "$*" in *stream_tags=encoder*) echo "Lavc60.31.102 $(cat "$f")";; *) echo 600;; esac''')

    def ffmpeg44(self):
        """What the Ubuntu 22.04 package (ffmpeg 4.4.2, the box host) prints, MEASURED 2026-11-17 on a Vast RTX 3090 box (traces/open_rows_guest_20261117/): the mp4 has NO
        `encoder` stream tag, the encoder is named only in the (indented) stream-mapping line, and the h264 decoder prints a `Reinit context` line with the SOFTWARE
        format first and a second one with `pix_fmt: cuda` when the hwaccel took over."""
        self.tool("bin/ffmpeg", r'''out="${@: -1}"; enc=$(echo "$@" | sed -n 's/.*-c:v \([a-z0-9_]*\).*/\1/p')
case "$*" in *"-f null"*) echo "[h264 @ 0x1] Reinit context to 1280x720, pix_fmt: yuv420p"; echo "Stream mapping:"; echo "  Stream #0:0 -> #0:0 (h264 (native) -> wrapped_avframe (native))"
  echo "$*" | grep -q 'hwaccel none' || { echo "[h264 @ 0x2] format supported: yes, max_mb_count: 65536"; echo "[h264 @ 0x2] Reinit context to 1280x720, pix_fmt: cuda"; }
  echo "frame=  600 fps=0.0"; exit 0;; esac
echo "$enc" > "$out"; echo "Input #0, lavfi, from 'testsrc=size=1280x720:rate=30:duration=20':"; echo "Stream mapping:"; echo "  Stream #0:0 -> #0:0 (wrapped_avframe (native) -> h264 ($enc))"
echo "Press [q] to stop, [?] for help"; echo "frame=  600 fps=398 q=12.0 Lsize=     890kB time=00:00:19.86"''')
        self.tool("bin/ffprobe", r'''case "$*" in *stream_tags=encoder*) echo "";; *) echo 600;; esac''')

    def test_nvenc_on_ffmpeg_4_4_is_proved_by_the_stream_mapping_line(self):
        self.ffmpeg44()
        res, log, calls = self.run_row("nvenc_h264")
        self.assertIn("verdict=PASS", res)
        self.assertIn("Stream #0:0 -> #0:0 (wrapped_avframe (native) -> h264 (h264_nvenc))", log)     # the row's own `grep -v "^  "` used to drop it
        self.assertEqual(rows.evaluate("nvenc_h264", "host", log)["app"]["ok"], True)
        res, log, calls = self.run_row("nvenc_h264", True)
        self.assertEqual(rows.evaluate("nvenc_h264", "host", log)["verdict"], gu.FAIL)

    def test_nvdec_on_ffmpeg_4_4_keeps_the_second_reinit_line(self):
        self.ffmpeg44()
        res, log, calls = self.run_row("nvdec_h264")
        self.assertIn("pix_fmt: cuda", log)                                   # `grep -m1` kept only the first (software) line and the proof called a GPU decode CPU
        self.assertEqual(rows.evaluate("nvdec_h264", "host", log)["app"]["ok"], True)
        res, log, calls = self.run_row("nvdec_h264", True)
        self.assertEqual(rows.evaluate("nvdec_h264", "host", log)["verdict"], gu.FAIL)

    def test_the_mapping_line_with_nested_parentheses_names_the_encoder(self):
        line = "  Stream #0:0 -> #0:0 (wrapped_avframe (native) -> h264 (h264_nvenc))\nENCODER_TAG \n"
        self.assertIs(gu.proof_nvenc(line, codec="h264")["ok"], True)
        self.assertIs(gu.proof_nvenc(line.replace("h264_nvenc", "libx264"), codec="h264")["ok"], False)
        self.assertIs(gu.proof_nvenc(line.replace("h264_nvenc", "hevc_nvenc"), codec="h264")["ok"], False)

    def test_nvenc_encoder_tag(self):
        self.ffmpeg()
        res, log, calls = self.run_row("nvenc_h264")
        self.assertIn("verdict=PASS", res)
        self.assertIn("-c:v h264_nvenc", calls)
        self.assertIn("ENCODER_TAG Lavc60.31.102 h264_nvenc", log)
        self.assertEqual(rows.evaluate("nvenc_h264", "host", log)["app"]["ok"], True)
        res, log, calls = self.run_row("nvenc_h264", True)
        self.assertIn("verdict=PASS", res)      # the row's own regex (600 frames) cannot tell: the judge is what catches the software encoder
        self.assertIn("-c:v libx264", calls)
        self.assertEqual(rows.evaluate("nvenc_h264", "host", log)["verdict"], gu.FAIL)

    def test_nvdec_decoder_pixel_format(self):
        self.ffmpeg()
        res, log, calls = self.run_row("nvdec_h264")
        self.assertIn("-hwaccel cuda -hwaccel_output_format cuda", calls)
        self.assertEqual(rows.evaluate("nvdec_h264", "host", log)["app"]["ok"], True)
        res, log, calls = self.run_row("nvdec_h264", True)
        self.assertIn("-hwaccel none", calls)
        self.assertEqual(rows.evaluate("nvdec_h264", "host", log)["verdict"], gu.FAIL)

    def test_framework_rows_hide_the_gpu(self):
        self.tool("venv/bin/python", r'''echo "CUDA_VISIBLE_DEVICES=[${CUDA_VISIBLE_DEVICES-unset}]" >> "$APPS_ROOT/calls.log"
echo "GPU_USE_PROOF torch device=cuda:0 name=NVIDIA GeForce RTX 4090 peak_mib=2310"; echo TORCH_CORRECT_DONE''')
        res, log, calls = self.run_row("torch_correct")
        self.assertIn("CUDA_VISIBLE_DEVICES=[unset]", calls)
        res, log, calls = self.run_row("torch_correct", True)
        self.assertIn("CUDA_VISIBLE_DEVICES=[]", calls)

    def test_clpeak_vkpeak_and_egl_switches(self):
        for tool in ("clpeak", "vkpeak", "egl_offscreen"):
            self.tool("bundle/bin/" + tool, 'echo "args=$* EGL_VENDOR=${__EGL_VENDOR_LIBRARY_FILENAMES-unset}" >> "$APPS_ROOT/calls.log"; echo CHECK ok')
        got = {app: self.run_row(app)[2] for app in ("clpeak", "vkpeak", "egl_offscreen")}
        self.assertIn("vkpeak 0", got["vkpeak"])
        self.assertIn("EGL_VENDOR=unset", got["egl_offscreen"])
        self.assertNotIn("-p 1", got["clpeak"])
        got = {app: self.run_row(app, True)[2] for app in ("clpeak", "vkpeak", "egl_offscreen")}
        self.assertIn("clpeak -p 1", got["clpeak"])
        self.assertIn("vkpeak 1", got["vkpeak"])
        self.assertIn("EGL_VENDOR=/usr/share/glvnd/egl_vendor.d/50_mesa.json", got["egl_offscreen"])


class ScriptProofs(unittest.TestCase):
    """The scripts print the proof lines the judge reads: producer and consumer are checked against each other with no Blender, torch or CuPy installed
    (the print format string is cut out of the script, filled with sample values and fed to the parser)."""

    def src(self, name):
        return (ROOT / "scripts/apps/src" / name).read_text()

    def fmt(self, name, prefix):
        m = re.search(r'print\("(%s[^"]*)" %% ' % re.escape(prefix), self.src(name))
        self.assertIsNotNone(m, "%s no longer prints %s" % (name, prefix))
        return m.group(1)

    def test_blender_script_prints_the_devices_line_before_the_render(self):
        text = self.src("blender_render.py")
        self.assertIn('print("BLENDER_DEVICES', text)
        self.assertLess(text.index('print("BLENDER_DEVICES'), text.index("bpy.ops.render.render"))

    def test_blender_devices_format_is_what_the_judge_reads(self):
        line = self.fmt("blender_render.py", "BLENDER_DEVICES") % ("CUDA", "GPU", True, False, 1, "NVIDIA GeForce RTX 4090")
        ok = "BLENDER_OK CUDA gpu=NVIDIA GeForce RTX 4090 mean=0.6405"
        self.assertIs(gu.proof_blender(line + "\n" + ok, devs=("CUDA",))["ok"], True)
        cpu = self.fmt("blender_render.py", "BLENDER_DEVICES") % ("CPU", "CPU", False, True, 1, "AMD EPYC")
        self.assertIs(gu.proof_blender(cpu + "\nBLENDER_OK CPU cpu=AMD EPYC mean=0.6", devs=("CUDA",))["ok"], False)

    def run_blender_script(self, dev):
        """blender_render.py itself, executed against a fake `bpy`: what it prints for CUDA, OPTIX and CPU is what the judge reads."""
        import io
        import runpy
        from contextlib import redirect_stdout
        from unittest import mock

        class Dev:
            def __init__(self, name, kind):
                self.name, self.type, self.use = name, kind, False
        prefs = mock.MagicMock()
        prefs.devices = [Dev("AMD EPYC 7B13 64-Core Processor", "CPU"), Dev("NVIDIA GeForce RTX 4090", "CUDA"), Dev("NVIDIA GeForce RTX 4090", "OPTIX")]
        bpy = mock.MagicMock()
        bpy.context.preferences.addons = {"cycles": mock.MagicMock(preferences=prefs)}
        bpy.data.images.load.return_value.pixels = [0.5] * 8
        argv, out = sys.argv, io.StringIO()
        sys.argv = ["blender", "--", dev, "/tmp/out.png"]
        try:
            with mock.patch.dict(sys.modules, {"bpy": bpy}), redirect_stdout(out):
                runpy.run_path(str(ROOT / "scripts/apps/src/blender_render.py"), run_name="__main__")
        finally:
            sys.argv = argv
        return out.getvalue(), prefs, bpy

    def test_blender_script_in_gpu_mode_enables_only_the_gpu_device_and_prints_what_the_judge_reads(self):
        for dev in ("CUDA", "OPTIX"):
            out, prefs, bpy = self.run_blender_script(dev)
            self.assertEqual(prefs.compute_device_type, dev)
            self.assertEqual([d.use for d in prefs.devices], [False, dev == "CUDA", dev == "OPTIX"])
            self.assertIn("BLENDER_DEVICES dev=%s scene=GPU active=True cpu_enabled=False enabled=1 names=NVIDIA GeForce RTX 4090" % dev, out)
            self.assertIs(gu.proof_blender(out, devs=(dev,))["ok"], True, out)

    def test_blender_script_in_cpu_mode_renders_on_the_cpu_and_the_judge_says_cpu(self):
        out, prefs, bpy = self.run_blender_script("CPU")
        self.assertEqual(prefs.compute_device_type, "NONE")
        self.assertEqual([d.use for d in prefs.devices], [True, False, False])
        self.assertEqual(bpy.context.scene.cycles.device, "CPU")
        self.assertIn("BLENDER_OK CPU cpu=AMD EPYC", out)
        self.assertIs(gu.proof_blender(out, devs=("CUDA",))["ok"], False)

    def test_blender_script_with_no_gpu_device_fails_instead_of_rendering_on_the_cpu(self):
        import runpy
        from unittest import mock
        prefs = mock.MagicMock()
        prefs.devices = []
        bpy = mock.MagicMock()
        bpy.context.preferences.addons = {"cycles": mock.MagicMock(preferences=prefs)}
        argv, sys.argv = sys.argv, ["blender", "--", "CUDA", "/tmp/out.png"]
        try:
            import io
            from contextlib import redirect_stdout
            with mock.patch.dict(sys.modules, {"bpy": bpy}), redirect_stdout(io.StringIO()), self.assertRaises(SystemExit) as cm:
                runpy.run_path(str(ROOT / "scripts/apps/src/blender_render.py"), run_name="__main__")
        finally:
            sys.argv = argv
        self.assertEqual(cm.exception.code, 1)

    def test_blender_script_has_a_cpu_mode_for_the_negative_control(self):
        text = self.src("blender_render.py")
        self.assertIn('cpu_mode = dev == "CPU"', text)
        self.assertIn('"CPU" if cpu_mode else "GPU"', text)

    def test_the_framework_scripts_print_the_proof_the_judge_reads(self):
        for name, fw in (("torch_correct.py", "torch"), ("ai_bench.py", "torch"), ("hf_generate.py", "torch"), ("cupy_check.py", "cupy")):
            with self.subTest(script=name):
                f = self.fmt(name, "GPU_USE_PROOF " + fw)
                line = f % (("cuda:0", "NVIDIA GeForce RTX 4090", 4000) if f.count("%s") == 2 else (0, "NVIDIA GeForce RTX 4090", 4000)) if "device=%s" in f \
                    else f % (0, "NVIDIA GeForce RTX 4090", 4000)
                self.assertIs(gu.proof_framework(line, framework=fw, min_mib=1)["ok"], True, line)

    def test_the_framework_proof_is_printed_after_the_work_not_before(self):
        self.assertGreater(self.src("torch_correct.py").index("GPU_USE_PROOF"), self.src("torch_correct.py").index("multi_stream"))
        self.assertGreater(self.src("cupy_check.py").index("GPU_USE_PROOF"), self.src("cupy_check.py").index("cupy_random"))

    def test_the_nvenc_and_hashcat_rows_keep_what_the_judge_reads(self):
        text = (ROOT / "scripts/apps/run_apps.sh").read_text()
        self.assertEqual(text.count("ENCODER_TAG"), 2)
        self.assertIn("tee $O/hashcat_full.txt | tail -30; grep -a -E '^(\\* Device #", text)
        self.assertIn("Reinit context", text)

    def test_the_negative_control_block_switches_every_class_b_row(self):
        text = (ROOT / "scripts/apps/run_apps.sh").read_text()
        block = text[text.index('if [ "${KF_NEGCTL:-0}" = 1 ]'):text.index("S=$B/samples") + 2000].split("\nfi\n")[0]
        for knob in ("KF_BLENDER_DEVS=CPU", "KF_NGL=0", "KF_HASHCAT_ARGS", "KF_CLPEAK_ARGS", "KF_VKPEAK_DEV", "KF_DEC_ARGS", "KF_ENC=libx264", "KF_EGL_ENV", "KF_GPU_ENV"):
            self.assertIn(knob, block)
            self.assertGreaterEqual(text.count(knob.split("=")[0]), 2, knob + " is set but no row command reads it")


class HookWithAFakeGuest(unittest.TestCase):
    """apps_hook.sh itself, in a copy of the tree, with a fake guest transport (scripts/bench/gssh_nv) that returns canned output and appends a kf3 window to
    the QEMU log while the app 'runs': what the hook records in guest.res and negctl.res is what the real one would record for the same inputs."""

    FAKE_G = r'''#!/bin/bash
case "$1" in
  true|"test -d "*) exit 0;;
  "sudo dmesg | wc -l") echo 0;;
  *"run_apps.sh guest"*) cat "$KF_FAKE_WINDOW" >> "$KF_FAKE_QLOG"; echo "$KF_FAKE_APPRES";;
  "sudo cat /opt/apps/out/guest/"*) cat "$KF_FAKE_APPLOG";;
  "sudo dmesg | tail"*) ;;
  nvidia-smi*) echo "NVIDIA GeForce RTX 4090, 580.65.06, Disabled";;
  *) cat > /dev/null 2>&1 < /dev/stdin || true;;
esac
'''

    def run_hook(self, app, applog, window, negctl=False, base=True, smi=False):
        tmp = tempfile.TemporaryDirectory()
        self.addCleanup(tmp.cleanup)
        r = Path(tmp.name)
        for rel in ("scripts/apps", "scripts/bench/windows", "scripts/apps/src"):
            (r / rel).mkdir(parents=True, exist_ok=True)
        for rel in ("scripts/apps/apps_hook.sh", "scripts/apps/run_apps.sh", "scripts/apps/gpu_use_rows.py", "scripts/bench/gpu_use.py", "scripts/bench/windows/gpu_assert.py"):
            (r / rel).write_text((ROOT / rel).read_text())
        g = r / "scripts/bench/gssh_nv"
        g.write_text(self.FAKE_G)
        g.chmod(0o755)
        bench = r / "bench"
        bench.mkdir()
        qlog = bench / "run_T_qemu.log"
        qlog.write_text((fx("kf3_base.txt") if base else "kf3: booting\n"))
        out = r / "out"
        env = dict(os.environ, APPS=app, APPS_OUT=str(out), BENCH_DIR=str(bench), APPS_KF3_SETTLE="0", APPS_WEDGE_PROBE="0", APPS_GUEST_DRM="0",
                   KF_FAKE_WINDOW=str(FIX / window), KF_FAKE_QLOG=str(qlog), KF_FAKE_APPLOG=str(FIX / applog), KF_NEGCTL="1" if negctl else "0",
                   KF_FAKE_APPRES="APPRES side=guest app=%s verdict=PASS rc=0 secs=3 quiet=0 note=-" % app)
        if smi:
            fake = r / "bin" / "nvidia-smi"
            fake.parent.mkdir()
            fake.write_text("#!/bin/sh\ni=0; while [ $i -lt 300 ]; do echo '2026/11/17 00:00:00.000, 5, 0, 300, 0, 0'; i=$((i+1)); sleep 0.05; done\n")
            fake.chmod(0o755)
            env["PATH"] = str(fake.parent) + os.pathsep + env["PATH"]
        p = subprocess.run(["bash", str(r / "scripts/apps/apps_hook.sh"), "T"], capture_output=True, text=True, stdin=subprocess.DEVNULL, timeout=120, env=env)
        self.assertIn("APPS_HOOK_DONE", p.stdout, p.stdout[-400:] + p.stderr[-400:])
        read = lambda n: (out / n).read_text() if (out / n).exists() else ""
        return {"guest": read("guest.res"), "negctl": read("negctl.res"), "base": read(app + ".kf3.base"), "dir": out, "stdout": p.stdout}

    def test_a_gpu_run_is_recorded_as_a_pass_with_the_host_fields(self):
        got = self.run_hook("egl_offscreen", "egl_gpu.log", "kf3_window_gpu.log")
        self.assertIn("verdict=PASS", got["guest"])
        self.assertRegex(got["guest"], r"gpu_use=PASS gpu_auth=host gpu_subs=411 gpu_births=6 boot=T")
        self.assertIn("kf3: family=", got["base"])
        self.assertTrue((got["dir"] / "egl_offscreen.gpu_use.json").exists())

    def test_a_guest_that_prints_a_gpu_log_while_the_host_saw_nothing_is_recorded_as_a_fail(self):
        got = self.run_hook("egl_offscreen", "egl_gpu.log", "kf3_window_idle.log")
        self.assertIn("verdict=FAIL", got["guest"])
        self.assertIn("gpu-use:FAIL:[host-census] FAIL: host census: no GPU work in the window", got["guest"])

    def test_a_cpu_log_with_host_work_is_recorded_as_a_fail(self):
        got = self.run_hook("egl_offscreen", "egl_cpu.log", "kf3_window_gpu.log")
        self.assertIn("verdict=FAIL", got["guest"])
        self.assertIn("not an NVIDIA GPU", got["guest"])

    def test_a_missing_baseline_is_not_run_and_never_a_pass(self):
        got = self.run_hook("egl_offscreen", "egl_gpu.log", "kf3_window_idle.log", base=False)
        self.assertIn("verdict=NOTRUN", got["guest"])

    def test_the_negative_control_writes_a_control_line_and_leaves_the_row_line_alone(self):
        got = self.run_hook("egl_offscreen", "egl_cpu.log", "kf3_window_idle.log", negctl=True)
        self.assertRegex(got["negctl"], r"^NEGCTL app=egl_offscreen control=CONTROL_OK judged=FAIL")
        self.assertIn("verdict=PASS", got["guest"])
        self.assertNotIn("gpu_use=", got["guest"])
        got = self.run_hook("egl_offscreen", "egl_gpu.log", "kf3_window_gpu.log", negctl=True)
        self.assertIn("control=CONTROL_BROKEN judged=PASS", got["negctl"])

    def test_the_box_nvidia_smi_is_sampled_around_the_app(self):
        got = self.run_hook("egl_offscreen", "egl_gpu.log", "kf3_window_gpu.log", smi=True)
        self.assertTrue((got["dir"] / "egl_offscreen.smi" / "host_query.csv").exists())


class WindowsRows(unittest.TestCase):
    """The Windows matrix rows the audit of 2026-11-17 found matched by a CPU run: llama.cpp (regexes that also match `offloaded 0/29 layers to GPU`, an error
    text with `CUDA`, llvmpipe next to the word `Vulkan`) and the CUDA equivalence script's CPU switch. Rows are read from the generator, not from apps.json."""

    AM = ROOT / "scripts/bench/windows/appmatrix"

    @classmethod
    def modules(cls):
        sys.path.insert(0, str(cls.AM))
        try:
            import importlib
            gen = importlib.import_module("gen_apps")
            ver = importlib.import_module("verdict")
        finally:
            sys.path.remove(str(cls.AM))
        return gen, ver

    def decide(self, app_id, log):
        gen, ver = self.modules()
        app = {a["id"]: a for a in gen.build()["apps"]}[app_id]
        facts = dict(rc=0, secs=3, quiet=0, timed_out=False, pdh=dict(nv={}, other={}), smi=dict(util_max=0, mem_used_max_mb=0, mem_used_base_mb=0),
                     events=dict(nvlddmkm_153=0, display_4101=0, wer_1001=0))
        return ver.decide(dict(app, rx=r"^DONE"), facts, log + "\nDONE\n")["verdict"]

    def test_llama_cuda_gen_needs_every_layer_offloaded(self):
        self.assertEqual(self.decide("llama_cuda_gen", "load_tensors: offloaded 29/29 layers to GPU"), "PASS")
        for bad in ("load_tensors: offloaded 0/29 layers to GPU", "load_tensors: offloaded 12/29 layers to GPU", "CUDA0: NVIDIA GeForce RTX 4090", "Device 0: NVIDIA"):
            self.assertEqual(self.decide("llama_cuda_gen", bad), "FAIL", bad)

    def test_llama_cuda_bench_needs_the_cuda_backend_with_layers_offloaded(self):
        self.assertEqual(self.decide("llama_cuda_bench", "| qwen2 1.5B | 1.04 GiB | 1.78 B | CUDA       |  99 |            tg64 |  417.12 |"), "PASS")
        for bad in ("| qwen2 1.5B | 1.04 GiB | 1.78 B | CUDA       |   0 |            tg64 |  417.12 |", "ggml_cuda_init: failed to initialize CUDA: no device",
                    "| qwen2 1.5B | 1.04 GiB | 1.78 B | CPU        |  99 |            tg64 |  17.12 |"):
            self.assertEqual(self.decide("llama_cuda_bench", bad), "FAIL", bad)

    def test_llama_vulkan_bench_needs_an_nvidia_vulkan_device_not_just_the_word_vulkan(self):
        self.assertEqual(self.decide("llama_vulkan_bench", "ggml_vulkan: 0 = NVIDIA GeForce RTX 4090 (NVIDIA) | uma: 0 | fp16: 1"), "PASS")
        for bad in ("ggml_vulkan: Found 1 Vulkan devices:\nggml_vulkan: 0 = llvmpipe (LLVM 15.0.7, 256 bits) (llvmpipe) | uma: 0", "Vulkan"):
            self.assertEqual(self.decide("llama_vulkan_bench", bad), "FAIL", bad)

    def test_the_cuda_equivalence_script_refuses_a_cpu_run_without_the_second_switch(self):
        with tempfile.TemporaryDirectory() as d:
            with open(os.path.join(d, "torch.py"), "w") as f:
                f.write("class Generator:\n    def manual_seed(self, s):\n        return self\n")
            env = dict(os.environ, KF_EQ_DEVICE="cpu", PYTHONPATH=d)
            env.pop("KF_EQ_ALLOW_CPU", None)
            p = subprocess.run([sys.executable, "-B", str(self.AM / "guest/py/win_cuda_equiv.py"), "list"], capture_output=True, text=True, stdin=subprocess.DEVNULL,
                               env=env, timeout=60)
            self.assertEqual(p.returncode, 3, p.stderr)
            self.assertIn("refused", p.stderr)

    def test_the_llama_rows_declare_the_exact_proofs(self):
        gen, _ = self.modules()
        proofs = {a["id"]: a["proof"][0] for a in gen.build()["apps"] if a["id"].startswith("llama_")}
        self.assertEqual(proofs["llama_cuda_gen"], r"out:offloaded ([1-9][0-9]*)/\1 layers to GPU")
        self.assertEqual(proofs["llama_cuda_bench"], r"out:\| CUDA +\| +[1-9][0-9]* \|")
        self.assertEqual(proofs["llama_vulkan_bench"], r"out:ggml_vulkan: [0-9]+ = NVIDIA")

    def test_the_script_keeps_its_cpu_logic_test_behind_the_explicit_switch(self):
        text = (self.AM / "guest/py/win_cuda_equiv.py").read_text()
        self.assertIn('os.environ.get("KF_EQ_ALLOW_CPU") != "1"', text)


class Wiring(unittest.TestCase):
    """Text pins on the glue (the shell cannot run here: it needs a guest). Each pin names the line that carries the rule."""

    hook = (ROOT / "scripts/apps/apps_hook.sh").read_text()
    matrix = (ROOT / "scripts/apps/apps_matrix.sh").read_text()

    def test_the_hook_samples_around_the_app_and_keeps_the_baseline_line(self):
        self.assertIn('gpu_use_rows.py', self.hook)
        self.assertRegex(self.hook, r'sample --out "\$OUT" --app "\$app" -- "\$G"')
        self.assertIn("kf3.base", self.hook)
        self.assertRegex(self.hook, r"head -n \"\$q0\" \"\$QLOG\".*kf3: family=")

    def test_the_hook_hands_the_baseline_and_the_sampler_files_to_the_judge(self):
        self.assertIn('--kf3-base "$OUT/$app.kf3.base"', self.hook)
        self.assertIn('--smi-dir "$OUT/$app.smi"', self.hook)
        self.assertIn('--kf3-log "$OUT/$app.kf3.log"', self.hook)

    def test_the_settle_default_is_longer_than_the_two_second_status_period(self):
        m = re.search(r"APPS_KF3_SETTLE:-(\d+)", self.hook)
        self.assertIsNotNone(m)
        self.assertGreaterEqual(int(m.group(1)), 3)

    def test_the_hook_waits_for_the_last_status_line_before_slicing(self):
        self.assertLess(self.hook.index("APPS_KF3_SETTLE"), self.hook.index('> "$OUT/$app.kf3.log"'))

    def test_the_hook_rewrites_the_line_before_it_is_recorded_and_a_failed_judge_never_blanks_it(self):
        self.assertLess(self.hook.index(" annotate "), self.hook.index('tee -a "$OUT/guest.res"'))
        self.assertIn('[ -n "$new" ] && line=$new', self.hook)

    def test_the_negative_control_writes_its_own_file_and_leaves_guest_res_unannotated(self):
        self.assertIn('--negctl | tee -a "$OUT/negctl.res"', self.hook)
        self.assertIn("KF_NEGCTL=${KF_NEGCTL:-0}", self.hook)

    def test_the_host_baseline_is_annotated_after_the_run(self):
        self.assertIn("annotate-res --side host", self.matrix)
        self.assertIn("host_negctl.res", self.matrix)

    def test_the_host_annotation_follows_the_run_and_the_unlock(self):
        self.assertLess(self.matrix.index('run_apps.sh" host'), self.matrix.index("annotate-res --side host"))
        self.assertLess(self.matrix.index("flock -u 9"), self.matrix.index("annotate-res --side host"))

    def test_the_shell_parses(self):
        for f in ("run_apps.sh", "apps_hook.sh", "apps_matrix.sh"):
            self.assertEqual(subprocess.run(["bash", "-n", str(ROOT / "scripts/apps" / f)], capture_output=True).returncode, 0, f)


class SmiJudge(unittest.TestCase):
    def test_no_sample_at_all_is_unreadable_and_not_run(self):
        rec = gu.smi_record("", "", ("gpu", "mem"))
        self.assertTrue(rec["unreadable"])
        self.assertEqual(gu.judge_smi(rec, ("gpu", "mem"))[0], gu.NOT_RUN)

    def test_nothing_required_is_a_pass_and_an_idle_engine_is_a_fail(self):
        self.assertEqual(gu.judge_smi(None, ())[0], gu.PASS)
        self.assertEqual(gu.judge_smi(smi_idle(), ("gpu", "mem"))[0], gu.FAIL)
        self.assertEqual(gu.judge_smi(smi_busy(), ("gpu", "mem"))[0], gu.PASS)

    def test_the_sampler_reads_a_na_utilisation_as_unknown_not_busy(self):
        q = "\n".join("2026/11/17 00:00:%02d.000, [N/A], 0, 300, 0, 0" % i for i in range(6))
        self.assertEqual(gu.judge_smi(gu.smi_record(q, "", ("gpu",)), ("gpu",))[0], gu.FAIL)


class MatrixTable(unittest.TestCase):
    def test_a_guest_notrun_from_the_gpu_use_judge_is_unattributed_not_a_defect(self):
        mt = load("matrix_table_t", "scripts/apps/matrix_table.py")
        cls = mt.classify("hashcat", {"verdict": "PASS"}, {"verdict": "NOTRUN", "note": "gpu-use:NOT_RUN:host census"}, {"verdict": "NOTRUN", "note": "gpu-use:NOT_RUN:host census"})
        self.assertEqual(cls[0], "UNATTRIBUTED")
        cls = mt.classify("hashcat", {"verdict": "PASS"}, {"verdict": "FAIL", "note": "gpu-use:FAIL:no work"}, {"verdict": "FAIL", "note": "gpu-use:FAIL:no work"})
        self.assertEqual(cls[0], "KAYFABE_DEFECT")      # the host used the GPU, the guest did not: a kayfabe defect, never hidden

    def test_an_isolated_gpu_use_notrun_after_a_batched_fail_is_still_unattributed(self):
        mt = load("matrix_table_t2", "scripts/apps/matrix_table.py")
        cls = mt.classify("hashcat", {"verdict": "PASS"}, {"verdict": "FAIL", "note": "x"}, {"verdict": "NOTRUN", "note": "gpu-use:NOT_RUN:no baseline"})
        self.assertEqual(cls[0], "UNATTRIBUTED")


if __name__ == "__main__":
    unittest.main()
