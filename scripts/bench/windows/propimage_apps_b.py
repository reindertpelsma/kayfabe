#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later
"""Application checks for the second proprietary image (creative applications), plugged into propimage_check.py with `--app-set b`.

STATUS: DESIGN-ONLY, 2026-11-16 until the first GPU run is recorded; GPU-free tests in scripts/ci/test_propimage_apps_b.py.

Same transport and evidence rules as the first image's checks: a guest script runs in the interactive session (here at the user's
normal integrity level: the applications' automation interfaces refuse an elevated caller), writes `result.json` plus the output
files it named into `C:\\kfapps\\<run-id>\\<app>`, the host pulls only those names (bounded, safe names) and grades them from the
file contents. A window is never a PASS by itself: rows that only prove a launch are PASS_LIMITED and say so. Every row that is
meant to exercise the GPU runs inside `gpu_assert.assert_gpu_used`; an idle engine FAILS the row. No screenshot is taken or
stored: the applications' windows show account details. Data (open-licence test assets) comes from a read-only volume labelled
KFDATA (scripts/bench/assets).

Verdicts: PASS, PASS_LIMITED (launch/main window only, or a diagnostic output; the row says which), FAIL, INCONCLUSIVE, NOT_RUN,
HARNESS (transport lost; never an application failure).
"""
import json
import math
import os
import re
import subprocess
import sys
import time
import uuid
from pathlib import Path

HERE = Path(__file__).resolve().parent
sys.path.insert(0, str(HERE))
import gpu_assert  # noqa: E402
import media_check  # noqa: E402

APPS_DIR = HERE / "apps_b"
GUEST_DIR = r"C:\kfapps"
SAFE_FILE = re.compile(r"[A-Za-z0-9][A-Za-z0-9_.-]{0,63}")
MAX_PULL = 96 * 1024 * 1024

# Opaque expectations for the open assets in the data volume (names as in scripts/bench/assets/test_assets.json).
EXPECT = {
    "photo": {"name": "ktiponissi_dirfys_4480x2928.jpg", "w": 4480, "h": 2928},
    "svg": {"name": "gradient_3d_colored.svg"},
    "video": {"name": "sintel_trailer_1080p.mp4"},
    "pdf": {"name": "apollo11_anomaly_report_3.pdf", "pages": 16},
}

