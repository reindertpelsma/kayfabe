#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later
"""The `apps_discovery` payload stage: the open Linux rows that report and never gate, run in the kayfabe guest on the box (stdlib only).

STATUS: LIVE, 2026-11-20 (design: docs/design/V3_STANDALONE_FULL_RUN.md section 4). Code and GPU-free tests. First run on a box 2026-11-20 (RTX 4090, stock Ubuntu 22.04
host): all six rows red or NOT_RUN, because the stage booted the BASE guest image, not the apps image, and the host-side ML setup refused two rows only after they had been
booted; this revision fixes both (see the 2026-11-20 note in the design document). Not re-run on a box since.

Rows: nvcodec_matrix and nvml_dcgm (scripts/apps/open_rows), vkcts (scripts/apps/vkcts), vllm, tensorrt and jax (scripts/apps/mlrows). They are in no gate list
and `apps_matrix.sh ... all` leaves out the first four of them; this stage names them. LANE: discovery. A row that RAN and failed, crashed, hung or timed out is a
known breakage: it is reported, never a failure of the stage (that is what a discovery row is for). A row that did NOT RUN (no result line, NOTRUN, a precondition
the harness could not meet, a GPU-use proof that could not be judged) fails the stage BY NAME (`discovery_not_run:<row>`) when the stage is strict, which `--full`
always is: "report, never gate" is about the verdict of a row, not about whether it ran. An unjudgeable proof is NOT_RUN too: a vkcts row whose note says CONFIG (the
device was not a real GPU, or the host counters did not show GPU use) and a HANG without a result line (nothing says the row ever started) are NOT_RUN, not breakages.

The guest is the APPS image (`guest_apps.qcow2`, named once in scripts/kfci/apps_gate.py `apps_image`, as is the base image's name), booted through a THROWAWAY overlay: the
stage makes `qemu-img create -b <apps image>` beside it (with a copy of the driver receipt `<image>.STAGED` that the boot helper reads), points KF_GUEST_IMG at the overlay for every
step, and removes both at the end, so no boot of this stage can write an image (the boot helper opens KF_GUEST_IMG writable). It refuses before anything starts: FAIL
`not_apps_image` when KF_GUEST_IMG names any other file, or when the apps image is the base image (a link); RETRY `no_apps_image` when the apps image is absent; RETRY
`overlay_failed` when the overlay cannot be made. The base guest.qcow2 has no /opt/apps and persists writes: it is never a boot target of this stage.

Steps (each bounded, each recorded): (1) `vkcts.py build` once on the host, (1b) `private_python.py ensure --if-needed` for the ML rows (2026-11-20: a Python 3.12 built from the python.org source below the mlrows root
when the host's is older than the locks, anchored through scripts/ci/kf_trust.py or refused by name; its outcome is `private_python` in the record, never a retry), (2) `mlrows.py setup` for the three ML rows on the host (network, hashed locks); a row it refuses (`MLROWS_SETUP row=R status=NOT_RUN reason=WHY`, for example
`python_too_old:3.10<3.12` on a 22.04 host) is NOT booted: it is reported NOT_RUN with that reason at once and the stage still fails by name at the end,
(2b) the throwaway overlay of the apps image (above), (3) when the host side of the matrix is installed (/opt/apps/bundle), `apps_matrix.sh host RUN <rows>` as the control (it runs FIRST: the comparison the matrix prints at the
end of a guest run reads the host log); its absence is recorded (`host_control=absent`), never a failure, (4) `apps_matrix.sh guest RUN <rows>` (the guest is the subject).
The comparison is a finding, not a verdict.

  apps_discovery_stage.py plan
  apps_discovery_stage.py run --sha SHA --checkout DIR --out FILE --run-id ID [--rows a,b] [--deadline-epoch N] [--lenient]
Exit codes: 0 PASS (every row ran), 1 FAIL (named), 3 RETRY (a harness problem: no bundle, no apps image, no time left).
One `KF_APPS_DISCOVERY_RESULT {json}` line, also written to --out.
"""

import argparse
import json
import os
from pathlib import Path
import re
import shutil
import subprocess
import sys
import time

HERE = Path(__file__).resolve().parent
if str(HERE.parent) not in sys.path:
    sys.path.insert(0, str(HERE.parent))        # scripts/: the kfci package (stdlib only); the apps image is named in ONE place, kfci/apps_gate.py
from kfci.apps_gate import apps_image, base_image  # noqa: E402

