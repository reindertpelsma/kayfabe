#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later
"""GPU-free fixtures for the Resolve GPU lane: the GPU-use assertion, the media readers, the FCPXML generator and the
asset manifest. No VM, network or third-party media (the PNG and MP4 headers are built here)."""

import importlib.util
import json
import os
import struct
import sys
import tempfile
import unittest
import zlib
from pathlib import Path
from xml.dom import minidom

WIN = Path(__file__).resolve().parents[2] / "scripts/bench/windows"


def load(name):
    spec = importlib.util.spec_from_file_location("kf_" + name, WIN / (name + ".py"))
    mod = importlib.util.module_from_spec(spec)
    sys.modules["kf_" + name] = mod
    spec.loader.exec_module(mod)
    return mod


G = load("gpu_use_assert")
M = load("resolve_media")
RG = load("resolve_gpu")
FIX = Path(__file__).resolve().parent / "fixtures/resolve_gpu"


def csv(rows, t0="2026/11/16 03:00:00"):
    out = []
    for i, (gpu, mem, enc, dec, used) in enumerate(rows):
        sec = i // 2
        out.append(f"2026/11/16 03:00:{sec:02d}.{(i % 2) * 5}00, {gpu}, {mem}, {enc}, {dec}, {used}")
    return "\n".join(out)


class Assertion(unittest.TestCase):
    def need(self, **kw):
        return G.Need(**kw)

    def test_busy_render_passes(self):
        rows = [(2, 1, 0, 0, 900)] * 4 + [(80, 40, 30, 0, 2500)] * 20 + [(3, 1, 0, 0, 950)] * 2
        s = G.parse_nvidia_smi_csv(csv(rows))
        t0 = s[4].t - 0.1
        v = G.assert_gpu_used(s, self.need(encoder=True), (t0, s[23].t + 0.1))
        self.assertTrue(v["ok"], v["reasons"])
        self.assertEqual(v["summary"]["gpu"]["peak"], 80)
        self.assertEqual(v["summary"]["mem_used"]["growth"], 1600)

    def test_idle_gpu_fails_every_way(self):
        s = G.parse_nvidia_smi_csv(csv([(1, 0, 0, 0, 900)] * 24))
        v = G.assert_gpu_used(s, self.need(encoder=True, decoder=True), (s[4].t, s[22].t))
        self.assertFalse(v["ok"])
        text = " ".join(v["reasons"])
        for frag in ("utilization.gpu peaked", "memory.used grew", "utilization.encoder", "utilization.decoder"):
            self.assertIn(frag, text)

    def test_na_everywhere_is_not_a_pass(self):
        rows = "\n".join(f"2026/11/16 03:00:{i:02d}.000, [N/A], [N/A], [N/A], [N/A], 900" for i in range(12))
        s = G.parse_nvidia_smi_csv(rows)
        v = G.assert_gpu_used(s, self.need(), (s[0].t, s[-1].t))
        self.assertFalse(v["ok"])
        self.assertIn("N/A in every sample", " ".join(v["reasons"]))

    def test_single_spike_is_not_enough(self):
        rows = [(1, 0, 0, 0, 900)] * 10 + [(90, 10, 0, 0, 1500)] + [(1, 0, 0, 0, 900)] * 10
        s = G.parse_nvidia_smi_csv(csv(rows))
        v = G.assert_gpu_used(s, self.need(), (s[2].t, s[-1].t))
        self.assertFalse(v["ok"])
        self.assertIn("only 1 samples", " ".join(v["reasons"]))

    def test_too_few_samples(self):
        s = G.parse_nvidia_smi_csv(csv([(90, 10, 0, 0, 1500)] * 3))
        self.assertFalse(G.assert_gpu_used(s, self.need(), (s[0].t, s[-1].t))["ok"])
        self.assertFalse(G.assert_gpu_used([], self.need())["ok"])

    def test_garbage_lines_skipped(self):
        s = G.parse_nvidia_smi_csv("junk\n2026/11/16 03:00:00.100, 5, 1, 0, 0, 900\n2026/11/16 03:00:00, x\n")
        self.assertEqual(len(s), 1)

    def test_utc_offset(self):
        a = G.parse_nvidia_smi_csv("2026/11/16 03:00:00.000, 5, 1, 0, 0, 900")[0].t
        b = G.parse_nvidia_smi_csv("2026/11/16 03:00:00.000, 5, 1, 0, 0, 900", utc_offset_s=3600)[0].t
        self.assertEqual(a - b, 3600)

    def test_run_with_sampling_judges_and_stops(self):
        class S:
            stopped = False
            def start(self): pass
            def stop(self):
                self.stopped = True
                return csv([(50, 5, 0, 0, 1500)] * 30)
        clock = iter([1_795_000_000.0 + 3.0, 1_795_000_000.0 + 12.0])
        base = G._epoch("2026/11/16 03:00:00")
        times = iter([base + 3, base + 12])
        s = S()
        v, samples = G.run_with_sampling(lambda: "done", s, G.Need(mem_growth_mib=None), settle_s=0, clock=lambda: next(times), sleep=lambda x: None)
        self.assertTrue(s.stopped)
        self.assertTrue(v["ok"], v["reasons"])
        s2 = S()
        times = iter([base + 3, base + 12])
        def boom(): raise RuntimeError("x")
        v, _ = G.run_with_sampling(boom, s2, G.Need(mem_growth_mib=None), settle_s=0, clock=lambda: next(times), sleep=lambda x: None)
        self.assertFalse(v["ok"])
        self.assertTrue(s2.stopped)

    def test_guest_fragments(self):
        self.assertIn("utilization.encoder", G.guest_sampler_start(r"C:\kfapps\x\smi.csv"))
        with self.assertRaises(ValueError):
            G.guest_sampler_start("C:\\a'b")