# lane: gate = must pass on kf3 (strict); discovery = exotic or heavy, allowed to fail and crash (the failures are the product; each runs in a fresh clone).
# name -> timeout (s), script, GPU engines that must be busy during the operation (() = informational sampling only)
CHECKS = {
    "gpudiag": {"timeout": 240, "script": "sniffers.ps1", "gpu": (), "lane": "gate", "path": "NVIDIA driver/CUDA enumeration by the applications own sniffers"},
    "photoshop": {"timeout": 1500, "script": "photoshop.ps1", "gpu": ("gpu",), "lane": "discovery", "path": "OpenGL/D3D12 canvas, GPU filters (smart sharpen), 16-bit/wide-gamut, large VRAM allocation (8K, 100 layers)"},
    "illustrator": {"timeout": 480, "script": "illustrator.ps1", "gpu": (), "lane": "gate", "path": "D3D11 canvas, large path counts, raster export"},
    "aftereffects": {"timeout": 1500, "script": "aftereffects.ps1", "gpu": ("gpu",), "lane": "discovery", "path": "CUDA/OptiX effects, 3D layers, 16-bit float, 4K render, particle effects"},
    "indesign": {"timeout": 480, "script": "indesign.ps1", "gpu": (), "lane": "gate", "path": "D3D11/Direct2D page rendering, PDF/JPEG export"},
    "illustratorsample": {"timeout": 480, "script": "illustrator_sample.ps1", "gpu": (), "lane": "gate", "path": "D3D11 canvas on shipped art"},
    "indesignsample": {"timeout": 480, "script": "indesign_sample.ps1", "gpu": (), "lane": "gate", "path": "D3D11/Direct2D on shipped documents"},
    "animatesample": {"timeout": 420, "script": "animate_sample.ps1", "gpu": (), "lane": "gate", "path": "OpenGL/D3D11 stage on a shipped document"},
    "photoshopsample": {"timeout": 900, "script": "photoshop_sample.ps1", "gpu": ("gpu",), "lane": "discovery", "path": "GPU filters on shipped 4K+ photo, 3840x2160 template"},
    "photoshopprojects": {"timeout": 900, "script": "photoshop_projects.ps1", "gpu": (), "lane": "discovery", "path": "open-licence PSD corpus: layer/colour-mode decoding and GPU canvas on foreign files"},
    "indesignprojects": {"timeout": 600, "script": "indesign_projects.ps1", "gpu": (), "lane": "discovery", "path": "open-licence INDD corpus open + PDF export"},
    "animateprojects": {"timeout": 600, "script": "animate_projects.ps1", "gpu": (), "lane": "discovery", "path": "open-licence FLA corpus open + stage export"},
    "aeprojects": {"timeout": 900, "script": "aeprojects.ps1", "gpu": (), "lane": "discovery", "path": "open-licence AEP corpus open (project load, GPU capability scan)"},
    "acrobat": {"timeout": 360, "script": "acrobat.ps1", "gpu": (), "lane": "gate", "path": "Direct2D page rendering"},
    "animate": {"timeout": 360, "script": "animate.ps1", "gpu": (), "lane": "gate", "path": "OpenGL/D3D11 stage, tween export"},
    "premiere": {"timeout": 420, "script": "launch.ps1", "gpu": (), "lane": "gate", "path": "Mercury engine start, CUDA/D3D12 init (launch only)"},
    "mediaencoder": {"timeout": 600, "script": "mediaencoder.ps1", "gpu": ("enc",), "lane": "discovery", "path": "NVENC H.264 via the After Effects hand-off"},
    "lightroom": {"timeout": 600, "script": "lightroom.ps1", "gpu": ("gpu",), "lane": "discovery", "path": "develop pipeline GPU processing on 12/24 MP raws"},
    "dimension": {"timeout": 420, "script": "launch.ps1", "gpu": (), "lane": "gate", "path": "OpenGL/OptiX init (launch only)"},
    "uxp": {"timeout": 300, "script": "launch.ps1", "gpu": (), "lane": "gate", "path": "UXP tooling start (launch only)"},
}
EMBED = {"aftereffects": {"AE_MAKE": "ae_make.jsx"}, "mediaencoder": {"AE_MAKE": "ae_make.jsx"}, "lightroom": {"LR_INFO": "lrplugin/kf.lrdevplugin/Info.lua", "LR_INIT": "lrplugin/kf.lrdevplugin/Init.lua"}}
LAUNCH_ONLY = ("premiere", "dimension", "uxp")


def render_check(name, run_id):
    """The whole guest script for one row: the run directory, the app name, the shared library, then the app's script."""
    if name not in CHECKS or not re.fullmatch(r"[0-9a-f]{32}", run_id):
        raise ValueError("bad row")
    lib = (APPS_DIR / "lib.ps1").read_text()
    body = (APPS_DIR / CHECKS[name]["script"]).read_text().replace(". 'C:\\kfhw\\lib.ps1'\n", "")
    head = "$D = '%s\\%s\\%s'\n$APPNAME = '%s'\n" % (GUEST_DIR, run_id, name, name)
    for var, rel in EMBED.get(name, {}).items():      # text files the script writes into the guest, carried as single-quoted here-strings
        text = (APPS_DIR / rel).read_text()
        if "\n'@" in text:
            raise ValueError("embedded file would end its here-string")
        head += "$%s = @'\n%s\n'@\n" % (var, text.rstrip("\n"))
    return head + lib + "\n" + body


PARSE_CHECK = (
    "$e=$null;$t=$null;[void][System.Management.Automation.Language.Parser]::ParseFile('{path}',[ref]$t,[ref]$e)\n"
    "if($e.Count){{ $e | Select-Object -First 3 | ForEach-Object {{ '{{0}}:{{1}} {{2}}' -f $_.Extent.StartLineNumber,$_.Extent.StartColumnNumber,$_.Message }} }} else {{ 'parse-ok' }}\n"
)


