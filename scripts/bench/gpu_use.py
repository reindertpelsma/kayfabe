#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later
"""Shared GPU-use judge for application rows that CAN run on the CPU (stdlib only).

STATUS: LIVE, 2026-11-18 (owner requirement of 2026-11-17: a row that passes because the application silently fell back to the CPU is a
false pass; 2026-11-18: under kf3 the host census is the PRIMARY proof, the host engine and memory readings are recorded as facts when the census proved the use
(`judge(..., census_primary=True)`, `proved_by`, `host_readings`; docs/design/V3_GPU_USE_ASSERTION.md section 10.1)). GPU-free: parsing, judging and the negative
controls are tested against committed fixture logs in scripts/ci/test_gpu_use.py and mutated by scripts/ci/gpu_use_mutants.py. The 2026-11-17 first hardware run
(section 10.2) and the 2026-11-18 one (section 10.1) calibrated what they name; every threshold below marked UNCALIBRATED is an inference until a run says otherwise.

    verdict = judge(name, census=window_census(lines, base_line), smi=smi_record(query_csv, dmon_txt, ("gpu",)), app=PROOFS["blender"](log, devs=(...)),
                    need_census=True, need_engines=("gpu",), side="guest")

Three independent observables, in order of authority (the guest is hostile: guest bytes are DATA, never a verdict):

  1. HOST CENSUS (host-authored): the kf3 device's own log lines for the row's window. The status line (`kf3: family=... chan[births=N ...
     tokens=[0x801:fwd=F,subs=S,serves=V ...]] ... dbfast[ON ... doorbells=D ...]`, printed by the drainer about every 2 s on change) and the
     `kf3: chan token ... RETIRED, forwarded=.. submissions=..` lines. Window work = the growth of the counters between the last status line before
     the window and the end of it. Every window counter is a LOWER BOUND (a channel retired between the baseline line and the window start is not
     seen). Work means "the guest created channels and submitted GPFIFO entries to the real GPU through kf3": a CPU-only run submits none.
  2. HOST engine counters (host-authored): `nvidia-smi` on the box sampled around the row (scripts/bench/windows/gpu_assert.py: HostProbe,
     evaluate): utilisation, memory, encoder, decoder. This is what separates "a CUDA context was created" from "the engine was busy".
  3. APP PROOF (guest self-report, a second signal): what the application printed about its own device (Cycles device list, llama.cpp offload line,
     OpenCL device block, GL_RENDERER, ffmpeg encoder tag...). It can only ADD a failure or support a pass: it never overrides a host "no work".

Rules (never weakened to pass; a row that cannot be judged says why and is NOT_RUN, never PASS):
  FAIL    the app reports a CPU or non-NVIDIA device, or the proof line a row must print is missing (the row dropped it), or the host census shows no
          work in a window that has a baseline, or a required host engine stayed idle.
  NOT_RUN a required observable is absent (no kf3 status line in the window, no baseline, unreadable sampler, unproven marker). An unreadable GPU and
          an idle GPU are different findings and the reason says which.
  PASS    every required observable holds. With no host observable available (the bare-metal baseline) the record says authority=guest.
A negative control (the same row with the GPU hidden or the application's own CPU switch) must be judged FAIL: control_verdict() says CONTROL_OK
when it was and CONTROL_BROKEN when the judge passed a CPU run.

Census primary (2026-11-18, opt-in per caller; the owner rule of 2026-11-17 puts the host census first). MEASURED on a 4090 box: five MATLAB rows failed on two host
nvidia-smi readings (utilisation peak 2.0 because ms-long kernels fall between 1 s samples; host memory flat because the declared fb-mb carve-out is reserved at launch,
INFERRED) while the census counted GPU work for every row. With `census_primary=True` and `side="guest"` a census that PROVES the use (PASS) decides: an engine or
memory reading that says idle or flat is kept as a FACT (`readings`, a `[host-smi] FACT` reason) and does not fail the row. The census wins in BOTH directions: a census
that shows no work fails the row whatever nvidia-smi shows. Everything else is unchanged: an unreadable census is NOT_RUN and then the readings still decide, an
unreadable sampler is NOT_RUN, the application proof can only add a failure, and a caller without a census (the bare-metal baseline, `need_census=False`) keeps the engine
and memory checks. `proved_by` names the observable that proved a PASS: "census", else "engine", else "memory", else None.
"""
import importlib.util
import re
from pathlib import Path

