#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later
"""The `kfml` payload stage (2026-11-16): ML research workloads (scripts/kfml) as an OPTIONAL stage of vast_payload.py.

    kfml_stage.py run --sha SHA --checkout DIR --prefix DIR --out FILE [--tier mid|large] [--reference DIR] [--record-reference DIR]
    kfml_stage.py run ... --where guest [--guest-ssh CMD]   (inside the offline guest: kfml_guest.py)
    kfml_stage.py plan

It runs on the box's own GPU, whatever single NVIDIA die and driver the box has: nothing here names a die or a driver. The wheel flavour is
derived by `kfml setup --flavor auto` from the driver's CUDA version and the GPU's compute capability. Steps, each a child process:

  1. plan       python3 -m kfml setup --plan                                   rc 3 = no pinned flavour fits: every row NOT_RUN, nothing downloaded
  2. setup      python3 -m kfml setup --venv PREFIX/venv --nvcc-if-needed      the wheels (about 4 GB), AT THE SAME TIME as
     fetch-data python3 -m kfml fetch-data (KF_ML_HOME=PREFIX/venv)            the sha256-pinned datasets (CIFAR-10 is slow: parallel ranges)
  3. run        PREFIX/venv/bin/python -m kfml run --tier T --json PREFIX/rows.json

and prints one line `KF_KFML_RESULT {...}` (the file given by --out has the same text) that vast_payload.py grades: verdict PASS (no row FAIL, at
least one PASS), NOT_RUN (every row NOT_RUN, with the named reason), FAIL (a failed step or a FAIL row). Per row: name, status, reason, wall_s
and the die/driver facts; metrics are kept to a bounded set of scalars. No credential is read or written; every download is a public, sha256-pinned
URL (pins.json) or a hashed wheel lock.
"""
import argparse
import json
import os
import re
import shlex
import subprocess
import sys
import threading
import time
from pathlib import Path

SCHEMA = 1
TIERS = ("mid", "large")
STEP_BUDGET_S = {"plan": 120, "setup": 3600, "fetch-data": 5400, "run": 7200}
STEP_NAMES = ("plan", "probe", "wheelhouse", "copy", "verify", "setup", "fetch-data", "run")  # the guest flow adds the first four
TRANSFER_KEYS = ("wheelhouse_bytes", "wheelhouse_build_s", "src_bytes", "copy_bytes", "copy_s", "install_s")
MAX_ROWS = 8
FACT_KEYS = ("gpu_name", "compute_cap", "vram_gb", "driver", "driver_cuda", "cuda", "torch", "flavor")
ROW_KEYS = {"name", "status", "reason", "wall_s", "metrics"}
METRIC_LIMIT = 12


class StageError(Exception):
    def __init__(self, reason):
        super().__init__(reason)
        self.reason = reason


def kfml_dir(checkout):
    d = Path(checkout) / "scripts" / "kfml"
    if not (d / "kfml" / "cli.py").is_file():
        raise StageError("no_kfml_in_checkout")
    return d


def scrub(text, limit=200):
    """Printable ASCII, no UUIDs, bounded."""
    t = re.sub(r"[^\x20-\x7e]", "?", str(text))
    t = re.sub(r"(?i)\b(?:GPU|MIG)-[0-9a-f]{8}(?:-[0-9a-f]{4}){3}-[0-9a-f]{12}\b", "GPU-<uuid>", t)
    t = re.sub(r"(?i)\b[0-9a-f]{8}(?:-[0-9a-f]{4}){3}-[0-9a-f]{12}\b", "<uuid>", t)
    return t[:limit]


def scalar_metrics(metrics):
    out = {}
    for k in sorted(metrics or {}):
        v = metrics[k]
        if type(v) in (int, float) and v == v and abs(v) < 1e15:
            out[scrub(k, 48)] = round(v, 6) if type(v) is float else v
        elif type(v) is bool:
            out[scrub(k, 48)] = v
        if len(out) >= METRIC_LIMIT:
            break
    return out


