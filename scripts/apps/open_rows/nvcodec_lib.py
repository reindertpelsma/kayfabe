#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later
"""Logic of the FFmpeg NVENC/NVDEC capability matrix row (stdlib only; GPU-free tested by scripts/ci/test_open_rows.py).

STATUS: LIVE, 2026-11-17 (design: docs/design/V3_OPEN_ROWS_FFMPEG_DCGM.md). Code and GPU-free tests exist; native results are in
traces/open_rows_ffdcgm_20261117/ (when that directory exists); no kayfabe guest has run it.

Everything the row needs to DECIDE lives here, as pure functions: parsing the probe (nvcodec_probe.c), validating the committed case
table, evaluating a case's `needs` expression against the probe (the expectation is derived from the machine's own answers, never from
a per-die table), building ffmpeg argument vectors (never a shell string), classifying ffmpeg's error output, the PSNR threshold, the
per-case verdict, the cross-side comparison (host control against kayfabe guest) and the three-bucket classification of a failure.
The runner (nvcodec_matrix.py) only executes processes and writes files.
"""
import fnmatch
import json
import re

CODECS = ("h264", "hevc", "av1", "vp9")
RESULTS = ("PASS", "NOT_SUPPORTED", "FAIL", "MISMATCH", "NOT_RUN")
OPS = ("enc", "dec", "sessions")
MODES = ("nvenc", "hwaccel_cuda", "cuvid")
CHROMAS = ("420", "444")
COLUMNS = ("id", "group", "op", "codec", "profile", "pix_fmt", "chroma", "bitdepth", "width", "height", "frames", "sessions", "mode",
           "flags", "needs", "psnr_min")
PIX_FMTS = ("yuv420p", "yuv420p10le", "yuv444p", "yuv444p10le", "yuv444p16le", "p010le", "nv12")
# the format both sides of a PSNR comparison are converted to, by (chroma, bit depth)
COMPARE_FMT = {("420", 8): "yuv420p", ("420", 10): "yuv420p10le", ("444", 8): "yuv444p", ("444", 10): "yuv444p10le"}
# the surface format ffmpeg's NVDEC hwaccel keeps on the device, by (chroma, bit depth); hwdownload must name it
HW_SW_FMT = {("420", 8): "nv12", ("420", 10): "p010le", ("444", 8): "yuv444p", ("444", 10): "yuv444p16le"}
ENCODERS = {"h264": "h264_nvenc", "hevc": "hevc_nvenc", "av1": "av1_nvenc"}
CUVID = {"h264": "h264_cuvid", "hevc": "hevc_cuvid", "vp9": "vp9_cuvid", "av1": "av1_cuvid"}
# software encoders that make the decode test streams, first available wins (names from `ffmpeg -encoders`)
SW_ENCODERS = {"h264": ("libx264",), "hevc": ("libx265",), "vp9": ("libvpx-vp9",), "av1": ("libsvtav1", "libaom-av1")}
SW_ENCODER_ARGS = {
    "libx264": ["-preset", "ultrafast", "-qp", "20", "-threads", "1"],
    "libx265": ["-preset", "ultrafast", "-x265-params", "log-level=error:qp=20:frame-threads=1:pools=1"],
    "libvpx-vp9": ["-deadline", "realtime", "-cpu-used", "8", "-crf", "30", "-b:v", "0", "-threads", "1"],
    "libsvtav1": ["-preset", "12", "-crf", "35"],
    "libaom-av1": ["-cpu-used", "8", "-crf", "35", "-b:v", "0", "-row-mt", "1"],
}
PROFILE_ARG = {"h264": "-profile:v", "hevc": "-profile:v", "av1": "-profile:v"}
FLAG_TOKEN = re.compile(r"^[A-Za-z0-9_:.=+-]+$")
SEED = 1234  # the noise filter's seed: the same frames on every run, on the host and in a guest


class TableError(ValueError):
    pass