HERE = Path(__file__).resolve().parent
PASS, FAIL, NOT_RUN = "PASS", "FAIL", "NOT_RUN"
MAX_TEXT = 1 << 21                 # one guest log, bytes read at most (the guest is hostile: its output is bounded before any regex runs)
MIN_SUBMISSIONS = 32               # UNCALIBRATED: GPFIFO entries submitted in the window. MEASURED (old per-app kf3 logs, traces/v3_candidates/cand1_20261004): opening
                                   # the device without CUDA (nvidia-smi) submits 2, creating a CUDA context and running vectorAdd 411 and matrixMul 424: 32 sits between
SOFT_GPU_NAMES = re.compile(r"llvmpipe|lavapipe|softpipe|swrast|swiftshader|warp\b|microsoft basic|gdi generic|software|\bcpu\b|pocl|portable computing|"
                            r"pthread|intel\(r\)|\bamd (epyc|ryzen)|xeon", re.I)


def _text(value):
    """A guest log, bounded, without the harness's own `=== name side=.. cmd: <the row's command line>` and `=== end` lines: the command text names the very
    strings a proof looks for (`Failed setup for format cuda`, `h264_nvenc`), and a proof must read what the APPLICATION printed."""
    if not isinstance(value, str):
        return ""
    return "\n".join(l for l in value[:MAX_TEXT].split("\n") if not l.startswith("=== "))


# ------------------------------------------------------------------------------------------------ the kf3 host log (host-authored)
PROVED_BY = ("census", "engine", "memory")          # the observables that can prove a use, in the order of authority (owner rule 2026-11-17: census first)
ENGINE_OBSERVABLE = {"gpu": "engine", "enc": "engine", "dec": "engine", "mem": "memory"}      # the proved_by name each host reading carries
CENSUS_PRIMARY_REASON = ("the census proved the use; the reading is recorded as a fact and does not decide "
                         "(docs/design/V3_GPU_USE_ASSERTION.md section 10.1)")

STATUS_RX = re.compile(r"kf3: family=\S+ phase=(\S+)")
BIRTHS_RX = re.compile(r"\bchan\[births=(\d+)")
TOKENS_RX = re.compile(r"\btokens=\[([^\]]*)\]")
TOKEN_RX = re.compile(r"(0x[0-9a-fA-F]+):fwd=(\d+),subs=(\d+),serves=(\d+)")
NSI_RX = re.compile(r"\bnsi=\[([^\]]*)\]")
NSI_ENTRY_RX = re.compile(r"([A-Za-z][A-Za-z0-9_]{0,15}):(\d+)/(\d+)raised")
DOORBELL_RX = re.compile(r"\bdbfast\[(?:ON|off)\b[^\]]*?\bdoorbells=(\d+)")
RETIRED_RX = re.compile(r"kf3: chan token (0x[0-9a-fA-F]+) \(host 0x[0-9a-fA-F]+\) RETIRED, forwarded=(\d+) submissions=(\d+)(?: splits=\d+/\d+)? serves=(\d+)")
GR_METHODS_RX = re.compile(r"\bgr\[methods=(\d+)")
DEAD_RX = re.compile(r"\bdead=Some\(")
XID_RX = re.compile(r"NVRM: Xid|\bXid\b")
MAX_TOKENS = 4096


def parse_status(line):
    """One `kf3: family=` status line -> {phase, births, tokens {token: (fwd, subs, serves)}, nsi {engine: wakes}, doorbells}; None for any other line."""
    m = STATUS_RX.search(line or "")
    if not m:
        return None
    b = BIRTHS_RX.search(line)
    tokens = {}
    t = TOKENS_RX.search(line)
    if t:
        for e in TOKEN_RX.finditer(t.group(1)):
            if len(tokens) < MAX_TOKENS:
                tokens[e.group(1).lower()] = (int(e.group(2)), int(e.group(3)), int(e.group(4)))
    nsi = {}
    n = NSI_RX.search(line)
    if n:
        for e in NSI_ENTRY_RX.finditer(n.group(1)):
            nsi[e.group(1)] = int(e.group(2))
    d = DOORBELL_RX.search(line)
    return {"phase": m.group(1), "births": int(b.group(1)) if b else None, "tokens": tokens, "nsi": nsi, "doorbells": int(d.group(1)) if d else None}


def last_status_line(lines):
    """The last `kf3: family=` line of an iterable of log lines (the baseline for the window that follows), or None."""
    found = None
    for line in lines:
        if "kf3: family=" in line:
            found = line
    return found