def png(w, h, fn):
    raw = b"".join(b"\x00" + bytes(v for x in range(w) for v in fn(x, y)) for y in range(h))
    def chunk(t, d):
        return struct.pack(">I", len(d)) + t + d + struct.pack(">I", zlib.crc32(t + d))
    return b"\x89PNG\r\n\x1a\n" + chunk(b"IHDR", struct.pack(">IIBBBBB", w, h, 8, 2, 0, 0, 0)) + chunk(b"IDAT", zlib.compress(raw)) + chunk(b"IEND", b"")


def box(t, body):
    return struct.pack(">I4s", 8 + len(body), t) + body


def mp4(codec=b"avc1", w=1920, h=1080, n=48, ts=24):
    mdhd = box(b"mdhd", bytes(4) + bytes(8) + struct.pack(">II", ts, n) + bytes(4))
    hdlr = box(b"hdlr", bytes(8) + b"vide" + bytes(12))
    entry = struct.pack(">I", 86) + codec + bytes(6) + struct.pack(">H", 1) + bytes(16) + struct.pack(">HH", w, h) + bytes(86 - 8 - 2 - 6 - 16 - 4 - 8 + 8)
    stsd = box(b"stsd", bytes(4) + struct.pack(">I", 1) + entry)
    stts = box(b"stts", bytes(4) + struct.pack(">I", 1) + struct.pack(">II", n, 1))
    stbl = box(b"stbl", stsd + stts)
    minf = box(b"minf", stbl)
    mdia = box(b"mdia", mdhd + hdlr + minf)
    return box(b"ftyp", b"isom" + bytes(4)) + box(b"moov", box(b"trak", mdia))


