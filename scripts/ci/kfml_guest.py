#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later
"""kfml in the OFFLINE open Linux guest (design 2026-11-17; GPU-free tested with a fake transport, NEVER run in a guest yet).

The guest has no internet, so the environment is built on the HOST at run time from the repository's hashed locks and copied in:

  1. probe     the guest's own facts over the transport: nvidia-smi (die, compute capability, driver, CUDA the driver supports), CPython,
               compiler, free disk. The flavour is chosen from THE GUEST's facts (the guest driver is the operator's choice, never pinned).
  2. wheelhouse  host: `python -m kfml wheelhouse --flavor auto --flavor-for <driver CUDA>:<cc> --guest-python <X.Y> --with-data`:
               `pip download --require-hashes --only-binary=:all:` for the guest's interpreter, the sha256-pinned redist nvcc, the pinned
               datasets. No flavour fits: NOT_RUN with the named reason (exit 3), nothing is copied. Resumable and idempotent: it can run
               BEFORE the guest boots (give --guest-python and --assume-facts) so the guest does not idle while wheels download.
  3. copy      `tar | ssh tar -x` (the transport apps_hook.sh already uses): scripts/kfml and the wheelhouse into /opt/kfml. Bytes and seconds
               are recorded.
  4. verify    in the guest: `kfml wheelhouse --verify` re-hashes every copied file against MANIFEST.json (truncation, tampering, extra files).
  5. setup     in the guest: `kfml setup --venv /opt/kfml/venv --wheelhouse /opt/kfml/wheelhouse --nvcc-if-needed`:
               `pip install --no-index --find-links <wheelhouse> --require-hashes -r <hashed lock>`. A guest that derives another flavour than the
               host shipped is a named FAIL (`wheelhouse-flavour-mismatch`), never a silent install of the wrong wheels.
  6. fetch-data  in the guest: `kfml fetch-data --from <wheelhouse>/data` (still checked against pins.json).
  7. run       in the guest: `kfml run --tier T --json` with KF_ML_GUEST=1; the rows JSON is read back over the transport.

The result is the same KF_KFML_RESULT document as the host run, with where=guest and a `transfer` record (bytes and seconds of the wheelhouse
build, the copy and the guest install), so native and guest rows compare directly. Nothing secret enters the guest: no token, no key but
the transport's own throwaway one. NVIDIA binaries are never stored by us; the wheelhouse is a transient directory on the box.
"""
import json
import os
import re
import shlex
import subprocess
import sys
import time
from pathlib import Path

import kfml_stage as ks

GUEST_ROOT = "/opt/kfml"
SMI_QUERY = "nvidia-smi --query-gpu=name,driver_version,memory.total,compute_cap --format=csv,noheader,nounits"
# Free space the guest needs on top of the wheelhouse itself: the venv unpacks the wheels (about 1.3x), plus caches and the datasets.
DISK_SLACK_BYTES = 6 * 2**30
STEP_BUDGET_S = {"probe": 120, "wheelhouse": 3600, "copy": 3600, "verify": 1800, "setup": 3600, "fetch-data": 900, "run": 7200}


class GuestError(Exception):
    def __init__(self, reason):
        super().__init__(reason)
        self.reason = reason


def dir_bytes(path):
    return sum(p.stat().st_size for p in Path(path).rglob("*") if p.is_file())