def _delta(end, start):
    """Growth of a cumulative counter; a counter that went DOWN (a restarted device) counts from zero; None when either side is unknown."""
    if end is None:
        return None
    if start is None:
        return end
    return end - start if end >= start else end


def window_census(lines, base_line=None):
    """The host census of one window. `lines`: the kf3 log lines added while the row ran; `base_line`: the last status line BEFORE the window (None when
    there was none: then every counter is a lower bound measured from the first status line inside the window and `base_missing` is True).

    -> {status_lines, base_missing, births, submissions, forwarded, serves, doorbells, wakes {engine: n}, retired, gr_methods, dead, xid}
    Submissions are derived from the identity total = retired + live: window work = sum(RETIRED lines) + live(end) - live(start), where live() sums the
    per-channel `subs` of the `tokens=[...]` list and a RETIRED line removes its token from the live set."""
    base = parse_status(base_line) if base_line else None
    first = None
    last = base
    live = dict(base["tokens"]) if base else {}
    retired = {"n": 0, "forwarded": 0, "submissions": 0, "serves": 0}
    gr_methods = dead = xid = statuses = 0
    for raw in lines:
        line = raw[:8192]
        if XID_RX.search(line):
            xid += 1
        st = parse_status(line) if "kf3: family=" in line else None
        if st is not None:
            statuses += 1
            first = first or st
            live = dict(st["tokens"])
            last = st
            continue
        m = RETIRED_RX.search(line)
        if m:
            retired["n"] += 1
            retired["forwarded"] += int(m.group(2))
            retired["submissions"] += int(m.group(3))
            retired["serves"] += int(m.group(4))
            live.pop(m.group(1).lower(), None)
            g = GR_METHODS_RX.search(line)
            gr_methods += int(g.group(1)) if g else 0
            dead += 1 if DEAD_RX.search(line) else 0
    start = base or first
    live_start = (base["tokens"] if base else (first["tokens"] if first else {}))
    live_end = live
    out = {"status_lines": statuses, "base_missing": base is None, "retired": retired["n"], "gr_methods": gr_methods, "dead": dead, "xid": xid,
           "births": None, "submissions": 0, "forwarded": 0, "serves": 0, "doorbells": None, "wakes": {}}
    if last is None or statuses == 0 and base is None:
        return out
    out["births"] = _delta(last["births"], start["births"] if start else None)
    out["doorbells"] = _delta(last["doorbells"], start["doorbells"] if start else None) if last["doorbells"] is not None else None
    out["submissions"] = max(0, retired["submissions"] + sum(v[1] for v in live_end.values()) - sum(v[1] for v in live_start.values()))
    out["forwarded"] = max(0, retired["forwarded"] + sum(v[0] for v in live_end.values()) - sum(v[0] for v in live_start.values()))
    out["serves"] = max(0, retired["serves"] + sum(v[2] for v in live_end.values()) - sum(v[2] for v in live_start.values()))
    names = set(last["nsi"]) | set((start or {}).get("nsi", {}))
    out["wakes"] = {n: _delta(last["nsi"].get(n, 0), (start or {}).get("nsi", {}).get(n, 0)) for n in sorted(names)
                    if _delta(last["nsi"].get(n, 0), (start or {}).get("nsi", {}).get(n, 0))}
    return out


def judge_census(c, min_submissions=MIN_SUBMISSIONS):
    """-> (verdict, reason). FAIL only when the window is measurable and shows no work; NOT_RUN when it cannot be measured."""
    if c is None:
        return NOT_RUN, "host census: not supplied"
    if c["status_lines"] == 0:
        return NOT_RUN, "host census: no kf3 status line in the window (empty log slice, or the device printed none)"
    if c["submissions"] >= min_submissions:
        return PASS, "host census: +%d submission(s), +%s channel birth(s) in the window" % (c["submissions"], c["births"] if c["births"] is not None else "?")
    if c["base_missing"]:
        return NOT_RUN, "host census: no status line before the window, so 'no work' cannot be told from 'work before the first line'"
    return FAIL, ("host census: no GPU work in the window (submissions +%d, channel births +%s, doorbells +%s, retired channels %d): "
                  "the guest never submitted work to the GPU through kf3" % (c["submissions"], c["births"], c["doorbells"], c["retired"]))


# ------------------------------------------------------------------------------------------------ the host's nvidia-smi (host-authored)
_GA = []


