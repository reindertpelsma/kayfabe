#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later
"""Tool-neutral timeline, effects and export-matrix SPEC generator (stdlib only).

STATUS: LIVE, 2026-11-16. Writes one editing timeline in three interchange formats plus an export
matrix, all byte-for-byte deterministic (no timestamps, no randomness):

  timeline.otio       OpenTimelineIO JSON, written by hand (no opentimelineio package needed)
  timeline.fcpxml     FCPXML 1.9: resources, project, sequence, spine, transitions, titles
  timeline.edl        CMX3600 list of the video cuts
  export_matrix.json  the encode jobs an editing application must be able to run

The timeline is 3840x2160 at 25 fps, 1500 frames (60 s), 12 clips cut from two pinned assets of
test_assets.json and from synthetic ffmpeg lavfi sources (their exact command lines are recorded in
the spec as `synthetic_sources`). Application-specific scripts layer their own effects on top; the
neutral per-clip colour grade lives in `metadata.kayfabe.grade` of every video clip.

Subcommands:
  spec --out DIR                   write the four spec files
  check --out DIR                  regenerate in memory, compare sha256 per file, validate structure
  make-sources --dest DIR [--assets DIR]
                                   run the ffmpeg commands for the synthetic sources, write
                                   sources.sha256 (and verify pinned assets if --assets is given)

Importers meant: OpenTimelineIO adapters (timeline.otio), the FCPXML importer of a non-linear
editor (timeline.fcpxml), and any CMX3600 EDL reader (timeline.edl).
"""

from __future__ import annotations

import argparse
import hashlib
import json
import math
import re
import shutil
import subprocess
import sys
import xml.etree.ElementTree as ET
from fractions import Fraction
from pathlib import Path

SPEC_SCHEMA = "kayfabe.timeline-spec/1"
FPS = 25
WIDTH, HEIGHT = 3840, 2160
TC_START_FRAMES = 3600 * FPS  # 01:00:00:00
TRANSITION_FRAMES = 12
FILES = ("timeline.otio", "timeline.fcpxml", "timeline.edl", "export_matrix.json")

# ---------------------------------------------------------------------------------------------
# Sources. Pinned assets are referenced by their manifest names. The manifest does not record
# duration or frame rate, so `available_frames` is a conservative LOWER BOUND (in timeline frames,
# i.e. assuming the importer conforms the source to 25 fps); a later ffprobe pass may raise it.
# ---------------------------------------------------------------------------------------------
SOURCES = {
    "sintel": dict(
        path="video/sintel_trailer_1080p.mp4", kind="pinned", width=1920, height=1080,
        available_frames=1250, reel="SINTEL01", has_audio=False),
    "bbb": dict(
        path="video/bbb_trailer_720p.mov", kind="pinned", width=1280, height=720,
        available_frames=600, reel="BBB01", has_audio=False),
    "testsrc2": dict(
        path="synthetic/testsrc2_2160p25.mp4", kind="synthetic", width=WIDTH, height=HEIGHT,
        available_frames=500, reel="TESTSRC2", has_audio=False),
    "mandelbrot": dict(
        path="synthetic/mandelbrot_2160p25.mp4", kind="synthetic", width=WIDTH, height=HEIGHT,
        available_frames=500, reel="MANDEL", has_audio=False),
    "smptehdbars": dict(
        path="synthetic/smptehdbars_2160p25.mp4", kind="synthetic", width=WIDTH, height=HEIGHT,
        available_frames=250, reel="SMPTEBAR", has_audio=False),
    "sine": dict(
        path="synthetic/sine_440hz.mp4", kind="synthetic", width=0, height=0,
        available_frames=1500, reel="SINE440", has_audio=True),
}

_X264 = ["-c:v", "libx264", "-preset", "veryfast", "-crf", "18", "-pix_fmt", "yuv420p",
         "-threads", "4", "-bitexact", "-fflags", "+bitexact", "-flags:v", "+bitexact"]
_FF = ["ffmpeg", "-y", "-hide_banner", "-loglevel", "error", "-nostdin"]


def synthetic_sources() -> list[dict]:
    """The exact ffmpeg command lines. `-threads 4` pins the x264 frame-thread count, which
    otherwise follows the CPU count and changes the bitstream."""
    def video(name: str, lavfi: str, seconds: int) -> dict:
        out = SOURCES[name]["path"]
        argv = _FF + ["-f", "lavfi", "-i", lavfi, "-t", str(seconds), "-an"] + _X264 + [out]
        return {"name": name, "output": out, "argv": argv}

    sine_out = SOURCES["sine"]["path"]
    sine = {"name": "sine", "output": sine_out, "argv": _FF + [
        "-f", "lavfi", "-i", "sine=frequency=440:sample_rate=48000", "-t", "60", "-vn",
        "-c:a", "aac", "-b:a", "192k", "-bitexact", "-fflags", "+bitexact", sine_out]}
    return [
        video("testsrc2", "testsrc2=size=3840x2160:rate=25", 20),
        video("mandelbrot", "mandelbrot=size=3840x2160:rate=25", 20),
        video("smptehdbars", "smptehdbars=size=3840x2160:rate=25", 10),
        sine,
    ]