OPEN_ROWS = HERE.parent / "apps" / "open_rows"
if str(OPEN_ROWS) not in sys.path:
    sys.path.insert(0, str(OPEN_ROWS))          # the row's own parsers (stdlib only): the host control is read with the code that wrote it
import nvcodec_lib as nvl  # noqa: E402
import rowio  # noqa: E402

ROWS = ("nvcodec_matrix", "nvml_dcgm", "vkcts", "vllm", "tensorrt", "jax")
# the row's own timeout in scripts/apps/run_apps.sh, in seconds (a stage budget is the sum of them, per side)
ROW_TIMEOUT_S = {"nvcodec_matrix": 3000, "nvml_dcgm": 3300, "vkcts": 2700, "vllm": 1800, "tensorrt": 1500, "jax": 1200}
ML_ROWS = ("vllm", "tensorrt", "jax")
APPS_ROOT = "/workspace/apps"
HOST_BUNDLE = "/opt/apps/bundle"
STEP_TIMEOUT_S = {"build_vkcts": 3600, "python312": 6000, "setup_mlrows": 3600, "overlay": 300, "nvcodec_provision": 1800, "nvcodec_control": 1800, "host": 4 * 3600, "guest": 6 * 3600}   # python312: above the sum of private_python.TIMEOUT_S, so its own timeouts fire first
RAN = {"PASS", "FAIL", "TIMEOUT", "HANG", "GUEST_DEAD", "BOOT_FAIL"}
NOT_RAN = {"NOTRUN"}
APPRES = re.compile(r"^APPRES side=(host|guest) app=([A-Za-z0-9_]{1,40}) verdict=([A-Z_]{1,16}) .*?note=(.*)$")
DCGM_STATE = re.compile(r"\bdcgm=([A-Za-z_]{1,16})\b")
SAFE = re.compile(r"[A-Za-z0-9_.:=-]{1,80}")
# what `mlrows.py setup` prints for a row (mlrows.py cmd_setup; kv() turns the reason's spaces into underscores)
SETUP_LINE = re.compile(r"^MLROWS_SETUP row=([a-z]{1,20}) status=(\S{1,20}) reason=(\S*)$", re.M)
# what `private_python.py ensure` prints (its cmd_ensure; kv() turns spaces into underscores)
PYTHON_LINE = re.compile(r"^PRIVATE_PYTHON status=(\S{1,20}) reason=(\S{0,160}) version=\S+ python=\S+$", re.M)
# the host control of nvcodec_matrix (2026-11-20): its own root below the apps root (the probe is built there; nothing of the apps bundle is used), and the vocabulary of its comparison
NVCODEC = "nvcodec_matrix"
CONTROL_DIR = "nvcodec_control"
CONTROL_STEPS = ("nvcodec_provision", "nvcodec_control")
CONTROL_ENV_DROPPED = ("KF_GUEST_IMG", "BENCH_DIR")         # the control never sees an image or the directory that holds one
GUEST_FAILED = ("FAIL", "MISMATCH")                         # what a guest case that failed says (nvcodec_lib RESULTS)
HOST_RAN = ("PASS", "FAIL", "MISMATCH")                     # a host case with one of these results was really attempted
CLASS_ORDER = ("GUEST_ONLY", "UNCLASSIFIED", "AGREEMENT")   # the most telling first: a row takes the first class any of its failing cases has
MAX_FINDINGS = 8
MAX_TABLE_ROWS = 200
GUEST_LOG_TAIL = 1 << 20                                    # the guest's log is the guest's: only its last MiB is read, where the emitted results.tsv block is
PROVISION_FAIL = re.compile(r"^OPENROWS_(nvcodec_probe|ffmpeg)=FAIL reason=(\S*)$", re.M)
TABLE_COLUMNS = ("case_id", "host_expect", "host", "host_reason", "guest_expect", "guest", "guest_reason", "class")
# a vkcts row whose note says CONFIG did not exercise a GPU (vkcts.py APPNOTE); a HANG note from apps_hook.sh says no result line came back
CONFIG_NOTE = "CONFIG"
NO_RESULT_NOTE = "no-result-line"


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


def setup_refusals(text, rows):
    """{row: reason} for the rows `mlrows.py setup` REFUSED (status=NOT_RUN: python too old, no GPU, toolchain missing ...), with its own reason, bounded and printable.
    A row it set up (ok) or failed on (FAIL: the guest run names that itself) is not here, and neither is a line for a row the stage did not ask for."""
    out = {}
    for row, status, reason in SETUP_LINE.findall(text or ""):
        if status == "NOT_RUN" and row in rows and row in ML_ROWS:
            tidy = re.sub(r"_*([<>:])_*", r"\1", reason)
            out[row] = (re.sub(r"[^A-Za-z0-9_.:=<>-]+", "_", tidy).strip("_")[:80]) or "setup_refused"
    return out


