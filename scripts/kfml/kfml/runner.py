"""Orchestration: gate, resolve pins, run each test in a child, build rows. Stdlib only."""
from __future__ import annotations

import os
import time
from pathlib import Path

from . import env as envmod
from . import executor, gating, pins, reference, rows
from .spec import guest_estimate_s


class RunnerError(Exception):
    pass


def select(specs, only=None, run_all=False):
    by = {s.name: s for s in specs}
    if only:
        bad = [n for n in only if n not in by]
        if bad:
            raise RunnerError("unknown test(s): %s (known: %s)" % (", ".join(bad), ", ".join(sorted(by))))
        return [by[n] for n in only]
    return [s for s in specs if s.gate or run_all]


def run_all(specs, env, environ=None, root=None, reference_dir=None, record_dir=None, log_dir=None,
            budget_scale=1.0, timings=None, exec_fn=executor.run_child, resolve_fn=pins.resolve,
            on_row=None, python=None):
    environ = dict(os.environ if environ is None else environ)
    dev_cpu = environ.get("KF_ML_DEV_CPU") == "1"
    root = Path(root) if root else envmod.venv_root(environ)
    if root is None and not dev_cpu:
        raise RunnerError("not inside a venv and KF_ML_HOME is unset: every cache must live under one "
                          "directory (create it with `kfml setup --venv DIR`, or set KF_ML_HOME=DIR)")
    cenv = {k: v for k, v in environ.items()}
    if root is not None:
        cenv.update(envmod.cache_env(root))
        envmod.ensure_dirs(envmod.cache_env(root))
        cuda_home = root / "cuda_home"
        if (cuda_home / "bin" / "nvcc").exists():
            cenv["CUDA_HOME"] = str(cuda_home)
    data_dir = cenv.get("KF_ML_DATA", os.path.join(os.getcwd(), "kfml-data"))
    log_dir = log_dir or os.path.join(cenv.get("TMPDIR", "."), "kfml-logs")
    out = []
    for spec in specs:
        t0 = time.time()
        budget = spec.budget_s * budget_scale
        extra = dict(tier=spec.tier, launch_bound=spec.launch_bound,
                     guest_estimate_s=guest_estimate_s(spec, timings or {}))
        v = gating.Verdict("ok") if dev_cpu else gating.evaluate(spec, env, environ)
        if v.kind != "ok":
            st = "NOT_RUN" if v.kind == "not_run" else "FAIL"
            reason = v.reason if v.kind == "not_run" else "env-missing: " + v.reason
            row = rows.make_row(spec.name, st, reason, time.time() - t0, env, **extra)
        else:
            row = _run_one(spec, env, cenv, data_dir, log_dir, budget, reference_dir, record_dir, exec_fn,
                           resolve_fn, root, t0, extra, dev_cpu, python)
        out.append(row)
        if on_row:
            on_row(row)
    return out


def _run_one(spec, env, cenv, data_dir, log_dir, budget, reference_dir, record_dir, exec_fn, resolve_fn,
             root, t0, extra, dev_cpu, python):
    commits, psha, paths = {}, {}, {}
    for up in spec.upstreams:
        try:
            r = resolve_fn(up, cenv.get("TMPDIR", "."))
        except pins.PinError as e:
            return rows.make_row(spec.name, "FAIL", "pin refused: %s" % e, time.time() - t0, env, **extra)
        commits[up.name] = up.commit
        if r and r.patch_sha256:
            psha[up.name] = r.patch_sha256
        if r and r.path:
            paths[up.name] = r.path
    ctx = {"name": spec.name, "module": spec.module, "cc": list(env.cc) if env.cc else None,
           "vram_total_gb": env.vram_total_gb, "vram_free_gb": env.vram_free_gb, "data_dir": data_dir,
           "reference": reference.load(reference_dir, spec.name), "budget_s": budget,
           "upstream_paths": paths, "dev_cpu": dev_cpu, "guest": cenv.get("KF_ML_GUEST") == "1"}
    res = exec_fn(ctx, budget, cenv, os.path.join(log_dir, spec.name + ".log"), python=python)
    status, reason = res.get("status", "FAIL"), res.get("reason", "")
    metrics = res.get("metrics", {})
    if res.get("log"):
        metrics = dict(metrics, log=res["log"])
    if status == "PASS" and record_dir and res.get("reference") is not None:
        reference.write(record_dir, spec.name, env, res["reference"])
        metrics = dict(metrics, reference_recorded=True)
    return rows.make_row(spec.name, status, reason, time.time() - t0, env, metrics,
                         upstream_commits=commits, patch_sha256=psha,
                         dataset_sha256=res.get("dataset_sha256", {}), **extra)