# ---------------------------------------------------------------------------------------------
# The cut list. Frames are timeline frames at 25 fps. `scalar` is the LinearTimeWarp time_scalar:
# 1 normal, 0.5 slow motion (50 percent), 2 fast (200 percent), -1 reverse, 0 freeze.
# `src_in` is the first source frame of the consumed media range (for a reverse the range is
# played backwards; for a freeze it is the single held frame).
# ---------------------------------------------------------------------------------------------
CLIPS = [
    # name,           source,        src_in, frames, scalar
    ("sintel_open",   "sintel",           0,   150,  1.0),
    ("testsrc_a",     "testsrc2",         0,   125,  1.0),
    ("bbb_a",         "bbb",            100,   125,  1.0),
    ("mandel_slow",   "mandelbrot",       0,   150,  0.5),
    ("sintel_fast",   "sintel",         300,   100,  2.0),
    ("bbb_reverse",   "bbb",            300,   100, -1.0),
    ("bars_freeze",   "smptehdbars",     50,    75,  0.0),
    ("sintel_b",      "sintel",         600,   150,  1.0),
    ("testsrc_slow",  "testsrc2",       200,   150,  0.5),
    ("mandel_b",      "mandelbrot",     200,   125,  1.0),
    ("bbb_b",         "bbb",            400,   100,  1.0),
    ("sintel_close",  "sintel",         900,   150,  1.0),
]
# Cross dissolves sit at the cut AFTER clip index i (0-based), centred: 6 frames either side.
TRANSITIONS_AFTER = (0, 3, 8)
TITLES = [
    # name, text, timeline start frame, frames
    ("title_open", "Kayfabe timeline spec", 25, 75),
    ("title_close", "The End", 1350, 75),
]
# Audio gain keyframes (timeline frame, dB): fade in, hold, duck under the second title, fade out.
GAIN_KEYFRAMES = [(0, -60.0), (25, 0.0), (1300, 0.0), (1350, -6.0), (1425, -6.0), (1500, -60.0)]
TOTAL_FRAMES = sum(c[3] for c in CLIPS)


def consumed_frames(frames: int, scalar: float) -> int:
    """Source frames covered by a clip of `frames` timeline frames at `scalar`."""
    if scalar == 0:
        return 1
    return math.ceil(frames * abs(scalar))


def grade(i: int) -> dict:
    """Neutral per-clip colour grade, a pure function of the clip index (no randomness)."""
    def r(x: float) -> float:
        return round(x, 4)
    k = i % 5
    return {
        "lift": [r(-0.03 + 0.01 * k), r(-0.02 + 0.005 * k), r(0.0 + 0.01 * ((i + 2) % 5))],
        "gamma": [r(1.0 + 0.04 * k - 0.08), r(1.0 + 0.02 * ((i + 1) % 5) - 0.04), r(1.0)],
        "gain": [r(1.0 + 0.02 * ((i + 3) % 5) - 0.04), r(1.0), r(1.0 - 0.02 * k + 0.04)],
        "saturation": r(0.8 + 0.1 * (i % 5)),
        "tone_curve": [[0.0, 0.0], [0.25, r(0.22 + 0.01 * k)], [0.5, 0.5],
                       [0.75, r(0.78 - 0.01 * k)], [1.0, 1.0]],
    }


# ---------------------------------------------------------------------------------------------
# Timecode and time helpers
# ---------------------------------------------------------------------------------------------
def tc(frames: int) -> str:
    s, f = divmod(frames, FPS)
    m, s = divmod(s, 60)
    h, m = divmod(m, 60)
    return f"{h:02d}:{m:02d}:{s:02d}:{f:02d}"


def parse_tc(text: str) -> int:
    h, m, s, f = (int(p) for p in text.split(":"))
    return ((h * 60 + m) * 60 + s) * FPS + f


def fcp_time(frames: int) -> str:
    fr = Fraction(frames, FPS)
    return f"{fr.numerator}s" if fr.denominator == 1 else f"{fr.numerator}/{fr.denominator}s"


def parse_fcp_time(text: str) -> int:
    fr = Fraction(text[:-1]) * FPS
    if fr.denominator != 1:
        raise ValueError(f"not a whole frame count at {FPS} fps: {text}")
    return int(fr)


def rt(frames: float) -> dict:
    return {"OTIO_SCHEMA": "RationalTime.1", "rate": float(FPS), "value": float(frames)}


def tr(start: int, duration: int) -> dict:
    return {"OTIO_SCHEMA": "TimeRange.1", "duration": rt(duration), "start_time": rt(start)}


# ---------------------------------------------------------------------------------------------
# OTIO
# ---------------------------------------------------------------------------------------------
def _clip_layout() -> list[dict]:
    out, pos = [], 0
    for i, (name, src, src_in, frames, scalar) in enumerate(CLIPS):
        out.append(dict(index=i, name=name, source=src, src_in=src_in, frames=frames, scalar=scalar,
                        consumed=consumed_frames(frames, scalar), rec_in=pos))
        pos += frames
    return out