class SshTransport:
    """The payload's guest transport: an ssh command (scripts/bench/gssh_nv: the tap network, a throwaway key) and tar over its stdin."""

    def __init__(self, ssh_argv, popen=subprocess.Popen, run=subprocess.run):
        self.ssh, self.popen, self.sub = list(ssh_argv), popen, run

    def run(self, cmd, timeout):
        try:
            p = self.sub([*self.ssh, cmd], capture_output=True, text=True, timeout=timeout)
        except subprocess.TimeoutExpired:
            return 124, ""
        return p.returncode, (p.stdout or "") + (p.stderr or "")

    def put_tree(self, local, remote, timeout):
        """tar -C local -cf - . | ssh 'tar -C remote -xf -'. Returns {bytes, s} (payload bytes: the sum of the file sizes)."""
        local = Path(local)
        t0 = time.time()
        tar = self.popen(["tar", "-C", str(local), "-cf", "-", "."], stdout=subprocess.PIPE)
        recv = self.popen([*self.ssh, "sudo install -d -o \"$(id -un)\" %s && tar -C %s -xf -" % (shlex.quote(remote), shlex.quote(remote))],
                          stdin=tar.stdout)
        tar.stdout.close()
        try:
            rc = recv.wait(timeout=timeout)
        except subprocess.TimeoutExpired:
            recv.kill()
            tar.kill()
            raise GuestError("copy timeout")
        if tar.wait() != 0 or rc != 0:
            raise GuestError("copy failed (tar %s, ssh %s)" % (tar.returncode, rc))
        return {"bytes": dir_bytes(local), "s": round(time.time() - t0, 1)}


def parse_facts(smi_csv, smi_header, python_version, have, disk_kb):
    """Guest facts from raw command output. Anything that cannot be read is None (the caller turns that into a named FAIL)."""
    f = {"gpu_name": None, "driver": None, "vram_gb": None, "cc": None, "driver_cuda": None, "python": None, "gcc": bool(have.get("gcc")),
         "python_h": bool(have.get("python_h")), "venv": bool(have.get("venv")), "disk_free_bytes": None}
    line = (smi_csv or "").strip().splitlines()[0] if (smi_csv or "").strip() else ""
    parts = [p.strip() for p in line.split(",")]
    if len(parts) >= 4:
        f["gpu_name"], f["driver"] = parts[0], parts[1]
        try:
            f["vram_gb"] = round(float(parts[2]) / 1024, 2)
        except ValueError:
            pass
        m = re.fullmatch(r"(\d+)\.(\d+)", parts[3])
        if m:
            f["cc"] = (int(m.group(1)), int(m.group(2)))
    m = re.search(r"CUDA Version:\s*(\d+\.\d+)", smi_header or "")
    f["driver_cuda"] = m.group(1) if m else None
    m = re.fullmatch(r"(\d+\.\d+)", (python_version or "").strip())
    f["python"] = m.group(1) if m else None
    try:
        f["disk_free_bytes"] = int(str(disk_kb).strip()) * 1024
    except ValueError:
        pass
    return f


def probe(tr):
    rc1, smi = tr.run(SMI_QUERY, STEP_BUDGET_S["probe"])
    rc2, head = tr.run("nvidia-smi | head -n 4", STEP_BUDGET_S["probe"])
    rc3, py = tr.run("python3 -c 'import sys; print(\"%d.%d\" % sys.version_info[:2])'", STEP_BUDGET_S["probe"])
    rc4, have = tr.run(
        "echo gcc=$(command -v g++ >/dev/null && echo 1); "
        "echo python_h=$(python3 -c 'import os,sysconfig; print(1 if os.path.exists(os.path.join(sysconfig.get_paths()[\"include\"], \"Python.h\")) else \"\")'); "
        "echo venv=$(python3 -c 'import ensurepip, venv; print(1)' 2>/dev/null); "
        "echo disk_kb=$(df -Pk /opt | awk 'NR==2{print $4}')", STEP_BUDGET_S["probe"])
    kv = dict(l.split("=", 1) for l in have.splitlines() if "=" in l)
    if rc1 != 0:
        raise GuestError("guest has no usable nvidia-smi (rc=%d)" % rc1)
    return parse_facts(smi if rc1 == 0 else "", head if rc2 == 0 else "", py if rc3 == 0 else "", kv, kv.get("disk_kb", ""))