def limited_task(name, user="kfimage"):
    """Like propimage_check.task_wrapper, at the user's normal integrity level (no -RunLevel Highest)."""
    if not re.fullmatch(r"[a-z0-9_]+", name) or not re.fullmatch(r"[A-Za-z][A-Za-z0-9_-]{0,19}", user):
        raise ValueError("bad task")
    return (
        f"$n='kf_{name}'; Unregister-ScheduledTask -TaskName $n -Confirm:$false -ErrorAction SilentlyContinue\n"
        f"$a=New-ScheduledTaskAction -Execute powershell.exe -Argument '-NoProfile -WindowStyle Hidden -ExecutionPolicy Bypass -File C:\\kfhw\\{name}.ps1'\n"
        f"$p=New-ScheduledTaskPrincipal -UserId {user} -LogonType Interactive -RunLevel Limited\n"
        "Register-ScheduledTask -TaskName $n -Action $a -Principal $p | Out-Null; Start-ScheduledTask -TaskName $n; 'started'\n"
    )


def _interactive(guest, name, body):
    import tempfile
    with tempfile.TemporaryDirectory(prefix="kf-interactive-") as td:
        tmp = Path(td) / (name + ".ps1")
        tmp.write_text(body)
        prep = guest.ps(r"New-Item -Force -ItemType Directory C:\kfhw | Out-Null")
        if prep.returncode:
            return prep
        sent = guest.scp(str(tmp), "C:/kfhw/" + name + ".ps1", True)
        if sent.returncode:
            return sent
        chk = guest.ps(PARSE_CHECK.format(path="C:\\kfhw\\" + name + ".ps1"))
        if chk.returncode or "parse-ok" not in chk.stdout:
            raise ValueError("guest script does not parse: " + (chk.stdout + chk.stderr).strip()[:300])
        return guest.ps(limited_task(name, guest.user))


def _json(path):
    return json.loads(Path(path).read_text(encoding="utf-8-sig"))


def run_check(name, out, guest, sleep=time.sleep, clock=time.monotonic, **_ignored):
    """One row. Same contract as propimage_check.run_check: returns the verdict row and writes verdict.json."""
    run_id = uuid.uuid4().hex
    remote = GUEST_DIR.replace("\\", "/") + "/" + run_id + "/" + name
    d = Path(out) / name / run_id
    d.mkdir(parents=True)
    task = "app_" + run_id
    t0 = clock()
    gpu_record = None

    def operation():
        p = _interactive(guest, task, render_check(name, run_id))
        if p.returncode:
            raise ValueError("Task start failed")
        while clock() - t0 < CHECKS[name]["timeout"]:
            q = guest.ps(f"if(Test-Path '{remote}/result.json'){{'ready'}}")
            if q.returncode == 0 and q.stdout.strip() == "ready":
                return True
            if q.returncode:
                raise ValueError("Guest transport lost; not an app failure")
            sleep(5)
        raise ValueError("Task timeout; no verdict")

    try:
        try:
            _, gpu_record = gpu_assert.assert_gpu_used(guest, operation, require=CHECKS[name]["gpu"], out_dir=d, name=name, sleep=sleep)
        except ValueError:
            raise
        except RuntimeError as exc:                      # the sampler could not start: the row says so, the operation still ran below
            gpu_record = {"verdict": "FAIL", "reasons": ["sampler: %s" % exc], "required": list(CHECKS[name]["gpu"]), "samples": 0}
            operation()
        for f in ("result.json", "steps.log"):
            guest.scp(remote + "/" + f, str(d / f), False)
        res = _json(d / "result.json")
        names = [o.get("file") for o in res.get("outputs", [])] + ["sniffer.txt", "ps_progress.txt"] + [f"sniffer_{a}.txt" for a in ("premiere", "aftereffects", "mediaencoder", "photoshop")]
        for fn in names:
            if not isinstance(fn, str) or not SAFE_FILE.fullmatch(fn):
                continue
            q = guest.ps(f"if(Test-Path '{remote}/{fn}'){{$f=Get-Item '{remote}/{fn}';if($f.Length -le {MAX_PULL}){{'bounded'}}}}")
            if q.returncode == 0 and q.stdout.strip() == "bounded":
                if guest.scp(remote + "/" + fn, str(d / fn), False).returncode:
                    raise ValueError("Artifact transfer failed")
        row = verdict(name, run_id, d, gpu_record)
    except (ValueError, TypeError, OSError, subprocess.TimeoutExpired) as exc:
        row = {"class": "harness", "app": name, "ok": False, "evidence": "NONE", "error": str(exc)}
    finally:
        try:
            guest.ps(f"Stop-ScheduledTask -TaskName kf_{task} -ErrorAction SilentlyContinue; Unregister-ScheduledTask -TaskName kf_{task} -Confirm:$false -ErrorAction SilentlyContinue")
        except (OSError, subprocess.TimeoutExpired):
            pass
    (d / "verdict.json").write_text(json.dumps(row, indent=2))
    return row