def _media_reference(src: str) -> dict:
    s = SOURCES[src]
    return {
        "OTIO_SCHEMA": "ExternalReference.1",
        "metadata": {"kayfabe": {"kind": s["kind"], "available_frames_is_lower_bound": True}},
        "name": Path(s["path"]).stem,
        "available_range": tr(0, s["available_frames"]),
        "target_url": s["path"],
    }


def _otio_effect(name: str, effect_name: str, meta: dict) -> dict:
    return {"OTIO_SCHEMA": "Effect.1", "metadata": meta, "name": name, "effect_name": effect_name}


def _timewarp(scalar: float) -> dict:
    return {"OTIO_SCHEMA": "LinearTimeWarp.1", "metadata": {}, "name": "speed",
            "effect_name": "LinearTimeWarp", "time_scalar": float(scalar)}


def build_otio() -> dict:
    layout = _clip_layout()
    v1: list[dict] = []
    for c in layout:
        effects = [_timewarp(c["scalar"])] if c["scalar"] != 1.0 else []
        v1.append({
            "OTIO_SCHEMA": "Clip.2",
            "metadata": {"kayfabe": {
                "role": "video",
                "source_id": c["source"],
                "speed": {
                    "percent": c["scalar"] * 100.0,
                    "time_scalar": float(c["scalar"]),
                    "timeline_frames": c["frames"],
                    "source_frames": c["consumed"],
                    "reverse": c["scalar"] < 0,
                    "freeze": c["scalar"] == 0,
                },
                "grade": grade(c["index"]),
            }},
            "name": c["name"],
            "source_range": tr(c["src_in"], c["frames"]),
            "effects": effects,
            "markers": [],
            "enabled": True,
            "media_references": {"DEFAULT_MEDIA": _media_reference(c["source"])},
            "active_media_reference_key": "DEFAULT_MEDIA",
        })
        if c["index"] in TRANSITIONS_AFTER:
            half = TRANSITION_FRAMES // 2
            v1.append({
                "OTIO_SCHEMA": "Transition.1",
                "metadata": {"kayfabe": {"role": "cross_dissolve"}},
                "name": f"dissolve_{c['index'] + 1:02d}_{c['index'] + 2:02d}",
                "transition_type": "SMPTE_Dissolve",
                "in_offset": rt(half),
                "out_offset": rt(half),
            })

    v2: list[dict] = []
    pos = 0
    for gi, (name, text, start, frames) in enumerate(TITLES):
        if start > pos:
            v2.append(_gap(f"gap_{gi}", start - pos))
        v2.append({
            "OTIO_SCHEMA": "Clip.2",
            "metadata": {"kayfabe": {"role": "title", "text": text, "font_size": 120,
                                     "color_rgba": [1.0, 1.0, 1.0, 1.0],
                                     "position": [0.0, -0.3]}},
            "name": name,
            "source_range": tr(0, frames),
            "effects": [_otio_effect("title", "Title", {"kayfabe": {"text": text}})],
            "markers": [],
            "enabled": True,
            "media_references": {"DEFAULT_MEDIA": {
                "OTIO_SCHEMA": "GeneratorReference.1",
                "metadata": {},
                "name": name,
                "available_range": tr(0, frames),
                "generator_kind": "Title",
                "parameters": {"text": text},
            }},
            "active_media_reference_key": "DEFAULT_MEDIA",
        })
        pos = start + frames
    if pos < TOTAL_FRAMES:
        v2.append(_gap("gap_end", TOTAL_FRAMES - pos))

    a1 = [{
        "OTIO_SCHEMA": "Clip.2",
        "metadata": {"kayfabe": {
            "role": "audio", "source_id": "sine",
            "gain_keyframes": [{"frame": f, "gain_db": g} for f, g in GAIN_KEYFRAMES]}},
        "name": "sine_bed",
        "source_range": tr(0, TOTAL_FRAMES),
        "effects": [_otio_effect("gain", "AudioGain", {"kayfabe": {
            "keyframes": [{"frame": f, "gain_db": g} for f, g in GAIN_KEYFRAMES]}})],
        "markers": [],
        "enabled": True,
        "media_references": {"DEFAULT_MEDIA": _media_reference("sine")},
        "active_media_reference_key": "DEFAULT_MEDIA",
    }]

    def track(name: str, kind: str, children: list[dict]) -> dict:
        return {"OTIO_SCHEMA": "Track.1", "metadata": {}, "name": name, "source_range": None,
                "effects": [], "markers": [], "enabled": True, "children": children, "kind": kind}

    return {
        "OTIO_SCHEMA": "Timeline.1",
        "metadata": {"kayfabe": {
            "spec": SPEC_SCHEMA,
            "width": WIDTH, "height": HEIGHT, "fps": FPS,
            "total_frames": TOTAL_FRAMES,
            "transition_frames": TRANSITION_FRAMES,
            "color_space": "Rec.709",
            "sources": {k: {kk: vv for kk, vv in v.items() if kk != "reel"}
                        for k, v in sorted(SOURCES.items())},
            "synthetic_sources": synthetic_sources(),
        }},
        "name": "kayfabe_timeline",
        "global_start_time": rt(TC_START_FRAMES),
        "tracks": {
            "OTIO_SCHEMA": "Stack.1", "metadata": {}, "name": "tracks", "source_range": None,
            "effects": [], "markers": [], "enabled": True,
            "children": [track("V1", "Video", v1), track("V2", "Video", v2),
                         track("A1", "Audio", a1)],
        },
    }