# ------------------------------------------------------------------------------------------------------------ the probe
class Probe:
    """What nvcodec_probe printed. `ok` is False when the probe could not reach NVENC/NVDEC at all."""

    def __init__(self):
        self.meta = {}
        self.enc_codec = {}
        self.enc_caps = {}  # codec -> {cap name or capNN -> int}
        self.enc_profiles = {}
        self.enc_inputs = {}
        self.dec = {}  # (codec, chroma, depth) -> dict
        self.sessions = {}
        self.errors = []
        self.lines = 0

    def enc_available(self):
        return bool(self.enc_codec) and "nvenc" not in self.error_keys()

    def dec_available(self):
        return any(not d["error"] for d in self.dec.values()) and "nvdec" not in self.error_keys()

    def error_text(self, key):
        return "; ".join(v for k, v in self.errors if k == key)

    def error_keys(self):
        return {k for k, _ in self.errors}

    def flat(self):
        """A flat {key: value} view, for a host/guest diff (device name excluded: the same die has the same name, but a guest may rename)."""
        out = {}
        for k, v in sorted(self.meta.items()):
            out["meta." + k] = v
        for c, v in sorted(self.enc_codec.items()):
            out["enc_codec." + c] = int(v)
        for c, caps in sorted(self.enc_caps.items()):
            for k, v in sorted(caps.items()):
                out["enc_cap." + c + "." + k] = v
        for c, v in sorted(self.enc_profiles.items()):
            out["enc_profile." + c] = ",".join(sorted(v))
        for c, v in sorted(self.enc_inputs.items()):
            out["enc_input." + c] = ",".join(sorted(v))
        for k, d in sorted(self.dec.items()):
            out["dec_cap." + ".".join(map(str, k))] = d["raw"]
        for k, v in sorted(self.sessions.items()):
            out["sessions." + k] = v
        return out


def parse_probe(text):
    p = Probe()
    for raw in text.splitlines():
        parts = raw.rstrip("\n").split("\t", 2)
        if len(parts) != 3:
            continue
        kind, key, val = parts
        p.lines += 1
        if kind == "meta":
            p.meta[key] = val
        elif kind == "enc_codec":
            if re.fullmatch(r"h264|hevc|av1", key) and val in ("0", "1"):
                p.enc_codec[key] = val == "1"
        elif kind == "enc_cap":
            codec, _, name = key.partition(".")
            if re.fullmatch(r"-?\d+", val):
                p.enc_caps.setdefault(codec, {})[name] = int(val)
            else:
                p.enc_caps.setdefault(codec, {})[name] = val  # ERR:<status>
        elif kind == "enc_profile":
            p.enc_profiles.setdefault(key, set()).add(val)
        elif kind == "enc_input":
            p.enc_inputs.setdefault(key, set()).add(val)
        elif kind == "dec_cap":
            m = re.fullmatch(r"([a-z0-9]+)\.(\d{3})\.(\d+)", key)
            if not m:
                continue
            d = {"raw": val, "supported": False, "maxw": 0, "maxh": 0, "minw": 0, "minh": 0, "error": val.startswith("ERR:")}
            kvs = dict(re.findall(r"(\w+)=(\S+)", val))
            if kvs.get("supported") is not None:
                d["supported"] = kvs["supported"] == "1"
                for name, k in (("max", "maxw"), ("min", "minw")):
                    mm = re.fullmatch(r"(\d+)x(\d+)", kvs.get(name, ""))
                    if mm:
                        d[k], d[k[:-1] + "h"] = int(mm[1]), int(mm[2])
            p.dec[(m[1], m[2], int(m[3]))] = d
        elif kind == "sessions":
            p.sessions[key] = val
        elif kind == "error":
            p.errors.append((key, val))
    return p


# ------------------------------------------------------------------------------------------------------- the case table
def _int(row, col, lo, hi):
    try:
        v = int(row[col])
    except ValueError:
        raise TableError("%s: %s is not a number: %r" % (row["id"], col, row[col]))
    if not lo <= v <= hi:
        raise TableError("%s: %s=%d outside %d..%d" % (row["id"], col, v, lo, hi))
    return v