def gpu_assert():
    """scripts/bench/windows/gpu_assert.py (the shared record format of the Windows rows), loaded by path."""
    if not _GA:
        spec = importlib.util.spec_from_file_location("kf_gpu_assert", HERE / "windows" / "gpu_assert.py")
        mod = importlib.util.module_from_spec(spec)
        spec.loader.exec_module(mod)
        _GA.append(mod)
    return _GA[0]


def smi_record(query_text, dmon_text, require, **thresholds):
    """The host's nvidia-smi samples of the row's window -> the gpu_assert.evaluate record (source host) plus `unreadable`: no sample at all is a
    sampler problem (NOT_RUN here), never a pass and never a statement about the guest."""
    ga = gpu_assert()
    q, d = ga.parse_query_csv(_text(query_text)), ga.parse_dmon(_text(dmon_text))
    rec = ga.evaluate(q, d, tuple(require), **thresholds)
    rec["source"] = "host"
    rec["unreadable"] = bool(require) and rec.get("samples", 0) == 0 and rec.get("dmon_samples", 0) == 0
    rec["readings"] = host_readings(q, d, tuple(require), **thresholds)
    return rec


def host_readings(query_rows, dmon_rows, require=(), **thresholds):
    """The host nvidia-smi samples as FACTS, whether or not a reading was required: the engine (utilisation) and the memory reading each say whether the shared
    threshold was met and with which numbers (the same gpu_assert.evaluate that judges them, one observable at a time, so a fact and a verdict never disagree about a
    threshold). `required` says whether the caller asked it to decide. -> {unreadable, engine, memory[, encoder, decoder]}; no sample at all is {"unreadable": True}
    (an unreadable GPU is not an idle one)."""
    ga = gpu_assert()
    if not any(r["gpu"] is not None or r["mem_mb"] is not None for r in query_rows) and not dmon_rows:
        return {"unreadable": True}
    eng, mem = ga.evaluate(query_rows, dmon_rows, ("gpu",), **thresholds), ga.evaluate(query_rows, dmon_rows, ("mem",), **thresholds)
    peak, base = eng["peak"], eng["baseline"]
    rise = None if peak["mem_mb"] is None or base["mem_mb"] is None else round(peak["mem_mb"] - base["mem_mb"], 1)
    out = {"unreadable": False,
           "engine": {"met": eng["verdict"] == PASS, "required": "gpu" in require, "busy_samples": peak["busy_samples"],
                      "need_samples": thresholds.get("min_samples", ga.MIN_SAMPLES), "min_pct": thresholds.get("min_gpu", ga.MIN_GPU_PCT),
                      "peak_pct": peak["gpu"] if peak["gpu"] is not None else peak["gpu_dmon_sm"]},
           "memory": {"met": mem["verdict"] == PASS, "required": "mem" in require, "baseline_mib": base["mem_mb"], "peak_mib": peak["mem_mb"], "rise_mib": rise,
                      "need_mib": thresholds.get("min_mem", ga.MIN_MEM_MB)}}
    for key, name in (("enc", "encoder"), ("dec", "decoder")):
        if key in require:
            out[name] = {"met": ga.evaluate(query_rows, dmon_rows, (key,), **thresholds)["verdict"] == PASS, "required": True, "peak": peak[key]}
    return out


def judge_smi(rec, require):
    if not require:
        return PASS, "host engines: none required"
    if rec is None:
        return NOT_RUN, "host engines: no sampler record"
    if rec.get("unreadable"):
        return NOT_RUN, "host engines: the host nvidia-smi could not be read (no sample): an unreadable GPU is not an idle one"
    if rec.get("verdict") != PASS:
        return FAIL, "host engines: " + "; ".join(rec.get("reasons") or ["not busy"])
    return PASS, "host engines: %s busy (peak %s)" % ("+".join(require), {k: v for k, v in (rec.get("peak") or {}).items() if v})


def proved_by(census_verdict, smi, require):
    """Which host observable proved the use of a PASSING row, in the order of authority: "census" (the kf3 census passed), else, only when NO census was asked of
    the caller (`census_verdict` None: the bare-metal baseline), "engine" (the sampler was readable and passed with an engine required) or "memory" (only host memory
    was required); else None. A census that failed or could not be read proved nothing and nothing else may take its place (the caller only asks about a PASS)."""
    if census_verdict == PASS:
        return "census"
    if census_verdict is None and smi and not smi.get("unreadable") and smi.get("verdict") == PASS and require:
        return "engine" if any(ENGINE_OBSERVABLE[e] == "engine" for e in require) else "memory"
    return None