class Media(unittest.TestCase):
    def test_png_and_flat_detection(self):
        with tempfile.TemporaryDirectory() as d:
            p = Path(d) / "a.png"
            p.write_bytes(png(64, 48, lambda x, y: (x * 4 % 256, y * 5 % 256, 90)))
            i = M.png_info(p)
            self.assertEqual((i["width"], i["height"]), (64, 48))
            self.assertGreater(i["luma_std"], 10)
            q = Path(d) / "b.png"
            q.write_bytes(png(64, 48, lambda x, y: (0, 0, 0)))
            j = M.png_info(q)
            self.assertEqual(j["luma_mean"], 0)
            self.assertNotEqual(i["pixel_sha256"], j["pixel_sha256"])

    def test_isobmff_reader(self):
        with tempfile.TemporaryDirectory() as d:
            p = Path(d) / "a.mp4"
            p.write_bytes(mp4())
            i = M.probe_isobmff(p)
            self.assertEqual((i["codec"], i["frames"], i["width"], i["height"]), ("avc1", 48, 1920, 1080))
            self.assertAlmostEqual(i["duration"], 2.0)
            p.write_bytes(mp4()[:40])
            with self.assertRaises(M.MediaError):
                M.probe_isobmff(p)
            p.write_bytes(b"")
            with self.assertRaises(M.MediaError):
                M.probe(p)

    def test_fcpxml_is_wellformed(self):
        x = M.build_fcpxml([
            {"name": "a & b", "path": "C:/kfmedia/a b.mp4", "frames": 48, "src_frames": 100, "fps": 24, "speed": 0.5, "scale": 2.0},
            {"name": "s", "path": "C:/kfmedia/s.jpg", "frames": 24},
        ])
        doc = minidom.parseString(x)
        self.assertEqual(len(doc.getElementsByTagName("asset-clip")), 2)
        self.assertIn("a%20b.mp4", x)
        self.assertEqual(doc.getElementsByTagName("sequence")[0].getAttribute("duration"), "7200/2400s")

    def test_manifest_is_pinned_and_open(self):
        a = M.load_manifest()
        self.assertGreaterEqual(len(a), 3)
        for x in a:
            self.assertIn(x["license"], M.LICENSES)
            self.assertNotIn("NC", x["license"])
        bad = json.loads(M.MANIFEST.read_text())
        bad["assets"][0]["license"] = "CC-BY-NC-4.0"
        with tempfile.TemporaryDirectory() as d:
            p = Path(d) / "m.json"
            p.write_text(json.dumps(bad))
            with self.assertRaises(M.MediaError):
                M.load_manifest(p)
            bad["assets"][0]["license"] = "CC0-1.0"
            bad["assets"][0]["sha256"] = None
            p.write_text(json.dumps(bad))
            with self.assertRaises(M.MediaError):
                M.load_manifest(p)

    def test_fetch_refuses_wrong_hash(self):
        import io
        class R(io.BytesIO):
            def geturl(self): return "https://x/"
            def __enter__(self): return self
            def __exit__(self, *a): pass
        with tempfile.TemporaryDirectory() as d:
            with self.assertRaises(M.MediaError):
                M.fetch_assets(Path(d), opener=lambda req, timeout: R(b"x"))



LOG = (FIX / "resolve_log_lines.txt").read_text().splitlines()


def stamp(t):
    import datetime
    return datetime.datetime.fromtimestamp(t, datetime.timezone.utc).strftime("%Y/%m/%d %H:%M:%S.") + f"{int((t % 1) * 1000):03d}"


