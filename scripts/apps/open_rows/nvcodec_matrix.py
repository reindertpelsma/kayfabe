#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later
"""FFmpeg NVENC/NVDEC capability matrix row: run it on this machine, or compare a host run with a guest run (stdlib only).

STATUS: LIVE, 2026-11-17 (design: docs/design/V3_OPEN_ROWS_FFMPEG_DCGM.md; decisions are in nvcodec_lib.py and GPU-free tested). Run
natively on Vast boxes (traces/open_rows_ffdcgm_20261117/); not run in a kayfabe guest yet: that is the next gate run.

    nvcodec_matrix.py run --out DIR [--only GLOB] [--probe PATH] [--ffmpeg PATH] [--ffprobe PATH] [--bundle DIR]
    nvcodec_matrix.py compare HOST_DIR GUEST_DIR
    nvcodec_matrix.py list

`run` writes DIR/probe.tsv (what the driver reports), DIR/results.tsv (one row per case), DIR/summary.txt and DIR/logs/. The expected
outcome of each case comes from probe.tsv of THE SAME MACHINE; a case is PASS when it is supported and worked, NOT_SUPPORTED when it is
not supported and failed, FAIL / MISMATCH otherwise, NOT_RUN when the machine could not be asked. The last line printed is
`NVCODEC_MATRIX_SUMMARY ...` and the exit status is 0 only when nothing FAILED or MISMATCHED and at least one case PASSED.
A path given with --ffmpeg/--ffprobe/--probe is trusted as given (its sha256 is recorded in the summary); by default ffmpeg is the
distribution's `ffmpeg` on PATH and the probe is built by provision.sh from the pinned upstream headers.
"""
import argparse
import fnmatch
import hashlib
import os
from pathlib import Path
import shlex
import shutil
import signal
import subprocess
import sys
import time

HERE = Path(__file__).resolve().parent
sys.path.insert(0, str(HERE))
import census  # noqa: E402
import nvcodec_lib as L  # noqa: E402
import rowio  # noqa: E402

CASES = HERE / "nvcodec_cases.tsv"
LIMITS = HERE / "nvcodec_known_limits.tsv"
TIMEOUT_S = {"size": 600, "decode": 240, "sessions": 120}


def run_cmd(argv, timeout, env=None):
    """-> (rc, stderr text, seconds). Own process group, killed whole on timeout (rc 124); never a shell."""
    t0 = time.time()
    try:
        p = subprocess.Popen(argv, stdout=subprocess.PIPE, stderr=subprocess.PIPE, text=True, errors="replace", start_new_session=True,
                             stdin=subprocess.DEVNULL, env=env)
    except OSError as e:
        return 127, "cannot execute %s: %s" % (argv[0], e), 0.0
    try:
        out, err = p.communicate(timeout=timeout)
    except subprocess.TimeoutExpired:
        os.killpg(p.pid, signal.SIGKILL)
        out, err = p.communicate()
        return 124, (err or "") + "\ntimeout after %ds" % timeout, time.time() - t0
    return p.returncode, (err or "") + (out or ""), time.time() - t0


def sha256_of(path):
    h = hashlib.sha256()
    with open(path, "rb") as f:
        for chunk in iter(lambda: f.read(1 << 20), b""):
            h.update(chunk)
    return h.hexdigest()


def find_tool(explicit, name, extra=()):
    if explicit:
        return explicit if os.access(explicit, os.X_OK) else None
    for c in extra:
        if c and os.access(c, os.X_OK):
            return str(c)
    return shutil.which(name)


def ffmpeg_lists(ffmpeg):
    """(encoders, decoders, hwaccels) names from ffmpeg's own listings."""
    sets = []
    for flag in ("-encoders", "-decoders", "-hwaccels"):
        rc, out, _ = run_cmd([ffmpeg, "-hide_banner", flag], 30)
        names = set()
        for line in out.splitlines():
            parts = line.split()
            if flag == "-hwaccels":
                if len(parts) == 1 and not line.startswith("Hardware"):
                    names.add(parts[0])
            elif len(parts) >= 2 and len(parts[0]) == 6 and parts[0][0] in "VAS.":
                names.add(parts[1])
        sets.append(names)
    return sets