# ------------------------------------------------------------------------------------------------ app proofs (guest self-report)
# Each parser: (log text, **params) -> {"ok": True|False|None, "device": str, "reasons": [..], "facts": {..}}.
# ok False = the app reports a CPU / non-NVIDIA device or a failure marker; ok None = no proof line at all (the row dropped it).
def _proof(ok, device="", reasons=(), **facts):
    return {"ok": ok, "device": str(device)[:120], "reasons": list(reasons)[:6], "facts": facts}


def _nvidia(name):
    return bool(re.search(r"nvidia|geforce|rtx|quadro|tesla", str(name), re.I)) and not SOFT_GPU_NAMES.search(str(name))


BLENDER_OK_RX = re.compile(r"^BLENDER_OK (\w+) (gpu|cpu)=(.+?) mean=", re.M)
BLENDER_DEV_RX = re.compile(r"^BLENDER_DEVICES dev=(\w+) scene=(\w+) active=(\w+) cpu_enabled=(\w+) enabled=(\d+) names=(.*)$", re.M)
BLENDER_BAD_RX = re.compile(r"^BLENDER_FAIL|falling back to cpu|no (compatible )?gpu|(cuda|optix) (error|initiali[sz]ation failed)|failed to (initiali[sz]e|create) (cuda|optix)", re.I | re.M)


def proof_blender(text, devs=("CUDA", "OPTIX"), **_):
    """Cycles: for each requested device type the script must print BLENDER_DEVICES (scene device GPU, an active device, the CPU device NOT enabled) and
    BLENDER_OK <dev> gpu=<NVIDIA name>. `BLENDER_OK CPU` or `cpu=` is a CPU render."""
    text = _text(text)
    oks, devices = BLENDER_OK_RX.findall(text), {m[0]: m for m in BLENDER_DEV_RX.findall(text)}
    why, shown = [], ""
    if BLENDER_BAD_RX.search(text):
        why.append("Blender reported a device problem or a fallback")
    if not oks and not devices:
        return _proof(None, "", ["no BLENDER_OK / BLENDER_DEVICES line"] + why)
    for dev in devs:
        ok = [o for o in oks if o[0] == dev]
        dv = devices.get(dev)
        if not ok or dv is None:
            why.append("%s: %s missing (the row dropped its device proof)" % (dev, "BLENDER_OK" if not ok else "BLENDER_DEVICES"))
            continue
        kind, name = ok[-1][1], ok[-1][2]
        shown = name
        if kind != "gpu" or not _nvidia(name):
            why.append("%s: rendered on %s=%s (not an NVIDIA GPU)" % (dev, kind, name[:60]))
        _, scene, active, cpu_on, n_enabled, names = dv
        if scene != "GPU" or active != "True" or cpu_on != "False" or int(n_enabled) < 1 or not _nvidia(names):
            why.append("%s: Cycles device scene=%s active=%s cpu_enabled=%s enabled=%s names=%s" % (dev, scene, active, cpu_on, n_enabled, names[:60]))
    for o in oks:
        if o[0] in ("CPU", "NONE") or o[1] == "cpu":
            why.append("a CPU render was reported (%s %s=%s)" % (o[0], o[1], o[2][:40]))
    return _proof(not why, shown, why, devices=sorted(devices), oks=len(oks))


OFFLOAD_RX = re.compile(r"offloaded (\d+)/(\d+) layers to GPU")
CUDA_FOUND_RX = re.compile(r"ggml_cuda_init: found (\d+) CUDA devices?")
CUDA_DEVICE_RX = re.compile(r"^\s*Device (\d+): (.+?), compute capability", re.M)
LLAMA_USING_RX = re.compile(r"using device (CUDA\d+) \((.+?)\)")
BENCH_ROW_RX = re.compile(r"^\|[^|]*\|[^|]*\|[^|]*\|\s*([^|\s][^|]*?)\s*\|\s*(\d+)\s*\|\s*(?:pp|tg)\d+", re.M)