# ----------------------------------------------------------------------------------------------------- grading
def _finite(*v):
    return all(type(x) in (int, float) and math.isfinite(x) for x in v)


def _png(directory, name, w=None, h=None, min_std=4.0, min_distinct=16):
    i = media_check.png_info(directory / name)
    if (w and i["width"] != w) or (h and i["height"] != h):
        raise ValueError("%s is %dx%d, expected %sx%s" % (name, i["width"], i["height"], w, h))
    if i["decoded"] and (i["std"] < min_std or i["distinct"] < min_distinct):
        raise ValueError("%s looks blank or flat (std %.2f, %d distinct levels)" % (name, i["std"], i["distinct"]))
    return i


def _outputs_fresh(r, directory, names):
    """Every named output was pulled, has the byte length the guest recorded, and was written during the run."""
    got = {o["file"]: o for o in r.get("outputs", []) if isinstance(o, dict)}
    for n in names:
        o = got.get(n)
        if not o:
            raise ValueError("guest did not report %s" % n)
        p = directory / n
        if not p.is_file() or p.stat().st_size != o.get("bytes") or o["bytes"] <= 0:
            raise ValueError("%s missing or length mismatch" % n)
        if not _finite(o.get("modified")) or not r["started"] <= o["modified"] <= r["ended"] + 1:
            raise ValueError("%s was not written during this run" % n)


PS_FEATURES = ("canvas8k", "16bit", "iccP3", "layers100", "adjustment_layers", "smart_object", "smart_sharpen", "brush_strokes")


def grade_photoshop(r, d):
    js = r.get("js") or {}
    if js.get("w0") != str(EXPECT["photo"]["w"]) or js.get("h0") != str(EXPECT["photo"]["h"]) or r.get("source") != EXPECT["photo"]["name"]:
        raise ValueError("opened %s %sx%s, expected the %s asset" % (r.get("source"), js.get("w0"), js.get("h0"), EXPECT["photo"]["name"]))
    feats = set(str(js.get("features", "")).split(","))
    for need in ("canvas8k", "layers100", "smart_sharpen"):
        if need not in feats:
            raise ValueError("required feature %s did not run (features: %s; layer error %s)" % (need, ",".join(sorted(feats))[:80], js.get("err_layer", "-")))
    if int(js.get("blend_modes", 0)) < 6:
        raise ValueError("only %s distinct blend modes used" % js.get("blend_modes"))
    if len(feats & set(PS_FEATURES)) < 5:
        raise ValueError("fewer than 5 of %d feature steps ran (%s)" % (len(PS_FEATURES), ",".join(sorted(feats))))
    _outputs_fresh(r, d, ("out.png", "out.psd"))
    w1, h1 = int(js["w1"]), int(js["h1"])
    if w1 != 2048:
        raise ValueError("final width %s" % w1)
    p = _png(d, "out.png", w1, h1)
    s = media_check.psd_info(d / "out.psd")
    if (s["width"], s["height"]) != (w1, h1):
        raise ValueError("PSD is %dx%d" % (s["width"], s["height"]))
    return ("8K-class canvas, %s layers with %s blend modes, features %s, saved %dx%d PNG (std %.1f) and PSD; %s ms"
            % (js.get("layers"), js.get("blend_modes"), ",".join(sorted(feats & set(PS_FEATURES))), w1, h1, p["std"], js.get("ms"))), js.get("gpuinfo", "")