def load_cases(text):
    """Parse and validate the committed case table (TSV with a header). Raises TableError; returns a list of dicts."""
    lines = [l for l in text.splitlines() if l.strip() and not l.startswith("#")]
    if not lines:
        raise TableError("empty case table")
    head = tuple(lines[0].split("\t"))
    if head != COLUMNS:
        raise TableError("header is %r, expected %r" % (head, COLUMNS))
    rows, seen = [], set()
    for n, line in enumerate(lines[1:], 2):
        cells = line.split("\t")
        if len(cells) != len(COLUMNS):
            raise TableError("line %d: %d cells, expected %d" % (n, len(cells), len(COLUMNS)))
        r = dict(zip(COLUMNS, cells))
        if not re.fullmatch(r"[a-z][a-z0-9_]{2,60}", r["id"]):
            raise TableError("line %d: bad id %r" % (n, r["id"]))
        if r["id"] in seen:
            raise TableError("duplicate id %s" % r["id"])
        seen.add(r["id"])
        if r["op"] not in OPS:
            raise TableError("%s: op %r" % (r["id"], r["op"]))
        if r["codec"] not in CODECS:
            raise TableError("%s: codec %r" % (r["id"], r["codec"]))
        if r["mode"] not in MODES:
            raise TableError("%s: mode %r" % (r["id"], r["mode"]))
        if (r["op"] in ("enc", "sessions")) != (r["mode"] == "nvenc"):
            raise TableError("%s: mode %s does not fit op %s" % (r["id"], r["mode"], r["op"]))
        if r["op"] != "dec" and r["codec"] not in ENCODERS:
            raise TableError("%s: %s has no NVENC encoder" % (r["id"], r["codec"]))
        if r["op"] == "dec" and r["mode"] == "cuvid" and r["codec"] not in CUVID:
            raise TableError("%s: %s has no cuvid decoder" % (r["id"], r["codec"]))
        if r["pix_fmt"] not in PIX_FMTS:
            raise TableError("%s: pix_fmt %r" % (r["id"], r["pix_fmt"]))
        if r["chroma"] not in CHROMAS:
            raise TableError("%s: chroma %r" % (r["id"], r["chroma"]))
        if int(r["bitdepth"] if r["bitdepth"].isdigit() else -1) not in (8, 10):
            raise TableError("%s: bitdepth %r" % (r["id"], r["bitdepth"]))
        r["bitdepth"] = int(r["bitdepth"])
        r["width"] = _int(r, "width", 64, 16384)
        r["height"] = _int(r, "height", 64, 16384)
        r["frames"] = _int(r, "frames", 1, 600)
        r["sessions"] = _int(r, "sessions", 1, 64)
        if r["op"] != "sessions" and r["sessions"] != 1:
            raise TableError("%s: sessions>1 only for op=sessions" % r["id"])
        try:
            r["psnr_min"] = float(r["psnr_min"])
        except ValueError:
            raise TableError("%s: psnr_min %r" % (r["id"], r["psnr_min"]))
        if not 0 <= r["psnr_min"] <= 100:
            raise TableError("%s: psnr_min outside 0..100" % r["id"])
        toks = [] if r["flags"] in ("", "-") else r["flags"].split()
        for t in toks:
            if not FLAG_TOKEN.match(t):
                raise TableError("%s: flag token %r" % (r["id"], t))
        r["flags"] = toks
        r["needs"] = parse_needs(r["id"], r["needs"])
        rows.append(r)
    return rows


