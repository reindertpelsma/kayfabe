#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later
"""DaVinci Resolve (free) real-GPU rendering coverage for image A, with a GPU-use assertion (stdlib only).

STATUS: DESIGN-ONLY, 2026-11-16. The pure parts (plan, verdict, sample parsing, output verification) are GPU-free tested in
scripts/ci/test_resolve_gpu.py. The guest script (resolve_gpu_guest.ps1) drives the Resolve UI and has run on the RTX 4070 PC only as
far as traces/resolve_gpu_20261116/README.md records (no full run is claimed here).

What one row does (propimage_check.py app `resolve`, wired in run_check/verdict):
  1. Host: fetch the open media (resolve_assets.json: Blender CC-BY clips; sources or the public mirror, sha256 pinned), generate an FCPXML
     timeline (UHD 3840x2160, 24 fps, four 1 s clips: normal and 0.5x / 2x retimed, 1080p and 720p sources), push both into the guest.
  2. Guest (interactive session, UI Automation for discovery and read-back, measured coordinates for the rest): create a project, import the
     timeline, grade two clips (three serial nodes: contrast/saturation/boost, temperature/highlights, blur), open the Deliver page and
     render five deliverables: H.264 MP4, Avid DNxHR MOV, Apple ProRes MOV, PNG sequence, H.265 MP4 (720p, last: software HEVC is slow).
     Samplers run in the guest the whole time: WDDM per-engine counters of the Resolve process (3D, compute, copy, video decode/encode, OFA,
     process dedicated memory) and `nvidia-smi` as the guest driver reports it.
  3. Host: pull every output (movies whole, PNG sequences first/middle/last frame plus the file list), verify them with ffprobe/ffmpeg or the
     built-in readers (codec, size, frames, duration, a decoded frame that is not black or flat, sha256), sample the PHYSICAL GPU with host
     `nvidia-smi` during the task, and grade.

PASS needs ALL of: every render verified; per render the guest engine counters show GPU engine use (Need below) AND the host GPU was busy;
Resolve's own log shows the CUDA compute API selected and an NVIDIA device, not CPU only; NVENC activity where the Encoder control
accepted an NVIDIA encoder (the free edition offers only Auto/Native: recorded, never faked). A thin sampler (no samples, N/A everywhere) is a
failure with the reason named. Thresholds are arguments, never relaxed to make a row pass.
"""

from __future__ import annotations

import csv
import io
import json
import re
import time
from pathlib import Path
from typing import Optional

HERE = Path(__file__).resolve().parent

import importlib.util


def _load(name):
    spec = importlib.util.spec_from_file_location("kf_rg_" + name, HERE / (name + ".py"))
    mod = importlib.util.module_from_spec(spec)
    import sys as _s
    _s.modules["kf_rg_" + name] = mod
    spec.loader.exec_module(mod)
    return mod


G = _load("gpu_use_assert")
M = _load("resolve_media")

GUEST_MEDIA = r"C:\kfmedia"
FPS = 24
CLIP_FRAMES = 24
TIMELINE = "kf_gpu"
FRAMES_TOTAL = 4 * CLIP_FRAMES
SOURCES = {  # name -> (guest file, source frame rate, total source frames)
    "sintel": ("sintel_trailer_1080p.mp4", 24, 1253),
    "bbb": ("bbb_trailer_720p.mov", 25, 812),
}
NAME_RE = re.compile(r"[A-Za-z0-9_][A-Za-z0-9_./-]{0,119}")
MAX_MOVIE_BYTES = 400 * 1024 * 1024
RENDER_ORDER = ("h264", "dnxhr", "prores", "png", "hevc")
# Two-lane model (owner, 2026-11-16): `gate` rows must pass on kf3 (strict); `discovery` rows try to break kf3 and may fail, the failure being the
# product. A row starts in `discovery` until it has passed on kf3; `expected_fail` names the known breakage per platform
# (failure class CRASH/HANG/TDR/XID/ASSERT/OUTPUT_MISMATCH, one-line reason) and is empty until a run records one. The five basic renders are
# `gate` by intent: NOT yet measured passing, so the lane field is DESIGN, flipped only by a run that records it (traces README).
FAILURE_CLASSES = ("CRASH", "HANG", "TDR", "XID", "ASSERT", "OUTPUT_MISMATCH")
LANES = ("gate", "discovery")

