#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later
"""The generic propimage stage, planning half: the boots, rows and harness commands of ANY bundle, from its configuration entry (stdlib only).

STATUS: LIVE for `plan`, 2026-11-19 (GPU-free; tests: test_propimage_stage.py; mutants: propimage_stage_mutants.py). DESIGN-ONLY for `run`: the
three per-image stages (`propimage_apps.py run`, `propimage_apps_b_stage.py run`, `matlab_stage.py run`) still do the booting, and nothing in the
payload calls this module yet (docs/design/V3_PROPIMAGE_CONFIG.md, section 4).

    propimage_stage.py plan --bundle ID [--config FILE] [--apps a,b] [--arm kf3|vfio --vfio-gpu 0000:01:00.0] [--isolation row|shared]

What `plan` is for: the plan the hardcoded stages produce today (the rows in order, the boots, the exact `propimage_check.py` argv of every boot) is
a function of the bundle's `run` entry and its rows table, and this module is that function. `test_propimage_stage.py` proves it for the
project's entries: the plan of `imga`, `imgb` and `imgc` is byte-identical to the committed golden (scripts/ci/fixtures/propimage_stage_golden.json,
captured from the hardcoded stages) AND to what the hardcoded stages compute now, so the generic stage cannot drift from them.

The rows table of a Windows bundle is the module named by `run.rows_file`: `CHECKS = {row: {"timeout": seconds, "lane": "gate"|"discovery",
"gpu": (...), "disabled": "reason"}}` (`lane`, `gpu` and `disabled` optional). A Linux bundle's module offers `ROWS` (row -> (script, timeout, ...)),
`ROW_ORDER`, `PREAMBLE` and `plan_boots(rows, isolation)`.

ARM. `arm` is `kf3` (the product, the default) or `vfio` (the real GPU as the reference arm of a differential run). It is a parameter of the
lane, not of the image: the harness already takes `--platform kf3|vfio` and `--vfio-gpu ADDRESS`, and this module adds exactly those words for
`vfio` and nothing for `kf3`, so the default plan is unchanged. Building a plan never touches a GPU or binds anything; a real VFIO bind cycle on
the maintainers' PC is a separate act that needs the owner's explicit go each time (OWNER_RULINGS, 2026-11-19).
"""
import argparse
import importlib.util
import json
import re
import sys
from pathlib import Path

HERE = Path(__file__).resolve().parent
ROOT = HERE.parents[1]
sys.path.insert(0, str(HERE))
import propimage_config as pc  # noqa: E402

ARMS = ("kf3", "vfio")
BOOT_BUDGET_S = 2700                      # propimage_apps.BOOT_BUDGET_S (a test compares them)
PCI_RE = re.compile(r"[0-9a-f]{4}:[0-9a-f]{2}:[0-9a-f]{2}\.0")
PYTHON = "<python>"                       # argv[0] of a planned command: the runner puts its own interpreter there
PATHS = {"dir": "/p/unpacked", "work": "/p/work", "out": "/p/out", "qemu": "/p/qemu", "flags": "/p/flags.env", "broker": "/p/broker",
         "gop": "/p/gop.efi", "data": "/p/data.iso", "check_py": "/p/propimage_check.py"}      # the paths of a plan (a run substitutes its own)


class StageError(ValueError):
    def __init__(self, reason):
        super().__init__(reason)
        self.reason = reason


def refuse(condition, reason):
    if not condition:
        raise StageError(reason)


def load_module(path, name):
    spec = importlib.util.spec_from_file_location(name, path)
    module = importlib.util.module_from_spec(spec)
    sys.path.insert(0, str(Path(path).resolve().parent))
    try:
        spec.loader.exec_module(module)
    finally:
        sys.path.pop(0)
    return module


def bundle_of(config, bundle_id):
    refuse(bundle_id in config["bundles"], "UNKNOWN_BUNDLE:" + str(bundle_id)[:32])
    return config["bundles"][bundle_id]