class Runner:
    def __init__(self, a):
        self.out = Path(a.out)
        self.work = self.out / "work"
        self.logs = self.out / "logs"
        for d in (self.out, self.work, self.logs):
            d.mkdir(parents=True, exist_ok=True)
        bundle = Path(a.bundle) if a.bundle else None
        self.ffmpeg = find_tool(a.ffmpeg, "ffmpeg")
        self.ffprobe = find_tool(a.ffprobe, "ffprobe")
        self.probe_bin = find_tool(a.probe, "nvcodec_probe", [bundle / "bin/nvcodec_probe" if bundle else None, HERE / "nvcodec_probe"])
        self.only = a.only
        self.emit = a.emit
        self.stream = census.SmiStream().start()
        self.probe = L.Probe()
        self.stream_cache = {}

    def log(self, case_id, argv, rc, secs, text):
        body = "$ %s\nrc=%s secs=%.1f\n%s\n" % (" ".join(shlex.quote(x) for x in argv), rc, secs, text[-6000:])
        (self.logs / (case_id + ".log")).write_text(census.scrub(body))

    def run_probe(self):
        if not self.probe_bin:
            return
        rc, text, _ = run_cmd([self.probe_bin], 120)
        (self.out / "probe.tsv").write_text(census.scrub(text))
        self.probe = L.parse_probe(text)

    # ---------------------------------------------------------------------------------------------- one case
    def attempt_encode(self, case, w):
        out = str(self.work / (case["id"] + ".mkv"))
        argv = L.encode_argv(self.ffmpeg, case, out)
        rc, err, secs = run_cmd(argv, TIMEOUT_S.get(case["group"], 240))
        self.log(case["id"], argv, rc, secs, err)
        w["secs"] = "%.1f" % secs
        w["rc"] = rc
        if rc != 0:
            return False, L.classify_reason(err), ""
        info = L.parse_stream_json(run_cmd(L.probe_stream_argv(self.ffprobe, out), 120)[1])
        if info is None or info["codec"] != {"h264": "h264", "hevc": "hevc", "av1": "av1"}[case["codec"]]:
            return False, "output_invalid", "ffprobe could not read a %s stream" % case["codec"]
        w["frames"] = info["frames"]
        if (info["width"], info["height"]) != (case["width"], case["height"]) or info["frames"] != case["frames"]:
            return False, "output_wrong_shape", "got %dx%d x%d frames" % (info["width"], info["height"], info["frames"])
        prc, perr, _ = run_cmd(L.psnr_argv(self.ffmpeg, case, out), TIMEOUT_S.get(case["group"], 240))
        psnr = L.parse_psnr(perr)
        w["psnr"] = "inf" if psnr == float("inf") else ("%.2f" % psnr if psnr is not None else "-")
        Path(out).unlink(missing_ok=True)
        if prc != 0 or psnr is None:
            return False, "output_not_decodable", L.classify_reason(perr)
        if not L.psnr_ok(psnr, case["psnr_min"]):
            return False, "psnr_below_threshold", "psnr %.2f < %.1f" % (psnr, case["psnr_min"])
        return True, "ok", ""

    def attempt_decode(self, case, w, enc_names, dec_names, hwaccels):
        swenc = L.pick_sw_encoder(case["codec"], enc_names)
        if swenc is None:
            return None, "no_sw_encoder", "none of %s in this ffmpeg" % ",".join(L.SW_ENCODERS[case["codec"]])
        if case["mode"] == "cuvid" and L.CUVID[case["codec"]] not in dec_names:
            return None, "unknown_decoder", L.CUVID[case["codec"]]
        if case["mode"] == "hwaccel_cuda" and "cuda" not in hwaccels:
            return None, "unknown_decoder", "this ffmpeg has no -hwaccel cuda"
        key = (case["codec"], case["pix_fmt"], case["width"], case["height"], case["frames"])
        stream = self.stream_cache.get(key)
        if stream is None:
            stream = str(self.work / ("src_%s_%s_%dx%d.mkv" % (case["codec"], case["pix_fmt"], case["width"], case["height"])))
            argv = L.source_stream_argv(self.ffmpeg, case, swenc, stream)
            rc, err, secs = run_cmd(argv, 600)
            self.log(case["id"] + ".source", argv, rc, secs, err)
            if rc != 0:
                return None, "source_generation_failed", "%s could not make the test stream" % swenc
            self.stream_cache[key] = stream
        argv = L.decode_compare_argv(self.ffmpeg, case, stream)
        rc, err, secs = run_cmd(argv, TIMEOUT_S.get(case["group"], 240))
        self.log(case["id"], argv, rc, secs, err)
        w["secs"] = "%.1f" % secs
        w["rc"] = rc
        psnr = L.parse_psnr(err)
        w["psnr"] = "inf" if psnr == float("inf") else ("%.2f" % psnr if psnr is not None else "-")
        if rc != 0 or psnr is None:
            return False, L.classify_reason(err), ""
        if not L.psnr_ok(psnr, case["psnr_min"]):
            return False, "psnr_below_threshold", "hardware decode differs from software: %.2f dB" % psnr
        return True, "ok", ""

    def attempt_sessions(self, case, w):
        n = case["sessions"]
        argv = L.session_argv(self.ffmpeg, case)
        t0 = time.time()
        procs = []
        for _ in range(n):
            procs.append(subprocess.Popen(argv, stdout=subprocess.DEVNULL, stderr=subprocess.PIPE, text=True, errors="replace",
                                          stdin=subprocess.DEVNULL, start_new_session=True))
        errs, n_ok = [], 0
        for p in procs:
            try:
                _, err = p.communicate(timeout=TIMEOUT_S["sessions"])
            except subprocess.TimeoutExpired:
                os.killpg(p.pid, signal.SIGKILL)
                _, err = p.communicate()
                err += "\ntimeout"
            errs.append(err or "")
            n_ok += p.returncode == 0
        secs = time.time() - t0
        self.log(case["id"], argv, "n_ok=%d/%d" % (n_ok, n), secs, "\n--\n".join(errs[:3]))
        w["secs"] = "%.1f" % secs
        w["n_ok"] = "%d/%d" % (n_ok, n)
        cap = int(self.probe.sessions.get("opened", "0") or 0)
        ok, note = L.session_outcome(n_ok, n, cap)
        reason = L.classify_reason("\n".join(errs)) if not ok else "ok"
        return ok, reason, note

    def run_case(self, case, lists):
        enc_names, dec_names, hwaccels = lists
        expect, why = L.evaluate_needs(case, self.probe)
        w = {"case_id": case["id"], "group": case["group"], "expect": expect}
        t0, ok, reason, note, forced = time.time(), None, "", "; ".join(why), None
        if not self.ffmpeg:
            forced = "no_ffmpeg"
        elif case["op"] != "dec" and L.ENCODERS[case["codec"]] not in enc_names:
            forced = "unknown_encoder"
        elif expect == "UNKNOWN":
            forced = "no_probe"
        if forced is None:
            if case["op"] == "enc":
                ok, reason, extra = self.attempt_encode(case, w)
            elif case["op"] == "dec":
                ok, reason, extra = self.attempt_decode(case, w, enc_names, dec_names, hwaccels)
            else:
                ok, reason, extra = self.attempt_sessions(case, w)
            note = (note + "; " + extra).strip("; ") if extra else note
        t1 = time.time()
        if case["op"] in ("enc", "sessions"):
            pk = census.peaks(self.stream.window(t0, t1))["sess"]
            w["sess_peak"] = "-" if pk is None else int(pk)
        res, rsn = L.verdict(expect, ok, reason, forced)
        if case["op"] == "sessions":
            res, rsn = L.sessions_result(res, rsn, note)
        w.update(result=res, reason=rsn, detail=note)
        return w

    def run(self):
        self.run_probe()
        header = {"tool": "nvcodec_matrix", "device": self.probe.meta.get("device_name", "-"), "cc": self.probe.meta.get("compute_capability", "-"),
                  "driver_api": self.probe.meta.get("driver_api_max", "-"), "header_api": self.probe.meta.get("header_api", "-"),
                  "sessions_opened": self.probe.sessions.get("opened", "-"), "ffmpeg": self.ffmpeg or "-",
                  "ffmpeg_sha256": sha256_of(self.ffmpeg)[:16] if self.ffmpeg else "-"}
        lists = ffmpeg_lists(self.ffmpeg) if self.ffmpeg else (set(), set(), set())
        cases = L.load_cases(CASES.read_text())
        rows = []
        for case in cases:
            if self.only and not any(fnmatch.fnmatchcase(case["id"], g) for g in self.only.split(",")):
                continue
            r = self.run_case(case, lists)
            rows.append(r)
            print("CASE %s expect=%s result=%s reason=%s rc=%s secs=%s" % (r["case_id"], r["expect"], r["result"], r["reason"],
                                                                         r.get("rc", "-"), r.get("secs", "-")), flush=True)
        samples = self.stream.stop()
        (self.out / "hostsmi.csv").write_text(census.samples_to_csv(samples))
        (self.out / "results.tsv").write_text("\t".join(L.RESULT_COLUMNS) + "\n" + "\n".join(L.row_to_tsv(r) for r in rows) + "\n")
        for d in self.work.iterdir():
            d.unlink()
        self.work.rmdir()
        counts, ok = L.summarize(rows)
        use, why = census.gpu_use_verdict(census.peaks(samples), "enc")
        line = ("NVCODEC_MATRIX_SUMMARY pass=%d not_supported=%d fail=%d mismatch=%d not_run=%d cases=%d gpu_use=%s device=%s driver_api=%s "
                "sessions_opened=%s" % (counts["PASS"], counts["NOT_SUPPORTED"], counts["FAIL"], counts["MISMATCH"], counts["NOT_RUN"],
                                         len(rows), use, header["device"].replace(" ", "_"), header["driver_api"], header["sessions_opened"]))
        (self.out / "summary.txt").write_text(census.scrub("# %s\n# gpu_use_why=%s\n%s\n" % (
            " ".join("%s=%s" % kv for kv in header.items()), why, line)))
        if self.emit:
            for name in ("probe.tsv", "results.tsv", "summary.txt"):
                if (self.out / name).is_file():
                    sys.stdout.write(rowio.emit(name, (self.out / name).read_text()))
        print(line)
        return 0 if ok and counts["PASS"] > 0 else 1