# One entry per deliverable. `expect` is what the host verifies on the pulled bytes.
RENDERS = {
    "h264": dict(lane="gate", expected_fail={}, label="H.264 MP4, UHD", fmt="MP4", codec="H.264", try_nvidia=True, resolution_prefix=None, timeout_s=900,
                 expect=dict(kind="movie", ext=".mp4", codecs=("h264", "avc1"), width=3840, height=2160)),
    "dnxhr": dict(lane="gate", expected_fail={}, label="Avid DNxHR MOV, UHD", fmt="QuickTime", codec="Avid DNxHR", try_nvidia=False, resolution_prefix=None, timeout_s=900,
                  expect=dict(kind="movie", ext=".mov", codecs=("dnxhd", "avdh", "avdn"), width=3840, height=2160)),
    "prores": dict(lane="gate", expected_fail={}, label="Apple ProRes MOV, UHD", fmt="QuickTime", codec="Apple ProRes", try_nvidia=False, resolution_prefix=None, timeout_s=900,
                   expect=dict(kind="movie", ext=".mov", codecs=("prores", "apch", "apcn", "apcs", "apco", "ap4h", "ap4x"), width=3840, height=2160)),
    "png": dict(lane="gate", expected_fail={}, label="PNG image sequence, UHD", fmt="PNG", codec=None, try_nvidia=False, resolution_prefix=None, timeout_s=900,
                expect=dict(kind="sequence", ext=".png", width=3840, height=2160)),
    "hevc": dict(lane="gate", expected_fail={}, label="H.265 MP4, 720p (software HEVC)", fmt="MP4", codec="H.265", try_nvidia=True, resolution_prefix="1280", timeout_s=900,
                 expect=dict(kind="movie", ext=".mp4", codecs=("hevc", "hvc1", "hev1"), width=1280, height=720)),
}

# Discovery rows (exotic paths meant to break kf3; allowed to fail, the failure is the product; they run in their own fresh clone with a hard
# time-box, never in the gate boot). `profile` is typed into the Encoding Profile control (prefix match, read back); a control or value the
# free edition does not offer is recorded as "not offered", which is a statement about the edition, not a kf3 result.
RENDERS.update({
    "prores4444xq": dict(lane="discovery", expected_fail={}, label="Apple ProRes 4444 XQ MOV (12-bit 4:4:4), UHD", fmt="QuickTime", codec="Apple ProRes",
                         profile="Apple ProRes 4444 XQ", try_nvidia=False, resolution_prefix=None, timeout_s=900,
                         expect=dict(kind="movie", ext=".mov", codecs=("prores", "ap4x"), width=3840, height=2160)),
    "dnxhr444": dict(lane="discovery", expected_fail={}, label="Avid DNxHR 444 10-bit MOV, UHD", fmt="QuickTime", codec="Avid DNxHR",
                     profile="DNxHR 444", try_nvidia=False, resolution_prefix=None, timeout_s=900,
                     expect=dict(kind="movie", ext=".mov", codecs=("dnxhd", "avdh", "avdn"), width=3840, height=2160)),
    "av1": dict(lane="discovery", expected_fail={}, label="AV1 MP4, UHD (NVENC AV1 is Studio, Ada+)", fmt="MP4", codec="AV1", try_nvidia=True,
                resolution_prefix=None, timeout_s=900,
                expect=dict(kind="movie", ext=".mp4", codecs=("av1", "av01"), width=3840, height=2160)),
    "hevc_uhd": dict(lane="discovery", expected_fail={}, label="H.265 MP4, UHD", fmt="MP4", codec="H.265", try_nvidia=True, resolution_prefix=None,
                     timeout_s=1200, expect=dict(kind="movie", ext=".mp4", codecs=("hevc", "hvc1", "hev1"), width=3840, height=2160)),
    "exr": dict(lane="discovery", expected_fail={}, label="OpenEXR sequence (16-bit float), UHD", fmt="EXR", codec=None, try_nvidia=False,
                resolution_prefix=None, timeout_s=1200, expect=dict(kind="sequence", ext=".exr", width=3840, height=2160)),
    "tiff": dict(lane="discovery", expected_fail={}, label="TIFF sequence (16-bit), UHD", fmt="TIFF", codec=None, try_nvidia=False,
                 resolution_prefix=None, timeout_s=1200, expect=dict(kind="sequence", ext=".tif", width=3840, height=2160)),
})
GATE_ORDER = RENDER_ORDER
DISCOVERY_ORDER = ("prores4444xq", "dnxhr444", "tiff", "exr", "av1", "hevc_uhd")

# Per-render requirement on the guest engine counters of the Resolve process (any engine counts: see engine_samples).
GUEST_NEED = G.Need(gpu_peak=10.0, gpu_active_samples=3, gpu_active_floor=5.0, mem_growth_mib=None, min_samples=4)
# The physical GPU (host nvidia-smi) must be busy: the guest scheduler does not see CUDA compute (MEASURED on the PC, see the README).
HOST_NEED = G.Need(gpu_peak=50.0, gpu_active_samples=3, gpu_active_floor=30.0, mem_growth_mib=None, min_samples=4)