def _gap(name: str, frames: int) -> dict:
    return {"OTIO_SCHEMA": "Gap.1", "metadata": {}, "name": name, "source_range": tr(0, frames),
            "effects": [], "markers": [], "enabled": True}


# ---------------------------------------------------------------------------------------------
# FCPXML 1.9
# ---------------------------------------------------------------------------------------------
def _uid(name: str) -> str:
    return hashlib.sha256(name.encode()).hexdigest()[:32].upper()


def build_fcpxml() -> str:
    layout = _clip_layout()
    root = ET.Element("fcpxml", version="1.9")
    res = ET.SubElement(root, "resources")
    ET.SubElement(res, "format", id="r1", name="FFVideoFormat3840x2160p25",
                  frameDuration=fcp_time(1), width=str(WIDTH), height=str(HEIGHT),
                  colorSpace="1-1-1 (Rec. 709)")
    ET.SubElement(res, "format", id="r2", name="FFVideoFormat1920x1080p25",
                  frameDuration=fcp_time(1), width="1920", height="1080",
                  colorSpace="1-1-1 (Rec. 709)")
    ET.SubElement(res, "format", id="r3", name="FFVideoFormat1280x720p25",
                  frameDuration=fcp_time(1), width="1280", height="720",
                  colorSpace="1-1-1 (Rec. 709)")
    fmt_of = {"sintel": "r2", "bbb": "r3", "testsrc2": "r1", "mandelbrot": "r1",
              "smptehdbars": "r1"}
    asset_id = {}
    for n, key in enumerate(("sintel", "bbb", "testsrc2", "mandelbrot", "smptehdbars", "sine")):
        s = SOURCES[key]
        aid = f"r{4 + n}"
        asset_id[key] = aid
        attrs = dict(id=aid, name=Path(s["path"]).stem, uid=_uid(s["path"]), start="0s",
                     duration=fcp_time(s["available_frames"]), src=s["path"],
                     hasVideo="0" if s["has_audio"] else "1",
                     hasAudio="1" if s["has_audio"] else "0")
        if not s["has_audio"]:
            attrs["format"] = fmt_of[key]
        else:
            attrs.update(audioSources="1", audioChannels="1", audioRate="48000")
        ET.SubElement(res, "asset", **attrs)
    # Neutral generator identifiers; an importer maps them to its own title and dissolve.
    ET.SubElement(res, "effect", id="r10", name="Title", uid="kayfabe://generator/title")
    ET.SubElement(res, "effect", id="r11", name="Cross Dissolve",
                  uid="kayfabe://transition/cross-dissolve")

    lib = ET.SubElement(root, "library")
    event = ET.SubElement(lib, "event", name="kayfabe")
    project = ET.SubElement(event, "project", name="kayfabe_timeline")
    seq = ET.SubElement(project, "sequence", format="r1", duration=fcp_time(TOTAL_FRAMES),
                        tcStart=fcp_time(TC_START_FRAMES), tcFormat="NDF", audioLayout="stereo",
                        audioRate="48k")
    spine = ET.SubElement(seq, "spine")

    clip_el: dict[int, ET.Element] = {}
    for c in layout:
        s = SOURCES[c["source"]]
        el = ET.SubElement(spine, "asset-clip", name=c["name"], ref=asset_id[c["source"]],
                           offset=fcp_time(c["rec_in"]), start=fcp_time(c["src_in"]),
                           duration=fcp_time(c["frames"]), format="r1", tcFormat="NDF")
        clip_el[c["index"]] = el
        # Fit the source into the UHD frame; the scale is the conform factor.
        fit = WIDTH / s["width"]
        ET.SubElement(el, "adjust-transform", position="0 0", rotation="0",
                      scale=f"{fit:g} {fit:g}")
        if c["scalar"] != 1.0:
            tm = ET.SubElement(el, "timeMap")
            if c["scalar"] >= 0:
                v0, v1 = c["src_in"], c["src_in"] + (0 if c["scalar"] == 0 else c["consumed"])
            else:  # reverse: media time runs from the end of the consumed range down to its start
                v0, v1 = c["src_in"] + c["consumed"], c["src_in"]
            ET.SubElement(tm, "timept", time="0s", value=fcp_time(v0), interp="linear")
            ET.SubElement(tm, "timept", time=fcp_time(c["frames"]), value=fcp_time(v1),
                          interp="linear")
        if c["index"] == 0:
            audio = ET.SubElement(el, "asset-clip", name="sine_bed", ref=asset_id["sine"],
                                  lane="-1", offset=fcp_time(c["src_in"]), start="0s",
                                  duration=fcp_time(TOTAL_FRAMES), tcFormat="NDF")
            vol = ET.SubElement(audio, "adjust-volume")
            param = ET.SubElement(vol, "param", name="amount")
            ka = ET.SubElement(param, "keyframeAnimation")
            for f, g in GAIN_KEYFRAMES:
                ET.SubElement(ka, "keyframe", time=fcp_time(f), value=f"{g:g}dB")
        if c["index"] in TRANSITIONS_AFTER:
            nxt = layout[c["index"] + 1]
            half = TRANSITION_FRAMES // 2
            tre = ET.SubElement(spine, "transition", name=f"dissolve_{c['index'] + 1:02d}",
                                offset=fcp_time(nxt["rec_in"] - half),
                                duration=fcp_time(TRANSITION_FRAMES))
            ET.SubElement(tre, "filter-video", ref="r11", name="Cross Dissolve")
    # Spine order must alternate clip, transition, clip: re-order so a transition follows its clip.
    spine[:] = _spine_in_order(spine)

    for ti, (name, text, start, frames) in enumerate(TITLES):
        host = next(c for c in layout if c["rec_in"] <= start < c["rec_in"] + c["frames"])
        parent = clip_el[host["index"]]
        tel = ET.SubElement(parent, "title", name=name, ref="r10", lane="1",
                            offset=fcp_time(host["src_in"] + (start - host["rec_in"])),
                            start=fcp_time(TC_START_FRAMES), duration=fcp_time(frames))
        txt = ET.SubElement(tel, "text")
        ET.SubElement(txt, "text-style", ref=f"ts{ti + 1}").text = text
        sd = ET.SubElement(tel, "text-style-def", id=f"ts{ti + 1}")
        ET.SubElement(sd, "text-style", font="Helvetica", fontSize="120", fontColor="1 1 1 1")

    ET.indent(root, space="  ")
    body = ET.tostring(root, encoding="unicode")
    return '<?xml version="1.0" encoding="UTF-8"?>\n<!DOCTYPE fcpxml>\n' + body + "\n"