def fact_problems(f):
    """Named FAIL reasons (env-missing) for facts the stage cannot work without. NOT hardware limits: those are NOT_RUN from flavor.choose."""
    out = []
    if f["gpu_name"] is None or f["cc"] is None:
        out.append("guest-gpu: nvidia-smi output not understood")
    if f["driver_cuda"] is None:
        out.append("guest-driver-cuda: nvidia-smi printed no 'CUDA Version'")
    if f["python"] is None:
        out.append("guest-python: python3 not usable")
    for key, what in (("gcc", "a C++ compiler"), ("python_h", "Python.h (python3-dev)"), ("venv", "python3-venv")):
        if not f[key]:
            out.append("guest-missing: " + what)
    return out


def host_wheelhouse(host_py, kfml_dir, out, facts, run=subprocess.run, with_data=True):
    """Run the host side of the wheelhouse. Returns (doc|None, not_run_reason|None, rc)."""
    cmd = [host_py, "-m", "kfml", "wheelhouse", "--out", str(out), "--flavor", "auto", "--flavor-for",
           "%s:%d.%d" % (facts["driver_cuda"], facts["cc"][0], facts["cc"][1]), "--guest-python", facts["python"]]
    if with_data:
        cmd.append("--with-data")
    p = run(cmd, cwd=str(kfml_dir), capture_output=True, text=True, timeout=STEP_BUDGET_S["wheelhouse"])
    last = (p.stdout or "").strip().splitlines()[-1:] or [""]
    try:
        doc = json.loads(last[0])
    except ValueError:
        doc = {}
    if p.returncode == 3:
        return None, doc.get("reason") or "no pinned flavour can run in this guest", 3
    return (doc if p.returncode == 0 else None), None, p.returncode