# ------------------------------------------------------------------------------------------------ plan

def timeline_clips(frames: int = CLIP_FRAMES) -> list[dict]:
    """Four clips at `frames` timeline frames each: sintel normal, sintel 0.5x, bbb normal, bbb 2x (retime = FCPXML timeMap)."""
    def clip(name, src, start_s, speed=1.0):
        fname, sfps, total = SOURCES[src]
        return {"name": name, "path": GUEST_MEDIA.replace("\\", "/") + "/" + fname, "frames": frames, "src_frames": total, "fps": sfps,
                "start": int(start_s * sfps), "speed": speed}
    return [clip("sintel_a", "sintel", 10), clip("sintel_slow", "sintel", 25, 0.5), clip("bbb_a", "bbb", 8), clip("bbb_fast", "bbb", 16, 2.0)]


def build_plan(run_id: str, renders=None, out_dir: Optional[str] = None, lane: str = "gate") -> dict:
    if not re.fullmatch("[a-f0-9]{32}", run_id):
        raise ValueError("bad run id")
    out_dir = out_dir or rf"C:\kfapps\{run_id}\resolve\render"
    if lane not in ("gate", "discovery"):
        raise ValueError("lane must be gate or discovery")
    renders = renders or (GATE_ORDER if lane == "gate" else DISCOVERY_ORDER)
    items = []
    for rid in renders:
        r = RENDERS[rid]
        items.append({"id": rid, "label": r["label"], "fmt": r["fmt"], "codec": r["codec"], "try_nvidia": r["try_nvidia"],
                      "resolution_prefix": r["resolution_prefix"], "timeout_s": r["timeout_s"], "name": "kf_" + rid,
                      "profile": r.get("profile")})
    return {"lane": lane, "project": "kf" + run_id[:8], "timeline": TIMELINE, "fcpxml_guest": GUEST_MEDIA + r"\kf_timeline.fcpxml", "out_dir": out_dir,
            "grade": lane == "discovery", "grade_clips": [70, 337], "renders": items}


def fcpxml_text() -> str:
    return M.build_fcpxml(timeline_clips(), name=TIMELINE)


def guest_body(plan: dict) -> str:
    """The PowerShell placed after propimage_check.PS_LIB (it defines $r and $D). The plan is data, passed as one quoted JSON literal."""
    sampler = (HERE / "resolve_gpu_sampler.ps1").read_text()
    body = (HERE / "resolve_gpu_guest.ps1").read_text()
    marker = "@'\n__SAMPLER__\n'@"
    if body.count("__SAMPLER__") != 1 or marker not in body or "'@" in sampler:
        raise ValueError("guest script template broken")
    literal = json.dumps(plan).replace("'", "''")
    return f"$plan=ConvertFrom-Json '{literal}'\n" + body.replace(marker, "@'\n" + sampler.rstrip("\n") + "\n'@")


# ------------------------------------------------------------------------------------------------ samples

ENGINE_COLS = ("e3d", "compute", "copy", "vdec", "venc", "ofa")


def parse_engine_csv(text: str) -> list[G.Sample]:
    """Guest sampler CSV -> Sample list. `gpu` is the busiest engine of the Resolve process; encoder/decoder are venc/vdec;
    memory is the process's dedicated GPU memory in MiB. Guest UTC epoch seconds."""
    out = []
    for row in csv.DictReader(io.StringIO(text)):
        try:
            vals = {c: float(row.get(c) or 0) for c in ENGINE_COLS}
            out.append(G.Sample(float(row["epoch"]), max(vals.values()), None, vals["venc"], vals["vdec"], float(row.get("mem_pid_mb") or 0)))
        except (KeyError, ValueError, TypeError):
            continue
    return out


def engine_peaks(text: str, t0: float, t1: float) -> dict:
    """Per-engine peak and nonzero-sample counts inside the window (the numbers that go into the evidence)."""
    rows = []
    for row in csv.DictReader(io.StringIO(text)):
        try:
            if t0 <= float(row["epoch"]) <= t1:
                rows.append(row)
        except (KeyError, ValueError):
            continue
    out = {"samples": len(rows)}
    for c in ENGINE_COLS + ("e3d_all", "compute_all"):
        vals = [float(r.get(c) or 0) for r in rows]
        out[c] = {"peak": max(vals) if vals else None, "over5": sum(1 for v in vals if v > 5)}
    mem = [float(r.get("mem_pid_mb") or 0) for r in rows]
    out["mem_pid_mb"] = {"min": min(mem) if mem else None, "max": max(mem) if mem else None}
    return out