# ------------------------------------------------------------------------------------------ the `needs` mini-language
def parse_needs(case_id, text):
    """A conjunction of atoms joined by ' & '. Atoms: `enc:<codec>` (the codec is offered), `enc:<codec>:dim` (the case's size is inside
    the reported min/max), `enc:<codec>:<CAP><op><n>` (a named cap compares), `enc:sessions>=N` (the session discovery opened at least
    N), `dec:<codec>:<chroma>:<depth>` (cuvidGetDecoderCaps says supported), `dec:<codec>:<chroma>:<depth>:dim`. No eval."""
    atoms = []
    for tok in [t.strip() for t in text.split("&")]:
        if not tok:
            raise TableError("%s: empty atom in needs" % case_id)
        m = re.fullmatch(r"enc:sessions(>=|<=|==)(\d+)", tok)
        if m:
            atoms.append(("sessions", m[1], int(m[2])))
            continue
        m = re.fullmatch(r"enc:(h264|hevc|av1)", tok)
        if m:
            atoms.append(("enc_codec", m[1]))
            continue
        m = re.fullmatch(r"enc:(h264|hevc|av1):dim", tok)
        if m:
            atoms.append(("enc_dim", m[1]))
            continue
        m = re.fullmatch(r"enc:(h264|hevc|av1):([A-Z][A-Z0-9_]+)(>=|<=|==)(\d+)", tok)
        if m:
            atoms.append(("enc_cap", m[1], m[2], m[3], int(m[4])))
            continue
        m = re.fullmatch(r"dec:([a-z0-9]+):(400|420|422|444):(8|10|12)", tok)
        if m:
            atoms.append(("dec", m[1], m[2], int(m[3])))
            continue
        m = re.fullmatch(r"dec:([a-z0-9]+):(400|420|422|444):(8|10|12):dim", tok)
        if m:
            atoms.append(("dec_dim", m[1], m[2], int(m[3])))
            continue
        raise TableError("%s: cannot parse need %r" % (case_id, tok))
    return atoms


def _cmp(a, op, b):
    return {">=": a >= b, "<=": a <= b, "==": a == b}[op]


def evaluate_needs(case, probe):
    """-> ("SUPPORTED" | "UNSUPPORTED" | "UNKNOWN", [the atoms that failed or could not be answered]).
    UNKNOWN means the probe did not answer (NVENC or NVDEC not reachable): the case is NOT_RUN, never a guess."""
    failed, unknown = [], []
    w, h = case["width"], case["height"]
    for atom in case["needs"]:
        kind = atom[0]
        if kind == "sessions":
            n = probe.sessions.get("opened")
            if n is None or not n.isdigit():
                unknown.append("sessions")
            elif not _cmp(int(n), atom[1], atom[2]):
                failed.append("sessions opened=%s need %s%d" % (n, atom[1], atom[2]))
        elif kind == "enc_codec":
            if atom[1] not in probe.enc_codec:
                unknown.append("enc:" + atom[1])
            elif not probe.enc_codec[atom[1]]:
                failed.append("enc:%s not offered" % atom[1])
        elif kind in ("enc_dim", "enc_cap"):
            caps = probe.enc_caps.get(atom[1])
            if not caps or atom[1] not in probe.enc_codec:
                unknown.append("enc:" + atom[1])
            elif not probe.enc_codec[atom[1]]:
                failed.append("enc:%s not offered" % atom[1])
            elif kind == "enc_dim":
                got = {n: caps.get(n) for n in ("WIDTH_MAX", "HEIGHT_MAX", "WIDTH_MIN", "HEIGHT_MIN")}
                if any(not isinstance(v, int) for v in got.values()):
                    unknown.append("enc:%s:dim" % atom[1])
                elif not (got["WIDTH_MIN"] <= w <= got["WIDTH_MAX"] and got["HEIGHT_MIN"] <= h <= got["HEIGHT_MAX"]):
                    failed.append("enc:%s %dx%d outside %dx%d..%dx%d" % (atom[1], w, h, got["WIDTH_MIN"], got["HEIGHT_MIN"],
                                                                         got["WIDTH_MAX"], got["HEIGHT_MAX"]))
            else:
                v = caps.get(atom[2])
                if not isinstance(v, int):
                    unknown.append("enc:%s:%s" % (atom[1], atom[2]))
                elif not _cmp(v, atom[3], atom[4]):
                    failed.append("enc:%s:%s=%d need %s%d" % (atom[1], atom[2], v, atom[3], atom[4]))
        else:  # dec, dec_dim
            d = probe.dec.get((atom[1], atom[2], atom[3]))
            if d is None or d["error"]:
                unknown.append("dec:%s:%s:%d" % atom[1:4])
            elif not d["supported"]:
                failed.append("dec:%s:%s:%d unsupported" % atom[1:4])
            elif kind == "dec_dim" and not (d["minw"] <= w <= d["maxw"] and d["minh"] <= h <= d["maxh"]):
                failed.append("dec:%s:%s:%d %dx%d outside %dx%d..%dx%d" % (atom[1:4] + (w, h, d["minw"], d["minh"], d["maxw"], d["maxh"])))
    if unknown and not failed:
        return "UNKNOWN", list(dict.fromkeys(unknown))
    if failed:
        return "UNSUPPORTED", failed
    return "SUPPORTED", []


