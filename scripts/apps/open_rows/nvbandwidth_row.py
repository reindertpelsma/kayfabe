#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later
"""nvbandwidth row: the copy-engine and SM-copy paths of one GPU, run on this machine, or compared between a host run and a guest run (stdlib only).

STATUS: LIVE, 2026-11-23 (design: docs/design/V3_OPEN_LANES_NVBW_LLAMAVK_LHM.md; the decisions are pure functions here and are GPU-free tested by
scripts/ci/test_open_lanes.py). The native run on a Vast box is in traces/open_lanes_20261123/; the row has not run in a kayfabe guest.

    nvbandwidth_row.py run --out DIR [--bin PATH] [--bundle DIR] [--only GLOB] [--buffer-mib N] [--samples N] [--emit]
    nvbandwidth_row.py compare HOST_DIR GUEST_DIR
    nvbandwidth_row.py list

`run` asks the binary for its own testcase list (`-l`), then runs every testcase in a process of its own (`-t ID -j`, so one crash costs one case and is
attributed to it). The expectation of each case comes from the machine: a case that needs two GPUs is waived by nvbandwidth itself on one GPU, and
that is the expected outcome; a case that needs one GPU and is waived, errors, crashes or reports no positive number is a FAIL. nvbandwidth verifies
the copied data itself (it is not run with -s). The row passes only when nothing failed or mismatched and a copy-engine case, an SM-copy case, the
pointer-chase latency case and the device-local copy all passed: those are the paths the row exists to exercise.

GPU use: a copy has no CPU form, so the row cannot silently fall back. It still prints one `NVBANDWIDTH_PROOF` line from its own measurements
(device name from the binary's JSON, the number of passed cases per class, the smallest positive figure) which scripts/bench/gpu_use.py judges, and
in a native run it samples the machine's nvidia-smi (census.py) and records whether memory.used rose by the buffers the cases allocate. The negative
control is the same row with the GPU hidden (CUDA_VISIBLE_DEVICES set empty): no device is listed, nothing passes, the row FAILs.
Last line: `NVBANDWIDTH_SUMMARY status=PASS|FAIL|NOT_RUN ...`; exit 0 on PASS, and on NOT_RUN after an `APPNOTRUN <reason>` line (run_apps.sh makes that a NOTRUN row).
"""
import argparse
import fnmatch
import json
import os
from pathlib import Path
import re
import shutil
import signal
import subprocess
import sys
import time

HERE = Path(__file__).resolve().parent
sys.path.insert(0, str(HERE))
import census  # noqa: E402
import rowio  # noqa: E402

CASES = HERE / "nvbandwidth_cases.tsv"
CLASSES = ("ce", "sm", "latency", "local")
REQUIRED_CLASSES = ("ce", "sm", "latency", "local")
DEFAULT_BUFFER_MIB = 512        # nvbandwidth's own default (it warns about anything smaller)
DEFAULT_SAMPLES = 3
CASE_TIMEOUT_S = 300
# the smallest figure a working copy can show, GB/s (latency: any positive ns): a PCIe 1.0 x1 link is 0.25 GB/s. UNCALIBRATED, a floor and not a performance claim
MIN_GBPS = 0.05
MEM_RISE_NEED_MIB = 16          # the census.py threshold of "memory.used rose" (a case allocates BUFFER_MIB per buffer)
UNEXPECTED = ("FAIL", "MISMATCH")

DEVICE_RX = re.compile(r"^(\d+): (.+?) \([0-9a-fA-F]{8}:[0-9a-fA-F]{2}:[0-9a-fA-F]{2}\)")
LIST_RX = re.compile(r"^\d+, ([a-z0-9_]+):\s*$", re.M)
NVIDIA_RX = re.compile(r"nvidia|geforce|rtx|quadro|tesla", re.I)
SOFT_RX = re.compile(r"llvmpipe|lavapipe|swiftshader|software|microsoft basic", re.I)
NUMBER_RX = re.compile(r"^\d+(\.\d+)?([eE][-+]?\d+)?$")


# ---------------------------------------------------------------------------------------------------------------- the table
def load_cases(text):
    """-> list of {id, class, min_gpus}. Raises ValueError on a malformed table (a duplicate id, an unknown class, a bad count)."""
    rows, seen = [], set()
    lines = [l for l in text.splitlines() if l.strip() and not l.startswith("#")]
    if not lines or lines[0].split("\t") != ["id", "class", "min_gpus"]:
        raise ValueError("the header must be id<TAB>class<TAB>min_gpus")
    for n, line in enumerate(lines[1:], 2):
        cells = line.split("\t")
        if len(cells) != 3:
            raise ValueError("row %d: 3 tab separated cells expected, got %d" % (n, len(cells)))
        cid, cls, need = cells
        if not re.fullmatch(r"[a-z][a-z0-9_]*", cid):
            raise ValueError("row %d: bad id %r" % (n, cid))
        if cid in seen:
            raise ValueError("row %d: duplicate id %s" % (n, cid))
        if cls not in CLASSES:
            raise ValueError("row %d: unknown class %r" % (n, cls))
        if need not in ("1", "2"):
            raise ValueError("row %d: min_gpus must be 1 or 2" % n)
        seen.add(cid)
        rows.append({"id": cid, "class": cls, "min_gpus": int(need)})
    return rows