def proof_llama(text, mode="gen", **_):
    """llama.cpp: a CUDA device was found, ALL layers were offloaded (gen: `offloaded N/N layers to GPU` with N equal and non-zero) or every benchmark row
    ran on the CUDA backend with ngl >= 1 (bench). `offloaded 0/29` is a CPU run that still prints a pass string."""
    text = _text(text)
    found = [int(n) for n in CUDA_FOUND_RX.findall(text)]
    dev = CUDA_DEVICE_RX.search(text) or LLAMA_USING_RX.search(text)
    name = (dev.group(2) if dev and dev.re is CUDA_DEVICE_RX else (dev.group(2) if dev else ""))
    why = []
    if not any(n >= 1 for n in found) and not LLAMA_USING_RX.search(text):
        why.append("no CUDA device found by llama.cpp (ggml_cuda_init says nothing): a CPU-only run")
    elif name and not _nvidia(name):
        why.append("llama.cpp device is not NVIDIA: " + name[:60])
    if mode == "gen":
        off = OFFLOAD_RX.findall(text)
        if not off:
            return _proof(None if not why else False, name, ["no `offloaded N/N layers to GPU` line"] + why)
        a, b = int(off[-1][0]), int(off[-1][1])
        if a == 0:
            why.append("offloaded 0/%d layers: the model ran on the CPU" % b)
        elif a != b:
            why.append("partial offload %d/%d layers: part of the model ran on the CPU" % (a, b))
        return _proof(not why, name, why, offloaded=a, layers=b)
    rows = BENCH_ROW_RX.findall(text)
    if not rows:
        return _proof(None if not why else False, name, ["no llama-bench table row"] + why)
    bad = [r for r in rows if "CUDA" not in r[0].upper() or int(r[1]) < 1]
    if bad:
        why.append("llama-bench ran on backend %r with ngl %s" % (bad[0][0], bad[0][1]))
    return _proof(not why, name, why, rows=len(rows))


PROOF_LINE_RX = re.compile(r"^GPU_USE_PROOF (\w+) device=(\S{1,40}) name=(.{1,120}?) peak_mib=(\d{1,12})\s*$", re.M)


def proof_framework(text, framework="torch", min_mib=1, **_):
    """torch / cupy / transformers: the script prints `GPU_USE_PROOF <framework> device=cuda:N name=<NVIDIA ..> peak_mib=<n>` after its work, from the
    framework's own allocator (`torch.cuda.max_memory_allocated`, the CuPy pool). A cpu device, a non-NVIDIA name or too little device memory fails."""
    text = _text(text)
    hits = [h for h in PROOF_LINE_RX.findall(text) if h[0] == framework]
    if not hits:
        return _proof(None, "", ["no GPU_USE_PROOF %s line (the row dropped its device proof)" % framework])
    _, device, name, peak = hits[-1]
    why = []
    if not device.startswith("cuda"):
        why.append("device is %s, not cuda" % device[:20])
    if not _nvidia(name):
        why.append("device name is not an NVIDIA GPU: " + name[:60])
    if int(peak) < min_mib:
        why.append("peak device memory %s MiB is below %d MiB: the work did not run on the GPU" % (peak, min_mib))
    return _proof(not why, name, why, framework_device=device, peak_mib=int(peak))


SOFT_ENCODERS_RX = re.compile(r"\b(libx26[45]|libsvtav1|libaom-av1|libopenh264|libvpx(?:-vp9)?|libkvazaar)\b")
NVENC_RX = re.compile(r"\b(h264|hevc|av1)_nvenc\b")
ENCODER_TAG_RX = re.compile(r"^ENCODER_TAG (.*)$", re.M)
MAPPING_RX = re.compile(r"Stream #\d+:\d+ -> #\d+:\d+ \(.*\)")     # greedy to the end of the line: `(wrapped_avframe (native) -> h264 (h264_nvenc))` nests parentheses


def proof_nvenc(text, codec="h264", **_):
    """ffmpeg encode: the output stream's own `encoder` tag (ffprobe, printed as ENCODER_TAG) or the stream-mapping line must name `<codec>_nvenc`, and no
    software encoder (libx264, libx265, ...) may be named anywhere."""
    text = _text(text)
    tags = ENCODER_TAG_RX.findall(text)
    maps = MAPPING_RX.findall(text)
    named = [t for t in tags + maps if NVENC_RX.search(t)]
    why = []
    soft = [t for t in tags + maps if SOFT_ENCODERS_RX.search(t)]
    if soft:
        why.append("a software encoder is named: " + soft[0][:70])
    if not tags and not maps:
        return _proof(None, "", ["no ENCODER_TAG / stream-mapping line"] + why)
    if not any(("%s_nvenc" % codec) in t for t in named):
        why.append("the stream was not encoded by %s_nvenc" % codec)
    return _proof(not why, "%s_nvenc" % codec if not why else "", why)


