# SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later
"""Chunked, crash-resuming runner of the VK-GL-CTS rows and their verdict (STATUS: DESIGN-ONLY, 2026-11-17; logic GPU-free, never run on a guest).

glcts / deqp-vk run a whole caselist in one process, so one driver crash (the nvoglv64.dll class seen in Resolve) ends the run. The runner
therefore feeds the binary chunks of cases. A chunk whose process died leaves a case that began and never ended: that case is booked as
Crash (or Timeout when the chunk timed out), the rest of the chunk goes back into the queue and a fresh process continues. Every case ends
with exactly one status, so the result list is complete and comparable with the native control.
"""
import os

from kfopen_common import OpenRowError, check_line, say, sha256_file, sha256_text, utc, write_json
from cts_caselist import parse as parse_caselist
from cts_results import (diff_results, format_details, format_diff, format_results, parse_console, parse_qpa, read_results, summarize)

DEFAULT_CHUNK_TIMEOUT_S = 1800


def cts_argv(exe, caselist_file, log_file, args):
    return [exe, "--deqp-caselist-file=" + caselist_file, "--deqp-log-filename=" + log_file] + list(args)


def _read(path):
    try:
        with open(path, encoding="utf-8", errors="replace") as f:
            return f.read()
    except OSError:
        return ""