class Lane(unittest.TestCase):
    def test_plan_is_bounded_and_scripts_parse(self):
        plan = RG.build_plan("a" * 32)
        self.assertEqual([r["id"] for r in plan["renders"]], list(RG.RENDER_ORDER))
        self.assertEqual(RG.RENDER_ORDER[-1], "hevc")  # slowest last: a software HEVC stall cannot hide the other four
        with self.assertRaises(ValueError):
            RG.build_plan("x';calc")
        body = RG.guest_body(plan)
        self.assertIn(plan["project"], body)
        self.assertNotRegex(body, r"(?i)password|Invoke-WebRequest|Invoke-RestMethod")
        self.assertEqual(body.count("'@"), body.count("@'"))
        self.assertEqual(body.count("param($out,$stop)"), 1)  # the sampler text lives only in its here-string (a comment once swallowed it)
        self.assertNotIn("__SAMPLER__", body)
        import shutil, subprocess
        pwsh = os.environ.get("KF_PWSH") or shutil.which("pwsh")
        if pwsh:
            f = Path(tempfile.mkdtemp()) / "x.ps1"
            f.write_text(body)
            chk = f.with_name("parse.ps1")
            chk.write_text("param($f)\n$e=$null;$t=$null\n[void][System.Management.Automation.Language.Parser]::ParseFile($f,[ref]$t,[ref]$e)\n$e | ForEach-Object { $_.Message }\nexit $e.Count\n")
            done = subprocess.run([pwsh, "-NoProfile", "-File", str(chk), str(f)], capture_output=True, text=True, timeout=60)
            self.assertEqual(done.returncode, 0, done.stdout)

    def test_discovery_plan_and_not_offered_is_not_a_kf3_failure(self):
        plan = RG.build_plan("b" * 32, lane="discovery")
        self.assertEqual([r["id"] for r in plan["renders"]], list(RG.DISCOVERY_ORDER))
        self.assertEqual(plan["lane"], "discovery")
        with self.assertRaises(ValueError):
            RG.build_plan("b" * 32, lane="gold")
        body = RG.guest_body(plan)
        self.assertIn("Encoding Profile", body)
        result = {"kind": "RESOLVE_GPU", "lane": "discovery", "t0": 1.0, "stage": "done", "log_lines": LOG, "steps": [
            {"n": n, "ok": True} for n in ("project manager", "main window", "deliver page", "timeline on deliver page")],
            "renders": [{"id": "av1", "fmt_ok": True, "codec_ok": False, "fmt_value": "MP4", "codec_value": "H.264"}]}
        row = RG.grade(result, Path(tempfile.mkdtemp()), "", None, 0.0)
        self.assertEqual(row["renders"]["av1"]["status"], "NOT_OFFERED")
        self.assertTrue(any("not offered" in f for f in row["findings"]))
        self.assertEqual(row["lane"], "discovery")

    def test_timeline_clips_and_fcpxml(self):
        x = RG.fcpxml_text()
        doc = minidom.parseString(x)
        self.assertEqual(len(doc.getElementsByTagName("asset-clip")), 4)
        self.assertEqual(doc.getElementsByTagName("sequence")[0].getAttribute("duration"), f"{RG.FRAMES_TOTAL * 100}/2400s")

    def test_log_judgement_on_measured_lines(self):
        ok, problems, facts = RG.judge_log(LOG)
        self.assertTrue(ok, problems)
        self.assertEqual(facts["compute_api"], "CUDA")
        self.assertIn("RTX 4070", facts["device"])
        self.assertTrue(facts["nvenc"] and facts["gpudetect_errors"])
        for drop, expect in (("Selected compute API", "compute API"), ("Detected 1 GPUs", "no detected GPU"), ("Let There Be CUDA", "board manager")):
            lines = [x for x in LOG if drop not in x and not (drop == "Let There Be CUDA" and "Initializing CUDA board" in x)]
            ok, problems, _ = RG.judge_log(lines)
            self.assertFalse(ok)
            self.assertIn(expect, " ".join(problems))
        cpu = [x.replace("CUDA", "CPU") if "compute API" in x else x for x in LOG]
        self.assertFalse(RG.judge_log(cpu)[0])
        nonv = [x.replace("NVIDIA", "Microsoft") for x in LOG if "Detected" not in x] + [x for x in LOG if "Detected" in x]
        self.assertFalse(RG.judge_log(nonv)[0])
        self.assertFalse(RG.judge_log(LOG + ["INFO | Rendering with CPU only"])[0])
        self.assertFalse(RG.judge_log([])[0])

    def test_engine_csv_from_the_pc_render(self):
        text = (FIX / "engine_h264_pc_20261116.csv").read_text()
        samples = RG.parse_engine_csv(text)
        self.assertGreater(len(samples), 300)
        peaks = RG.engine_peaks(text, samples[0].t, samples[-1].t)
        self.assertGreater(peaks["vdec"]["peak"], 90)
        self.assertEqual(peaks["compute"]["peak"], 0)   # MEASURED: the guest scheduler showed no compute engine while CUDA ran
        self.assertGreater(peaks["mem_pid_mb"]["max"], 3000)
        v = G.assert_gpu_used(samples, RG.GUEST_NEED, (samples[0].t, samples[-1].t))
        self.assertTrue(v["ok"], v["reasons"])
        idle = text.splitlines()[:8]
        self.assertFalse(G.assert_gpu_used(RG.parse_engine_csv("\n".join(idle)), RG.GUEST_NEED)["ok"])

    def test_smi_report_na(self):
        r = RG.smi_report("2026/11/16 03:00:00.000, [N/A], [N/A], [N/A], [N/A], 3544\n" * 3)
        self.assertEqual((r["samples"], r["utilization_numeric"]), (3, 0))