# ---------------------------------------------------------------------------------------------------------------- parsing nvbandwidth's output
def parse_listing(text):
    """The testcase names of `nvbandwidth -l` (lines `N, name:`), in order."""
    return LIST_RX.findall(text or "")


def parse_json(text):
    """The JSON document nvbandwidth -j prints -> its `nvbandwidth` object, or None (no document, not an object, no such key). Anything the binary
    printed before the document (a warning line) is skipped."""
    text = text or ""
    start = text.find("{")
    if start < 0:
        return None
    try:
        doc, _ = json.JSONDecoder().raw_decode(text[start:])
    except ValueError:
        return None
    body = doc.get("nvbandwidth") if isinstance(doc, dict) else None
    return body if isinstance(body, dict) else None


def device_names(body):
    """The device names of the `GPU Device list` (PCI address and host name dropped). A body without the list has no devices."""
    names = []
    for item in (body or {}).get("GPU Device list") or []:
        m = DEVICE_RX.match(str(item))
        names.append(m.group(2) if m else str(item).split(":", 1)[-1].strip()[:80])
    return names


def matrix_values(testcase):
    """The positive numbers of a testcase's bandwidth_matrix (strings, `N/A` for cells with no value)."""
    vals = []
    for row in testcase.get("bandwidth_matrix") or []:
        for cell in row if isinstance(row, list) else [row]:
            s = str(cell).strip()
            if NUMBER_RX.match(s) and float(s) > 0:
                vals.append(float(s))
    return vals


def judge_case(case, ngpus, rc, body, timed_out=False):
    """-> (result, reason, status, figures). result: PASS | WAIVED | FAIL | MISMATCH | NOT_RUN.
    `case` is a table row, `ngpus` the number of devices the binary listed, `rc` the process exit status, `body` the parsed JSON (or None)."""
    if timed_out:
        return "FAIL", "timeout", "Timeout", []
    if rc is None:
        return "NOT_RUN", "binary_missing", "-", []
    if rc != 0 and body is None:
        return "FAIL", "crash_rc%s" % rc, "Crash", []
    if body is None:
        return "FAIL", "no_json", "NoOutput", []
    if body.get("error"):
        return "FAIL", "binary_error", "Error", []
    cases = [t for t in body.get("testcases") or [] if isinstance(t, dict) and t.get("name") == case["id"]]
    if not cases:
        return "FAIL", "case_not_reported", "NoCase", []
    tc = cases[-1]
    status = str(tc.get("status", "-"))
    needs = case["min_gpus"]
    if status == "Waived":
        if ngpus >= needs:
            return "FAIL", "unexpected_waive", status, []
        return "WAIVED", "needs_%d_gpus_has_%d" % (needs, ngpus), status, []
    if status != "Passed":
        return "FAIL", "status_" + re.sub(r"[^A-Za-z0-9]+", "_", status).strip("_").lower()[:24], status, []
    if ngpus < needs:
        return "MISMATCH", "passed_without_%d_gpus" % needs, status, []
    vals = matrix_values(tc)
    if not vals:
        return "FAIL", "no_positive_figure", status, []
    if min(vals) < MIN_GBPS and case["class"] != "latency":
        return "FAIL", "below_floor_%g" % min(vals), status, vals
    return "PASS", "ok", status, vals


def class_counts(results):
    """results: list of {class, result}. -> {class: passed cases}."""
    out = dict.fromkeys(CLASSES, 0)
    for r in results:
        if r["result"] == "PASS":
            out[r["class"]] += 1
    return out


def summarize(results):
    """-> (status, counts, reason). PASS: nothing FAILed or MISMATCHed and every required class has a passed case. NOT_RUN: nothing ran at all
    (our own configuration, never a statement about the GPU). FAIL otherwise."""
    counts = {k: 0 for k in ("PASS", "WAIVED", "FAIL", "MISMATCH", "NOT_RUN")}
    for r in results:
        counts[r["result"]] += 1
    ran = counts["PASS"] + counts["WAIVED"] + counts["FAIL"] + counts["MISMATCH"]
    if ran == 0:
        return "NOT_RUN", counts, "nothing_ran"
    if counts["FAIL"] or counts["MISMATCH"]:
        return "FAIL", counts, "unexpected_results"
    passed = class_counts(results)
    missing = [c for c in REQUIRED_CLASSES if not passed[c]]
    if missing:
        return "FAIL", counts, "class_not_passed:" + "+".join(missing)
    return "PASS", counts, "ok"