def check_arm(arm, vfio_gpu):
    refuse(arm in ARMS, "ARM:" + str(arm)[:16])
    refuse(arm == "vfio" or vfio_gpu is None, "VFIO_GPU_WITHOUT_VFIO")
    refuse(arm != "vfio" or (type(vfio_gpu) is str and PCI_RE.fullmatch(vfio_gpu)), "VFIO_GPU")


# ---------------------------------------------------------------- windows
def windows_rows(run, table, only=None):
    """(rows in run order, {row: disabled reason}). Without `only`: the entry's `rows`, else the table's enabled rows in table order. With `only`
    (the stage's `--apps`): exactly those rows, in table order, named rows that the table lacks being refused."""
    checks = table.CHECKS
    if only is not None:
        refuse(only and all(r in checks for r in only), "ROW_UNKNOWN")
        order = tuple(r for r in checks if r in only)
    elif "rows" in run:
        refuse(all(r in checks for r in run["rows"]), "ROW_UNKNOWN")
        order = tuple(run["rows"])
    else:
        order = tuple(r for r in checks if not checks[r].get("disabled"))
    not_run = {r: str(c["disabled"])[:160] for r, c in checks.items() if c.get("disabled") and r not in order}
    refuse(order, "NO_ROWS")
    return order, not_run


def plan_boots(timeouts, order, budget=BOOT_BUDGET_S, lanes=None, solo=()):
    """Greedy groups in row order whose timeouts sum to at most `budget` (the evaluation timer is per boot). A discovery-lane row, and a row named in
    `solo`, boots ALONE in a fresh clone: it may crash or hang the guest, and then only its own row fails."""
    boots, cur, total = [], [], 0
    for row in order:
        t = timeouts[row]
        if (lanes and lanes.get(row) == "discovery") or row in solo:
            if cur:
                boots.append(cur)
                cur, total = [], 0
            boots.append([row])
            continue
        if cur and total + t > budget:
            boots.append(cur)
            cur, total = [], 0
        cur.append(row)
        total += t
    if cur:
        boots.append(cur)
    return boots


def boot_command(run, a, exe=PYTHON, arm="kf3", vfio_gpu=None):
    """The exact `propimage_check.py` argv of one boot. `a` carries paths (`check_py dir work out qemu flags broker gop` and `data` when the entry
    passes a data volume) and the boot's `apps`; nothing here comes from the guest. Order: the fixed words, the entry's before_apps, `--data-iso`,
    `--apps`, after_apps, `--ram-mb`, `--audio`, the per_row words of the rows on this boot (row order), then the arm."""
    check_arm(arm, vfio_gpu)
    args = run.get("harness_args", {})
    cmd = [exe, "-B", "-I", a["check_py"], "--dir", a["dir"], "--work", a["work"], "--out", a["out"], "--qemu", a["qemu"], "--flags", a["flags"],
           "--broker", a["broker"], "--gop-efi", a["gop"], "--display", "broker"]
    cmd += args.get("before_apps", [])
    if run.get("data_iso"):
        cmd += ["--data-iso", a["data"]]
    cmd += ["--apps", ",".join(a["apps"])]
    cmd += args.get("after_apps", [])
    if run.get("guest_ram_mb"):
        cmd += ["--ram-mb", str(run["guest_ram_mb"])]
    if run.get("audio"):
        cmd.append("--audio")
    for row in a["apps"]:
        cmd += args.get("per_row", {}).get(row, [])
    if arm == "vfio":
        cmd += ["--platform", "vfio", "--vfio-gpu", vfio_gpu]
    return cmd