def _spine_in_order(spine: ET.Element) -> list[ET.Element]:
    """Children were appended clip by clip with the transition right after its clip already; keep
    them sorted by offset (stable), which puts each transition 6 frames before the next clip's
    offset, i.e. between the two clips it joins."""
    items = list(spine)
    clips = [e for e in items if e.tag == "asset-clip"]
    trans = {parse_fcp_time(e.get("offset")) + TRANSITION_FRAMES // 2: e
             for e in items if e.tag == "transition"}
    out: list[ET.Element] = []
    for c in clips:
        t = trans.get(parse_fcp_time(c.get("offset")))
        if t is not None:
            out.append(t)
        out.append(c)
    return out


# ---------------------------------------------------------------------------------------------
# EDL (CMX3600)
# ---------------------------------------------------------------------------------------------
EDL_LINE = re.compile(
    r"^(\d{3})  (\S{1,8}) +V {5}C {8}(\d\d:\d\d:\d\d:\d\d) (\d\d:\d\d:\d\d:\d\d) "
    r"(\d\d:\d\d:\d\d:\d\d) (\d\d:\d\d:\d\d:\d\d)$")


def build_edl() -> str:
    lines = ["TITLE: KAYFABE_TIMELINE", "FCM: NON-DROP FRAME", ""]
    for c in _clip_layout():
        s = SOURCES[c["source"]]
        src_in = c["src_in"]
        src_out = src_in + c["consumed"]
        rec_in = TC_START_FRAMES + c["rec_in"]
        rec_out = rec_in + c["frames"]
        lines.append(f"{c['index'] + 1:03d}  {s['reel']:<8} V     C        "
                     f"{tc(src_in)} {tc(src_out)} {tc(rec_in)} {tc(rec_out)}")
        lines.append(f"* FROM CLIP NAME: {c['name']}")
        lines.append(f"* SOURCE FILE: {s['path']}")
        if c["scalar"] not in (1.0,):
            kind = ("FREEZE" if c["scalar"] == 0 else "REVERSE" if c["scalar"] < 0 else "RETIME")
            lines.append(f"* KAYFABE SPEED {kind} {c['scalar'] * 100:g} PERCENT")
            lines.append(f"M2   {s['reel']:<8} {c['scalar'] * FPS:05.1f}"
                         f"                {tc(src_in)}")
        if c["index"] in TRANSITIONS_AFTER:
            lines.append(f"* KAYFABE CROSS DISSOLVE {TRANSITION_FRAMES} FRAMES CENTRED ON THE NEXT CUT")
    return "\n".join(lines) + "\n"