def run_guest(args, tr, host_py=None, run=subprocess.run):
    """The whole flow. Returns a result document (kfml_stage.summarize/failure/not_run shapes) plus where=guest and transfer."""
    kdir = ks.kfml_dir(args.checkout)
    host_py = host_py or sys.executable
    prefix = Path(args.prefix)
    prefix.mkdir(parents=True, exist_ok=True)
    wh_dir = prefix / "wheelhouse"
    steps, transfer = [], {}

    def finish(doc):
        doc["where"] = "guest"
        doc["transfer"] = transfer
        return doc

    def step(name, rc, t0):
        steps.append({"step": name, "rc": rc, "s": round(time.time() - t0, 1)})

    t0 = time.time()
    try:
        facts = probe(tr)
    except GuestError as e:
        step("probe", 1, t0)
        return finish(ks.failure(args.sha, args.tier, steps, e.reason, args.run_id))
    probs = fact_problems(facts)
    step("probe", 1 if probs else 0, t0)
    facts_row = {"gpu_name": facts["gpu_name"], "compute_cap": "%d.%d" % facts["cc"] if facts["cc"] else None, "driver": facts["driver"],
                 "driver_cuda": facts["driver_cuda"], "vram_gb": facts["vram_gb"]}
    if probs:
        return finish(ks.failure(args.sha, args.tier, steps, "; ".join(probs), args.run_id))

    t0 = time.time()
    whdoc, not_run, rc = host_wheelhouse(host_py, kdir, wh_dir, facts, run)
    step("wheelhouse", rc, t0)
    if not_run:
        doc = ks.not_run(args.sha, args.tier, steps, not_run, ks.test_names(kdir, args.tier, run), args.run_id)
        doc["facts"] = {k: v for k, v in facts_row.items() if v is not None}
        return finish(doc)
    if whdoc is None:
        return finish(ks.failure(args.sha, args.tier, steps, "host wheelhouse failed rc=%d" % rc, args.run_id))
    transfer.update({"wheelhouse_bytes": whdoc["total_bytes"], "wheelhouse_build_s": whdoc["build_s"]})
    flavor = whdoc["flavors"][0]
    need = 2 * whdoc["total_bytes"] + DISK_SLACK_BYTES
    if facts["disk_free_bytes"] is not None and facts["disk_free_bytes"] < need:
        return finish(ks.failure(args.sha, args.tier, steps, "guest-disk: %d MB free, %d MB needed" % (
            facts["disk_free_bytes"] >> 20, need >> 20), args.run_id))

    t0 = time.time()
    try:
        import shutil
        real = prefix / "src-real"
        shutil.rmtree(real, ignore_errors=True)
        shutil.copytree(kdir, real / "kfml", ignore=shutil.ignore_patterns("__pycache__", "reference", "tests"))
        a = tr.put_tree(real, GUEST_ROOT + "/src", STEP_BUDGET_S["copy"])
        b = tr.put_tree(wh_dir, GUEST_ROOT + "/wheelhouse", STEP_BUDGET_S["copy"])
    except GuestError as e:
        step("copy", 1, t0)
        return finish(ks.failure(args.sha, args.tier, steps, e.reason, args.run_id))
    step("copy", 0, t0)
    transfer.update({"src_bytes": a["bytes"], "copy_bytes": a["bytes"] + b["bytes"], "copy_s": round(a["s"] + b["s"], 1)})

    env = "cd %s/src/kfml && export PYTHONPATH=%s/src/kfml KF_ML_GUEST=1 KF_ML_RUN_ID=%s && " % (GUEST_ROOT, GUEST_ROOT, shlex.quote(args.run_id or ""))
    py = GUEST_ROOT + "/venv/bin/python"
    t0 = time.time()
    rc, out = tr.run(env + "python3 -m kfml wheelhouse --out %s/wheelhouse --verify" % GUEST_ROOT, STEP_BUDGET_S["verify"])
    step("verify", rc, t0)
    if rc != 0:
        return finish(ks.failure(args.sha, args.tier, steps, "guest wheelhouse verify failed: " + out.strip()[-120:], args.run_id))

    t0 = time.time()
    rc, out = tr.run(env + "python3 -m kfml setup --venv %s/venv --wheelhouse %s/wheelhouse --nvcc-if-needed 2>&1" % (GUEST_ROOT, GUEST_ROOT),
                     STEP_BUDGET_S["setup"])
    step("setup", rc, t0)
    transfer["install_s"] = steps[-1]["s"]
    if rc == 3:
        return finish(ks.not_run(args.sha, args.tier, steps, "no pinned flavour can run in this guest", ks.test_names(kdir, args.tier, run), args.run_id))
    if rc != 0:
        tail = out.strip().splitlines()[-1:] or [""]
        reason = "wheelhouse-flavour-mismatch: the host shipped %s; %s" % (flavor, tail[0][:120]) if "has no flavour" in out else "guest setup failed rc=%d" % rc
        return finish(ks.failure(args.sha, args.tier, steps, reason, args.run_id))

    t0 = time.time()
    rc, out = tr.run(env + "%s -m kfml fetch-data --from %s/wheelhouse/data 2>&1" % (py, GUEST_ROOT), STEP_BUDGET_S["fetch-data"])
    step("fetch-data", rc, t0)
    if rc != 0:
        return finish(ks.failure(args.sha, args.tier, steps, "guest fetch-data failed rc=%d (pin mismatch or file missing)" % rc, args.run_id))

    t0 = time.time()
    rc, out = tr.run(env + "%s -m kfml run --tier %s --json %s/rows.json --log-dir %s/logs 2>&1 | tail -n 5" % (py, args.tier, GUEST_ROOT, GUEST_ROOT),
                     STEP_BUDGET_S["run"])
    step("run", rc, t0)
    rc2, text = tr.run("cat %s/rows.json" % GUEST_ROOT, 120)
    try:
        rows = json.loads(text)
    except ValueError:
        rows = None
    if rc2 != 0 or not isinstance(rows, list) or not rows:
        return finish(ks.failure(args.sha, args.tier, steps, "guest kfml run wrote no rows", args.run_id))
    doc = ks.summarize(rows, args.sha, args.tier, steps, args.run_id)
    if steps[-1]["rc"] not in (0, 1) and doc["verdict"] == "PASS":
        doc["verdict"], doc["reason"] = "FAIL", "guest kfml run exited rc=%d" % steps[-1]["rc"]
    doc["facts"].setdefault("flavor", flavor)
    return finish(doc)