# ----------------------------------------------------------------------------------------------------- ffmpeg commands
def source_spec(case):
    """The lavfi source of an encode case: deterministic frames (testsrc2 plus seeded noise), no download."""
    return "testsrc2=size=%dx%d:rate=30,noise=alls=12:allf=t:all_seed=%d" % (case["width"], case["height"], SEED)


def compare_fmt(case):
    return COMPARE_FMT[(case["chroma"], case["bitdepth"])]


def encode_argv(ffmpeg, case, out):
    enc = ENCODERS[case["codec"]]
    argv = [ffmpeg, "-hide_banner", "-nostdin", "-y", "-f", "lavfi", "-i", source_spec(case), "-frames:v", str(case["frames"]),
            "-pix_fmt", case["pix_fmt"], "-c:v", enc]
    if case["profile"] not in ("", "-"):
        argv += [PROFILE_ARG[case["codec"]], case["profile"]]
    argv += ["-preset", "p4"]
    if not any(t in ("-rc", "-b:v") for t in case["flags"]):
        argv += ["-rc", "constqp", "-qp", "24"]
    return argv + case["flags"] + [out]


def psnr_argv(ffmpeg, case, encoded):
    f = compare_fmt(case)
    graph = "[0:v]format=%s[a];[1:v]format=%s[b];[a][b]psnr" % (f, f)
    return [ffmpeg, "-hide_banner", "-nostdin", "-i", encoded, "-f", "lavfi", "-i", source_spec(case), "-filter_complex", graph,
            "-frames:v", str(case["frames"]), "-f", "null", "-"]


def probe_stream_argv(ffprobe, path):
    return [ffprobe, "-v", "error", "-count_frames", "-select_streams", "v:0", "-show_entries",
            "stream=codec_name,pix_fmt,width,height,nb_read_frames", "-of", "json", path]


def session_argv(ffmpeg, case, seconds=3):
    return [ffmpeg, "-hide_banner", "-nostdin", "-y", "-f", "lavfi", "-i", "testsrc2=size=%dx%d:rate=60:duration=%d" % (
        case["width"], case["height"], seconds), "-c:v", ENCODERS[case["codec"]], "-preset", "p4"] + case["flags"] + ["-f", "null", "-"]


def pick_sw_encoder(codec, available):
    for name in SW_ENCODERS.get(codec, ()):
        if name in available:
            return name
    return None


def source_stream_argv(ffmpeg, case, swenc, out):
    """The test stream of a decode case, made by a SOFTWARE encoder at run time from the same lavfi source."""
    return [ffmpeg, "-hide_banner", "-nostdin", "-y", "-f", "lavfi", "-i", source_spec(case), "-frames:v", str(case["frames"]),
            "-pix_fmt", case["pix_fmt"], "-c:v", swenc] + SW_ENCODER_ARGS[swenc] + [out]