def smi_report(text: str) -> dict:
    """What the GUEST nvidia-smi printed: counts of numeric versus N/A utilisation samples (a finding when all are N/A)."""
    n = na = 0
    for line in text.splitlines():
        parts = [p.strip() for p in line.split(",")]
        if len(parts) != 6:
            continue
        n += 1
        if all(G._num(p) is None for p in parts[1:5]):
            na += 1
    return {"samples": n, "all_na": na, "utilization_numeric": n - na}


# ------------------------------------------------------------------------------------------------ Resolve log

def judge_log(lines: list[str]) -> tuple[bool, list[str], dict]:
    """Resolve's own statement of its processing device. Never passes on CPU only or a missing line."""
    blob = "\n".join(lines)
    facts = {}
    m = re.search(r"Selected compute API: (\w+)", blob)
    facts["compute_api"] = m.group(1) if m else None
    m = re.search(r"Detected (\d+) GPUs", blob)
    facts["gpus_detected"] = int(m.group(1)) if m else None
    m = re.search(r"on CUDA device '([^']+)'", blob) or re.search(r"Initialized OpenGL [\d.]+ .*on device '([^']*)'", blob)
    facts["device"] = m.group(1) if m else None
    facts["cuda_board_manager"] = bool(re.search(r"Let There Be CUDA|Initializing CUDA board manager", blob))
    facts["nvenc"] = bool(re.search(r"NVENC encodes", blob))
    facts["nvdec"] = bool(re.search(r"NVDEC decodes", blob))
    facts["gpudetect_errors"] = [ln[-120:] for ln in lines if "GPUDetect" in ln and "ERROR" in ln][:3]
    problems = []
    if facts["compute_api"] != "CUDA":
        problems.append(f"Resolve selected compute API {facts['compute_api']!r}, not CUDA")
    if not facts["gpus_detected"]:
        problems.append("Resolve's log shows no detected GPU")
    if not facts["device"] or "nvidia" not in facts["device"].lower():
        problems.append(f"Resolve's processing device is {facts['device']!r}, not an NVIDIA GPU")
    if not facts["cuda_board_manager"]:
        problems.append("log has no CUDA board manager line")
    if re.search(r"CPU only", blob, re.I):
        problems.append("log reports CPU only")
    return not problems, problems, facts


# ------------------------------------------------------------------------------------------------ outputs

def files_of(rr: dict) -> list[tuple[str, int, float]]:
    out = []
    for item in rr.get("files") or []:
        parts = str(item).split("|")
        if len(parts) == 3 and NAME_RE.fullmatch(parts[0]) and ".." not in parts[0].split("/"):
            try:
                out.append((parts[0], int(parts[1]), float(parts[2])))
            except ValueError:
                pass
    return out