def private_python_summary(text, rc):
    """One printable token for the record: `<status>:<reason>` of the PRIVATE_PYTHON line the step printed (the last one), or FAIL:no_result_line_rc<rc> when it printed none
    (a crash, a timeout). Bounded and printable, whatever the build tools said."""
    found = PYTHON_LINE.findall(text or "")
    if not found:
        return "FAIL:no_result_line_rc%d" % rc
    status, reason = found[-1]
    return re.sub(r"[^A-Za-z0-9_.:=<>-]+", "_", "%s:%s" % (status, reason))[:100]


def tidy(text, limit=40):
    return re.sub(r"[^A-Za-z0-9_.:=-]+", "_", str(text or "")).strip("_")[:limit] or "unspecified"


def safe_case(case_id):
    """A case id as the record's validator reads one: lower case, digits and underscores, starting with a letter, at most 40 characters (the guest's output is the guest's)."""
    name = re.sub(r"[^a-z0-9_]+", "_", str(case_id).lower()).strip("_")[:40]
    return name if name[:1].isalpha() else ("c_" + name)[:40]


def classify_case(host, guest):
    """-> (class, why) of ONE case that FAILED in the guest. `guest` is its results row, `host` the control's row of the same case or None.
    GUEST_ONLY: the host passes it (a gap of kayfabe). AGREEMENT: the host fails it too (not a statement about kayfabe). UNCLASSIFIED: the host could not judge it
    (not run, not supported, not listed): the failure stays an ordinary breakage and the host's own word is the `why`."""
    if host is None:
        return "UNCLASSIFIED", "host_case_missing"
    h, g = host["result"], guest["result"]
    if h == "PASS":
        return "GUEST_ONLY", "host_pass"
    if h in GUEST_FAILED:
        return "AGREEMENT", "host_" + h.lower()
    if h == "NOT_RUN":
        return "UNCLASSIFIED", "host_not_run:" + tidy(host.get("reason"))
    return "UNCLASSIFIED", "host_%s_guest_%s" % (h.lower(), g.lower())


def host_control_state(provision_text, provision_rc, results_text, control_rc):
    """What the control on the bare host amounted to. -> {status ran|NOT_RUN, reason, rows, verdict}. NOT_RUN names the real reason: the stock ffmpeg or the probe could
    not be provided (provision.sh's own OPENROWS_ lines), the provisioning died, the matrix wrote no results, or no case was attempted at all (no NVENC: the cases' own reason)."""
    failed = dict(PROVISION_FAIL.findall(provision_text or ""))
    if "ffmpeg" in failed:
        return {"status": "NOT_RUN", "reason": "no_ffmpeg:" + tidy(failed["ffmpeg"]), "rows": [], "verdict": "-"}
    if "nvcodec_probe" in failed:
        return {"status": "NOT_RUN", "reason": "no_probe:" + tidy(failed["nvcodec_probe"]), "rows": [], "verdict": "-"}
    if provision_rc:
        return {"status": "NOT_RUN", "reason": "provision_failed_rc%d" % provision_rc, "rows": [], "verdict": "-"}
    rows = nvl.parse_results(results_text)[:MAX_TABLE_ROWS] if results_text else []
    if not rows:
        return {"status": "NOT_RUN", "reason": "no_results_rc%s" % control_rc, "rows": [], "verdict": "-"}
    if not any(r["result"] in HOST_RAN for r in rows):
        counts = {}
        for r in rows:
            counts[tidy(r["reason"])] = counts.get(tidy(r["reason"]), 0) + 1
        return {"status": "NOT_RUN", "reason": "no_case_ran:" + max(sorted(counts), key=counts.get), "rows": rows, "verdict": "-"}
    verdict = "FAIL" if any(r["result"] in GUEST_FAILED for r in rows) else "PASS"
    return {"status": "ran", "reason": "", "rows": rows, "verdict": verdict}