def cmd_compare(a):
    hr, gr = rowio.load(a.host, "results.tsv"), rowio.load(a.guest, "results.tsv")
    if hr is None or gr is None:
        print("NVCODEC_COMPARE error=missing_results_tsv host=%s guest=%s" % (hr is not None, gr is not None))
        return 2
    host, guest = L.parse_results(hr), L.parse_results(gr)
    limits = L.load_known_limits(LIMITS.read_text()) if LIMITS.exists() else []
    rows = L.compare_results(host, guest, limits)
    for r in rows:
        print("CMP %(case_id)s host=%(host)s guest=%(guest)s %(agreement)s bucket=%(bucket)s host_reason=%(host_reason)s guest_reason=%(guest_reason)s" % r)
    hp, gp = rowio.load(a.host, "probe.tsv"), rowio.load(a.guest, "probe.tsv")
    diffs = L.compare_probes(L.parse_probe(hp), L.parse_probe(gp)) if hp and gp else []
    for k, x, y in diffs:
        print("CAPDIFF %s host=%s guest=%s" % (k, x, y))
    bad = [r for r in rows if r["agreement"] != "AGREE"]
    cand = [r for r in bad if r["bucket"] == L.BUCKET_CANDIDATE]
    print("NVCODEC_COMPARE cases=%d agree=%d disagree=%d candidates=%d capdiffs=%d" % (len(rows), len(rows) - len(bad), len(bad), len(cand), len(diffs)))
    return 1 if cand else 0


def main(argv):
    ap = argparse.ArgumentParser(description=__doc__.split("\n\n")[0])
    sub = ap.add_subparsers(dest="cmd", required=True)
    r = sub.add_parser("run")
    r.add_argument("--out", required=True)
    r.add_argument("--only")
    r.add_argument("--probe")
    r.add_argument("--ffmpeg")
    r.add_argument("--ffprobe")
    r.add_argument("--bundle")
    r.add_argument("--emit", action="store_true")
    c = sub.add_parser("compare")
    c.add_argument("host")
    c.add_argument("guest")
    sub.add_parser("list")
    a = ap.parse_args(argv)
    if a.cmd == "list":
        for case in L.load_cases(CASES.read_text()):
            print(case["id"])
        return 0
    if a.cmd == "compare":
        return cmd_compare(a)
    return Runner(a).run()


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