def decode_compare_argv(ffmpeg, case, stream):
    """One ffmpeg process decodes the stream twice, in software (input 0) and on NVDEC (input 1), and prints their PSNR. With
    -hwaccel_output_format cuda a failed NVDEC setup is an error: ffmpeg cannot fall back to software and still hand device frames
    to hwdownload (measured on the first matrix, V3_VIDEO_ENGINES.md section 5)."""
    f = compare_fmt(case)
    key = (case["chroma"], case["bitdepth"])
    argv = [ffmpeg, "-hide_banner", "-nostdin", "-i", stream]
    if case["mode"] == "hwaccel_cuda":
        # AV1: ffmpeg prefers libdav1d, which has no NVDEC hwaccel, so the frames would be software frames and hwdownload fails; the native
        # `av1` decoder is the one that takes the hwaccel (measured on ffmpeg 4.4.2, traces/open_rows_ffdcgm_20261117)
        argv += (["-c:v", "av1"] if case["codec"] == "av1" else []) + ["-hwaccel", "cuda", "-hwaccel_output_format", "cuda", "-i", stream]
        lower = "hwdownload,format=%s,format=%s" % (HW_SW_FMT[key], f)
    else:
        argv += ["-c:v", CUVID[case["codec"]], "-i", stream]
        lower = "format=%s" % f
    graph = "[0:v]format=%s[a];[1:v]%s[b];[a][b]psnr" % (f, lower)
    return argv + ["-filter_complex", graph, "-frames:v", str(case["frames"]), "-f", "null", "-"]


def unavailable_reason(case, probe):
    """When the probe could not reach the engine the case needs, the case is NOT_RUN with a named reason (and the probe's own words in the
    detail), never a guess about what the die can do. -> (reason, detail) or (None, "")."""
    if case["op"] in ("enc", "sessions"):
        if not probe.enc_available():
            return "nvenc_unavailable", probe.error_text("nvenc") or "the probe printed no NVENC rows"
    elif not probe.dec_available():
        return "nvdec_unavailable", probe.error_text("nvdec") or "cuvidGetDecoderCaps answered an error for every codec (%s)" % next(
            (d["raw"] for d in probe.dec.values() if d["error"]), "no rows")
    return None, ""


# ----------------------------------------------------------------------------------------- reading ffmpeg's own output
PSNR_RE = re.compile(r"PSNR y:\S+ u:\S+ v:\S+ average:(inf|[0-9.]+)")


def parse_psnr(stderr):
    """Average PSNR in dB of the last psnr summary line; float('inf') for identical frames; None when there is no summary."""
    last = None
    for m in PSNR_RE.finditer(stderr):
        last = m[1]
    if last is None:
        return None
    return float("inf") if last == "inf" else float(last)


def psnr_ok(value, minimum):
    return value is not None and value >= minimum


def parse_stream_json(text):
    try:
        s = json.loads(text)["streams"][0]
        return {"codec": s.get("codec_name"), "pix_fmt": s.get("pix_fmt"), "width": int(s["width"]), "height": int(s["height"]),
                "frames": int(s["nb_read_frames"])}
    except (ValueError, KeyError, IndexError, TypeError):
        return None