def grade_illustrator(r, d):
    js = r.get("js") or {}
    _outputs_fresh(r, d, ("out.png", "out.pdf"))
    w0, h0 = int(float(js["w0"])), int(float(js["h0"]))
    p = _png(d, "out.png", w0 * 4, h0 * 4)
    pdf = media_check.pdf_info(d / "out.pdf")
    if pdf["pages"] < 1:
        raise ValueError("PDF without pages")
    if int(js.get("paths", 0)) < 400:
        raise ValueError("generated art missing (%s paths)" % js.get("paths"))
    return "opened %s (%dx%d), added 400 shapes, exported %dx%d PNG (std %.1f) and a %d-page PDF" % (r.get("source"), w0, h0, w0 * 4, h0 * 4, p["std"], pdf["pages"]), ""


def grade_aftereffects(r, d):
    if r.get("aerender_exit") != 0:
        raise ValueError("aerender exit %s" % r.get("aerender_exit"))
    n = r.get("frame_count", 0)
    if n < 60:
        raise ValueError("%s frames rendered, expected 60" % n)
    names = [o["file"] for o in r["outputs"]]
    pngs = [x for x in names if x.endswith(".png") and x != "ame_out.png"]
    _outputs_fresh(r, d, names)
    for x in pngs:
        _png(d, x, 3840, 2160)
    for x in [x for x in names if x.endswith(".tif")]:
        i = media_check.tiff_info(d / x)
        if (i["width"], i["height"]) != (3840, 2160) or i["bytes"] < 1_000_000:
            raise ValueError("%s is %dx%d, %d bytes" % (x, i["width"], i["height"], i["bytes"]))
    log = (d / "aerender.log").read_text(errors="replace")
    if "Finished composition" not in log:
        raise ValueError("aerender log has no 'Finished composition'")
    made = str(r.get("make_log", ""))
    have = set(made.strip().splitlines()[0].split(",")) if made.strip() else set()
    for want in ("fractalnoise", "blur", "precomp", "text", "16bpc"):
        if want not in have:
            raise ValueError("generated project lacks %s (%s)" % (want, made[:120]))
    extra = sorted(have & {"turbulent", "glow", "particles", "3d_layer", "expression", "lumetri", "camera", "footage", "cuda"})
    return "generated 4K 16-bit project (%s) rendered by aerender: %d frames 3840x2160" % (",".join(extra), n), made


def grade_indesign(r, d):
    js = r.get("js") or {}
    _outputs_fresh(r, d, ("out.pdf", "out.jpg"))
    pdf = media_check.pdf_info(d / "out.pdf")
    if str(js.get("graphics")) != "1":
        raise ValueError("the photo was not placed (graphics=%s)" % js.get("graphics"))
    if pdf["pages"] != 2 or str(js.get("pages")) != "2":
        raise ValueError("PDF has %d pages, expected 2" % pdf["pages"])
    w, h = media_check.jpeg_size(d / "out.jpg")
    if not (w > 800 and h > 1000):
        raise ValueError("page JPEG is %dx%d" % (w, h))
    return "built a 2-page document with a placed photo, exported a 2-page PDF (%d bytes) and a %dx%d page JPEG" % (pdf["bytes"], w, h), ""


