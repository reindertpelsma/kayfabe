"""Run one test in a child process with a hard wall-clock budget; map every exit to a result dict."""
from __future__ import annotations

import json
import os
import signal
import subprocess
import sys
import tempfile
from pathlib import Path

PKG_PARENT = str(Path(__file__).resolve().parent.parent)


def run_child(ctx: dict, budget_s: float, env: dict, log_path: str, python=None, extra_path=()) -> dict:
    python = python or sys.executable
    work = tempfile.mkdtemp(prefix="kfml-run-", dir=env.get("TMPDIR") or None)
    ctx_p, out_p = os.path.join(work, "ctx.json"), os.path.join(work, "out.json")
    with open(ctx_p, "w") as f:
        json.dump(ctx, f)
    cenv = dict(env)
    cenv["PYTHONPATH"] = os.pathsep.join([PKG_PARENT, *extra_path, env.get("PYTHONPATH", "")])
    os.makedirs(os.path.dirname(log_path) or ".", exist_ok=True)
    with open(log_path, "wb") as log:
        proc = subprocess.Popen([python, "-m", "kfml._child", ctx_p, out_p], env=cenv, stdout=log,
                                stderr=subprocess.STDOUT, start_new_session=True)
        try:
            rc = proc.wait(timeout=budget_s)
        except subprocess.TimeoutExpired:
            try:
                os.killpg(proc.pid, signal.SIGKILL)
            except ProcessLookupError:
                pass
            proc.wait()
            return {"status": "FAIL", "reason": "timeout: exceeded wall-clock budget of %.0f s (killed)" % budget_s,
                    "metrics": {}, "log": log_path}
    if os.path.exists(out_p):
        with open(out_p) as f:
            res = json.load(f)
        res["log"] = log_path
        return res
    how = ("killed by signal %d" % -rc) if rc < 0 else ("exit code %d" % rc)
    try:
        tail = Path(log_path).read_bytes()[-600:].decode("utf-8", "replace")
    except Exception:
        tail = ""
    return {"status": "FAIL", "reason": "test process died without a result (%s): %s" % (how, tail.strip()),
            "metrics": {}, "log": log_path}