# ---------------------------------------------------------------------------------------------
# Export matrix
# ---------------------------------------------------------------------------------------------
def build_export_matrix() -> dict:
    dur = TOTAL_FRAMES / FPS

    def job(name, container, codec, bit_depth, chroma, res, fps, hw, fourcc, *, crf=None,
            mbps=None, ext=None, **extra):
        w, h = res
        d = {
            "name": name, "container": container, "codec": codec, "bit_depth": bit_depth,
            "chroma": chroma, "resolution": f"{w}x{h}", "fps": fps,
            "rate_control": "crf" if crf is not None else ("bitrate" if mbps is not None
                                                           else "profile"),
            "crf": crf, "bitrate_mbps": mbps, "hardware": hw,
            "expected_duration_s": dur, "expected_frames": int(round(dur * fps)),
            "expected_video_codec_fourcc": fourcc,
            "audio": {"codec": "aac", "bitrate_kbps": 192} if container != "image_sequence"
            else None,
        }
        d.update(extra)
        return d

    uhd, fhd, hd = (3840, 2160), (1920, 1080), (1280, 720)
    jobs = [
        job("h264_sw_8bit_2160p25", "mp4", "h264", 8, "4:2:0", uhd, 25, "software", "avc1", crf=18),
        job("h264_nvenc_8bit_2160p25", "mp4", "h264", 8, "4:2:0", uhd, 25, "nvenc", "avc1", mbps=80),
        job("h264_sw_8bit_1080p25", "mp4", "h264", 8, "4:2:0", fhd, 25, "software", "avc1", crf=18),
        job("h264_nvenc_8bit_1080p25", "mp4", "h264", 8, "4:2:0", fhd, 25, "nvenc", "avc1", mbps=20),
        job("h264_nvenc_8bit_1080p50_retime", "mp4", "h264", 8, "4:2:0", fhd, 50, "nvenc", "avc1",
            mbps=35, retime=True),
        job("h264_either_8bit_720p25", "mp4", "h264", 8, "4:2:0", hd, 25, "either", "avc1", mbps=8),
        job("h264_sw_8bit_720p50_retime", "mp4", "h264", 8, "4:2:0", hd, 50, "software", "avc1",
            crf=20, retime=True),
        job("hevc_nvenc_8bit_2160p25", "mp4", "hevc", 8, "4:2:0", uhd, 25, "nvenc", "hvc1", mbps=50),
        job("hevc_nvenc_10bit_2160p25", "mp4", "hevc", 10, "4:2:0", uhd, 25, "nvenc", "hvc1",
            mbps=50),
        job("hevc_nvenc_10bit_1080p50_retime", "mp4", "hevc", 10, "4:2:0", fhd, 50, "nvenc", "hvc1",
            mbps=25, retime=True),
        job("av1_nvenc_10bit_2160p25", "mp4", "av1", 10, "4:2:0", uhd, 25, "nvenc", "av01", mbps=40,
            requires_ada=True),
        job("prores_422hq_10bit_2160p25", "mov", "prores_422_hq", 10, "4:2:2", uhd, 25, "software",
            "apch", profile="ProRes 422 HQ"),
        job("prores_422hq_10bit_1080p25", "mov", "prores_422_hq", 10, "4:2:2", fhd, 25, "software",
            "apch", profile="ProRes 422 HQ"),
        job("dnxhr_hqx_10bit_2160p25", "mov", "dnxhr_hqx", 10, "4:2:2", uhd, 25, "software", "AVdh",
            profile="DNxHR HQX"),
        job("dnxhr_hqx_10bit_1080p25", "mov", "dnxhr_hqx", 10, "4:2:2", fhd, 25, "software", "AVdh",
            profile="DNxHR HQX"),
        job("png16_sequence_2160p25", "image_sequence", "png", 16, "4:4:4", uhd, 25, "software",
            "png ", pattern="frame_%05d.png"),
    ]
    return {
        "schema": "kayfabe.export-matrix/1",
        "comment": "Encode jobs for the timeline in timeline.otio. Bitrates are targets, not "
                   "assertions; a run checks container, fourcc, resolution, fps and duration "
                   "(tolerance_s) of the produced file.",
        "timeline": {"width": WIDTH, "height": HEIGHT, "fps": FPS, "frames": TOTAL_FRAMES,
                     "duration_s": TOTAL_FRAMES / FPS},
        "tolerance_s": 0.5,
        "jobs": jobs,
    }


# ---------------------------------------------------------------------------------------------
# Generation, writing, checking
# ---------------------------------------------------------------------------------------------
def _json(obj) -> bytes:
    return (json.dumps(obj, indent=2, ensure_ascii=True) + "\n").encode()


def generate() -> dict[str, bytes]:
    return {
        "timeline.otio": _json(build_otio()),
        "timeline.fcpxml": build_fcpxml().encode(),
        "timeline.edl": build_edl().encode(),
        "export_matrix.json": _json(build_export_matrix()),
    }


def sha256(data: bytes) -> str:
    return hashlib.sha256(data).hexdigest()


def cmd_spec(out: Path) -> int:
    out.mkdir(parents=True, exist_ok=True)
    for name, data in generate().items():
        (out / name).write_bytes(data)
        print(f"{sha256(data)}  {name}")
    return 0


def _dur(d: dict) -> int:
    return int(d["duration"]["value"])


def _start(d: dict) -> int:
    return int(d["start_time"]["value"])