def pick_pull(rid: str, rr: dict) -> list[str]:
    """Guest-relative files to pull for one render: a movie whole (bounded), a sequence's first, middle and last frame."""
    exp = RENDERS[rid]["expect"]
    files = [f for f in files_of(rr) if f[0].lower().endswith(exp["ext"])]
    if exp["kind"] == "movie":
        return [f[0] for f in files if f[1] <= MAX_MOVIE_BYTES][:1]
    files.sort()
    if not files:
        return []
    return sorted({files[0][0], files[len(files) // 2][0], files[-1][0]})


def verify_render(rid: str, rr: dict, directory: Path) -> tuple[bool, list[str], dict]:
    """Facts and problems for one render from the PULLED bytes (the guest's claims are only a file list)."""
    exp = RENDERS[rid]["expect"]
    problems, facts = [], {}
    listed = [f for f in files_of(rr) if f[0].lower().endswith(exp["ext"])]
    if not listed:
        return False, ["no output file with the expected extension was listed"], facts
    if exp["kind"] == "movie":
        name, size, _ = listed[0]
        path = directory / name
        if not path.is_file() or path.stat().st_size != size:
            return False, [f"{name} was not transferred or its size differs from the guest listing"], facts
        try:
            info = M.probe(path)
        except M.MediaError as exc:
            return False, [f"{name}: {exc}"], facts
        facts.update(info)
        codec = str(info.get("codec") or "").lower()
        tag = str(info.get("tag") or "").lower()
        if codec not in exp["codecs"] and tag not in exp["codecs"]:
            problems.append(f"codec {info.get('codec')!r} not in {exp['codecs']}")
        if (info.get("width"), info.get("height")) != (exp["width"], exp["height"]):
            problems.append(f"size {info.get('width')}x{info.get('height')} != {exp['width']}x{exp['height']}")
        if info.get("frames") is not None and info["frames"] != FRAMES_TOTAL:
            problems.append(f"{info['frames']} frames != {FRAMES_TOTAL}")
        dur = info.get("duration")
        if dur is not None and abs(dur - FRAMES_TOTAL / FPS) > 0.25:
            problems.append(f"duration {dur:.2f}s != {FRAMES_TOTAL / FPS:.2f}s")
        fs = M.frame_stats(path, FRAMES_TOTAL // 2)
        if fs:
            facts["frame_mid"] = fs
            if fs["luma_std"] < 2.0:
                problems.append(f"decoded middle frame is flat (luma std {fs['luma_std']})")
        return not problems, problems, facts
    # sequence
    count = len(listed)
    facts["files"] = count
    if count != FRAMES_TOTAL:
        problems.append(f"{count} frames in the sequence != {FRAMES_TOTAL}")
    pulled = []
    for name, size, _ in sorted(listed):
        p = directory / name
        if p.is_file():
            pulled.append((name, p))
    if len(pulled) < 2:
        return False, problems + ["fewer than two sequence frames were transferred"], facts
    seen = []
    if exp["ext"] != ".png":
        # no stdlib decoder for EXR/TIFF: the frames must be non-trivial files that differ between first and last
        for name, p in pulled:
            size = p.stat().st_size
            if size < 100_000:
                problems.append(f"{name}: only {size} bytes for a UHD frame")
            seen.append(M.sha256_file(p))
            facts.setdefault("frames_checked", []).append({"name": name, "bytes": size, "sha256": seen[-1]})
        if len(set(seen)) < 2:
            problems.append("first and last pulled frames are identical (nothing was rendered over time)")
        return not problems, problems, facts
    for name, p in pulled:
        try:
            info = M.png_info(p)
        except (M.MediaError, OSError, ValueError) as exc:
            problems.append(f"{name}: {exc}")
            continue
        if (info["width"], info["height"]) != (exp["width"], exp["height"]):
            problems.append(f"{name}: size {info['width']}x{info['height']}")
        if info.get("luma_std", 0) < 2.0:
            problems.append(f"{name}: decoded frame is flat")
        seen.append(info.get("pixel_sha256"))
        facts.setdefault("frames_checked", []).append({"name": name, "pixel_sha256": info.get("pixel_sha256"), "luma_mean": info.get("luma_mean")})
    if len(set(seen)) < 2:
        problems.append("first and last pulled frames are identical (nothing was rendered over time)")
    return not problems, problems, facts


# ------------------------------------------------------------------------------------------------ the row

def grade(result: dict, directory: Path, host_csv: str = "", host_t0: Optional[float] = None, host_utc_offset_s: float = 0.0,
          census_lines: Optional[list] = None, platform: str = "kf3") -> dict:
    """Return {'ok','status','error'/'signal','renders': {...},'findings': [...]} for the Resolve row.

    `result` is the guest's result.json (a source of file lists and timestamps only); `directory` holds the pulled bytes plus engine.csv and
    smi.csv; `host_csv` is host nvidia-smi output sampled during the task, `host_t0` the host epoch at which the guest script was started
    (guest epoch result['t0'] maps to it; the skew is the start-up latency, so windows are trimmed by 2 s each side)."""
    row = {"ok": False, "status": "FAIL", "evidence": "FILE", "platform": platform, "renders": {}, "findings": [], "problems": []}
    if census_lines is not None:
        row["census_whole_row"] = census(census_lines)
    engine_text = (directory / "engine.csv").read_text(errors="replace") if (directory / "engine.csv").is_file() else ""
    smi_text = (directory / "smi.csv").read_text(errors="replace") if (directory / "smi.csv").is_file() else ""
    guest_samples = parse_engine_csv(engine_text)
    host_samples = G.parse_nvidia_smi_csv(host_csv, host_utc_offset_s) if host_csv else []
    row["guest_nvidia_smi"] = smi_report(smi_text)
    if row["guest_nvidia_smi"]["samples"] and not row["guest_nvidia_smi"]["utilization_numeric"]:
        row["findings"].append("guest nvidia-smi reported N/A for utilization.gpu/memory/encoder/decoder in every sample; the guest assertion therefore "
                               "uses the WDDM per-engine counters of the Resolve process")
    ok_log, log_problems, log_facts = judge_log([str(x) for x in result.get("log_lines") or []])
    row["resolve_log"] = log_facts
    if not ok_log:
        row["problems"] += log_problems
    steps = {s.get("n"): s.get("ok") for s in result.get("steps") or []}
    for needed in ("project manager", "main window", "deliver page", "timeline on deliver page"):
        if not steps.get(needed):
            row["problems"].append(f"driver step '{needed}' did not complete (stage {result.get('stage')})")
    grade_nodes = result.get("grade_nodes") or []
    row["grade_nodes"] = grade_nodes
    if result.get("grade") is not False and not any(len(g.get("nodes") or []) >= 3 for g in grade_nodes if isinstance(g, dict)):
        row["findings"].append("grading was driven but fewer than three nodes could be read back on any clip: grade not proven")
    if result.get("error"):
        row["problems"].append("guest script error: " + str(result["error"])[:160])
    row["guest_goto_debug"] = [re.sub(r"[^\x20-\x7e]", "?", str(x))[:300] for x in (result.get("goto_debug") or [])][:12]
    for key in ("app_events", "nvlddmkm_events"):
        row["guest_" + key] = [re.sub(r"[^\x20-\x7e]", "?", str(x))[:240] for x in (result.get(key) or [])][:8]
    if result.get("resolve_running") is False and result.get("app_events"):
        row["failure_class"] = "CRASH"
        row["problems"].append("Resolve.exe died during the row: " + row["guest_app_events"][0][:160])
    by_id = {rr.get("id"): rr for rr in result.get("renders") or [] if isinstance(rr, dict)}
    order = DISCOVERY_ORDER if result.get("lane") == "discovery" else GATE_ORDER
    row["lane"] = "discovery" if order is DISCOVERY_ORDER else "gate"
    for rid in order:
        row["renders"][rid] = {"label": RENDERS[rid]["label"], "status": "NOT_RUN"}
    for rid in order:
        rr = by_id.get(rid)
        entry = row["renders"][rid]
        if rr is None:
            entry["reason"] = "not reached"
            row["problems"].append(f"render {rid} was not reached")
            continue
        probs = []
        for key, what in (("fmt_ok", "format"), ("codec_ok", "codec"), ("name_ok", "file name"), ("loc_ok", "location"), ("queued", "add to queue"),
                          ("render_pressed", "render"), ("started", "render start"), ("finished", "render end")):
            if rr.get(key) is not True:
                probs.append(f"{what} step failed")
        not_offered = [what for key, what in (("fmt_ok", "format"), ("codec_ok", "codec"), ("profile_ok", "profile"))
                       if key in rr and rr.get(key) is not True]
        if not_offered and RENDERS[rid]["lane"] == "discovery" and not rr.get("render_pressed"):
            entry.update(status="NOT_OFFERED", reason="the free edition's controls do not offer: " + ", ".join(not_offered),
                         fmt=rr.get("fmt_value"), codec=rr.get("codec_value"))
            row["findings"].append(f"{rid}: not offered by Resolve free ({', '.join(not_offered)}); a statement about the edition, not about kf3")
            continue
        entry.update(fmt=rr.get("fmt_value"), codec=rr.get("codec_value"), encoder=rr.get("encoder_value"), resolution=rr.get("res_value"),
                     nvidia_encoder_selectable=rr.get("nvidia_encoder_selectable"), ui_speed=[re.sub(r"[^\x20-\x7e]", "?", str(x))[:120] for x in (rr.get("speed_texts") or [])][:12], queue=[str(x)[:60] for x in (rr.get("queue") or [])][:8])
        t0, t1 = rr.get("t_started"), rr.get("t_end")
        if isinstance(t0, (int, float)) and isinstance(t1, (int, float)) and t1 > t0:
            entry["seconds"] = round(t1 - t0, 1)
            need = G.Need(**{**GUEST_NEED.__dict__, "encoder": bool(rr.get("nvidia_encoder_selectable"))})
            g = G.assert_gpu_used(guest_samples, need, (t0 + 1, t1 - 1))
            entry["guest_gpu"] = {"ok": g["ok"], "reasons": g["reasons"], "engines": engine_peaks(engine_text, t0, t1)}
            if not g["ok"]:
                probs += ["guest: " + x for x in g["reasons"]]
            if host_samples and host_t0 is not None and isinstance(result.get("t0"), (int, float)):
                h0, h1 = host_t0 + (t0 - result["t0"]) + 2, host_t0 + (t1 - result["t0"]) - 2
                h = G.assert_gpu_used(host_samples, HOST_NEED, (h0, h1))
                entry["host_gpu"] = {"ok": h["ok"], "reasons": h["reasons"], "gpu": h["summary"]["gpu"], "samples": h["summary"]["samples_in_window"]}
                if not h["ok"]:
                    probs += ["host: " + x for x in h["reasons"]]
            else:
                probs.append("host GPU samples missing (the physical GPU was not observed)")
        else:
            probs.append("no render window")
        okv, vprobs, vfacts = verify_render(rid, rr, directory)
        entry["output"] = {k: vfacts.get(k) for k in ("codec", "tag", "profile", "width", "height", "pix_fmt", "frames", "duration", "container", "bytes",
                                                      "sha256", "files", "frame_mid", "frames_checked", "reader") if k in vfacts}
        probs += vprobs
        if rr.get("try_nvidia") or RENDERS[rid].get("try_nvidia"):
            if rr.get("nvidia_encoder_selectable") is False:
                row["findings"].append(f"{rid}: the Encoder control offered no NVIDIA encoder (Auto/Native only): NVENC not exercised (Studio-only feature of Resolve)")
        entry["status"] = "PASS" if not probs else "FAIL"
        entry["problems"] = probs
        entry["lane"] = RENDERS[rid]["lane"]
        if probs:
            entry["failure_class"] = classify_failure(entry, row.get("census_whole_row"))
            known = RENDERS[rid]["expected_fail"].get(platform)
            entry["expected_fail"] = bool(known and known.get("class") == entry["failure_class"])
        if probs:
            row["problems"].append(f"{rid}: " + "; ".join(probs[:3]))
        if not rr.get("finished"):
            break
    row["other_suites"] = {
        "pugetbench": "NOT RUN: package not pinned (private object, owner to supply); PugetBench for DaVinci Resolve supports the free edition only up to v21.0 "
                      "and the image has 21.2.0.6 (scripting was removed from free in 21.1)",
        "resolve_builtin_benchmark": "none exists in Resolve; the Deliver page's render speed is a UI readout only (not parsed)",
    }
    if row["problems"]:
        row["error"] = "; ".join(row["problems"][:6])
        row["status"] = "INCONCLUSIVE" if any("driver step" in p for p in row["problems"]) else "FAIL"
        return row
    row.update(ok=True, status="PASS", signal=(
        "Resolve rendered " + ", ".join(order) + " (outputs verified on the host); guest engine counters and the host GPU were busy during every render; "
        f"Resolve's log: compute API {log_facts['compute_api']}, device {log_facts['device']!r}"))
    return row


# ------------------------------------------------------------------------------------------------ host side of the run

def prepare(guest, run_id: str, cfg: dict) -> dict:
    """Fetch the open media (sources, then the public mirror; pinned sha256), push it and the FCPXML to the guest, return the plan."""
    media = Path(cfg.get("media_dir") or Path.cwd() / "resolve_media")
    try:
        M.fetch_assets(media)
    except (M.MediaError, OSError) as exc:
        raise ValueError(f"open media not available: {exc}") from exc
    made = guest.ps(rf"New-Item -Force -ItemType Directory {GUEST_MEDIA} | Out-Null")
    if made.returncode:
        raise ValueError("guest transport lost while preparing media; not an app failure")
    for fname, _, _ in SOURCES.values():
        src = next(media.rglob(fname), None)
        if src is None:
            raise ValueError(f"media file {fname} missing after fetch")
        sent = guest.scp(str(src), GUEST_MEDIA.replace("\\", "/") + "/" + fname, True)
        if sent.returncode:
            raise ValueError(f"could not copy {fname} to the guest: {sent.stderr[:120]}")
    xml = Path(media) / "kf_timeline.fcpxml"
    xml.write_text(fcpxml_text())
    sent = guest.scp(str(xml), GUEST_MEDIA.replace("\\", "/") + "/kf_timeline.fcpxml", True)
    if sent.returncode:
        raise ValueError("could not copy the timeline to the guest")
    return build_plan(run_id, lane=cfg.get("lane", "gate"))


def pull_outputs(guest, d: Path, plan: dict, remote: str) -> None:
    """Pull the sampler files and the rendered outputs the verdict needs (bounded; names validated). Failures leave files missing, which
    the verdict reports; a missing file never turns into a pass."""
    for f in ("engine.csv", "smi.csv"):
        guest.scp(remote + "/" + f, str(d / f), False)
    try:
        result = json.loads((d / "result.json").read_text(encoding="utf-8-sig"))
    except (OSError, ValueError):
        return
    out_remote = plan["out_dir"].replace("\\", "/")
    for rr in result.get("renders") or []:
        rid = rr.get("id")
        if rid not in RENDERS:
            continue
        for name in pick_pull(rid, rr):
            target = d / name
            target.parent.mkdir(parents=True, exist_ok=True)
            guest.scp(out_remote + "/" + name, str(target), False)


def row_definitions() -> list[dict]:
    """The rows of this lane in the shape the generic runner consumes (id, lane, expected_fail per platform)."""
    out = []
    for rid in GATE_ORDER + DISCOVERY_ORDER:
        r = RENDERS[rid]
        if r["lane"] not in LANES or any(v.get("class") not in FAILURE_CLASSES for v in r["expected_fail"].values()):
            raise ValueError(f"bad lane definition for {rid}")
        out.append({"id": "resolve." + rid, "label": r["label"], "lane": r["lane"], "expected_fail": r["expected_fail"],
                    "platforms": ["kf3", "vfio"]})
    return out


# ------------------------------------------------------------------------------------------------ kf3 census

_UUID = re.compile(r"GPU-[0-9a-fA-F-]{8,}|[0-9a-fA-F]{8}-[0-9a-fA-F]{4}-[0-9a-fA-F]{4}-[0-9a-fA-F]{4}-[0-9a-fA-F]{12}")


def census(lines: list[str], top: int = 12) -> dict:
    """RM-call census of the kf3 host QEMU log lines of one row's window: unserviced GSP controls (by code), ForeignClass, DEAD channels,
    retired channels, Xid/TDR/timeouts. Counts and normalised patterns only: UUIDs and numbers are stripped, no raw log line is kept."""
    uns = {}
    pat = {}
    out = {"lines": len(lines), "unserviced": 0, "unserviced_codes": {}, "foreign_class": 0, "dead_channels": 0, "channels_retired": 0,
           "xid": 0, "tdr_or_timeout": 0, "refused_leaves_absent": 0}
    for ln in lines:
        if "UNSERVICED" in ln:
            out["unserviced"] += 1
            m = re.search(r"code: (\d+)", ln)
            if m:
                uns[m.group(1)] = uns.get(m.group(1), 0) + 1
            continue
        low = ln.lower()
        hit = False
        if "foreignclass" in low or "foreign class" in low:
            out["foreign_class"] += 1
            hit = True
        if re.search(r"dead=some|\bdead channel|channel .*\bdead\b", low):
            out["dead_channels"] += 1
            hit = True
        if "retired" in low and "chan token" in low:
            out["channels_retired"] += 1
        if "xid" in low and "nvrm" in low:
            out["xid"] += 1
            hit = True
        if not ln.startswith("kf3: family=") and re.search(r"\btdr\b|timed out|\btimeout\b", low):
            out["tdr_or_timeout"] += 1
            hit = True
        if "refused leaves are absent" in low:
            out["refused_leaves_absent"] += 1
        if hit:
            key = re.sub(r"\d+", "N", _UUID.sub("UUID", ln))[:140]
            pat[key] = pat.get(key, 0) + 1
    out["unserviced_codes"] = dict(sorted(uns.items(), key=lambda kv: -kv[1])[:top])
    out["patterns"] = [{"n": n, "pattern": k} for k, n in sorted(pat.items(), key=lambda kv: -kv[1])[:top]]
    return out


def classify_failure(entry: dict, cen: Optional[dict]) -> Optional[str]:
    """One failure class for a failed render entry (None when it passed): the first that applies, evidence-based."""
    if entry.get("status") == "PASS":
        return None
    probs = " ".join(entry.get("problems") or [])
    if cen and cen.get("xid"):
        return "XID"
    if cen and cen.get("tdr_or_timeout") and entry.get("status") != "NOT_RUN":
        return "TDR"
    if "render end" in probs or "stopped by driver" in probs:
        return "HANG"
    if re.search(r"no output file|not transferred|codec|size|frames|duration|flat|identical|sequence", probs):
        return "OUTPUT_MISMATCH"
    return "ASSERT"


def compare_platforms(kf3: Optional[dict], vfio: Optional[dict]) -> dict:
    """Differential classification per render id from two graded rows (verdict.json of each platform)."""
    out = {}
    ids = set((kf3 or {}).get("renders", {})) | set((vfio or {}).get("renders", {}))
    for rid in sorted(ids):
        a = ((kf3 or {}).get("renders", {}).get(rid) or {}).get("status")
        b = ((vfio or {}).get("renders", {}).get(rid) or {}).get("status")
        if a is None or b is None:
            c = "VFIO_CONTROL_PENDING" if b is None else "KF3_PENDING"
        elif a == "PASS" and b == "PASS":
            c = "BOTH_PASS"
        elif a != "PASS" and b != "PASS":
            c = "BOTH_FAIL (row or application problem)"
        elif a != "PASS":
            c = "KAYFABE_DEFECT (kf3 fails, bare GPU passes)"
        else:
            c = "KF3_PASS_VFIO_FAIL (odd)"
        out[rid] = {"kf3": a, "vfio": b, "class": c}
    return out