# first match wins; every id is a short word that goes into the result file
REASONS = (
    ("driver_api_too_old", re.compile(r"Driver does not support the required nvenc API version|minimum required Nvidia driver", re.I)),
    ("session_limit", re.compile(r"OpenEncodeSessionEx failed: (out of memory|incompatible client key)|OpenEncodeSessionEx failed: .*\(10\)", re.I)),
    ("caps_not_supported", re.compile(r"\bbit encode not supported|\bnot supported\b|\bexceeds\b|Codec not supported", re.I)),
    ("no_encode_device", re.compile(r"No (NVENC )?capable devices found|Cannot load libnvidia-encode|OpenEncodeSessionEx failed: (no encode device|unsupported device)", re.I)),
    ("no_decode_device", re.compile(r"CUDA_ERROR_NO_DEVICE|no CUDA-capable device", re.I)),
    ("unknown_encoder", re.compile(r"Unknown encoder|Encoder .* not found|Unrecognized option", re.I)),
    ("unknown_decoder", re.compile(r"Unknown decoder|Decoder .* not found", re.I)),
    ("no_cuvid", re.compile(r"Cannot load libnvcuvid|cuvid.*not (found|available)|Cannot load cuvid", re.I)),
    ("hw_decode_unsupported", re.compile(r"Hardware is lacking required capabilities|Failed setup for format cuda|doesn't support|"
                                         r"cuvidCreateDecoder|No decoder surfaces left|decoding for stream \d+ failed|Impossible to convert between the formats", re.I)),
    ("encoder_params_rejected", re.compile(r"InitializeEncoder failed|invalid param|Provided device doesn't support required NVENC", re.I)),
    ("cuda_error", re.compile(r"CUDA_ERROR_|cuCtxCreate|Failed to create .*CUDA|cuInit", re.I)),
    ("out_of_memory", re.compile(r"out of memory|Cannot allocate memory|OUT_OF_MEMORY", re.I)),
)
CONFIG_REASONS = ("driver_api_too_old", "no_encode_device", "no_decode_device", "nvenc_unavailable", "nvdec_unavailable", "unknown_encoder",
                  "unknown_decoder", "no_cuvid", "no_sw_encoder",
                  "source_generation_failed", "no_ffmpeg", "no_probe", "no_gpu")


def classify_reason(stderr):
    for name, rx in REASONS:
        if rx.search(stderr):
            return name
    return "other"


# --------------------------------------------------------------------------------------------------------- the verdict
def verdict(expect, attempt_ok, attempt_reason, forced=None):
    """expect: SUPPORTED/UNSUPPORTED/UNKNOWN; attempt_ok: True/False/None (None = the attempt could not be made).
    -> (result, reason). PASS: supported and worked. NOT_SUPPORTED: unsupported and it failed (the driver and ffmpeg agree it cannot).
    FAIL: supported and it did not work. MISMATCH: unsupported and it worked (the caps lie or the probe is wrong). NOT_RUN: no answer."""
    if forced:
        return "NOT_RUN", forced
    if expect == "UNKNOWN" or attempt_ok is None:
        return "NOT_RUN", attempt_reason or "no_probe"
    if attempt_reason in CONFIG_REASONS and not attempt_ok:
        return "NOT_RUN", attempt_reason
    if expect == "SUPPORTED":
        return ("PASS", "ok") if attempt_ok else ("FAIL", attempt_reason or "other")
    return ("MISMATCH", "unexpected_success") if attempt_ok else ("NOT_SUPPORTED", attempt_reason or "other")


def sessions_result(result, reason, note):
    """A parallel-sessions case that went the expected way overall but with a partial count (n_ok other than the discovered cap) is a
    MISMATCH: the cap is being applied differently from what the discovery saw."""
    if result in ("PASS", "NOT_SUPPORTED") and "n_ok=" in note:
        return "MISMATCH", "partial_sessions"
    return result, reason


def session_outcome(n_ok, n, cap):
    """Parallel sessions: `cap` is what the probe's discovery opened. Up to the cap every session must work; above it exactly `cap`
    may. Returns (attempt_ok, note): attempt_ok is the value verdict() takes (all n worked), note names an unexpected partial result."""
    all_ok = n_ok == n
    if n <= cap:
        return all_ok, "" if all_ok else "n_ok=%d of %d under cap %d" % (n_ok, n, cap)
    if all_ok:
        return True, "cap %d not enforced" % cap
    return False, "" if n_ok == cap else "n_ok=%d but cap %d" % (n_ok, cap)


def summarize(rows):
    counts = {k: 0 for k in RESULTS}
    for r in rows:
        counts[r["result"]] += 1
    ok = counts["FAIL"] == 0 and counts["MISMATCH"] == 0
    return counts, ok