def validate_otio(doc: dict, errors: list[str]) -> list[dict]:
    """Return the V1 clip records [(name, rec_in, frames, consumed)] for cross-file checks."""
    def err(msg):
        errors.append("otio: " + msg)
    if doc.get("OTIO_SCHEMA") != "Timeline.1":
        err("root is not Timeline.1")
        return []
    stack = doc.get("tracks", {})
    if stack.get("OTIO_SCHEMA") != "Stack.1":
        err("tracks is not Stack.1")
        return []
    totals, v1 = [], []
    for tk in stack.get("children", []):
        if tk.get("OTIO_SCHEMA") != "Track.1":
            err("stack child is not Track.1")
            continue
        kids = tk["children"]
        pos = 0
        for idx, item in enumerate(kids):
            schema = item.get("OTIO_SCHEMA")
            if schema == "Transition.1":
                prev = kids[idx - 1] if idx > 0 else None
                nxt = kids[idx + 1] if idx + 1 < len(kids) else None
                if not prev or not nxt or prev.get("OTIO_SCHEMA") != "Clip.2" \
                        or nxt.get("OTIO_SCHEMA") != "Clip.2":
                    err(f"{item.get('name')}: transition is not between two clips")
                    continue
                a, b = int(item["in_offset"]["value"]), int(item["out_offset"]["value"])
                if a + b != TRANSITION_FRAMES:
                    err(f"{item.get('name')}: offsets {a}+{b} != {TRANSITION_FRAMES}")
                if _dur(prev["source_range"]) < a or _dur(nxt["source_range"]) < b:
                    err(f"{item.get('name')}: a neighbouring clip is shorter than its offset")
                continue
            if schema not in ("Clip.2", "Gap.1"):
                err(f"unexpected item {schema}")
                continue
            sr = item["source_range"]
            frames = _dur(sr)
            if schema == "Clip.2":
                ref = item["media_references"][item["active_media_reference_key"]]
                scalar = 1.0
                for e in item.get("effects", []):
                    if e.get("OTIO_SCHEMA") == "LinearTimeWarp.1":
                        scalar = e["time_scalar"]
                used = consumed_frames(frames, scalar)
                if ref.get("OTIO_SCHEMA") == "ExternalReference.1":
                    ar = ref["available_range"]
                    if _start(sr) < _start(ar) or _start(sr) + used > _start(ar) + _dur(ar):
                        err(f"{item['name']}: source range {_start(sr)}+{used} is outside "
                            f"available range {_start(ar)}+{_dur(ar)}")
                sp = item["metadata"]["kayfabe"].get("speed")
                if sp is not None:
                    if abs(sp["percent"] - scalar * 100.0) > 1e-9 or sp["source_frames"] != used \
                            or sp["timeline_frames"] != frames:
                        err(f"{item['name']}: speed metadata disagrees with the time warp")
                if tk["kind"] == "Video" and ref.get("OTIO_SCHEMA") == "ExternalReference.1":
                    v1.append(dict(name=item["name"], rec_in=pos, frames=frames, consumed=used,
                                   src_in=_start(sr)))
            pos += frames
        totals.append(pos)
    if len(set(totals)) != 1:
        err(f"track durations differ: {totals}")
    elif totals and totals[0] != doc["metadata"]["kayfabe"]["total_frames"]:
        err(f"track duration {totals[0]} != metadata total_frames")
    if sum(c["frames"] for c in v1) != (totals[0] if totals else -1):
        err("V1 clip durations do not sum to the timeline duration")
    return v1


def validate_fcpxml(text: str, v1: list[dict], errors: list[str]) -> None:
    def err(msg):
        errors.append("fcpxml: " + msg)
    try:
        root = ET.fromstring(text.split("<!DOCTYPE fcpxml>", 1)[-1])
    except ET.ParseError as e:
        err(f"does not parse: {e}")
        return
    if root.get("version") != "1.9":
        err("version is not 1.9")
    spine = root.find("./library/event/project/sequence/spine")
    if spine is None:
        err("no spine")
        return
    kids = list(spine)
    clips = [e for e in kids if e.tag == "asset-clip"]
    if len(clips) != len(v1):
        err(f"{len(clips)} spine clips, expected {len(v1)}")
        return
    pos = 0
    for el, c in zip(clips, v1):
        if el.get("name") != c["name"]:
            err(f"clip name {el.get('name')} != {c['name']}")
        if parse_fcp_time(el.get("offset")) != pos or parse_fcp_time(el.get("duration")) != c["frames"]:
            err(f"{c['name']}: offset/duration do not match the OTIO layout")
        pos += c["frames"]
    for i, e in enumerate(kids):
        if e.tag == "transition":
            if i == 0 or i + 1 >= len(kids) or kids[i - 1].tag != "asset-clip" \
                    or kids[i + 1].tag != "asset-clip":
                err("transition is not between two clips")
            elif parse_fcp_time(e.get("offset")) + TRANSITION_FRAMES // 2 != \
                    parse_fcp_time(kids[i + 1].get("offset")):
                err("transition is not centred on the cut")
    ids = {e.get("id") for e in root.find("resources")}
    for e in root.iter():
        r = e.get("ref")
        if r is not None and e.tag != "text-style" and r not in ids:
            err(f"{e.tag} references the missing resource {r}")


def validate_edl(text: str, v1: list[dict], errors: list[str]) -> None:
    def err(msg):
        errors.append("edl: " + msg)
    events = []
    for ln in text.splitlines():
        if ln[:3].isdigit():
            m = EDL_LINE.match(ln)
            if not m:
                err(f"malformed event line: {ln!r}")
                continue
            events.append(m)
    if len(events) != len(v1):
        err(f"{len(events)} events, expected {len(v1)}")
        return
    expect = TC_START_FRAMES
    for m, c in zip(events, v1):
        s_in, s_out, r_in, r_out = (parse_tc(m.group(i)) for i in (3, 4, 5, 6))
        if r_in != expect:
            err(f"event {m.group(1)}: record in {tc(r_in)} is not continuous")
        if r_out - r_in != c["frames"]:
            err(f"event {m.group(1)}: record duration differs from the clip")
        if s_out - s_in != c["consumed"] or s_in != c["src_in"]:
            err(f"event {m.group(1)}: source range differs from the clip")
        expect = r_out
    if expect != TC_START_FRAMES + TOTAL_FRAMES:
        err("record timeline does not end at the total duration")


