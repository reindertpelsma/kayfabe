# SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later
"""The standalone FULL run: the manifest of stages as data, the refusals by name, and the evidence record (stdlib only).

STATUS: LIVE, 2026-11-17 (design: docs/design/V3_STANDALONE_FULL_RUN.md, usage: docs/howto/full-standalone-run.md). Code and GPU-free tests;
no box has run `--full` yet.

`vast_payload.py --full --instruction FILE SHA NONCE` plans EVERY stage of the manifest below in one invocation and refuses, by name and
before the first stage runs, any stage whose input is missing. A stage may leave the run only by an explicit `--skip STAGE`, and every skip is
printed (`KF_PAYLOAD_FULL ... skipped=`) and recorded in the result file (`full.skipped`), so a stage can never silently disappear. A run with a skip
says `complete=0`. Nothing in this module weakens a gate: it only decides what is planned and what is said about the plan.

The by-design NOT_RUN table is data for the evidence and the docs; it names the rows that cannot run in a single-box run for a reason that is not
a defect of the run, so that "no skips" is checkable: everything else that does not run is a failure.
"""

import socket

MANIFEST_VERSION = 1

# Every stage of the full run, in plan order (vast_payload.plan_stage_names with full=True returns the same names in the same order).
FULL_STAGES = (
    "provision", "cuda_host", "merge_bar", "cuda_guest",
    "apps", "apps_discovery", "kfml", "install_flow",
    "windows_gate", "windows_apps", "windows_gate_2", "windows_open_rows",
    "propimage_apps", "propimage_apps_b", "matlab", "specviewperf",
)

# A row that cannot run in a single-box run, and why. Not a skip: it is named in the record. Anything else that does not run fails by name.
BY_DESIGN_NOT_RUN = {
    "apps": "UnifiedMemoryStreams, UnifiedMemoryPerf, conjugateGradientUM, attach_verify: unsupported (UVM), owner ruling 2026-10-02",
    "apps_discovery": "nvml_dcgm DCGM half: the payload's apt is cache-only and holds no NVIDIA package (decided 2026-11-17)",
    "windows_open_rows": "dxsamples_multiadapter needs two GPUs; vkglcts_gl46 and vkglcts_vk need a native-Windows control from a second machine",
    "propimage_apps": "wukong has no verdict driver and stays out of a strict list; resolve, 3dmark and superposition are discovery rows",
}

# What a stage needs from the instruction and the command line. (stage, input name) -> predicate over (instruction, args).
# `args` is the parsed command line (only `matlab_image` is read here). Order = the order refusals are printed in.
REQUIREMENTS = (
    ("apps", "linux_apps", lambda i, a: i.linux_apps is not None),
    ("windows_gate", "windows", lambda i, a: bool(i.windows)),
    ("windows_apps", "winapps", lambda i, a: i.winapps is not None),
    ("windows_gate_2", "windows_runs_2", lambda i, a: i.windows_runs == 2),
    ("windows_open_rows", "windows", lambda i, a: bool(i.windows)),
    ("propimage_apps", "propimages", lambda i, a: bool(i.propimages)),
    ("matlab", "matlab_image", lambda i, a: i.matlab is not None or bool(getattr(a, "matlab_image", None))),
    ("specviewperf", "spec_both", lambda i, a: "specviewperf" in i.stages and i.spec == "both"),
)

# A stage that runs on what an earlier stage left behind: skipping the first while keeping the second is a different run, refused by name.
NEEDS = {"windows_apps": "windows_gate", "windows_open_rows": "windows_gate", "apps_discovery": "apps"}

ONLINE_PROBE_HOST = ("github.com", 443)


class FullRefused(Exception):
    """One or more named refusals; `.tokens` is a list of `stage:missing_input` strings."""

    def __init__(self, tokens):
        super().__init__(" ".join(tokens))
        self.tokens = list(tokens)


def online_capable(connect=socket.create_connection, target=ONLINE_PROBE_HOST, timeout=5):
    """True when this machine can open a TCP connection to the host the online guest's downloads start from. One plain connect, no data."""
    try:
        connection = connect(target, timeout=timeout)
    except OSError:
        return False
    try:
        connection.close()
    except (OSError, AttributeError):
        pass
    return True


def refusals(instruction, args, skip=(), online=None):
    """The named reasons `--full` cannot start: a list of `stage:input` tokens (empty = every stage of the manifest has what it needs).
    A stage named in `skip` is not checked (its skip is recorded elsewhere). `online` is the result of online_capable(); None = not asked."""
    out = []
    if instruction.tier != "large":
        out.append("tier:large")
    if instruction.propimages and not instruction.propimages_strict:
        out.append("propimage_apps:strict")
    for stage, name, ok in REQUIREMENTS:
        if stage in skip:
            continue
        if not ok(instruction, args):
            out.append(stage + ":" + name)
    if online is False and "windows_open_rows" not in skip:
        out.append("windows_open_rows:online_host")
    return out


def check_flags(only, skip, instruction_given):
    """Flag combinations that make `--full` mean something else. Returns a token or None."""
    if only:
        return "full:with_only"
    if not instruction_given:
        return "full:needs_instruction_file"
    bad = [s for s in skip if s not in FULL_STAGES]
    if bad:
        return "skip:unknown:" + bad[0]
    # the base stages are prerequisites of every later stage: skipping one is a different run, not a shorter full run
    base = [s for s in skip if s in ("provision", "merge_bar")]
    if base:
        return "skip:prerequisite:" + base[0]
    for stage, needs in NEEDS.items():
        if needs in skip and stage not in skip:
            return "skip:needs:%s:%s" % (stage, needs)
    return None


def record(planned, skip):
    """The evidence object for the result file (`full`, validated by vast_report.validate_full) and the printed line's fields."""
    skipped = [s for s in FULL_STAGES if s in skip]
    return {
        "manifest": MANIFEST_VERSION,
        "stages": list(planned),
        "skipped": skipped,
        "complete": not skipped and [s for s in FULL_STAGES if s in planned] == list(FULL_STAGES),
        "by_design": sorted(k for k in BY_DESIGN_NOT_RUN if k in planned),
    }


def line(rec):
    return "KF_PAYLOAD_FULL manifest=%d stages=%d skipped=%s complete=%d by_design=%s" % (
        rec["manifest"], len(rec["stages"]), ",".join(rec["skipped"]) or "none", int(rec["complete"]), ",".join(rec["by_design"]) or "none")