# ------------------------------------------------------------------------------------------------ result file (TSV)
RESULT_COLUMNS = ("case_id", "group", "expect", "result", "reason", "rc", "secs", "psnr", "frames", "n_ok", "sess_peak", "detail")


def row_to_tsv(r):
    cells = []
    for c in RESULT_COLUMNS:
        v = r.get(c, "-")
        v = "-" if v in (None, "") else str(v)
        cells.append(re.sub(r"[\t\r\n]+", " ", v)[:200])
    return "\t".join(cells)


def parse_results(text):
    rows = []
    for line in text.splitlines():
        if not line or line.startswith("#") or line.startswith("case_id\t"):
            continue
        cells = line.split("\t")
        if len(cells) != len(RESULT_COLUMNS):
            continue
        r = dict(zip(RESULT_COLUMNS, cells))
        if r["result"] in RESULTS:
            rows.append(r)
    return rows


# ------------------------------------------------------------------------------- host control against guest, 3 buckets
BUCKET_CONFIG = "config_error"
BUCKET_HARNESS = "harness_or_app_bug_confirmed_by_host_control"
BUCKET_KNOWN = "documented_kayfabe_limitation"
BUCKET_CANDIDATE = "kayfabe_candidate_bug_undocumented"


def load_known_limits(text):
    """TSV: case_glob, result (the guest's result the limitation explains), reason (free text naming the document)."""
    out = []
    for line in text.splitlines():
        if not line.strip() or line.startswith("#") or line.startswith("case_glob\t"):
            continue
        cells = line.split("\t")
        if len(cells) != 3 or cells[1] not in RESULTS or not cells[2].strip():
            raise TableError("bad known-limit line: %r" % line)
        out.append(tuple(cells))
    return out


def classify_pair(host, guest, limits=()):
    """host and guest are result rows of one case (either may be None). -> (agreement, bucket or ''), agreement AGREE | DISAGREE |
    MISSING. A case that works on the host and fails in the guest is a candidate kayfabe bug unless a committed limit explains it."""
    if host is None or guest is None:
        return "MISSING", BUCKET_CONFIG
    if "NOT_RUN" in (host["result"], guest["result"]):
        return ("AGREE" if host["result"] == guest["result"] else "DISAGREE"), BUCKET_CONFIG
    if host["result"] == guest["result"]:
        return "AGREE", (BUCKET_HARNESS if host["result"] in ("FAIL", "MISMATCH") else "")
    if host["result"] in ("FAIL", "MISMATCH"):  # the control itself is bad: not a statement about kayfabe
        return "DISAGREE", BUCKET_HARNESS
    for glob, result, _reason in limits:
        if fnmatch.fnmatchcase(guest["case_id"], glob) and result == guest["result"]:
            return "DISAGREE", BUCKET_KNOWN
    return "DISAGREE", BUCKET_CANDIDATE


def compare_results(host_rows, guest_rows, limits=()):
    h = {r["case_id"]: r for r in host_rows}
    g = {r["case_id"]: r for r in guest_rows}
    out = []
    for cid in sorted(set(h) | set(g)):
        agree, bucket = classify_pair(h.get(cid), g.get(cid), limits)
        out.append({"case_id": cid, "host": h[cid]["result"] if cid in h else "-", "guest": g[cid]["result"] if cid in g else "-",
                    "agreement": agree, "bucket": bucket or "-",
                    "host_reason": h[cid]["reason"] if cid in h else "-", "guest_reason": g[cid]["reason"] if cid in g else "-"})
    return out


def compare_probes(host, guest):
    """Per-key differences of two probes (every cap id, every decoder row, the session discovery). The device name and the CUDA
    driver version are listed but are not differences that matter on their own."""
    a, b = host.flat(), guest.flat()
    diffs = []
    for k in sorted(set(a) | set(b)):
        if a.get(k) != b.get(k):
            diffs.append((k, a.get(k, "-"), b.get(k, "-")))
    return diffs