def compare_with_guest(control, guest_rows):
    """The control with the per-case table (host next to guest) and the findings, from the guest's results rows. Pure."""
    host = {r["case_id"]: r for r in control["rows"]}
    guest = {r["case_id"]: r for r in guest_rows[:MAX_TABLE_ROWS]}
    table, findings = [], []
    for cid in list(guest) + [c for c in host if c not in guest]:
        h, g = host.get(cid), guest.get(cid)
        cls, why = classify_case(h, g) if g is not None and g["result"] in GUEST_FAILED else ("-", "")
        table.append({"case_id": cid, "host_expect": h["expect"] if h else "-", "host": h["result"] if h else "-", "host_reason": h["reason"] if h else "-",
                      "guest_expect": g["expect"] if g else "-", "guest": g["result"] if g else "-", "guest_reason": g["reason"] if g else "-", "class": cls})
        if cls != "-":
            findings.append({"case": safe_case(cid), "guest": g["result"], "host": h["result"] if h else "-", "class": cls, "why": why})
    findings.sort(key=lambda f: CLASS_ORDER.index(f["class"]))              # a stable sort: the telling ones survive the bound
    cls = next((c for c in CLASS_ORDER if any(f["class"] == c for f in findings)), "")
    return dict(control, table=table, findings=findings, count=len(findings), cls=cls)


def table_tsv(table):
    def cell(v):
        return re.sub(r"[\t\r\n]+", " ", str(v or "-"))[:100]
    return "\t".join(TABLE_COLUMNS) + "\n" + "".join("\t".join(cell(r[c]) for c in TABLE_COLUMNS) + "\n" for r in table)


def read_tail(path, limit):
    try:
        with open(path, "rb") as f:
            f.seek(0, 2)
            f.seek(max(0, f.tell() - limit))
            return f.read().decode("utf-8", "replace")
    except OSError:
        return ""


def grade(rows, guest, host=None, dcgm_text="", refused=None, control=None):
    """The stage record. `guest` and `host` are read_res() dicts; `refused` is {row: reason} for rows that were never booted; `control` is the host control of
    nvcodec_matrix (host_control_state, then compare_with_guest): a guest row that failed is explained by it per case. Pure."""
    entries, not_run, breakages = [], [], []
    for name in rows:
        verdict, note = guest.get(name, (None, ""))
        host_verdict = (host or {}).get(name, (None, ""))[0]
        if name in (refused or {}):
            verdict = None
            status, reason = "NOT_RUN", refused[name]
        elif verdict is None:
            status, reason = "NOT_RUN", "no_guest_row"
        elif verdict in NOT_RAN:
            status, reason = "NOT_RUN", reason_of(note)
        elif verdict == "PASS":
            status, reason = "PASS", ""
        elif verdict == "FAIL" and note.startswith(CONFIG_NOTE):
            status, reason = "NOT_RUN", reason_of(note)          # the row's own configuration check said it did not exercise the GPU: nothing was judged
        elif verdict == "HANG" and NO_RESULT_NOTE in note:
            status, reason = "NOT_RUN", "hang_without_result_line"   # no result line came back: nothing says the row ever started
        elif verdict in RAN:
            status, reason = "KNOWN_BREAKAGE", verdict
        else:
            status, reason = "NOT_RUN", "unknown_verdict_" + reason_of(verdict)
        if not host_verdict and name == NVCODEC and control and control["status"] == "ran":
            host_verdict = control["verdict"]                    # the apps_matrix host side wins when it ran; else the control's own verdict of the row
        explained = name == NVCODEC and status == "KNOWN_BREAKAGE" and verdict == "FAIL" and bool(control and control.get("findings"))
        if explained:
            n = control["count"]
            if control["cls"] == "AGREEMENT":
                status, reason = "PASS", "agrees_with_host:%d_failing_case%s" % (n, "" if n == 1 else "s")     # the host fails them too: not a breakage of kayfabe
            elif control["cls"] == "GUEST_ONLY":
                reason = "host-capable,guest-fails"
        if status == "NOT_RUN":
            not_run.append(name)
        if status == "KNOWN_BREAKAGE":
            breakages.append(name)
        entry = {"name": name, "status": status, "guest": verdict or "-", "host": host_verdict or "-", "reason": reason}
        if explained:
            entry["class"], entry["findings"] = control["cls"], control["findings"][:MAX_FINDINGS]
        if name == "nvml_dcgm" and status == "PASS":
            state = DCGM_STATE.search(dcgm_text)
            if state:
                entry["dcgm"] = state.group(1)
        entries.append(entry)
    return {"rows": entries, "not_run": not_run, "breakages": breakages, "host_control": "present" if host or (control and control["status"] == "ran") else "absent"}


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
        steps += ["python312", "setup_mlrows"]         # the private Python 3.12 first (a host whose Python is already inside the pinned range builds nothing), then the venvs
    steps.append("overlay")                         # the throwaway overlay of the apps image first
    if NVCODEC in rows:
        steps += list(CONTROL_STEPS)               # the nvcodec host control: the bare host, stock ffmpeg, no bundle and no image; before any boot
    steps += ["host", "guest"]                      # the apps_matrix host side only when it is installed
    return {"rows": list(rows), "steps": steps, "budget_s_per_side": sum(ROW_TIMEOUT_S[r] for r in rows)}


