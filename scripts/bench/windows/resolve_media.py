#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later
"""Host-side media facts for the Resolve GPU lane: assets, FCPXML project generation, probing of rendered files.

STATUS: DESIGN-ONLY, 2026-11-16. GPU-free tested (scripts/ci/test_resolve_gpu.py). Stdlib only; uses ffprobe/ffmpeg when
they are on PATH and a built-in ISO-BMFF / PNG reader otherwise, so a box without ffmpeg still verifies the renders.

Downloaded files are untrusted data: this module hashes them and parses container headers with bounds; it never executes
them. ffprobe/ffmpeg are run with fixed argv and a timeout, never through a shell.
"""

from __future__ import annotations

import hashlib
import json
import shutil
import struct
import subprocess
import urllib.request
import zlib
from fractions import Fraction
from pathlib import Path
from typing import Optional

HERE = Path(__file__).resolve().parent
MANIFEST = HERE / "resolve_assets.json"
USER_AGENT = "kayfabe-resolve-assets/1 (stdlib urllib)"
MAX_FILE = 100 * 1024 * 1024
# Public mirror of the pinned files (same names, same sha256). Tried after the upstream URL; the pin decides, not the source.
MIRROR = "https://bundles.kayfabe.dev/test-assets/20261116/"
LICENSES = {"CC0-1.0", "Public-Domain", "CC-BY-3.0", "CC-BY-4.0", "CC-BY-SA-3.0", "CC-BY-SA-4.0"}


class MediaError(Exception):
    pass


# ------------------------------------------------------------------------------------------------ assets

def load_manifest(path: Path = MANIFEST) -> list[dict]:
    m = json.loads(Path(path).read_text())
    if m.get("schema") != "kayfabe.test-assets/1":
        raise MediaError("manifest schema")
    seen = set()
    for a in m["assets"]:
        name = a["name"]
        if name in seen or name.startswith("/") or ".." in name.split("/") or "\\" in name:
            raise MediaError(f"unsafe or duplicate asset name {name!r}")
        seen.add(name)
        if a["license"] not in LICENSES:
            raise MediaError(f"{name}: licence {a['license']!r} not admitted")
        if not (a["url"].startswith("https://") and a["license_url"].startswith("https://")):
            raise MediaError(f"{name}: https only")
        if not (isinstance(a.get("sha256"), str) and len(a["sha256"]) == 64 and isinstance(a.get("size"), int)):
            raise MediaError(f"{name}: sha256 and size must be pinned")
    return m["assets"]


def sha256_file(path: Path) -> str:
    h = hashlib.sha256()
    with open(path, "rb") as f:
        for chunk in iter(lambda: f.read(1 << 20), b""):
            h.update(chunk)
    return h.hexdigest()


def verify_asset(path: Path, a: dict) -> None:
    if not path.is_file():
        raise MediaError(f"{a['name']}: missing")
    if path.stat().st_size != a["size"]:
        raise MediaError(f"{a['name']}: size {path.stat().st_size} != pinned {a['size']}")
    if sha256_file(path) != a["sha256"]:
        raise MediaError(f"{a['name']}: sha256 differs from the pin")


def _download(url: str, part: Path, a: dict, opener) -> None:
    req = urllib.request.Request(url, headers={"User-Agent": USER_AGENT})
    with opener(req, timeout=120) as r:
        if not r.geturl().startswith("https://"):
            raise MediaError(f"{a['name']}: redirected away from https")
        n = 0
        with open(part, "wb") as f:
            while True:
                b = r.read(1 << 20)
                if not b:
                    break
                n += len(b)
                if n > min(MAX_FILE, a["size"] + 1):
                    raise MediaError(f"{a['name']}: larger than the pinned size")
                f.write(b)
    if n != a["size"] or sha256_file(part) != a["sha256"]:
        raise MediaError(f"{a['name']}: {url} does not match the pin")


def fetch_assets(dest: Path, manifest: Path = MANIFEST, opener=urllib.request.urlopen) -> list[Path]:
    """Download every asset from its source URL (https, no redirect to http), refuse a size or hash that differs from the pin."""
    out = []
    for a in load_manifest(manifest):
        target = Path(dest) / a["name"]
        target.parent.mkdir(parents=True, exist_ok=True)
        if target.is_file():
            try:
                verify_asset(target, a)
                out.append(target)
                continue
            except MediaError:
                target.unlink()
        part = target.with_suffix(target.suffix + ".part")
        last = None
        for url in (a["url"], MIRROR + a["name"]):
            try:
                _download(url, part, a, opener)
                break
            except (MediaError, OSError) as exc:  # urllib errors are OSError subclasses
                last = exc
                part.unlink(missing_ok=True)
        else:
            raise MediaError(f"{a['name']}: no source worked (last: {last})")
        part.replace(target)
        verify_asset(target, a)
        out.append(target)
    return out