def plan_windows(bundle, root, only=None, arm="kf3", vfio_gpu=None):
    run = bundle["run"]
    table = load_module(Path(root) / run["rows_file"], "kf_stage_rows_" + re.sub(r"\W", "_", run["rows_file"]))
    order, not_run = windows_rows(run, table, only)
    timeouts = {r: table.CHECKS[r]["timeout"] for r in order}
    lanes = {r: table.CHECKS[r].get("lane") or "gate" for r in order}
    boots = plan_boots(timeouts, order, lanes=lanes, solo=run.get("solo_rows", ()))
    commands = []
    for index, rows in enumerate(boots, 1):
        a = dict(PATHS, work="%s/boot%d" % (PATHS["work"], index), out="%s/boot%d" % (PATHS["out"], index), apps=rows)
        commands.append(boot_command(run, a, PYTHON, arm, vfio_gpu))
    return {"rows": list(order), "lanes": lanes, "timeouts": timeouts, "not_run": not_run, "boots": boots, "commands": commands}


# ---------------------------------------------------------------- linux
def plan_linux(bundle, root, only=None, isolation=None):
    run = bundle["run"]
    mod = load_module(Path(root) / run["rows_file"], "kf_stage_rows_" + re.sub(r"\W", "_", run["rows_file"]))
    all_rows = [r for r in mod.ROW_ORDER if r != mod.PREAMBLE]
    if only is not None:
        refuse(only and all(r in mod.ROWS for r in only), "ROW_UNKNOWN")
        rows = [r for r in mod.ROW_ORDER if r in only and r != mod.PREAMBLE]
    else:
        rows = all_rows
    refuse(rows, "NO_ROWS")
    isolation = isolation or run["isolation"]
    refuse(isolation in pc.ISOLATIONS, "ISOLATION")
    return {"rows": rows, "timeouts": {r: mod.ROWS[r][1] for r in rows}, "isolation": isolation, "boots": mod.plan_boots(rows, isolation)}


def plan(config, bundle_id, root=ROOT, only=None, arm="kf3", vfio_gpu=None, isolation=None):
    """The plan of one bundle: {bundle, os, arm, rows, boots, ...}. A bundle without a `run` entry is refused by name (NO_RUN): it is a fetch-only bundle."""
    pc.validate(config)
    bundle = bundle_of(config, bundle_id)
    refuse("run" in bundle, "NO_RUN:" + bundle_id)
    check_arm(arm, vfio_gpu)
    refuse(arm == "kf3" or bundle["os"] == "windows", "ARM_NOT_SUPPORTED:" + bundle["os"])
    refuse(isolation is None or bundle["os"] == "linux", "ISOLATION_ON_WINDOWS")
    head = {"bundle": bundle_id, "os": bundle["os"], "arm": arm}
    if bundle["os"] == "windows":
        return {**head, **plan_windows(bundle, root, only, arm, vfio_gpu)}
    return {**head, **plan_linux(bundle, root, only, isolation)}


def main(argv=None):
    p = argparse.ArgumentParser(prog="propimage_stage.py", description=__doc__.split("\n\n")[0])
    sub = p.add_subparsers(dest="cmd", required=True)
    q = sub.add_parser("plan")
    q.add_argument("--bundle", required=True)
    q.add_argument("--config", default=str(pc.DEFAULT_CONFIG))
    q.add_argument("--apps", help="comma list: only these rows (default: the entry's rows)")
    q.add_argument("--arm", default="kf3", choices=ARMS, help="kf3 (default) or vfio (the real GPU as the reference arm)")
    q.add_argument("--vfio-gpu", help="PCI address of the GPU function 0 (with --arm vfio)")
    q.add_argument("--isolation", choices=pc.ISOLATIONS)
    a = p.parse_args(argv)
    try:
        doc = pc.load(a.config)
        only = [x for x in a.apps.split(",") if x] if a.apps is not None else None
        print(json.dumps(plan(doc, a.bundle, ROOT, only, a.arm, a.vfio_gpu, a.isolation), indent=1))
        return 0
    except (pc.ConfigError, StageError) as exc:
        print("KF_PROPIMAGE_STAGE_REFUSED " + exc.reason, flush=True)
        return 2
    except OSError as exc:
        print("KF_PROPIMAGE_STAGE_UNREADABLE " + type(exc).__name__, flush=True)
        return 2


if __name__ == "__main__":
    sys.exit(main())