def grade_illustratorsample(r, d):
    js = r.get("js") or {}
    _outputs_fresh(r, d, ("out.png", "out.pdf"))
    w0, h0 = int(float(js["w0"])), int(float(js["h0"]))
    p = _png(d, "out.png", w0 * 2, h0 * 2)
    if media_check.pdf_info(d / "out.pdf")["pages"] < 1:
        raise ValueError("PDF without pages")
    return "opened the shipped sample %s (%dx%d, %s paths), exported %dx%d PNG (std %.1f) and a PDF" % (r.get("source"), w0, h0, js.get("paths"), w0 * 2, h0 * 2, p["std"]), ""


def grade_indesignsample(r, d):
    js = r.get("js") or {}
    _outputs_fresh(r, d, ("out.pdf", "out.jpg"))
    pdf = media_check.pdf_info(d / "out.pdf")
    if pdf["pages"] != int(js.get("pages", -1)) or pdf["pages"] < 1:
        raise ValueError("PDF has %d pages, the document has %s" % (pdf["pages"], js.get("pages")))
    w, h = media_check.jpeg_size(d / "out.jpg")
    if min(w, h) < 400:
        raise ValueError("page JPEG is %dx%d" % (w, h))
    return "opened the shipped sample %s (%s pages), exported a %d-page PDF and a %dx%d page JPEG" % (r.get("source"), js.get("pages"), pdf["pages"], w, h), ""


def grade_animatesample(r, d):
    log = str(r.get("jsfl_log", ""))
    m = re.search(r"size=(\d+)x(\d+)", log)
    if "opened" not in log or "png" not in log or not m:
        raise ValueError("script did not open and export: %s" % log[:120])
    _outputs_fresh(r, d, ("out.png",))
    w, h = int(m[1]), int(m[2])
    p = _png(d, "out.png", w, h, min_std=2.0, min_distinct=4)
    return "opened the shipped sample %s (%dx%d), exported a frame as PNG (std %.1f)" % (r.get("source"), w, h, p["std"]), ""


def grade_photoshopsample(r, d):
    js = r.get("js") or {}
    if (js.get("tw"), js.get("th")) != ("3840", "2160"):
        raise ValueError("template opened as %sx%s, expected 3840x2160" % (js.get("tw"), js.get("th")))
    if int(js.get("sharpen", 0)) < 3:
        raise ValueError("fewer than 3 GPU-class filter passes succeeded")
    _outputs_fresh(r, d, ("out.png",))
    w1, h1 = int(js["w1"]), int(js["h1"])
    p = _png(d, "out.png", w1, h1)
    return "opened the shipped 3840x2160 PSD template and a %sx%s sky photo, %s filter passes, saved %dx%d PNG (std %.1f)" % (js.get("w0"), js.get("h0"), js.get("sharpen"), w1, h1, p["std"]), r.get("gpuinfo", "") or "NVIDIA"


def grade_acrobat(r, d):
    if r.get("pages") != EXPECT["pdf"]["pages"] or r.get("source") != EXPECT["pdf"]["name"]:
        raise ValueError("opened %s with %s pages" % (r.get("source"), r.get("pages")))
    _outputs_fresh(r, d, ("out.pdf",))
    pdf = media_check.pdf_info(d / "out.pdf")
    if pdf["pages"] != 5 or r.get("extracted_pages") != 5:
        raise ValueError("extracted PDF has %d pages, expected 5" % pdf["pages"])
    words = None
    return "opened the 16-page PDF through the automation interface, extracted 5 pages into a new PDF%s" % ("" if words is None else " (page 1: %d words)" % words), ""


def grade_animate(r, d):
    log = str(r.get("jsfl_log", ""))
    if "png" not in log or "saved" not in log:
        raise ValueError("script did not reach the export: %s" % log[:120])
    _outputs_fresh(r, d, ("out.png", "out.fla"))
    p = _png(d, "out.png", 1280, 720, min_std=1.0, min_distinct=2)
    return "script built a document with a motion tween, saved it and exported a 1280x720 PNG", ""