def write_licenses(dest: Path, manifest: Path = MANIFEST) -> Path:
    lines = ["Open media for the Resolve GPU lane. Each file keeps its own licence.", ""]
    for a in load_manifest(manifest):
        lines += [f"{a['name']}", f"  licence:     {a['license']} ({a['license_url']})", f"  attribution: {a['attribution']}",
                  f"  source:      {a['source_page']}", f"  sha256:      {a['sha256']}  size {a['size']}", ""]
    p = Path(dest) / "LICENSES.txt"
    p.write_text("\n".join(lines))
    return p


# ------------------------------------------------------------------------------------------------ FCPXML project

FPS = 24


def _t(frames: int, fps: int = FPS) -> str:
    return f"{frames * 100}/{fps * 100}s"


def build_fcpxml(clips: list[dict], width: int = 3840, height: int = 2160, fps: int = FPS, name: str = "kf_resolve_gpu") -> str:
    """FCPXML 1.8 with one spine of clips. Each clip: {name, path (guest file URL path, e.g. C:/kfmedia/x.mp4), kind: video|still,
    start (source frame), frames (timeline frames), src_frames, w, h, fps (source), scale (float), speed (retime factor)}.
    Only what Resolve keeps on FCPXML import is used: cuts, transform scale, speed (conform) and still durations."""
    from xml.sax.saxutils import quoteattr
    assets, spine, offset = [], [], 0
    for i, c in enumerate(clips, 1):
        rid = f"r{i + 1}"
        uri = "file:///" + c["path"].replace("\\", "/").replace(" ", "%20")
        dur = c.get("src_frames", c["frames"])
        sfps = c.get("fps", fps)
        assets.append(
            f'<asset id="{rid}" name={quoteattr(c["name"])} src={quoteattr(uri)} start="0s" '
            f'duration="{dur * 100}/{sfps * 100}s" hasVideo="1" format="r1"/>'
        )
        start = c.get("start", 0)
        speed = c.get("speed", 1.0)
        retime = ""
        if speed != 1.0:
            sp = Fraction(speed).limit_denominator(100)
            retime = (f'<timeMap><timept time="0s" value="0s" interp="linear"/>'
                      f'<timept time="{_t(c["frames"])}" value="{_t(int(c["frames"] * sp))}" interp="linear"/></timeMap>')
        scale = c.get("scale", 1.0)
        xf = f'<adjust-transform scale="{scale:.4f} {scale:.4f}"/>' if scale != 1.0 else ""
        spine.append(
            f'<asset-clip ref="{rid}" name={quoteattr(c["name"])} offset="{_t(offset, fps)}" start="{_t(start, sfps)}" '
            f'duration="{_t(c["frames"], fps)}" format="r1">{retime}{xf}</asset-clip>'
        )
        offset += c["frames"]
    return (
        '<?xml version="1.0" encoding="UTF-8"?>\n<!DOCTYPE fcpxml>\n<fcpxml version="1.8">\n<resources>\n'
        f'<format id="r1" name="FFVideoFormat{height}p{fps}" frameDuration="100/{fps * 100}s" width="{width}" height="{height}"/>\n'
        + "\n".join(assets)
        + f'\n</resources>\n<library><event name="kf_event"><project name={quoteattr(name)}>'
        f'<sequence format="r1" duration="{_t(offset, fps)}" tcStart="0s" tcFormat="NDF"><spine>\n'
        + "\n".join(spine)
        + "\n</spine></sequence></project></event></library>\n</fcpxml>\n"
    )


# ------------------------------------------------------------------------------------------------ probing

def _run(cmd: list[str], timeout: int = 120) -> subprocess.CompletedProcess:
    return subprocess.run(cmd, capture_output=True, text=True, timeout=timeout, stdin=subprocess.DEVNULL)


def have_ffprobe() -> bool:
    return shutil.which("ffprobe") is not None