class Grade(unittest.TestCase):
    """End-to-end verdict on synthetic guest results and pulled bytes; every failure arm is a counterexample."""
    T0 = 1_794_000_000.0
    HOST0 = 1_800_000_000.0

    def setUp(self):
        self.dir = Path(tempfile.mkdtemp())
        self.saved = {k: dict(v["expect"]) for k, v in RG.RENDERS.items()}
        for k in ("h264", "dnxhr", "prores", "png", "hevc"):
            RG.RENDERS[k]["expect"]["width"], RG.RENDERS[k]["expect"]["height"] = 64, 48
        self._ffprobe = M.have_ffprobe
        M.have_ffprobe = lambda: False
        self._fs = M.frame_stats
        M.frame_stats = lambda *a, **k: {"luma_std": 20.0, "luma_mean": 90.0}
        self.result = {"kind": "RESOLVE_GPU", "t0": self.T0, "stage": "done", "grade": True, "log_lines": LOG,
                       "steps": [{"n": n, "ok": True} for n in ("project manager", "main window", "import timeline", "deliver page", "timeline on deliver page")],
                       "grade_nodes": [{"clip_x": 70, "nodes": ["01", "02", "03"]}], "renders": []}
        rows = ["epoch,iso,pid,e3d,compute,copy,vdec,venc,ofa,mem_pid_mb,mem_all_mb,e3d_all,compute_all,venc_all,vdec_all"]
        host = []
        t = 0.0
        for i, rid in enumerate(RG.RENDER_ORDER):
            a, b = 100 + i * 100, 160 + i * 100
            ext = RG.RENDERS[rid]["expect"]["ext"]
            files = []
            if RG.RENDERS[rid]["expect"]["kind"] == "movie":
                codec = {"h264": b"avc1", "dnxhr": b"AVdh", "prores": b"apch", "hevc": b"hvc1"}[rid]
                (self.dir / f"kf_{rid}{ext}").write_bytes(mp4(codec, 64, 48, RG.FRAMES_TOTAL, 24))
                files = [f"kf_{rid}{ext}|{(self.dir / f'kf_{rid}{ext}').stat().st_size}|{self.T0 + b}"]
            else:
                (self.dir / "kf_png").mkdir()
                for n in range(RG.FRAMES_TOTAL):
                    files.append(f"kf_png/kf_png_{n:08d}.png|1000|{self.T0 + b}")
                for n in (0, RG.FRAMES_TOTAL // 2, RG.FRAMES_TOTAL - 1):
                    (self.dir / f"kf_png/kf_png_{n:08d}.png").write_bytes(png(64, 48, lambda x, y, n=n: ((x * 4 + n * 7) % 256, (y * 5) % 256, 90)))
            self.result["renders"].append({"id": rid, "fmt_ok": True, "codec_ok": True, "name_ok": True, "loc_ok": True, "queued": True,
                                           "render_pressed": True, "started": True, "finished": True, "t_start": self.T0 + a - 2, "t_started": self.T0 + a,
                                           "t_end": self.T0 + b, "fmt_value": "x", "codec_value": "y", "encoder_value": "Native", "res_value": "z",
                                           "nvidia_encoder_selectable": False, "try_nvidia": False, "files": files, "queue": ["Job 1"]})
        for k in range(0, 800):
            tt = self.T0 + k
            busy = any(100 + i * 100 <= k <= 160 + i * 100 for i in range(5))
            rows.append(f"{tt:.3f},x,1,0,0,0,{60 if busy else 0},0,0,3000,3500,0,0,0,0")
        for k in range(0, 1600):
            tt = self.HOST0 + k * 0.5
            busy = any(100 + i * 100 <= k / 2 <= 160 + i * 100 for i in range(5))
            host.append(f"{stamp(tt)}, {96 if busy else 2}, 8, 0, 0, 5300")
        (self.dir / "engine.csv").write_text("\n".join(rows))
        (self.dir / "smi.csv").write_text("2026/11/16 03:00:00.000, [N/A], [N/A], [N/A], [N/A], 3544\n")
        self.host_csv = "\n".join(host)

    def tearDown(self):
        for k, v in self.saved.items():
            RG.RENDERS[k]["expect"].update(v)
        M.have_ffprobe = self._ffprobe
        M.frame_stats = self._fs

    def grade(self, host=True):
        return RG.grade(self.result, self.dir, self.host_csv if host else "", self.HOST0 if host else None, 0.0)

    def test_pass(self):
        row = self.grade()
        self.assertTrue(row["ok"], row.get("error"))
        self.assertEqual(row["status"], "PASS")
        self.assertTrue(any("N/A" in f for f in row["findings"]))
        self.assertTrue(any("NVIDIA encoder" in f for f in row["findings"]))
        self.assertIn("NOT RUN", row["other_suites"]["pugetbench"])

    def test_idle_guest_engines_fail(self):
        (self.dir / "engine.csv").write_text("\n".join(l if not l[0].isdigit() else ",".join(l.split(",")[:6] + ["0"] + l.split(",")[7:]) for l in (self.dir / "engine.csv").read_text().splitlines()))
        row = self.grade()
        self.assertFalse(row["ok"])
        self.assertIn("guest:", row["error"])

    def test_idle_host_gpu_fails(self):
        self.host_csv = "\n".join(l.replace(", 96,", ", 1,") for l in self.host_csv.splitlines())
        row = self.grade()
        self.assertFalse(row["ok"])
        self.assertIn("host:", row["error"])

    def test_no_host_samples_fails(self):
        row = self.grade(host=False)
        self.assertFalse(row["ok"])
        self.assertIn("host GPU samples missing", row["error"])

    def test_wrong_codec_size_frames(self):
        (self.dir / "kf_h264.mp4").write_bytes(mp4(b"hvc1", 64, 48, RG.FRAMES_TOTAL, 24))
        self.result["renders"][0]["files"] = [f"kf_h264.mp4|{(self.dir / 'kf_h264.mp4').stat().st_size}|1"]
        self.assertIn("codec", self.grade()["renders"]["h264"]["problems"][0])
        (self.dir / "kf_h264.mp4").write_bytes(mp4(b"avc1", 64, 48, 50, 24))
        self.result["renders"][0]["files"] = [f"kf_h264.mp4|{(self.dir / 'kf_h264.mp4').stat().st_size}|1"]
        self.assertTrue(any("frames" in p for p in self.grade()["renders"]["h264"]["problems"]))

    def test_missing_or_truncated_output(self):
        (self.dir / "kf_dnxhr.mov").unlink()
        self.assertFalse(self.grade()["ok"])
        (self.dir / "kf_prores.mov").write_bytes(b"")
        row = self.grade()
        self.assertEqual(row["renders"]["prores"]["status"], "FAIL")

    def test_sequence_count_and_identical_frames(self):
        self.result["renders"][3]["files"] = self.result["renders"][3]["files"][:50]
        self.assertTrue(any("sequence" in p for p in self.grade()["renders"]["png"]["problems"]))
        self.setUp()
        a = (self.dir / "kf_png/kf_png_00000000.png").read_bytes()
        (self.dir / f"kf_png/kf_png_{RG.FRAMES_TOTAL - 1:08d}.png").write_bytes(a)
        (self.dir / f"kf_png/kf_png_{RG.FRAMES_TOTAL // 2:08d}.png").write_bytes(a)
        self.assertTrue(any("identical" in p for p in self.grade()["renders"]["png"]["problems"]))

    def test_log_without_cuda_fails_and_missing_step_is_inconclusive(self):
        self.result["log_lines"] = [x for x in LOG if "Selected compute API" not in x]
        self.assertFalse(self.grade()["ok"])
        self.result["log_lines"] = LOG
        self.result["steps"][4]["ok"] = False
        row = self.grade()
        self.assertEqual(row["status"], "INCONCLUSIVE")

    def test_unfinished_render_stops_the_row(self):
        self.result["renders"][1]["finished"] = False
        row = self.grade()
        self.assertFalse(row["ok"])
        self.assertEqual(row["renders"]["prores"]["status"], "NOT_RUN")

    def test_guest_script_error_is_a_problem(self):
        self.result["error"] = "Deliver page did not open"
        self.assertIn("guest script error", self.grade()["error"] + " ".join(self.grade()["problems"]))


class Differential(unittest.TestCase):
    def test_census_counts_and_strips(self):
        lines = ["kf3: GSP rpc UNSERVICED Unserviced { code: 1234, sequence: 5 }"] * 3 + [
            "kf3: GSP rpc UNSERVICED Unserviced { code: 99, sequence: 6 }",
            "kf3: chan token 0x1 (host 2) RETIRED, forwarded=1 dead=Some(Fault) uuid GPU-57a1c1f8-d5e0-2e79-d42f-98f8f35ca554",
            "kf-rm: ForeignClass 0xc7b0 object refused",
            "NVRM: Xid (PCI:0000:00:07): 31, pid=1",
            "kf3: wait timed out after 5000 ms",
            "kf3: family=Ada phase=Cold ok"]
        c = RG.census(lines)
        self.assertEqual((c["unserviced"], c["unserviced_codes"]["1234"], c["foreign_class"], c["dead_channels"], c["xid"], c["tdr_or_timeout"]), (4, 3, 1, 1, 1, 1))
        text = json.dumps(c)
        self.assertNotIn("GPU-57a1", text)
        self.assertNotIn("0xc7b0", text)

    def test_census_of_a_clean_window_is_zero(self):
        c = RG.census(["kf3: family=Ada phase=Hot trapped=3"] * 10)
        self.assertEqual((c["unserviced"], c["foreign_class"], c["dead_channels"], c["xid"], c["tdr_or_timeout"]), (0, 0, 0, 0, 0))

    def test_failure_classes(self):
        self.assertIsNone(RG.classify_failure({"status": "PASS"}, None))
        self.assertEqual(RG.classify_failure({"status": "FAIL", "problems": ["codec 'x' not in"]}, None), "OUTPUT_MISMATCH")
        self.assertEqual(RG.classify_failure({"status": "FAIL", "problems": ["render end step failed"]}, None), "HANG")
        self.assertEqual(RG.classify_failure({"status": "FAIL", "problems": ["guest: utilization.gpu peaked"]}, None), "ASSERT")
        self.assertEqual(RG.classify_failure({"status": "FAIL", "problems": ["render end step failed"]}, {"xid": 2}), "XID")
        self.assertEqual(RG.classify_failure({"status": "FAIL", "problems": ["x"]}, {"tdr_or_timeout": 1, "xid": 0}), "TDR")

    def test_platform_comparison_classes(self):
        def row(*st):
            return {"renders": {k: {"status": v} for k, v in zip(("a", "b", "c", "d"), st)}}
        c = RG.compare_platforms(row("PASS", "FAIL", "FAIL", "PASS"), row("PASS", "FAIL", "PASS", "FAIL"))
        self.assertTrue(c["a"]["class"].startswith("BOTH_PASS"))
        self.assertTrue(c["b"]["class"].startswith("BOTH_FAIL"))
        self.assertTrue(c["c"]["class"].startswith("KAYFABE_DEFECT"))
        self.assertTrue(c["d"]["class"].startswith("KF3_PASS_VFIO_FAIL"))
        self.assertEqual(RG.compare_platforms(row("PASS"), None)["a"]["class"], "VFIO_CONTROL_PENDING")

    def test_lane_definitions(self):
        rows = RG.row_definitions()
        self.assertEqual([r["id"] for r in rows], ["resolve." + x for x in RG.GATE_ORDER + RG.DISCOVERY_ORDER])
        self.assertEqual({r["lane"] for r in rows if r["id"].split(".")[1] in RG.DISCOVERY_ORDER}, {"discovery"})
        self.assertTrue(all(r["lane"] in RG.LANES for r in rows))
        saved = RG.RENDERS["h264"]["expected_fail"]
        RG.RENDERS["h264"]["expected_fail"] = {"kf3": {"class": "BOGUS"}}
        try:
            with self.assertRaises(ValueError):
                RG.row_definitions()
        finally:
            RG.RENDERS["h264"]["expected_fail"] = saved

    def test_vfio_switch_rewrites_the_device_and_nothing_else(self):
        spec = importlib.util.spec_from_file_location("kf_pc_diff", WIN / "propimage_check.py")
        pc = importlib.util.module_from_spec(spec)
        sys.modules["kf_pc_diff"] = pc
        spec.loader.exec_module(pc)
        argv0 = ["qemu", "-machine", "q35", "-device", "kf3-gpu,id=kf0,fb-mb=4096", "-device", "virtio-net-pci", "-drive", "file=x"]
        pc.PI.build_argv = lambda *a, **k: list(argv0)
        with self.assertRaises(ValueError):
            pc.enable_vfio("0000:01:00.1")
        pc.enable_vfio("0000:01:00.0")
        argv = pc.PI.build_argv()
        self.assertEqual(pc.PLATFORM, "vfio")
        self.assertNotIn("kf3-gpu", " ".join(argv))
        self.assertIn("vfio-pci,host=0000:01:00.0,multifunction=on,addr=0x4.0", argv)
        self.assertIn("vfio-pci,host=0000:01:00.1,addr=0x4.1", argv)
        self.assertEqual([a for a in argv if "vfio" not in a and a != "-device"], ["qemu", "-machine", "q35", "virtio-net-pci", "-drive", "file=x"])
        self.assertEqual(pc.gpu_fix(None, "u")[0], True)


if __name__ == "__main__":
    unittest.main()