def run_cases(runner, exe, cwd, args, cases, chunk, workdir, chunk_timeout_s=DEFAULT_CHUNK_TIMEOUT_S, reader=_read, max_attempts=None):
    """Run `cases` in chunks. Returns ({case: (status, detail)}, [attempt records]). Never returns a case without a status."""
    os.makedirs(workdir, exist_ok=True)
    results, log, queue, attempt = {}, [], list(cases), 0
    wanted = set(cases)
    max_attempts = max_attempts or (len(cases) + len(cases) // max(chunk, 1) + 5)
    while queue:
        attempt += 1
        if attempt > max_attempts:
            for c in queue:
                results[c] = ("InternalError", "runner gave up after %d attempts" % max_attempts)
            break
        batch, rest = queue[:chunk], queue[chunk:]
        cl = os.path.join(workdir, "chunk%04d.txt" % attempt)
        qpa = os.path.join(workdir, "chunk%04d.qpa" % attempt)
        with open(cl, "w", encoding="utf-8", newline="\n") as f:
            f.write("# kfopen chunk %d\n" % attempt + "\n".join(batch) + "\n")
        res = runner.run(cts_argv(exe, cl, qpa, args), cwd=cwd, timeout=chunk_timeout_s)
        text = reader(qpa)
        parsed, inflight = parse_qpa(text)
        if not parsed and not inflight and res.out:       # the qpa was lost: fall back on stdout
            parsed = parse_console(res.out)
        got = 0
        for c in parsed:
            if c.name in wanted and c.name in batch and c.name not in results:
                results[c.name] = (c.status, c.detail)
                got += 1
        missing = [c for c in batch if c not in results]
        died = bool(missing)
        if missing:
            culprit = inflight if inflight in missing else missing[0]
            results[culprit] = ("Timeout" if res.timed_out else "Crash",
                                "process rc=%s after %.0f s; case did not finish" % (res.rc, res.secs))
            missing.remove(culprit)
        log.append({"attempt": attempt, "rc": res.rc, "secs": round(res.secs, 1), "timed_out": res.timed_out, "recorded": got, "died": died})
        queue = missing + rest
    return results, log


def rerun_differences(runner, exe, cwd, args, results, control, workdir, chunk_timeout_s=DEFAULT_CHUNK_TIMEOUT_S, reader=_read):
    """One-case-per-process rerun of every case that differs from the control; a rerun that equals the control is a flake, adopted and listed."""
    differing = sorted(c for c in results if c in control and results[c][0] != control[c])
    if not differing:
        return results, []
    again, _ = run_cases(runner, exe, cwd, args, differing, 1, os.path.join(workdir, "rerun"), chunk_timeout_s, reader)
    flaky, out = [], dict(results)
    for c in differing:
        if again[c][0] == control[c]:
            out[c] = again[c]
            flaky.append(c)
    return out, flaky


def probe_gpu(runner):
    """GPU name and driver version for the results header. Never the UUID (nvidia-smi is asked for these two fields only)."""
    r = runner.run(["nvidia-smi", "--query-gpu=name,driver_version", "--format=csv,noheader"], timeout=60)
    if r.rc != 0 or not r.out.strip():
        return "unknown", "unknown"
    name, _, drv = r.out.strip().splitlines()[0].partition(",")
    return name.strip(), drv.strip()


def check_control(header, control_header, gpu_name, allow_different_gpu=False):
    """The control is only a control for the same CTS commit, the same caselist and the same die."""
    for key in ("cts_commit", "caselist_sha256", "binary"):
        if header.get(key) != control_header.get(key):
            raise OpenRowError("CONTROL_MISMATCH %s: run has %s, control has %s" % (key, header.get(key), control_header.get(key)))
    if not allow_different_gpu and control_header.get("gpu_name") not in (gpu_name, None, "unknown") and gpu_name != "unknown":
        raise OpenRowError("CONTROL_MISMATCH gpu_name: run on %s, control from %s (--allow-different-gpu to compare anyway, recorded)"
                           % (gpu_name, control_header.get("gpu_name")))
    return True


def row_header(row_id, role, commit, caselist_sha, binary, exe_sha, gpu, extra=None):
    h = {"row": row_id, "role": role, "cts_commit": commit, "caselist_sha256": caselist_sha, "binary": binary,
         "binary_sha256": exe_sha, "gpu_name": gpu[0].replace(" ", "_"), "driver": gpu[1].replace(" ", "_"), "date": utc()}
    h.update(extra or {})
    return h


def emit_verdict(row_id, results, control, header, out_dir, flaky=()):
    """Write results + diff files, print the KFOPEN lines of the row and return the exit code (0 PASS, 1 FAIL, 3 no verdict)."""
    status_only = {c: v[0] for c, v in results.items()}
    totals, groups = summarize(status_only)
    say("RESULTS", "row=%s cases=%d %s" % (row_id, len(results), " ".join("%s=%d" % kv for kv in sorted(totals.items()))))
    for g, c in groups.items():
        say("GROUP", "%s %s" % (g, " ".join("%s=%d" % kv for kv in sorted(c.items()))))
    if flaky:
        say("FLAKY_RESOLVED", "count=%d first=%s" % (len(flaky), sorted(flaky)[0]))
    if control is None:
        print("KFNOTRUN %s reason=no-native-control (discovery run, %d cases recorded in %s; run the same row on native Windows and pass --control)"
              % (row_id, len(results), out_dir), flush=True)
        return 3
    d = diff_results(control, status_only)
    with open(os.path.join(out_dir, row_id + ".diff.tsv"), "w", encoding="utf-8", newline="\n") as f:
        f.write(format_diff(d))
    say("DIFF", "row=%s verdict=%s %s candidates=%d" % (row_id, d["verdict"], " ".join("%s=%d" % kv for kv in sorted(d["counts"].items())), len(d["candidates"])))
    for r in d["candidates"][:20]:
        say("CANDIDATE", "%s native=%s guest=%s %s" % (r[0], r[1], r[2], r[3]))
    if d["verdict"] == "PASS":
        say("VERDICT", "PASS row=%s cases=%d equal-to-control" % (row_id, d["cases"]))
        return 0
    if d["verdict"] == "INCOMPLETE":
        print("KFNOTRUN %s reason=incomplete-case-sets (missing_in_guest=%d missing_in_native=%d)"
              % (row_id, len(d["missing_in_guest"]), len(d["missing_in_native"])), flush=True)
        return 3
    say("VERDICT", "FAIL row=%s differing=%d candidates=%d" % (row_id, len(d["rows"]), len(d["candidates"])))
    check_line("cts_equals_control", False, "%d of %d cases differ from the native control" % (len(d["rows"]), d["cases"]))
    return 1


def execute_row(row_id, row, commit, caselist_path, exe, cwd, runner, out_dir, role="guest", control_path=None, allow_different_gpu=False,
                rerun=True, reader=_read):
    """The whole row, given an already built binary. Returns the process exit code."""
    text = open(caselist_path, encoding="utf-8", newline="").read()
    cases = parse_caselist(text)
    cl_sha = sha256_text(text)
    gpu = probe_gpu(runner)
    exe_sha = sha256_file(exe) if os.path.exists(exe) else "absent"
    os.makedirs(out_dir, exist_ok=True)
    header = row_header(row_id, role, commit, cl_sha, row["binary"], exe_sha, gpu)
    say("START", "row=%s role=%s cases=%d chunk=%d gpu=%s" % (row_id, role, len(cases), row["chunk"], header["gpu_name"]))
    control = None
    if control_path:
        c_header, control = read_results(open(control_path, encoding="utf-8").read())
        check_control(header, c_header, gpu[0].replace(" ", "_"), allow_different_gpu)
    work = os.path.join(out_dir, "work")
    results, log = run_cases(runner, exe, cwd, row["args"], cases, row["chunk"], work, row.get("chunk_timeout_s", DEFAULT_CHUNK_TIMEOUT_S), reader)
    flaky = []
    if control is not None and rerun:
        results, flaky = rerun_differences(runner, exe, cwd, row["args"], results, control, work, row.get("chunk_timeout_s", DEFAULT_CHUNK_TIMEOUT_S), reader)
    with open(os.path.join(out_dir, row_id + "." + role + ".tsv"), "w", encoding="utf-8", newline="\n") as f:
        f.write(format_results(header, results))
    with open(os.path.join(out_dir, row_id + "." + role + ".details.tsv"), "w", encoding="utf-8", newline="\n") as f:
        f.write(format_details(results))
    write_json(os.path.join(out_dir, row_id + "." + role + ".attempts.json"), {"attempts": log, "flaky_resolved": sorted(flaky)})
    say("ATTEMPTS", "count=%d died=%d" % (len(log), sum(1 for a in log if a["died"])))
    return emit_verdict(row_id, results, control, header, out_dir, flaky)