NVDEC_FALLBACK_RX = re.compile(r"NVDEC_NOT_USED|Failed setup for format cuda|hwaccel initiali[sz]ation returned error|Device creation failed|"
                               r"No device available for decoder|Cannot load libcuda|cuvid.*(?:error|failed)", re.I)
NVDEC_POSITIVE_RX = re.compile(r"Reinit context to \d+x\d+, pix_fmt: cuda|Using auto hwaccel type cuda|Format cuda chosen by get_format")
NVDEC_SOFTWARE_RX = re.compile(r"Reinit context to \d+x\d+, pix_fmt: (?:yuv|nv12|p010|gray)\w*")


def proof_nvdec(text, **_):
    """ffmpeg decode with `-hwaccel cuda`: the fallback markers fail the row; the POSITIVE marker (`pix_fmt: cuda` of the decoder, at -v verbose) is
    INFERRED from the ffmpeg sources and not measured, so its absence is `ok None` (the row records NOT_RUN, never a pass)."""
    text = _text(text)
    bad = NVDEC_FALLBACK_RX.search(text)
    if bad:
        return _proof(False, "", ["ffmpeg fell back to software decoding: " + bad.group(0)[:60]])
    soft = NVDEC_SOFTWARE_RX.search(text)
    if soft and not NVDEC_POSITIVE_RX.search(text):
        return _proof(False, "", ["the decoder outputs a software pixel format (%s): decoded on the CPU" % soft.group(0).split(", ")[-1]])
    if NVDEC_POSITIVE_RX.search(text):
        return _proof(True, "cuda hwaccel", [])
    return _proof(None, "", ["no positive hwaccel marker (`pix_fmt: cuda`) in the log: hardware decoding is not proven"])


HASHCAT_DEV_RX = re.compile(r"^\* Device #(\d+): (.+?), (skipped|\d+/\d+ MB.*|\d+ MB.*)$", re.M)     # a device line ends in `, skipped` or `, <used>/<total> MB`: hashcat also prints NOTICES as `* Device #1: <text>`


def proof_hashcat(text, **_):
    """hashcat: the devices it USES (a `* Device #N:` line that does not end in `skipped`) must all be NVIDIA GPUs, and there must be at least one."""
    text = _text(text)
    devs = [(m[0], m[1], m[2]) for m in HASHCAT_DEV_RX.findall(text)]
    if not devs:
        return _proof(None, "", ["no `* Device #N:` line (hashcat prints it at the top of its output)"])
    used = [d for d in devs if d[2] != "skipped"]
    why = []
    if not used:
        why.append("every hashcat device was skipped")
    cpu = [d for d in used if not _nvidia(d[1])]
    if cpu:
        why.append("hashcat used a non-NVIDIA device: #%s %s" % (cpu[0][0], cpu[0][1][:60]))
    return _proof(not why, used[0][1] if used else "", why, devices=len(devs), used=len(used))


def proof_opencl_bench(text, **_):
    """clpeak: it benchmarks EVERY OpenCL device (the image also carries pocl, a CPU device). The row passes only when an NVIDIA device's own block holds
    a global-memory-bandwidth result; a CPU block passing the same regex proves nothing."""
    text = _text(text)
    blocks, cur = [], None
    for line in text.splitlines():
        m = re.match(r"\s*Device: (.+?)\s*$", line)
        if m:
            cur = {"name": m.group(1), "bw": False, "skipped": False}
            blocks.append(cur)
        elif cur is not None:
            if "Global memory bandwidth" in line:
                cur["bw"] = True
            if re.search(r"Tests skipped|clFinish \(-", line):
                cur["skipped"] = True
    if not blocks:
        return _proof(None, "", ["no `Device:` block in the clpeak output"])
    gpu = [b for b in blocks if _nvidia(b["name"]) and b["bw"]]
    why = []
    if not gpu:
        why.append("no NVIDIA device block holds a bandwidth result (devices: %s)" % ", ".join(b["name"][:30] for b in blocks[:4]))
    elif any(b["skipped"] for b in gpu):
        why.append("an NVIDIA device block reports skipped tests")
    return _proof(not why, gpu[0]["name"] if gpu else "", why, devices=[b["name"][:40] for b in blocks[:4]])


def proof_vkpeak(text, **_):
    text = _text(text)
    m = re.search(r"^device\s*=\s*(.+?)\s*$", text, re.M)
    if not m:
        return _proof(None, "", ["no `device =` line (vkpeak prints the device it benchmarked)"])
    why = [] if _nvidia(m.group(1)) else ["vkpeak ran on %r, not an NVIDIA GPU (device 0 may be a CPU Vulkan driver)" % m.group(1)[:60]]
    return _proof(not why, m.group(1), why)