def grade_gpudiag(r, d):
    names = [o["file"] for o in r["outputs"]]
    _outputs_fresh(r, d, names)
    found = []
    for n in names:
        text = (d / n).read_text(errors="replace")
        if n == "sniffer_photoshop.txt":
            continue
        m = re.search(r"CUDA device(?:s)?[^\n]*|Device name[^\n]*|GPU [Cc]omputation Info[\s\S]{0,300}", text)
        if "No CUDA devices Found" in text or "Did not find any devices" in text:
            raise ValueError("%s: the application's own GPU sniffer finds no compute device" % n)
        if "nvidia" not in text.lower():
            raise ValueError("%s: no NVIDIA device named in the sniffer output" % n)
        found.append(n[8:-4])
    return "the applications' GPU sniffers (%s) report an NVIDIA compute device" % ", ".join(found), ""


def grade_launch(r, d, name):
    if not r.get("main_window"):
        raise ValueError("main window never appeared")
    if not r.get("alive"):
        raise ValueError("process not alive after the hold")
    return "LIMITED: launched, main window reached (%d windows), process responding=%s, %s MB; no document work" % (len(r.get("titles") or []), r.get("responding"), r.get("private_mb")), ""


def grade_lightroom(r, d):
    res = str(r.get("plugin_result", ""))
    if not res.startswith("OK renditions=2"):
        raise ValueError("plugin did not export both photos: %s" % res.strip()[:120])
    names = [o["file"] for o in r.get("outputs", [])]
    if len(names) != 2:
        raise ValueError("%d exported JPEGs, expected 2" % len(names))
    _outputs_fresh(r, d, names)
    dims = sorted(media_check.jpeg_size(d / n) for n in names)
    if max(dims[0]) < 3000 or max(dims[1]) < 5000:
        raise ValueError("exports are %s: full-size develop output expected (>= 3000 and >= 5000 px long side)" % (dims,))
    return "imported 2 raw photos, applied a 13-setting develop stack and exported full-size JPEGs %s through the develop pipeline" % (dims,), ""


def grade_mediaencoder(r, d):
    _outputs_fresh(r, d, ("ame_out.mp4",))
    i = media_check.mp4_info(d / "ame_out.mp4")
    if i["codec"] not in ("avc1", "avc3"):
        raise ValueError("encoded file codec %s, expected H.264" % i["codec"])
    if not i["duration_s"] or abs(i["duration_s"] - 2.0) > 0.25 or (i["width"], i["height"]) != (1920, 1080):
        raise ValueError("encoded file is %sx%s, %ss; expected 1920x1080, 2 s" % (i["width"], i["height"], i["duration_s"]))
    return "Media Encoder encoded the generated comp: H.264 (%s) %dx%d, %.2f s" % (i["codec"], i["width"], i["height"], i["duration_s"]), ""


def _project_rows(r, minimum):
    rows = [x for x in str(r.get("rows", "")).strip().split("||") if x]
    ok = [x for x in rows if x.startswith("OK ")]
    bad = [x for x in rows if x.startswith("ERR")]
    if not rows or r.get("count", 0) != len(rows):
        raise ValueError("the script reported %d of %s files: %s" % (len(rows), r.get("count"), str(r.get("rows", ""))[:100]))
    if len(ok) < max(1, int(minimum * len(rows))):
        raise ValueError("%d of %d project files opened (need %d%%); first errors: %s" % (len(ok), len(rows), int(minimum * 100), " ; ".join(bad[:3])[:200]))
    return ok, bad


def grade_photoshopprojects(r, d):
    ok, bad = _project_rows(r, 0.7)
    names = [o["file"] for o in r.get("outputs", [])]
    if not names:
        raise ValueError("no flattened PNG saved")
    _outputs_fresh(r, d, names)
    flat = 0
    for n in names:
        i = media_check.png_info(d / n)
        if i["width"] > 1024 or i["height"] > 4096:
            raise ValueError("%s is %dx%d" % (n, i["width"], i["height"]))
        flat += 1 if (i["decoded"] and i["std"] < 1.0) else 0
    return "%d of %d open-licence PSD projects opened (layers/mode read), %d flattened PNGs (%d flat); errors: %d" % (len(ok), len(ok) + len(bad), len(names), flat, len(bad)), ""


