"""Run one accepted offer. Thin integration: this is where real stages would be called.

Logs/progress/result go ONLY over the Worker's HTTPS box protocol (POST /report/<run>, Bearer run token)
through scripts/ci/vast_report.py's Reporter; the WebSocket never carries them.
"""

import json
import os
import shutil
import subprocess
import sys
import tempfile
import threading
import time
from pathlib import Path

from .downloads import DownloadPolicy
from .fetch import FetchError, fetch_commit
from .profile import needs_ancestry, standalone

# Stage names (profile.STAGES) -> kfci step ids. INTEGRATION POINT: identity until the Worker's planner
# assignment (scripts/kfci/planner_assignment.py) fixes the real mapping.
STAGE_TO_KFCI = {n: n for n in ("provision", "cuda_host", "merge_bar", "cuda_guest", "apps", "windows_gate",
                                "windows_apps", "specviewperf", "ladder")}
# Best effort: no_new_privs for the whole job tree, then exec the real command.
NNP_WRAPPER = ("import ctypes,os,sys;ctypes.CDLL(None,use_errno=True).prctl(38,1,0,0,0);"
               "os.execv(sys.executable,[sys.executable,'-I']+sys.argv[1:])")

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


def load_payload_module():
    """scripts/ci/vast_payload.py, for its `hardware()` helper (the Vast payload's own nvidia-smi reading)."""
    import importlib.util

    spec = importlib.util.spec_from_file_location("kfhost_vast_payload", CI_DIR / "vast_payload.py")
    mod = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(mod)
    return mod


def load_run_report_module():
    import importlib.util

    spec = importlib.util.spec_from_file_location("kfhost_vast_run_report", CI_DIR / "vast_run_report.py")
    mod = importlib.util.module_from_spec(spec)
    sys.path.insert(0, str(CI_DIR))
    spec.loader.exec_module(mod)
    return mod


def post_run_report(cfg, offer, log_lines, result, tree_key_mismatch, module=None, transport=None):
    """POST /run-report/<run_id> through vast_run_report.py's builder and Transport. tree_key_mismatch travels
    ONLY here (a boolean fact), never in the result. Best effort: returns True when a post was attempted OK."""
    try:
        mod = module or load_run_report_module()
        doc = mod.build_report(log_lines, result)
        if tree_key_mismatch:
            doc["tree_key_mismatch"] = True
        body = json.dumps(doc, separators=(",", ":")).encode()
        access = {"KF_ACCESS_ID": cfg["access_id"], "KF_ACCESS_SECRET": cfg["access_secret"]}
        transport = transport or mod.Transport(offer["run_token"], access=access)
        status, _ = transport.post("%s/run-report/%s" % (cfg["worker_url"], offer["run_id"]), body)
        return status == 200
    except Exception:  # noqa: BLE001  (a report problem must never touch the run)
        return False


class JobContext:
    """What a running job may do besides computing: read the offer, register a seed, check cancel."""

    def __init__(self, offer, profile, cfg, seed, cancelled):
        self.offer, self.profile, self.cfg = offer, profile, cfg
        self.seed, self.cancelled = seed, cancelled
        self.downloads = None  # DownloadPolicy, set by the runner once the checkout exists