def proof_line(device, ngpus, results):
    """The one line the GPU-use judge reads (gpu_use.proof_nvbandwidth), from what this run measured."""
    passed = class_counts(results)
    figures = [v for r in results if r["result"] == "PASS" and r["class"] != "latency" for v in r.get("figures", [])]
    return "NVBANDWIDTH_PROOF device=%s gpus=%d passed=%d ce=%d sm=%d latency=%d local=%d min_gbps=%s" % (
        (device or "none").replace(" ", "_"), ngpus, sum(passed.values()), passed["ce"], passed["sm"], passed["latency"], passed["local"],
        ("%g" % min(figures)) if figures else "0")


# ---------------------------------------------------------------------------------------------------------------- the runner
def run_cmd(argv, timeout):
    """-> (rc, stdout, seconds, timed_out). Own process group, killed whole on timeout; never a shell."""
    t0 = time.time()
    try:
        p = subprocess.Popen(argv, stdout=subprocess.PIPE, stderr=subprocess.STDOUT, text=True, errors="replace", start_new_session=True, stdin=subprocess.DEVNULL)
    except OSError as e:
        return None, "cannot execute %s: %s" % (argv[0], e), 0.0, False
    try:
        out, _ = p.communicate(timeout=timeout)
    except subprocess.TimeoutExpired:
        os.killpg(p.pid, signal.SIGKILL)
        out, _ = p.communicate()
        return 124, out or "", time.time() - t0, True
    return p.returncode, out or "", time.time() - t0, False


def case_argv(binary, case_id, buffer_mib, samples):
    return [binary, "-j", "-t", case_id, "-b", str(int(buffer_mib)), "-i", str(int(samples))]


def find_binary(explicit, bundle):
    if explicit:
        return explicit if os.access(explicit, os.X_OK) else None
    for c in (Path(bundle) / "bin" / "nvbandwidth" if bundle else None, HERE / "nvbandwidth"):
        if c and os.access(c, os.X_OK):
            return str(c)
    return shutil.which("nvbandwidth")


def cmd_run(a):
    out = Path(a.out)
    (out / "logs").mkdir(parents=True, exist_ok=True)
    table = load_cases(CASES.read_text())
    binary = find_binary(a.bin, a.bundle)
    stream = census.SmiStream().start()
    t_start = time.time()
    listed, ngpus, device, results, header_note = [], 0, "", [], ""
    if binary:
        rc, text, _, _ = run_cmd([binary, "-l"], 60)
        listed = parse_listing(text) if rc == 0 else []
        (out / "logs" / "list.log").write_text(census.scrub(text[-4000:]))
    else:
        header_note = "binary_missing"
    untabled = [n for n in listed if n not in {c["id"] for c in table}]
    cases = table + [{"id": n, "class": "sm" if n.endswith("_sm") else "ce", "min_gpus": 2 if "device_to_device" in n else 1} for n in untabled]
    for case in cases:
        if a.only and not any(fnmatch.fnmatchcase(case["id"], g) for g in a.only.split(",")):
            continue
        if not binary or (listed and case["id"] not in listed):
            res, why, status, vals, secs = "NOT_RUN", "binary_missing" if not binary else "not_in_binary", "-", [], 0.0
        else:
            rc, text, secs, timed_out = run_cmd(case_argv(binary, case["id"], a.buffer_mib, a.samples), CASE_TIMEOUT_S)
            (out / "logs" / (case["id"] + ".log")).write_text(census.scrub(text[-6000:]))
            body = parse_json(text)
            if body is not None and not device:
                names = device_names(body)
                ngpus, device = len(names), (names[0] if names else "")
            elif body is not None and not ngpus:
                ngpus = len(device_names(body))
            res, why, status, vals = judge_case(case, ngpus, rc, body, timed_out)
        results.append({"id": case["id"], "class": case["class"], "result": res, "reason": why, "status": status, "figures": vals, "secs": secs})
        print("CASE %s class=%s result=%s reason=%s status=%s best=%s secs=%.1f" % (case["id"], case["class"], res, why, status,
                                                                                 ("%g" % max(vals)) if vals else "-", secs), flush=True)
    samples = stream.stop()
    t_end = time.time()
    status, counts, reason = summarize(results)
    use, use_why = (census.gpu_use_verdict(census.peaks(samples), "mem") if counts["PASS"] else ("NA", "no case passed, so there was no work to prove"))
    tsv = "id\tclass\tresult\treason\tstatus\tbest\tsecs\n" + "".join(
        "%s\t%s\t%s\t%s\t%s\t%s\t%.1f\n" % (r["id"], r["class"], r["result"], r["reason"], r["status"], ("%g" % max(r["figures"])) if r["figures"] else "-", r["secs"])
        for r in results)
    proof = proof_line(device, ngpus, results)
    summary = ("NVBANDWIDTH_SUMMARY status=%s reason=%s pass=%d waived=%d fail=%d mismatch=%d not_run=%d cases=%d gpus=%d gpu_use=%s device=%s "
               "buffer_mib=%d secs=%.0f%s" % (status, reason, counts["PASS"], counts["WAIVED"], counts["FAIL"], counts["MISMATCH"], counts["NOT_RUN"],
                                              len(results), ngpus, use, (device or "none").replace(" ", "_"), a.buffer_mib, t_end - t_start,
                                              (" note=" + header_note) if header_note else ""))
    (out / "results.tsv").write_text(tsv)
    (out / "hostsmi.csv").write_text(census.samples_to_csv(samples))
    (out / "summary.txt").write_text(census.scrub("# gpu_use_why=%s\n%s\n%s\n" % (use_why, proof, summary)))
    if a.emit:
        for name in ("results.tsv", "summary.txt"):
            sys.stdout.write(rowio.emit(name, (out / name).read_text()))
    print(proof)
    print(summary)
    if status == "NOT_RUN":                   # run_apps.sh turns this line (exit 0) into a NOTRUN row: our own configuration, never a result about the GPU
        print("APPNOTRUN nvbandwidth:%s" % (header_note or reason))
        return 0
    return 0 if status == "PASS" else 1