def proof_egl(text, **_):
    text = _text(text)
    m = re.search(r"^\[egl\] renderer: (.+?)\s*$", text, re.M)
    if not m:
        return _proof(None, "", ["no `[egl] renderer:` line"])
    why = [] if _nvidia(m.group(1)) else ["GL_RENDERER %r is not an NVIDIA GPU (software rasteriser?)" % m.group(1)[:60]]
    return _proof(not why, m.group(1), why)


PROOFS = {"blender": proof_blender, "llama": proof_llama, "framework": proof_framework, "nvenc": proof_nvenc, "nvdec": proof_nvdec,
          "hashcat": proof_hashcat, "opencl_bench": proof_opencl_bench, "vkpeak": proof_vkpeak, "egl": proof_egl}


# ------------------------------------------------------------------------------------------------ the verdict
def judge(name, census=None, smi=None, app=None, need_census=True, need_engines=(), side="guest", app_missing="FAIL", app_required=True,
          min_submissions=MIN_SUBMISSIONS, census_primary=False):
    """One row -> {row, verdict, reasons [named, source-tagged], authority, census, smi, app, proved_by, readings}.

    side "guest": the host census is the authority (need_census True means it MUST show work). side "host": a bare-metal baseline: there is no kf3 log,
    the census is not asked for and the record says authority=guest unless the host engines were sampled.
    census_primary (opt-in, only meaningful for side "guest" with a census asked for, which is how the kf3 platform is known here; 2026-11-18): a census that PASSES
    decides, and an engine or memory reading that FAILS is kept as a fact (`[host-smi] FACT`, `readings`) instead of failing the row. A census that fails still fails
    the row, whatever nvidia-smi shows; a census that is not run decides nothing, so the readings decide as they always did.
    app_missing: what a row whose proof line is absent becomes ("FAIL": the row dropped it; "NOT_RUN": the marker is unmeasured)."""
    reasons, verdicts, facts = [], [], []

    def add(v, why, src):
        verdicts.append(v)
        reasons.append("[%s] %s: %s" % (src, v, why))

    host_seen, census_v = False, None
    if need_census and side == "guest":
        census_v, why = judge_census(census, min_submissions)
        add(census_v, why, "host-census")
        host_seen = host_seen or census_v != NOT_RUN
    if need_engines:
        v, why = judge_smi(smi, need_engines)
        if census_primary and census_v == PASS and v == FAIL:
            facts.append("[host-smi] FACT: %s (%s)" % (why, CENSUS_PRIMARY_REASON))
        else:
            add(v, why, "host-smi")
        host_seen = host_seen or v != NOT_RUN
    if app_required:
        if app is None:
            add(NOT_RUN, "no application log to read", "app")
        elif app["ok"] is True:
            add(PASS, "the application reports %s" % (app["device"] or "its GPU device"), "app")
        elif app["ok"] is False:
            add(FAIL, "; ".join(app["reasons"]) or "the application reports a non-GPU device", "app")
        else:
            add(app_missing, "; ".join(app["reasons"]) or "no device proof", "app")
    failed, not_run = FAIL in verdicts, NOT_RUN in verdicts
    verdict = FAIL if failed else (NOT_RUN if not_run else PASS)
    if failed and any(v == FAIL and r.startswith("[host-") for v, r in zip(verdicts, reasons)) and app and app["ok"] is True:
        reasons.append("[note] the guest's own output claims a GPU (%s) but the host observables say otherwise: the host census is the authority" % (app["device"] or "?"))
    return {"row": name, "verdict": verdict, "reasons": reasons + facts, "authority": "host" if host_seen else "guest", "census": census, "smi": _slim_smi(smi), "app": app,
            "proved_by": proved_by(census_v, smi, need_engines) if verdict == PASS else None, "readings": (smi or {}).get("readings")}


def _slim_smi(rec):
    if not rec:
        return None
    return {k: rec.get(k) for k in ("required", "verdict", "reasons", "peak", "baseline", "samples", "source", "unreadable", "readings")}


def control_verdict(judged):
    """A negative control (the row with the GPU hidden, or the application's own CPU switch) is judged by the same function: it must be FAIL."""
    return "CONTROL_OK" if judged["verdict"] == FAIL else "CONTROL_BROKEN"