def ffprobe(path: Path) -> dict:
    p = _run(["ffprobe", "-v", "error", "-count_frames", "-select_streams", "v:0", "-show_entries",
              "stream=codec_name,codec_tag_string,profile,width,height,pix_fmt,nb_read_frames,nb_frames,avg_frame_rate,duration:format=duration,format_name",
              "-of", "json", str(path)], timeout=300)
    if p.returncode:
        raise MediaError(f"ffprobe failed: {p.stderr.strip()[:200]}")
    j = json.loads(p.stdout)
    s = (j.get("streams") or [{}])[0]
    if not s:
        raise MediaError("no video stream")
    frames = s.get("nb_read_frames") or s.get("nb_frames")
    dur = s.get("duration") or j.get("format", {}).get("duration")
    return {"reader": "ffprobe", "codec": s.get("codec_name"), "tag": s.get("codec_tag_string"), "profile": s.get("profile"),
            "width": int(s["width"]), "height": int(s["height"]), "pix_fmt": s.get("pix_fmt"),
            "frames": int(frames) if frames not in (None, "N/A") else None,
            "duration": float(dur) if dur not in (None, "N/A") else None, "container": j.get("format", {}).get("format_name")}


def _boxes(data: bytes, start: int, end: int):
    pos = start
    while pos + 8 <= end:
        size, typ = struct.unpack(">I4s", data[pos:pos + 8])
        hdr = 8
        if size == 1:
            size = struct.unpack(">Q", data[pos + 8:pos + 16])[0]
            hdr = 16
        elif size == 0:
            size = end - pos
        if size < hdr or pos + size > end:
            raise MediaError("bad box size")
        yield typ.decode("latin1"), pos + hdr, pos + size
        pos += size


def probe_isobmff(path: Path) -> dict:
    """Minimal MP4/MOV reader: video track codec fourcc, size, sample count (stts), duration (mdhd). moov must be < 64 MiB."""
    size = path.stat().st_size
    with open(path, "rb") as f:
        pos = 0
        moov = None
        while pos + 8 <= size:
            f.seek(pos)
            h = f.read(16)
            bsz, typ = struct.unpack(">I4s", h[:8])
            hdr = 8
            if bsz == 1:
                bsz = struct.unpack(">Q", h[8:16])[0]
                hdr = 16
            elif bsz == 0:
                bsz = size - pos
            if bsz < hdr:
                raise MediaError("bad top-level box")
            if typ == b"moov":
                if bsz > 64 << 20:
                    raise MediaError("moov too large")
                f.seek(pos)
                moov = f.read(bsz)
                break
            pos += bsz
    if moov is None:
        raise MediaError("no moov box (unfinished or not an MP4/MOV file)")
    out = {"reader": "isobmff", "codec": None, "width": None, "height": None, "frames": None, "duration": None}

    def track(s, e):
        t = {}
        def walk(s2, e2):
            for typ, a, b in _boxes(moov, s2, e2):
                if typ in ("mdia", "minf", "stbl"):
                    walk(a, b)
                elif typ == "hdlr":
                    t["hdlr"] = moov[a + 8:a + 12].decode("latin1")
                elif typ == "mdhd":
                    ver = moov[a]
                    ts, dur = struct.unpack(">II", moov[a + 12:a + 20]) if ver == 0 else (
                        struct.unpack(">I", moov[a + 20:a + 24])[0], struct.unpack(">Q", moov[a + 24:a + 32])[0])
                    t["duration"] = dur / ts if ts else None
                elif typ == "stsd":
                    t["codec"] = moov[a + 12:a + 16].decode("latin1")
                    t["width"], t["height"] = struct.unpack(">HH", moov[a + 40:a + 44]) if b - a >= 44 else (None, None)
                elif typ == "stts":
                    n = struct.unpack(">I", moov[a + 4:a + 8])[0]
                    t["frames"] = sum(struct.unpack(">I", moov[a + 8 + 8 * i:a + 12 + 8 * i])[0] for i in range(min(n, 100000)))
        walk(s, e)
        return t

    for typ, a, b in _boxes(moov, 8, len(moov)):
        if typ == "trak":
            t = track(a, b)
            if t.get("hdlr") == "vide":
                out.update({k: t.get(k) for k in ("codec", "width", "height", "frames", "duration")})
                break
    if not out["codec"]:
        raise MediaError("no video sample entry")
    return out