def known_rows(checkout, runner=None):
    """The names run_apps.sh knows (its `list`); every row of this stage must be one of them."""
    scratch = Path(os.environ.get("TMPDIR", "/tmp")) / ("kf-discovery-list-%d" % os.getpid())
    scratch.mkdir(parents=True, exist_ok=True)
    env = dict(os.environ, APPS_ROOT=str(scratch))
    out = subprocess.run(["bash", str(Path(checkout) / "scripts/apps/run_apps.sh"), "x", "list"], env=env, capture_output=True, text=True, timeout=30, check=False).stdout
    return set(out.split())


def run(args, runner=real_runner, known=None, now=time.time, host_bundle=HOST_BUNDLE, environ=None):
    rows = tuple(args.rows.split(",")) if args.rows else ROWS
    base_env = dict(os.environ if environ is None else environ)
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
    bench_env = {"BENCH_DIR": base_env.get("BENCH_DIR", "")}
    image = apps_image(bench_env)                     # the apps image of BENCH_DIR, never an override: this stage boots THE image the `apps` stage provisioned
    requested = base_env.get("KF_GUEST_IMG")
    if (requested and os.path.realpath(requested) != os.path.realpath(image)) or os.path.realpath(image) == os.path.realpath(base_image(bench_env)):
        return finish("FAIL", "not_apps_image")       # a refusal by name before any boot: the base guest.qcow2 (or any other file) is not the apps image
    if not Path(image).is_file():
        return finish("RETRY", "no_apps_image")       # the `apps` stage provisions it; the base guest.qcow2 is NOT a substitute (no /opt/apps, and a boot writes to it)
    overlay = Path(image).with_name("discovery_%s.qcow2" % args.run_id)
    budget = plan(rows)["budget_s_per_side"]
    deadline = args.deadline_epoch
    if deadline and now() + budget > deadline:
        return finish("RETRY", "no_time_left")
    env = dict(base_env, KF_APPS_ROOT=args.apps_root, APPS_APP_TMO=base_env.get("APPS_APP_TMO", "3600"), APPS_PER_BOOT="1", KF_GUEST_IMG=str(overlay))
    log_dir = result_dir.parent
    log_dir.mkdir(parents=True, exist_ok=True)

    def step(name, argv, key, step_env=None):
        s = run_step(runner, name, argv, env if step_env is None else step_env, log_dir, STEP_TIMEOUT_S[key])
        steps.append(s)
        return s["rc"]

    def cleanup():
        for leftover in (overlay, Path(str(overlay) + ".STAGED")):
            leftover.unlink(missing_ok=True)

    try:
        if "vkcts" in rows and step("build_vkcts", ["python3", str(apps / "vkcts/vkcts.py"), "build", "--root", args.apps_root + "/vkcts"], "build_vkcts"):
            return finish("RETRY", "vkcts_build_failed")
        ml = [r for r in rows if r in ML_ROWS]
        refused = {}
        if ml:
            ml_root = args.apps_root + "/mlrows"           # one root for the private Python, the venvs, the host control and the push: mlrows.py finds the private Python below it
            # a Python older than the locks (Ubuntu 22.04: 3.10) gets a private 3.12 built from the python.org source (scripts/apps/mlrows/private_python.py); idempotent, and a
            # refusal or a failed build is not a retry: it is recorded, and the rows then say python_too_old below (a named NOT_RUN), as before
            rc_py = step("python312", ["python3", "-B", "-I", str(apps / "mlrows/private_python.py"), "ensure", "--if-needed", "--root", ml_root], "python312")
            try:
                record["private_python"] = private_python_summary((log_dir / "python312.log").read_text(errors="replace")[-20000:], rc_py)
            except OSError:
                record["private_python"] = "FAIL:no_result_line_rc%d" % rc_py
            # rc 0 on a refusal is by design (mlrows.py cmd_setup); a setup that did not finish is a named NOT_RUN of those rows (the guest run says why), not a harness retry
            step("setup_mlrows", ["python3", "-B", "-I", str(apps / "mlrows/mlrows.py"), "setup", "--root", ml_root, "--rows", ",".join(ml)], "setup_mlrows")
            try:
                refused = setup_refusals((log_dir / "setup_mlrows.log").read_text(errors="replace")[-200000:], ml)
            except OSError:
                pass
        # a row the host setup refused has no venv to run in: booting it costs a boot per row (about 26 minutes for two rows on 2026-11-20) to learn what setup already said
        boot_rows = tuple(r for r in rows if r not in refused)
        host_ok = Path(host_bundle).is_dir()
        guest, host, guest_rc, dcgm_text, control = {}, None, None, "", None
        if boot_rows:
            # the guest boots THIS file, writable, and every boot of the stage lands in it: a child of the apps image, which no boot touches. The boot helper reads the driver
            # receipt `<image>.STAGED` beside the path it is given, so the child carries a copy (an image without one is refused by name there, as the apps image would be)
            overlay.unlink(missing_ok=True)
            if step("overlay", ["qemu-img", "create", "-q", "-f", "qcow2", "-F", "qcow2", "-b", os.path.realpath(image), str(overlay)], "overlay"):
                return finish("RETRY", "overlay_failed")
            receipt = Path(image + ".STAGED")
            if receipt.is_file():
                shutil.copyfile(receipt, str(overlay) + ".STAGED")
            if NVCODEC in boot_rows:
                # the HOST CONTROL (2026-11-20), before any guest boot: the row's own matrix on the bare host through the distribution's ffmpeg. It needs no apps bundle (the probe is
                # built into its own root by provision.sh, the two steps it asks for) and never sees an image: its environment has no KF_GUEST_IMG and no BENCH_DIR
                ctl_root, ctl_out = args.apps_root + "/" + CONTROL_DIR, result_dir / "nvcodec_host"
                ctl_env = {k: v for k, v in env.items() if k not in CONTROL_ENV_DROPPED}
                rc_p = step("nvcodec_provision", ["bash", str(apps / "open_rows/provision.sh"), ctl_root, "nvcodec_probe ffmpeg"], "nvcodec_provision", ctl_env)
                provision_text = read_tail(log_dir / "nvcodec_provision.log", 20000)
                rc_c, results_text = None, None
                if rc_p == 0 and not PROVISION_FAIL.search(provision_text):          # the stock ffmpeg and the probe are there: run the matrix; else there is nothing to run it with
                    rc_c = step("nvcodec_control", ["python3", "-B", "-I", str(apps / "open_rows/nvcodec_matrix.py"), "run", "--out", str(ctl_out),
                                                    "--probe", ctl_root + "/bin/nvcodec_probe"], "nvcodec_control", ctl_env)
                    results_text = read_tail(ctl_out / "results.tsv", 200000) or None
                control = host_control_state(provision_text, rc_p, results_text, rc_c)
                record["nvcodec_host_control"] = "ran" if control["status"] == "ran" else "NOT_RUN:" + control["reason"]
            if host_ok:
                step("host", ["bash", str(apps / "apps_matrix.sh"), "host", args.run_id, *boot_rows], "host")
            guest_rc = step("guest", ["bash", str(apps / "apps_matrix.sh"), "guest", args.run_id, *boot_rows], "guest")
            guest = read_res(result_dir / "guest_isolated.res", result_dir / "guest.res")
            if control and control["status"] == "ran":
                # the host's verdict per case next to the guest's (the guest's results.tsv is in its log between @@FILE markers, rowio): a TSV beside the logs, findings in the record
                control = compare_with_guest(control, nvl.parse_results(rowio.extract(read_tail(result_dir / "nvcodec_matrix.guest.log", GUEST_LOG_TAIL)).get("results.tsv", "")))
                (result_dir / "nvcodec_host_vs_guest.tsv").write_text(table_tsv(control["table"]))
            host = read_res(result_dir / "host.res") if host_ok else None
            try:
                dcgm_text = (result_dir / "nvml_dcgm.guest.log").read_text(errors="replace")[-20000:]
            except OSError:
                pass
        graded = grade(rows, guest, host, dcgm_text, refused, control)
        record.update(graded)
        if boot_rows and not guest:
            return finish("RETRY", "no_guest_results", {"guest_rc": guest_rc})
        verdict = verdict_of(graded, strict=not args.lenient)
        return finish(verdict, "discovery_not_run:" + ",".join(graded["not_run"]) if graded["not_run"] else "")
    finally:
        cleanup()


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