def summarize(rows, sha, tier, steps, run_id=None):
    """The bounded result document from `kfml run --json` rows."""
    verdict = "FAIL"
    statuses = [r.get("status") for r in rows]
    if rows and all(s == "NOT_RUN" for s in statuses):
        verdict = "NOT_RUN"
    elif rows and "FAIL" not in statuses and "PASS" in statuses and all(s in ("PASS", "NOT_RUN") for s in statuses):
        verdict = "PASS"
    facts = {}
    for k in FACT_KEYS:
        for r in rows:
            if r.get(k) is not None:
                facts[k] = scrub(r[k], 64) if type(r[k]) is str else r[k]
                break
    return {
        "schema": SCHEMA, "verdict": verdict, "sha": sha, "tier": tier, "run_id": scrub(run_id or "", 48), "facts": facts, "steps": steps,
        "rows": [{"name": scrub(r.get("name"), 48), "status": r.get("status"), "reason": scrub(r.get("reason", "")),
                  "wall_s": round(float(r.get("wall_s") or 0), 3), "metrics": scalar_metrics(r.get("metrics"))} for r in rows[:MAX_ROWS]],
    }


def failure(sha, tier, steps, reason, run_id=None):
    return {"schema": SCHEMA, "verdict": "FAIL", "sha": sha, "tier": tier, "run_id": scrub(run_id or "", 48), "facts": {}, "steps": steps,
            "rows": [], "reason": scrub(reason)}


def not_run(sha, tier, steps, reason, names, run_id=None):
    return {"schema": SCHEMA, "verdict": "NOT_RUN", "sha": sha, "tier": tier, "run_id": scrub(run_id or "", 48), "facts": {}, "steps": steps,
            "rows": [{"name": n, "status": "NOT_RUN", "reason": scrub(reason), "wall_s": 0.0, "metrics": {}} for n in names[:MAX_ROWS]],
            "reason": scrub(reason)}


def run_step(name, cmd, cwd, log_path, env, runner=subprocess.run):
    t0 = time.time()
    with open(log_path, "wb") as log:
        try:
            p = runner(cmd, cwd=str(cwd), env=env, stdout=log, stderr=subprocess.STDOUT, timeout=STEP_BUDGET_S[name])
            rc = p.returncode
        except subprocess.TimeoutExpired:
            rc = 124
    return {"step": name, "rc": rc, "s": round(time.time() - t0, 1)}


def test_names(kdir, tier, runner=subprocess.run):
    p = runner([sys.executable, "-m", "kfml", "list", "--tier", tier, "--json"] if tier == "mid"
               else [sys.executable, "-m", "kfml", "list", "--json"], cwd=str(kdir), capture_output=True, text=True)
    try:
        return [t["name"] for t in json.loads(p.stdout)["tests"]]
    except (ValueError, KeyError):
        return ["kfml"]


