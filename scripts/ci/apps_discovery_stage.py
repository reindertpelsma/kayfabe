#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later
"""The `apps_discovery` payload stage: the open Linux rows that report and never gate, run in the kayfabe guest on the box (stdlib only).

STATUS: LIVE, 2026-11-17 (design: docs/design/V3_STANDALONE_FULL_RUN.md section 4). Code and GPU-free tests; it has not run on a box.

Rows: nvcodec_matrix and nvml_dcgm (scripts/apps/open_rows), vkcts (scripts/apps/vkcts), vllm, tensorrt and jax (scripts/apps/mlrows). They are in no gate list
and `apps_matrix.sh ... all` leaves out the first four of them; this stage names them. LANE: discovery. A row that RAN and failed, crashed, hung or timed out is a
known breakage: it is reported, never a failure of the stage (that is what a discovery row is for). A row that did NOT RUN (no result line, NOTRUN, a precondition
the harness could not meet, a GPU-use proof that could not be judged) fails the stage BY NAME (`discovery_not_run:<row>`) when the stage is strict, which `--full`
always is: "report, never gate" is about the verdict of a row, not about whether it ran.

Steps (each bounded, each recorded): (1) `vkcts.py build` once on the host, (2) `mlrows.py setup` for the three ML rows on the host (network, hashed locks),
(3) when the host side of the matrix is installed (/opt/apps/bundle), `apps_matrix.sh host RUN <rows>` as the control (it runs FIRST: the comparison the matrix prints at the
end of a guest run reads the host log); its absence is recorded (`host_control=absent`), never a failure, (4) `apps_matrix.sh guest RUN <rows>` (the guest is the subject).
The comparison is a finding, not a verdict.

  apps_discovery_stage.py plan
  apps_discovery_stage.py run --sha SHA --checkout DIR --out FILE --run-id ID [--rows a,b] [--deadline-epoch N] [--lenient]
Exit codes: 0 PASS (every row ran), 1 FAIL (named), 3 RETRY (a harness problem: no bundle, no time left).
One `KF_APPS_DISCOVERY_RESULT {json}` line, also written to --out.
"""

import argparse
import json
import os
from pathlib import Path
import re
import subprocess
import sys
import time

ROWS = ("nvcodec_matrix", "nvml_dcgm", "vkcts", "vllm", "tensorrt", "jax")
# the row's own timeout in scripts/apps/run_apps.sh, in seconds (a stage budget is the sum of them, per side)
ROW_TIMEOUT_S = {"nvcodec_matrix": 3000, "nvml_dcgm": 3300, "vkcts": 2700, "vllm": 1800, "tensorrt": 1500, "jax": 1200}
ML_ROWS = ("vllm", "tensorrt", "jax")
APPS_ROOT = "/workspace/apps"
HOST_BUNDLE = "/opt/apps/bundle"
STEP_TIMEOUT_S = {"build_vkcts": 3600, "setup_mlrows": 3600, "host": 4 * 3600, "guest": 6 * 3600}
RAN = {"PASS", "FAIL", "TIMEOUT", "HANG", "GUEST_DEAD", "BOOT_FAIL"}
NOT_RAN = {"NOTRUN"}
APPRES = re.compile(r"^APPRES side=(host|guest) app=([A-Za-z0-9_]{1,40}) verdict=([A-Z_]{1,16}) .*?note=(.*)$")
DCGM_STATE = re.compile(r"\bdcgm=([A-Za-z_]{1,16})\b")
SAFE = re.compile(r"[A-Za-z0-9_.:=-]{1,80}")


def read_res(*paths):
    """{app: (verdict, note)} from APPRES lines, the later file and the later line winning (an isolated re-run replaces the batched row)."""
    out = {}
    for path in paths:
        try:
            text = Path(path).read_text(errors="replace")
        except OSError:
            continue
        for raw in text.splitlines():
            m = APPRES.match(raw.strip())
            if m:
                out[m.group(2)] = (m.group(3), m.group(4).strip()[:120])
    return out


def reason_of(note):
    token = re.sub(r"[^A-Za-z0-9_.:=-]+", "_", note.replace("not-run:", "")).strip("_")[:80]
    return token or "unspecified"


