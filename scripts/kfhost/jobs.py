"""Run one accepted offer. Thin integration: this is where real stages would be called.

Logs/progress/result go ONLY over the Worker's HTTPS box protocol (POST /report/<run>, Bearer run token)
through scripts/ci/vast_report.py's Reporter; the WebSocket never carries them.
"""

import json
import os
import shutil
import subprocess
import sys
import threading
import time
from pathlib import Path

from .fetch import FetchError, fetch_commit
from .profile import standalone

CI_DIR = Path(__file__).resolve().parents[1] / "ci"


def load_reporter_module():
    """Import scripts/ci/vast_report.py (the box protocol 7 client) without installing anything."""
    import importlib.util

    spec = importlib.util.spec_from_file_location("kfhost_vast_report", CI_DIR / "vast_report.py")
    mod = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(mod)
    return mod


class Reporting:
    """Wraps Reporter with this run's token. Injectable for tests (see tests: FakeReporting)."""

    def __init__(self, cfg, offer, module=None):
        mod = module or load_reporter_module()
        access = {"KF_ACCESS_ID": cfg["access_id"], "KF_ACCESS_SECRET": cfg["access_secret"]}
        self.reporter = mod.Reporter(
            cfg["worker_url"], offer["run_id"], offer["run_token"], 600,
            transport=mod.HttpsTransport(offer["run_token"], access=access),
        )

    def progress(self, text):
        self.reporter.progress(text)

    def feed(self, data):
        self.reporter.feed(data)
        self.reporter.pump()

    def finish(self, result, note):
        return self.reporter.finish(result, note, None)


class JobContext:
    """What a running job may do besides computing: read the offer, register a seed, check cancel."""

    def __init__(self, offer, profile, cfg, seed, cancelled):
        self.offer, self.profile, self.cfg = offer, profile, cfg
        self.seed, self.cancelled = seed, cancelled


class JobRunner:
    """Fetch, verify, run through scripts/kfci, report. Subclass or replace `execute` in tests."""

    def __init__(self, cfg, caps, reporting=Reporting, fetch=fetch_commit):
        self.cfg, self.caps, self.reporting, self.fetch = cfg, caps, reporting, fetch
        self.proc = None

    def prepare(self, offer):
        """Fetch + verify BEFORE the offer is acked. Returns the checkout dir; raises FetchError."""
        a = offer["assignment"]
        return self.fetch(a["repo"], a["commit"], self.cfg["work_dir"])

    def kfci_assignment(self, offer):
        a = offer["assignment"]
        steps = [{"id": s} for s in a.get("stages", [])] or [{"id": "merge"}]
        # INTEGRATION POINT: stage names map to kfci step ids 1:1 here; the real mapping/budget comes
        # from the Worker's planner assignment (scripts/kfci/planner_assignment.py), not yet specified.
        deadline = offer.get("deadline")
        budget = max(60, int(deadline - time.time())) if isinstance(deadline, (int, float)) else 3600
        return {"version": 1, "id": offer["run_id"][:90], "budget_seconds": budget, "steps": steps}

    def child_env(self, ctx, checkout):
        keep = {"PATH": os.environ.get("PATH", "/usr/bin:/bin")}
        if standalone(ctx.profile):
            keep.update(HOME=checkout, KF_STANDALONE="1", KF_NO_SYSTEM_CHANGES="1")
        else:
            keep.update({k: v for k, v in os.environ.items() if k in ("HOME", "LANG")})
            keep.update(ctx.offer["assignment"].get("env", {}))
        return keep

    def cancel(self):
        proc = self.proc
        if proc and proc.poll() is None:
            proc.terminate()

    def execute(self, ctx, checkout, rep):
        """Run scripts/kfci in the checkout; stream its output to the reporter; return rc."""
        plan = Path(checkout) / "kfhost-assignment.json"
        plan.write_text(json.dumps(self.kfci_assignment(ctx.offer)))
        self.proc = subprocess.Popen(
            [sys.executable, "-I", str(Path(checkout) / "scripts/kfci/main.py"), "direct", str(plan)],
            cwd=checkout, env=self.child_env(ctx, checkout), stdout=subprocess.PIPE, stderr=subprocess.STDOUT,
        )
        for chunk in iter(lambda: self.proc.stdout.read1(8192), b""):
            rep.feed(chunk)
        self.proc.stdout.close()
        return self.proc.wait()

    def result(self, ctx, rc):
        a = ctx.offer["assignment"]
        caps = self.caps
        cap = caps.get("compute_cap")
        hardware = None
        if caps.get("gpu_name") and cap and caps.get("driver"):
            hardware = {"gpu_name": caps["gpu_name"], "compute_cap": cap, "driver_version": caps["driver"]}
        # OPEN QUESTION for W1: nonce / runner_nonce binding of a custom-host result is not specified.
        out = {"verdict": "PASS" if rc == 0 else "FAIL", "suites": [{"name": "kfci", "pass": int(rc == 0), "fail": int(rc != 0), "skip": 0}], "sha": a["commit"]}
        if hardware:
            out["hardware"] = hardware
        return out

    def run(self, ctx, checkout):
        rep = self.reporting(self.cfg, ctx.offer)
        rep.progress("Custom host %s accepted the run" % self.cfg["box_id"])
        ctx.log = lambda text: rep.feed((text + "\n").encode())
        try:
            rc = self.execute(ctx, checkout, rep)
        finally:
            shutil.rmtree(checkout, ignore_errors=True)
        if ctx.cancelled():
            rc = 3
        rep.finish(self.result(ctx, rc), "Run finished on a custom host.")
        return rc