class JobRunner:
    """Fetch, verify, run through scripts/kfci, report. Subclass or replace `execute` in tests."""

    def __init__(self, cfg, caps, reporting=Reporting, fetch=fetch_commit):
        self.cfg, self.caps, self.reporting, self.fetch = cfg, caps, reporting, fetch
        self.proc = None
        self.tree_key_mismatch = False
        self.hardware_fn = None  # injectable; default: the Vast payload's own helper
        self.report_fn = post_run_report
        self.log_lines = []

    def prepare(self, offer, profile):
        """Fetch + verify BEFORE running. Returns the checkout dir; raises FetchError."""
        a = offer["assignment"]
        checkout, mismatch = self.fetch(a["repo"], a["commit"], self.cfg["work_dir"], a["tree_key"],
                                        strict=needs_ancestry(profile))
        self.tree_key_mismatch = mismatch  # only ever True here for non-strict profiles (strict refuses)
        return checkout

    def kfci_assignment(self, offer):
        a = offer["assignment"]
        # Exactly the steps the Worker listed, in order; nothing is derived or interpreted here.
        steps = [dict({"id": STAGE_TO_KFCI[st["stage"]]}, **({"parameters": {"apps": list(st["apps"])}} if "apps" in st else {}))
                 for st in a.get("steps", [])] or [{"id": "merge_bar"}]
        budget = max(60, int(offer["deadline_ms"] / 1000 - time.time()))  # deadline is epoch milliseconds
        return {"version": 1, "id": offer["run_id"][:90], "budget_seconds": budget, "steps": steps}

    def child_env(self, ctx, scratch):
        """Scrubbed: no config path, no box/Access token, no run token. Throwaway HOME/XDG/TMPDIR for every
        profile but owned (which keeps its real HOME)."""
        env = {"PATH": os.environ.get("PATH", "/usr/bin:/bin"), "LANG": os.environ.get("LANG", "C.UTF-8")}
        home = Path(scratch) / "home"
        tmp = Path(scratch) / "tmp"
        for d in (home, tmp):
            d.mkdir(parents=True, exist_ok=True)
        env.update(TMPDIR=str(tmp))
        if ctx.profile != "owned":
            env.update(HOME=str(home), XDG_CONFIG_HOME=str(home / ".config"), XDG_CACHE_HOME=str(home / ".cache"),
                       XDG_DATA_HOME=str(home / ".local/share"))
        elif os.environ.get("HOME"):
            env["HOME"] = os.environ["HOME"]
        if standalone(ctx.profile):
            env.update(KF_STANDALONE="1", KF_NO_SYSTEM_CHANGES="1")
        else:
            env.update(ctx.offer["assignment"].get("env", {}))
        if getattr(ctx, "seed_path", None):
            env["KF_SEED_FILE"] = ctx.seed_path
        return env

    def cancel(self):
        proc = self.proc
        if proc and proc.poll() is None:
            proc.terminate()

    def execute(self, ctx, checkout, rep, scratch):
        """Run scripts/kfci in the checkout; stream its output to the reporter; return rc."""
        plan = Path(checkout) / "kfhost-assignment.json"
        plan.write_text(json.dumps(self.kfci_assignment(ctx.offer)))
        main = str(Path(checkout) / "scripts/kfci/main.py")
        argv = [sys.executable, "-I", main, "direct", str(plan)]
        if standalone(ctx.profile):
            argv = [sys.executable, "-I", "-c", NNP_WRAPPER, main, "direct", str(plan)]
        self.proc = subprocess.Popen(argv, cwd=checkout, env=self.child_env(ctx, scratch),
                                     stdout=subprocess.PIPE, stderr=subprocess.STDOUT)
        for chunk in iter(lambda: self.proc.stdout.read1(8192), b""):
            rep.feed(chunk)
            if sum(map(len, self.log_lines)) < 2_000_000:  # the run report reads the same lines
                self.log_lines.extend(chunk.decode("utf-8", "replace").splitlines())
        self.proc.stdout.close()
        return self.proc.wait()

    def hardware(self):
        fn = self.hardware_fn
        if fn is None:
            try:
                fn = load_payload_module().hardware
            except Exception:  # noqa: BLE001
                fn = None
        if fn is None:
            return {"gpu_name": "unknown", "compute_cap": 0, "driver_version": "unknown"}
        return fn()

    def result(self, ctx, rc):
        """EXACTLY the Worker's key set (ci/vast-broker/src/results.js): hardware, nonce, runner_nonce, sha, suites,
        verdict. nonce/runner_nonce/sha come from the offer's run section; without it there is no valid result
        (None). tree_key_mismatch is NOT here: it goes through the run-report route."""
        a = ctx.offer["assignment"]
        run = ctx.offer.get("run")
        if not run or "nonce" not in run or "runner_nonce" not in run:
            return None
        return {
            "hardware": self.hardware(),
            "nonce": run["nonce"],
            "runner_nonce": run["runner_nonce"],
            "sha": run.get("sha", a["commit"]),
            "suites": [{"name": "kfci", "pass": int(rc == 0), "fail": int(rc != 0), "skip": 0}],
            "verdict": "PASS" if rc == 0 else "FAIL",
        }

    def consume_seed(self, ctx, scratch):
        """Fetch the Worker-named seed (kf_fetch, SHA-256 + size pinned, fallback_url tried second). Any failure
        only logs: the job builds locally. borrowed never gets here with a seed (the offer is refused earlier)."""
        seed = ctx.offer["assignment"].get("seed")
        if not seed:
            return
        out = Path(scratch) / "seed"
        out.mkdir(exist_ok=True)
        target = out / seed["name"]
        urls = [seed["url"]] + ([seed["fallback_url"]] if seed.get("fallback_url") else [])
        for url in urls:
            if ctx.downloads.fetch(url, seed["sha256"], seed["size"], target, source="worker") == 0:
                ctx.seed_path = str(target)
                ctx.log("seed fetched from %s; verified sha256 and size" % url.split("/")[2])
                return
        ctx.log("seed unavailable (all URLs failed); building locally")

    def run(self, ctx, checkout):
        rep = self.reporting(self.cfg, ctx.offer)
        rep.progress("Custom host %s accepted the run" % self.cfg["box_id"])
        def log(text):
            self.log_lines.append(text)
            rep.feed((text + "\n").encode())

        ctx.log = log
        if self.tree_key_mismatch:
            ctx.log("WARNING: the tree key recomputed on this host differs from the offer's tree_key "
                    "(tree_key_mismatch=true in the result); running anyway because the Worker is trusted here")
        ctx.downloads = DownloadPolicy(ctx.profile, checkout, log=ctx.log)
        os.makedirs(self.cfg["work_dir"], mode=0o700, exist_ok=True)
        scratch = tempfile.mkdtemp(prefix="scratch-", dir=self.cfg["work_dir"])
        try:
            self.consume_seed(ctx, scratch)
            rc = self.execute(ctx, checkout, rep, scratch)
        finally:
            shutil.rmtree(checkout, ignore_errors=True)
            shutil.rmtree(scratch, ignore_errors=True)
        if ctx.cancelled():
            rc = 3
        result = self.result(ctx, rc)
        if result is None:
            ctx.log("ERROR: the offer carries no run section (nonce, runner_nonce); no valid result can be reported")
        rep.finish(result, "Run finished on a custom host.")
        if result is not None:
            self.report_fn(self.cfg, ctx.offer, self.log_lines, result, self.tree_key_mismatch)
        return rc