def grade(rows, guest, host=None, dcgm_text=""):
    """The stage record. `guest` and `host` are read_res() dicts. Pure."""
    entries, not_run, breakages = [], [], []
    for name in rows:
        verdict, note = guest.get(name, (None, ""))
        host_verdict = (host or {}).get(name, (None, ""))[0]
        if verdict is None:
            status, reason = "NOT_RUN", "no_guest_row"
        elif verdict in NOT_RAN:
            status, reason = "NOT_RUN", reason_of(note)
        elif verdict == "PASS":
            status, reason = "PASS", ""
        elif verdict in RAN:
            status, reason = "KNOWN_BREAKAGE", verdict
        else:
            status, reason = "NOT_RUN", "unknown_verdict_" + reason_of(verdict)
        if status == "NOT_RUN":
            not_run.append(name)
        if status == "KNOWN_BREAKAGE":
            breakages.append(name)
        entry = {"name": name, "status": status, "guest": verdict or "-", "host": host_verdict or "-", "reason": reason}
        if name == "nvml_dcgm" and status == "PASS":
            state = DCGM_STATE.search(dcgm_text)
            if state:
                entry["dcgm"] = state.group(1)
        entries.append(entry)
    return {"rows": entries, "not_run": not_run, "breakages": breakages, "host_control": "present" if host else "absent"}


def verdict_of(record, strict=True):
    """PASS when every row ran. NOT_RUN rows: a failure by name when strict, otherwise the record says PARTIAL and the stage passes."""
    if record["not_run"]:
        return "FAIL" if strict else "PARTIAL"
    return "PASS"


def run_step(runner, name, argv, env, log_dir, timeout):
    """One bounded step. Returns {step, rc, s}. The runner is injectable so the tests start nothing."""
    t0 = time.monotonic()
    rc = runner(argv, env, Path(log_dir) / (name + ".log"), timeout)
    return {"step": name, "rc": rc, "s": int(time.monotonic() - t0)}


def real_runner(argv, env, log_path, timeout):
    with open(log_path, "wb") as log:
        try:
            return subprocess.run(argv, env=env, stdout=log, stderr=subprocess.STDOUT, timeout=timeout, check=False).returncode
        except subprocess.TimeoutExpired:
            return 124
        except OSError:
            return 127


def plan(rows=ROWS):
    steps = []
    if "vkcts" in rows:
        steps.append("build_vkcts")
    if any(r in rows for r in ML_ROWS):
        steps.append("setup_mlrows")
    steps += ["host", "guest"]          # the host control only when the host side of the matrix is installed
    return {"rows": list(rows), "steps": steps, "budget_s_per_side": sum(ROW_TIMEOUT_S[r] for r in rows)}


def known_rows(checkout, runner=None):
    """The names run_apps.sh knows (its `list`); every row of this stage must be one of them."""
    scratch = Path(os.environ.get("TMPDIR", "/tmp")) / ("kf-discovery-list-%d" % os.getpid())
    scratch.mkdir(parents=True, exist_ok=True)
    env = dict(os.environ, APPS_ROOT=str(scratch))
    out = subprocess.run(["bash", str(Path(checkout) / "scripts/apps/run_apps.sh"), "x", "list"], env=env, capture_output=True, text=True, timeout=30, check=False).stdout
    return set(out.split())