def grade_indesignprojects(r, d):
    ok, bad = _project_rows(r, 0.5)
    names = [o["file"] for o in r.get("outputs", [])]
    _outputs_fresh(r, d, names)
    for n in names:
        if media_check.pdf_info(d / n)["pages"] < 1:
            raise ValueError("%s has no pages" % n)
    return "%d of %d open-licence INDD projects opened and exported to PDF (%d PDFs)" % (len(ok), len(ok) + len(bad), len(names)), ""


def grade_animateprojects(r, d):
    ok, bad = _project_rows(r, 0.5)
    names = [o["file"] for o in r.get("outputs", [])]
    _outputs_fresh(r, d, names)
    return "%d of %d open-licence FLA projects opened, %d PNGs exported" % (len(ok), len(ok) + len(bad), len(names)), ""


def grade_aeprojects(r, d):
    ok, bad = _project_rows(r, 0.7)
    return "%d of %d open-licence AEP projects opened (items, comps and layers counted)" % (len(ok), len(ok) + len(bad)), ""


GRADERS = {"photoshopprojects": grade_photoshopprojects, "indesignprojects": grade_indesignprojects, "animateprojects": grade_animateprojects,
           "aeprojects": grade_aeprojects, "illustratorsample": grade_illustratorsample, "indesignsample": grade_indesignsample, "animatesample": grade_animatesample,
           "photoshopsample": grade_photoshopsample, "mediaencoder": grade_mediaencoder, "lightroom": grade_lightroom, "photoshop": grade_photoshop, "illustrator": grade_illustrator, "aftereffects": grade_aftereffects, "indesign": grade_indesign,
           "acrobat": grade_acrobat, "animate": grade_animate, "gpudiag": grade_gpudiag}


def verdict(name, run_id, directory, gpu_record):
    """Grade one row from the pulled files. Returns the same row shape as propimage_check.verdict plus `limited`, `gpu`."""
    row = {"app": name, "ok": False, "evidence": "NONE", "signal": "", "error": "", "class": "app", "limited": False,
           "lane": CHECKS[name].get("lane", "gate"), "path": CHECKS[name].get("path", ""), "platform": os.environ.get("KF_PLATFORM", "kf3")}
    try:
        r = _json(directory / "result.json")
        if r.get("app") not in (name, "sniffers"):
            raise ValueError("Result identity mismatch")
        if not _finite(r.get("started"), r.get("ended")) or not r["started"] < r["ended"]:
            raise ValueError("Invalid guest timestamps")
        row["secs"] = int(r["ended"] - r["started"])
        if r.get("error"):
            raise ValueError(str(r["error"])[:200])
        if name in LAUNCH_ONLY:
            signal, info = grade_launch(r, directory, name)
            row.update(limited=True, evidence="UIA", status="PASS_LIMITED")
        else:
            signal, info = GRADERS[name](r, directory)
            row.update(evidence="FILE")
        if r.get("quit") not in ("clean", "process"):
            raise ValueError("application did not exit cleanly (%s)" % r.get("quit"))
        if CHECKS[name]["gpu"] and (gpu_record is None or gpu_record.get("verdict") != "PASS"):
            reasons = "; ".join(gpu_record.get("reasons", ["no GPU record"])) if gpu_record else "no GPU record"
            raise ValueError("GPU assertion failed: " + reasons[:300])
        if name == "photoshop" and "nvidia" not in str(info).lower():
            raise ValueError("Photoshop's own system information does not name an NVIDIA GPU: " + str(info)[:200])
        row["ok"] = True
        row["signal"] = signal
        if gpu_record:
            row["gpu"] = {k: gpu_record.get(k) for k in ("required", "verdict", "peak", "baseline", "samples")}
    except (OSError, ValueError, KeyError, TypeError, AttributeError) as exc:
        row["error"] = str(exc)
        if gpu_record:
            row["gpu"] = {k: gpu_record.get(k) for k in ("required", "verdict", "peak", "baseline", "samples")}
    return row