def run(args, runner=subprocess.run):
    kdir = kfml_dir(args.checkout)
    prefix = Path(args.prefix)
    prefix.mkdir(parents=True, exist_ok=True)
    logs = prefix / "logs"
    logs.mkdir(exist_ok=True)
    venv = prefix / "venv"
    env = dict(os.environ)
    for k in list(env):  # nothing secret rides along into the workload processes
        if re.search(r"TOKEN|SECRET|PASSWORD|API_KEY|AWS_", k):
            del env[k]
    env["PYTHONPATH"] = str(kdir)
    env["KF_ML_RUN_ID"] = args.run_id or ""
    steps = []
    py = sys.executable
    # 1. the flavour decision alone: a box that no flavour fits is NOT_RUN before anything is created or downloaded
    s = run_step("plan", [py, "-m", "kfml", "setup", "--venv", str(venv), "--plan"], kdir, logs / "plan.log", env, runner)
    steps.append(s)
    if s["rc"] == 3:
        reason, facts = "no pinned flavour can run on this box", {}
        try:
            plan = json.loads((logs / "plan.log").read_text().splitlines()[-1])
            reason, facts = plan["reason"], plan.get("facts") or {}
        except (OSError, ValueError, KeyError, IndexError, AttributeError):
            pass
        doc = not_run(args.sha, args.tier, steps, reason, test_names(kdir, args.tier, runner), args.run_id)
        doc["facts"] = {k: (scrub(v, 64) if type(v) is str else v) for k, v in facts.items() if k in FACT_KEYS and type(v) in (str, int, float)}
        return doc
    if s["rc"] != 0:
        return failure(args.sha, args.tier, steps, "flavour plan failed rc=%d" % s["rc"], args.run_id)
    # 2. the venv shell exists first so the datasets (stdlib only, the slow server) download WHILE the wheels install
    if not (venv / "pyvenv.cfg").exists():
        v = runner([py, "-m", "venv", str(venv)], cwd=str(kdir), env=env, capture_output=True, text=True)
        if v.returncode != 0:
            return failure(args.sha, args.tier, steps, "venv creation failed rc=%d" % v.returncode, args.run_id)
    fenv = dict(env, KF_ML_HOME=str(venv))
    done = {}

    def go(name, cmd, e):
        done[name] = run_step(name, cmd, kdir, logs / (name + ".log"), e, runner)

    threads = [threading.Thread(target=go, args=("setup", [py, "-m", "kfml", "setup", "--venv", str(venv), "--nvcc-if-needed"], env)),
               threading.Thread(target=go, args=("fetch-data", [py, "-m", "kfml", "fetch-data"], fenv))]
    for t in threads:
        t.start()
    for t in threads:
        t.join()
    steps += [done["setup"], done["fetch-data"]]
    if done["setup"]["rc"] != 0:
        return failure(args.sha, args.tier, steps, "setup failed rc=%d" % done["setup"]["rc"], args.run_id)
    if done["fetch-data"]["rc"] != 0:
        return failure(args.sha, args.tier, steps, "fetch-data failed rc=%d (pin mismatch or unreachable)" % done["fetch-data"]["rc"], args.run_id)
    vpy = str(venv / "bin" / "python")
    env["PYTHONPATH"] = str(kdir)
    rows_json = prefix / "rows.json"
    cmd = [vpy, "-m", "kfml", "run", "--tier", args.tier, "--json", str(rows_json), "--log-dir", str(logs)]
    if args.reference:
        cmd += ["--reference", args.reference]
    if args.record_reference:
        cmd += ["--record-reference", args.record_reference]
    s = run_step("run", cmd, kdir, logs / "run.log", env, runner)
    steps.append(s)
    try:
        rows = json.loads(rows_json.read_text())
    except (OSError, ValueError):
        return failure(args.sha, args.tier, steps, "kfml run wrote no rows (rc=%d)" % s["rc"], args.run_id)
    if not isinstance(rows, list) or not rows:
        return failure(args.sha, args.tier, steps, "kfml run wrote an empty row list", args.run_id)
    doc = summarize(rows, args.sha, args.tier, steps, args.run_id)
    if s["rc"] not in (0, 1) and doc["verdict"] == "PASS":
        doc["verdict"] = "FAIL"
        doc["reason"] = "kfml run exited rc=%d" % s["rc"]
    return doc


def parse_args(argv):
    p = argparse.ArgumentParser(prog="kfml_stage.py", description=__doc__.split("\n")[0])
    sub = p.add_subparsers(dest="cmd", required=True)
    sub.add_parser("plan")
    r = sub.add_parser("run")
    r.add_argument("--sha", required=True)
    r.add_argument("--checkout", required=True)
    r.add_argument("--prefix", required=True)
    r.add_argument("--out", required=True)
    r.add_argument("--tier", choices=TIERS, default="mid")
    r.add_argument("--reference")
    r.add_argument("--record-reference")
    r.add_argument("--run-id", default="")
    r.add_argument("--where", choices=("host", "guest"), default="host",
                   help="host (default): on this box's own GPU. guest: inside the offline open Linux guest, over --guest-ssh (kfml_guest.py)")
    r.add_argument("--guest-ssh", metavar="CMD", help="the guest transport, e.g. scripts/bench/gssh_nv (default: $KF_GUEST_SSH, else that script)")
    return p.parse_args(argv)


def main(argv=None):
    args = parse_args(sys.argv[1:] if argv is None else argv)
    if args.cmd == "plan":
        print("kfml_stage: setup (flavour auto) -> fetch-data -> run --tier mid|large; rows = one per test; NOT_RUN when no flavour fits")
        return 0
    if not re.fullmatch("[0-9a-f]{40}", args.sha):
        print("KF_KFML_REFUSED reason=sha", flush=True)
        return 2
    try:
        if args.where == "guest":
            import kfml_guest
            ssh = args.guest_ssh or os.environ.get("KF_GUEST_SSH") or str(Path(args.checkout) / "scripts" / "bench" / "gssh_nv")
            doc = kfml_guest.run_guest(args, kfml_guest.SshTransport(shlex.split(ssh)))
        else:
            doc = run(args)
    except StageError as error:
        print("KF_KFML_REFUSED reason=" + error.reason, flush=True)
        return 2
    text = json.dumps(doc, separators=(",", ":"), sort_keys=True)
    Path(args.out).write_text(text + "\n")
    print("KF_KFML_RESULT " + text, flush=True)
    return 0 if doc["verdict"] in ("PASS", "NOT_RUN") else 1


if __name__ == "__main__":
    raise SystemExit(main())