def run(args, runner=real_runner, known=None, now=time.time, host_bundle=HOST_BUNDLE):
    rows = tuple(args.rows.split(",")) if args.rows else ROWS
    sha = args.sha
    checkout = Path(args.checkout)
    steps = []
    record = {"schema": 1, "sha": sha, "run_id": args.run_id, "rows": [], "steps": steps}
    apps = checkout / "scripts/apps"
    result_dir = Path(args.apps_root) / "results" / args.run_id
    result_dir.parent.mkdir(parents=True, exist_ok=True)

    def finish(verdict, reason="", extra=None):
        record.update(verdict=verdict)
        if reason:
            record["reason"] = reason
        if extra:
            record.update(extra)
        text = json.dumps(record, sort_keys=True, separators=(",", ":"))
        Path(args.out).write_text(text + "\n")
        print("KF_APPS_DISCOVERY_RESULT " + text, flush=True)
        return {"PASS": 0, "PARTIAL": 0, "FAIL": 1}.get(verdict, 3)

    names = known if known is not None else known_rows(checkout)
    unknown = [r for r in rows if r not in names or r not in ROW_TIMEOUT_S]
    if unknown:
        return finish("FAIL", "unknown_row_" + unknown[0])
    if not Path(args.apps_root, "bundle.tgz").is_file():
        return finish("RETRY", "no_bundle")           # the `apps` stage makes it; --only apps_discovery on a fresh box is a retry, never a pass
    budget = plan(rows)["budget_s_per_side"]
    deadline = args.deadline_epoch
    if deadline and now() + budget > deadline:
        return finish("RETRY", "no_time_left")
    env = dict(os.environ, KF_APPS_ROOT=args.apps_root, APPS_APP_TMO=os.environ.get("APPS_APP_TMO", "3600"), APPS_PER_BOOT="1")
    log_dir = result_dir.parent
    log_dir.mkdir(parents=True, exist_ok=True)

    def step(name, argv, key):
        s = run_step(runner, name, argv, env, log_dir, STEP_TIMEOUT_S[key])
        steps.append(s)
        return s["rc"]

    if "vkcts" in rows and step("build_vkcts", ["python3", str(apps / "vkcts/vkcts.py"), "build", "--root", args.apps_root + "/vkcts"], "build_vkcts"):
        return finish("RETRY", "vkcts_build_failed")
    ml = [r for r in rows if r in ML_ROWS]
    if ml and step("setup_mlrows", ["python3", "-B", "-I", str(apps / "mlrows/mlrows.py"), "setup", "--root", args.apps_root + "/mlrows", "--rows", ",".join(ml)], "setup_mlrows"):
        # a setup that did not finish is a named NOT_RUN of those rows (they say why), not a harness retry: the matrix below still runs the rest
        pass
    host_ok = Path(host_bundle).is_dir()
    if host_ok:
        step("host", ["bash", str(apps / "apps_matrix.sh"), "host", args.run_id, *rows], "host")
    guest_rc = step("guest", ["bash", str(apps / "apps_matrix.sh"), "guest", args.run_id, *rows], "guest")
    guest = read_res(result_dir / "guest_isolated.res", result_dir / "guest.res")
    host = read_res(result_dir / "host.res") if host_ok else None
    dcgm_text = ""
    try:
        dcgm_text = (result_dir / "nvml_dcgm.guest.log").read_text(errors="replace")[-20000:]
    except OSError:
        pass
    graded = grade(rows, guest, host, dcgm_text)
    record.update(graded)
    if not guest:
        return finish("RETRY", "no_guest_results", {"guest_rc": guest_rc})
    verdict = verdict_of(graded, strict=not args.lenient)
    return finish(verdict, "discovery_not_run:" + ",".join(graded["not_run"]) if graded["not_run"] else "")


def main(argv=None):
    p = argparse.ArgumentParser(prog="apps_discovery_stage.py")
    sub = p.add_subparsers(dest="cmd", required=True)
    sub.add_parser("plan")
    r = sub.add_parser("run")
    r.add_argument("--sha", required=True)
    r.add_argument("--checkout", required=True)
    r.add_argument("--out", required=True)
    r.add_argument("--run-id", required=True)
    r.add_argument("--rows", default="")
    r.add_argument("--apps-root", default=APPS_ROOT)
    r.add_argument("--deadline-epoch", type=int, default=0)
    r.add_argument("--lenient", action="store_true", help="a row that did not run makes the record PARTIAL instead of failing the stage (never used by --full)")
    args = p.parse_args(argv)
    if args.cmd == "plan":
        print(json.dumps(plan(), sort_keys=True))
        return 0
    if not re.fullmatch(r"[0-9a-f]{40}", args.sha) or not SAFE.fullmatch(args.run_id):
        return 2
    return run(args)


if __name__ == "__main__":
    sys.exit(main())