def validate_matrix(doc: dict, errors: list[str]) -> None:
    def err(msg):
        errors.append("matrix: " + msg)
    keys = {"name", "container", "codec", "bit_depth", "chroma", "resolution", "fps", "hardware",
            "expected_duration_s", "expected_video_codec_fourcc"}
    names = set()
    for j in doc.get("jobs", []):
        miss = keys - set(j)
        if miss:
            err(f"{j.get('name')}: missing {sorted(miss)}")
            continue
        if j["name"] in names:
            err(f"duplicate job {j['name']}")
        names.add(j["name"])
        if j["hardware"] not in ("nvenc", "software", "either"):
            err(f"{j['name']}: bad hardware {j['hardware']}")
        if j["crf"] is None and j["bitrate_mbps"] is None and j.get("profile") is None \
                and j["container"] != "image_sequence":
            err(f"{j['name']}: no crf, bitrate or profile")
        if not re.fullmatch(r"\d+x\d+", j["resolution"]):
            err(f"{j['name']}: bad resolution")
    if not names:
        err("no jobs")


def cmd_check(out: Path) -> int:
    errors: list[str] = []
    expected = generate()
    texts: dict[str, bytes] = {}
    for name, data in expected.items():
        p = out / name
        if not p.is_file():
            errors.append(f"{name}: missing")
            continue
        got = p.read_bytes()
        texts[name] = got
        if sha256(got) != sha256(data):
            errors.append(f"{name}: sha256 {sha256(got)} differs from the regenerated {sha256(data)}")
    v1: list[dict] = []
    try:
        if "timeline.otio" in texts:
            v1 = validate_otio(json.loads(texts["timeline.otio"]), errors)
        if "timeline.fcpxml" in texts:
            validate_fcpxml(texts["timeline.fcpxml"].decode(), v1, errors)
        if "timeline.edl" in texts:
            validate_edl(texts["timeline.edl"].decode(), v1, errors)
        if "export_matrix.json" in texts:
            validate_matrix(json.loads(texts["export_matrix.json"]), errors)
    except (ValueError, KeyError, TypeError, UnicodeDecodeError) as e:
        errors.append(f"unreadable spec content: {type(e).__name__}: {e}")
    for e in errors:
        print("FAIL " + e)
    print("check: " + ("FAILED" if errors else "ok"))
    return 1 if errors else 0


def cmd_make_sources(dest: Path, assets: Path | None) -> int:
    ffmpeg = shutil.which("ffmpeg")
    if ffmpeg is None:
        print("make-sources: ffmpeg not found on PATH", file=sys.stderr)
        return 2
    dest.mkdir(parents=True, exist_ok=True)
    lines = []
    for src in synthetic_sources():
        out = dest / src["output"]
        out.parent.mkdir(parents=True, exist_ok=True)
        argv = [ffmpeg] + src["argv"][1:-1] + [str(out)]
        print("run:", " ".join(src["argv"]))
        subprocess.run(argv, check=True)
        lines.append(f"{hashlib.sha256(out.read_bytes()).hexdigest()}  {src['output']}")
    (dest / "sources.sha256").write_text("\n".join(sorted(lines, key=lambda s: s.split()[1])) + "\n")
    print("\n".join(lines))
    if assets is not None:
        manifest = json.loads((Path(__file__).with_name("test_assets.json")).read_text())
        pinned = {a["name"]: a for a in manifest["assets"]}
        bad = 0
        for s in SOURCES.values():
            if s["kind"] != "pinned":
                continue
            p, a = assets / s["path"], pinned.get(s["path"])
            if a is None:
                print(f"assets: {s['path']} is not in test_assets.json")
                bad += 1
            elif not p.is_file() or hashlib.sha256(p.read_bytes()).hexdigest() != a["sha256"]:
                print(f"assets: {s['path']} missing or differs from the pinned sha256")
                bad += 1
            else:
                print(f"assets: {s['path']} ok")
        return 1 if bad else 0
    return 0


def main(argv: list[str] | None = None) -> int:
    ap = argparse.ArgumentParser(description=__doc__.split("\n")[0])
    sub = ap.add_subparsers(dest="cmd", required=True)
    p = sub.add_parser("spec")
    p.add_argument("--out", required=True, type=Path)
    p = sub.add_parser("check")
    p.add_argument("--out", required=True, type=Path)
    p = sub.add_parser("make-sources")
    p.add_argument("--dest", required=True, type=Path)
    p.add_argument("--assets", type=Path)
    a = ap.parse_args(argv)
    if a.cmd == "spec":
        return cmd_spec(a.out)
    if a.cmd == "check":
        return cmd_check(a.out)
    return cmd_make_sources(a.dest, a.assets)


if __name__ == "__main__":
    sys.exit(main())