# ---------------------------------------------------------------------------------------------------------------- host against guest
def parse_results(text):
    rows = {}
    for line in (text or "").splitlines()[1:]:
        c = line.split("\t")
        if len(c) == 7:
            rows[c[0]] = {"class": c[1], "result": c[2], "reason": c[3], "status": c[4], "best": None if c[5] == "-" else float(c[5])}
    return rows


def compare_rows(host, guest):
    """-> list of (id, host result, guest result, agreement, ratio). A case whose result differs, or that one side lacks, is a DISAGREE; the ratio is
    guest/host of the best figure (informational: no threshold decides anything until a guest run has calibrated one)."""
    out = []
    for cid in sorted(set(host) | set(guest)):
        h, g = host.get(cid), guest.get(cid)
        if h is None or g is None:
            out.append((cid, h and h["result"], g and g["result"], "DISAGREE", None))
            continue
        ratio = (g["best"] / h["best"]) if h["best"] and g["best"] else None
        out.append((cid, h["result"], g["result"], "AGREE" if h["result"] == g["result"] else "DISAGREE", ratio))
    return out


def cmd_compare(a):
    hr, gr = rowio.load(a.host, "results.tsv"), rowio.load(a.guest, "results.tsv")
    if hr is None or gr is None:
        print("NVBANDWIDTH_COMPARE error=missing_results_tsv host=%s guest=%s" % (hr is not None, gr is not None))
        return 2
    rows = compare_rows(parse_results(hr), parse_results(gr))
    for cid, h, g, agree, ratio in rows:
        print("CMP %s host=%s guest=%s %s ratio=%s" % (cid, h, g, agree, "-" if ratio is None else "%.2f" % ratio))
    bad = [r for r in rows if r[3] != "AGREE"]
    print("NVBANDWIDTH_COMPARE cases=%d agree=%d disagree=%d" % (len(rows), len(rows) - len(bad), len(bad)))
    return 1 if bad else 0


def main(argv):
    ap = argparse.ArgumentParser(description=__doc__.split("\n\n")[0])
    sub = ap.add_subparsers(dest="cmd", required=True)
    r = sub.add_parser("run")
    r.add_argument("--out", required=True)
    r.add_argument("--bin")
    r.add_argument("--bundle")
    r.add_argument("--only")
    r.add_argument("--buffer-mib", type=int, default=DEFAULT_BUFFER_MIB)
    r.add_argument("--samples", type=int, default=DEFAULT_SAMPLES)
    r.add_argument("--emit", action="store_true")
    c = sub.add_parser("compare")
    c.add_argument("host")
    c.add_argument("guest")
    sub.add_parser("list")
    a = ap.parse_args(argv)
    if a.cmd == "list":
        for case in load_cases(CASES.read_text()):
            print(case["id"])
        return 0
    return cmd_compare(a) if a.cmd == "compare" else cmd_run(a)


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