def png_info(path: Path) -> dict:
    """Dimensions, bit depth and a checksum of the decoded pixels of a PNG (non-interlaced, 8 bit, gray/RGB/RGBA)."""
    data = path.read_bytes()
    if data[:8] != b"\x89PNG\r\n\x1a\n":
        raise MediaError("not a PNG")
    pos, idat, w = 8, [], None
    while pos + 8 <= len(data):
        ln, typ = struct.unpack(">I4s", data[pos:pos + 8])
        body = data[pos + 8:pos + 8 + ln]
        if typ == b"IHDR":
            w, h, depth, ctype, _, _, inter = struct.unpack(">IIBBBBB", body)
        elif typ == b"IDAT":
            idat.append(body)
        pos += 12 + ln
    if w is None or not idat:
        raise MediaError("truncated PNG")
    info = {"reader": "png", "width": w, "height": h, "bit_depth": depth, "color_type": ctype, "frames": 1}
    ch = {0: 1, 2: 3, 4: 2, 6: 4}.get(ctype)
    if depth == 8 and ch and not inter:
        raw = zlib.decompress(b"".join(idat))
        stride = w * ch
        prev = bytearray(stride)
        lum_sum = lum_sq = n = 0
        h2 = hashlib.sha256()
        for y in range(h):
            row = raw[y * (stride + 1): (y + 1) * (stride + 1)]
            ft, line = row[0], bytearray(row[1:])
            for i in range(stride):
                a = line[i - ch] if i >= ch else 0
                b = prev[i]
                c = prev[i - ch] if i >= ch else 0
                if ft == 1:
                    line[i] = (line[i] + a) & 255
                elif ft == 2:
                    line[i] = (line[i] + b) & 255
                elif ft == 3:
                    line[i] = (line[i] + ((a + b) >> 1)) & 255
                elif ft == 4:
                    pa, pb, pc = abs(b - c), abs(a - c), abs(a + b - 2 * c)
                    pr = a if pa <= pb and pa <= pc else (b if pb <= pc else c)
                    line[i] = (line[i] + pr) & 255
            h2.update(bytes(line))
            if y % 16 == 0:
                for x in range(0, w, 8):
                    v = line[x * ch] if ch in (1, 2) else (line[x * ch] * 299 + line[x * ch + 1] * 587 + line[x * ch + 2] * 114) // 1000
                    lum_sum += v
                    lum_sq += v * v
                    n += 1
            prev = line
        mean = lum_sum / n
        info["pixel_sha256"] = h2.hexdigest()
        info["luma_mean"] = round(mean, 2)
        info["luma_std"] = round(max(lum_sq / n - mean * mean, 0) ** 0.5, 2)
    return info


def probe(path: Path) -> dict:
    """Facts about one rendered file. ffprobe when present; else the built-in readers (MP4/MOV/PNG)."""
    path = Path(path)
    if path.stat().st_size == 0:
        raise MediaError("empty file")
    info = None
    if have_ffprobe():
        info = ffprobe(path)
    elif path.suffix.lower() in (".mp4", ".mov", ".m4v"):
        info = probe_isobmff(path)
    elif path.suffix.lower() == ".png":
        info = png_info(path)
    else:
        raise MediaError(f"no reader for {path.suffix} without ffprobe")
    if path.suffix.lower() == ".png" and "pixel_sha256" not in info:
        info.update({k: v for k, v in png_info(path).items() if k in ("pixel_sha256", "luma_mean", "luma_std")})
    info["bytes"] = path.stat().st_size
    info["sha256"] = sha256_file(path)
    return info


def frame_stats(path: Path, frame: int = 0) -> Optional[dict]:
    """Mean/std of the luma of one decoded frame (ffmpeg gray8 at 64x36), the pixels' sha256 and whether it is not black/flat.
    None without ffmpeg."""
    if not shutil.which("ffmpeg"):
        return None
    p = subprocess.run(["ffmpeg", "-v", "error", "-i", str(path), "-vf", f"select=eq(n\\,{frame}),scale=64:36", "-frames:v", "1",
                        "-f", "rawvideo", "-pix_fmt", "gray", "-"], capture_output=True, timeout=120, stdin=subprocess.DEVNULL)
    if p.returncode or len(p.stdout) != 64 * 36:
        return None
    v = p.stdout
    mean = sum(v) / len(v)
    std = (sum((x - mean) ** 2 for x in v) / len(v)) ** 0.5
    return {"frame": frame, "luma_mean": round(mean, 2), "luma_std": round(std, 2), "pixel_sha256": hashlib.sha256(v).hexdigest()}
