# SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later
"""Join committed trace evidence to the pinned OGKM interface."""

import argparse
import json
from itertools import chain, groupby
import re
import subprocess
from collections import Counter, defaultdict
from dataclasses import dataclass, field
from pathlib import Path

from .evidence import CANDIDATES, Census, parse
from .interface import CONTROL, extract

ROOT = Path(__file__).resolve().parents[3]
KINDS = ("control", "class", "method", "escape", "ioctl", "drm_ioctl", "modeset_ioctl", "host_escape", "route", "rpc", "xid")
OUTCOMES = ("serviced", "refused", "unserviced", "fabricated", "passthrough",
            "non_ok_status", "observed")
TRACE_PATHS = ("*.log", "*.txt", "*.out", "*.tail", "*.summary")
FEATURES = (
    (r"HDCP|DISPLAYPORT|\bDP_|NV0073|NV5070|MODESET|VBLANK|BRIGHTNESS|EDID|HDR", 5,
     "Display and modesetting", "Compare monitor enumeration and mode changes with a native run."),
    (r"NVENC|NVDEC|ENCODER|DECODER|VIDEO|OPTICAL_FLOW|NVJPEG", 5,
     "Video and optical flow", "Compare capability enumeration and a short encode/decode or optical-flow run."),
    (r"UVM|GMMU|VASPACE|PEER|P2P|PAGE_FAULT|NVLINK|VMMU|DECOMP_LCE|DMABUF|ALLOC_MEMORY|MEM_OP|MMU_OPERATION", 5,
     "Memory and sharing", "Compare managed-memory migration, then repeat with memory sharing enabled."),
    (r"ECC|THERM|POWER|CLOCK|TEMPERATURE|PERF|GPU_CACHE|TELEMETRY|USER_SHARED_DATA", 4,
     "Monitoring and power", "Compare read-only management queries at idle and under load."),
    (r"PREEMPT|SCHEDUL|RUNLIST|CHANNEL|CONTEXT|CTXSW|PROMOTE_CTX|ZCULL", 4,
     "Scheduling and contexts", "Compare context creation and alternating compute and graphics workloads."),
    (r"COMPUTE|CUDA|GFX|GRAPHICS|RENDER|VULKAN|SM_QUERY|GR_GET|GR_SET", 4,
     "Graphics and compute", "Compare Vulkan and CUDA device capabilities, then start a minimal workload."),
    (r"EVENT|NOTIFICATION|INTERRUPT|SEMAPHORE", 3,
     "Events and synchronization", "Compare event delivery and completion under a short repeated workload."),
    (r"GET_INFO|GPU_GET|DEVICE_GET|CAPS|GET_CLASS", 3,
     "Device discovery", "Compare device enumeration and capability queries before launching work."),
)
UNKNOWN_FEATURE = (1, "Feature unclear", "Compare boot or application-stage census windows on native and guest runs.")


def git(*args, cwd=ROOT, check=True):
    return subprocess.run(["git", *args], cwd=cwd, check=check, stdout=subprocess.PIPE).stdout


def feature(names, owner_names):
    for pattern, weight, area, probe in FEATURES:
        if re.search(pattern, names + " " + owner_names):
            return weight, area, probe
    return UNKNOWN_FEATURE


def handler_index():
    """Index named controls in the product's policy modules and row table."""
    rm = ROOT / "crates/kf-rm/src"
    abi = {}
    for path in (ROOT / "crates/kf-abi/src").rglob("*.rs"):
        for name, value in re.findall(r"\bconst (\w+):\s*u32\s*=\s*(0x[0-9a-fA-F_]+)\s*;",
                                      path.read_text()):
            if CONTROL.fullmatch(name):
                abi.setdefault(name, set()).add(int(value, 16))

    matrix = (ROOT / "crates/kf-abi/src/generated/matrix.rs").read_text()
    for name, runs in re.findall(r'name: "ctrl_cmds:(\w+)".*?runs: &\[(.*?)\]', matrix, re.S):
        for value in re.findall(r"Some\((0x[0-9a-fA-F]+)\)", runs):
            abi.setdefault(name, set()).add(int(value, 16))
    handlers = defaultdict(set)
    for path in sorted(rm.glob("*.rs")):
        code = re.sub(r"(?m)^\s*//.*$", "", path.read_text().split("#[cfg(test)]", 1)[0])
        if path.stem in {"census", "unserviced", "hostquery", "hostfacts", "consttab",
                         "authored", "rpc", "abi", "sticky"}:
            continue
        for name in set(re.findall(r"\bNV\w+_CTRL_CMD_\w+\b", code)):
            for value in abi.get(name, ()):
                handlers[value].add(f"kf-rm::{path.stem}")
        if path.stem == "rowserve":
            for value in re.findall(r'"rm\.(0x[0-9a-fA-F]+)"', code):
                handlers[int(value, 16)].add("kf-rm::rowserve (see USES)")
    return handlers


@dataclass
class Observation:
    issued: int = 0
    outcomes: Counter = field(default_factory=Counter)
    traces: set = field(default_factory=set)

    def add(self, trace, counts, issued):
        self.issued += issued
        self.outcomes.update(counts)
        self.traces.add(trace)


def trace_evidence(commit, prefix):
    """Read committed text by blob, skipping identical copies in a trace directory."""
    listing = git("ls-tree", "-rz", commit, "--", prefix)
    traces = set()
    blobs = {}
    for record in listing.split(b"\0"):
        if not record:
            continue
        info, raw_path = record.split(b"\t", 1)
        path = raw_path.decode()
        parts = path.split("/")
        if len(parts) < 3:
            continue
        trace = parts[1]
        traces.add(trace)
        mode, kind, oid = info.decode().split()
        if kind == "blob" and mode.startswith("100"):
            blobs[path] = oid

    seen = set()
    paths = [f":(glob){prefix}/**/{pattern}" for pattern in TRACE_PATHS]
    with subprocess.Popen(
        ["git", "grep", "--no-textconv", "-z", "-I", "-E", CANDIDATES, commit, "--", *paths],
        cwd=ROOT, stdout=subprocess.PIPE,
    ) as source:
        def records():
            for raw in source.stdout:
                if b"\0" in raw:
                    label, line = raw.split(b"\0", 1)
                    yield label.decode().split(":", 1)[1], line.decode(errors="replace")

        for path, records_in_blob in groupby(records(), key=lambda item: item[0]):
            trace = path.split("/")[1]
            key = (trace, blobs.get(path))
            if path in blobs and key not in seen:
                seen.add(key)
                yield trace, parse(line for _, line in records_in_blob)
        if source.wait() not in (0, 1):
            raise RuntimeError("git grep failed")
    for path in sorted(blobs):
        if not path.endswith("kf3_census_whole_boot.json"):
            continue
        trace = path.split("/")[1]
        key = (trace, blobs[path])
        if key in seen:
            continue
        seen.add(key)
        data = json.loads(git("show", f"{commit}:{path}"))
        census = Census()
        for code, count in data.get("unserviced_codes", {}).items():
            census.snapshot(("rpc", None, int(code)), "unserviced", int(count))
        if data.get("xid"):
            census.snapshot(("xid", None, 0), "observed", int(data["xid"]))
        yield trace, census
    yield from ((trace, None) for trace in sorted(traces))


def trace_dates(commit, prefix):
    """Find when each evidence directory first reached the revision's first parent."""
    lines = git("log", "--first-parent", "--diff-merges=first-parent", "--diff-filter=A",
                "--format=@%ct", "--name-only", commit, "--", prefix).decode().splitlines()
    dates = {}
    for line in lines:
        if line.startswith("@"):
            timestamp = int(line[1:])
        elif line.startswith("traces/") and line.count("/") >= 2:
            trace = line.split("/")[1]
            dates[trace] = min(timestamp, dates.get(trace, timestamp))
    return dates


def windows(commit, path):
    """Load disjoint per-case windows with typed public IDs and outcome counts."""
    payload = json.loads(git("show", f"{commit}:{path}"))
    if payload["schema"] != 1:
        raise ValueError("unsupported window schema")
    seen = set()
    for window in payload["windows"]:
        case = window["case"]
        if not re.fullmatch(r"[a-zA-Z0-9_.-]+", case) or case in seen:
            raise ValueError("invalid or duplicate census window case")
        seen.add(case)
        census = Census()
        for row in window["rows"]:
            kind, owner, value = row["kind"], row.get("class"), row["id"]
            status, count = row["outcome"], row["count"]
            if (kind not in KINDS or status not in OUTCOMES or type(value) is not int
                    or not 0 <= value <= 0xffffffff or type(count) is not int or count <= 0
                    or (owner is not None and (type(owner) is not int or not 0 <= owner <= 0xffff))
                    or (kind == "control" and owner != value >> 16)
                    or (owner is not None and kind not in {"control", "method"})):
                raise ValueError("invalid census window row")
            census.event((kind, owner, value), status, count)
        yield case, census


def build(args):
    definitions = defaultdict(set)
    unresolved = {}
    pins = dict(zip(("ogkm-580", "ogkm-595"), re.findall(
        r"^PIN(?:_595)?=([0-9a-f]{40})$", (ROOT / "scripts/ci/fetch_oracles.sh").read_text(), re.M
    ), strict=True))
    for name, pin in pins.items():
        root = args.oracles / name
        if not root.is_dir() or git("rev-parse", "HEAD", cwd=root).decode().strip() != pin:
            raise ValueError(f"{name} needs the pinned source checkout")
        if git("status", "--porcelain", "--untracked-files=no", cwd=root):
            raise ValueError(f"{name} has modified source files")
        entries, skipped = extract(root, name)
        unresolved[name] = sorted(skipped)
        for key, names in entries.items():
            definitions[key].update(names)

    commit = git("rev-parse", "--verify", f"{args.revision}^{{commit}}").decode().strip()
    extra = []
    for argument in args.extra_evidence:
        revision, separator, prefix = argument.partition(":")
        if not separator or not re.fullmatch(r"traces/[a-zA-Z0-9_.-]+", prefix):
            raise ValueError("extra evidence must be REVISION:traces/DIRECTORY")
        extra_commit = git("rev-parse", "--verify", f"{revision}^{{commit}}").decode().strip()
        if git("ls-tree", commit, "--", prefix) or any(p == prefix for _, p in extra):
            raise ValueError("extra evidence directory already included")
        if not git("ls-tree", extra_commit, "--", prefix):
            raise ValueError("extra evidence directory does not exist")
        extra.append((extra_commit, prefix))
    inputs = chain(trace_evidence(commit, "traces"), *(trace_evidence(*item) for item in extra))
    if args.windows:
        inputs = windows(commit, args.windows)
    observations = defaultdict(Observation)
    traces = set()
    matched = set()
    trace_order = {}
    for trace, census in inputs:
        traces.add(trace)
        trace_order.setdefault(trace, len(trace_order))
        if census is not None:
            for key, counts, issued in census.rows():
                if issued:
                    matched.add(trace)
                    observations[key].add(trace, counts, issued)

    if not args.windows:
        trace_order = trace_dates(commit, "traces")
        for extra_commit, prefix in extra:
            trace_order.update(trace_dates(extra_commit, prefix))
    handlers = handler_index()
    family_source = (ROOT / "crates/kf-chip/src/classes.rs").read_text().split(
        "pub const FAMILIES:", 1
    )[1].split("\n];", 1)[0]
    families = defaultdict(set)
    for name, contents in re.findall(r"family: Family::(\w+),(.*?)(?=ClassSet \{|$)",
                                     family_source, re.S):
        contents = re.sub(r"/\*.*?\*/", "", contents, flags=re.S)
        for value in re.findall(r"0x[0-9a-fA-F]+", contents):
            families[int(value, 16)].add(name)
    rows = []
    never = Counter()
    for key in sorted(definitions.keys() | observations.keys(),
                      key=lambda k: (k[0], -1 if k[1] is None else k[1], k[2])):
        kind, owner, value = key
        observation = observations.get(key, Observation())
        names = sorted(definitions.get(key, ()))
        name_text = " ".join(n.name for n in names)
        owner_names = sorted({n.name for n in definitions.get(("class", None, owner), ())})
        weight, area, probe = feature(name_text, " ".join(owner_names))
        if names and not observation.issued:
            never[kind, owner, area] += 1
        labels = sorted(observation.traces, key=lambda label: (trace_order.get(label, 0), label))
        rows.append({
            "kind": kind, "class": owner, "id": value, "definitions": [vars(n) for n in names],
            "class_names": owner_names, "issued": observation.issued,
            "outcomes": {outcome: observation.outcomes[outcome] for outcome in OUTCOMES},
            "first_trace": labels[0] if labels else None, "last_trace": labels[-1] if labels else None,
            "handler_candidates": sorted(handlers.get(value, ())) if kind == "control" else [],
            "generated_families": sorted(families.get(value if kind == "class" else owner, ())),
            "feature_inferred": area, "probe_inferred": probe, "feature_weight": weight,
        })

    refused = [r for r in rows if r["outcomes"]["refused"]]
    unserviced = [r for r in rows if r["outcomes"]["unserviced"]]
    status_only = [r for r in rows if r["outcomes"]["non_ok_status"]]
    leads = []
    for row in rows:
        outcomes = row["outcomes"]
        issues = max(outcomes["refused"] + outcomes["unserviced"], outcomes["non_ok_status"])
        gap = not row["issued"] and row["definitions"] and row["feature_weight"] > 1
        if row["kind"] in ("rpc", "xid", "host_escape", "route") or not (issues or gap):
            continue
        leads.append({**row, "issue_count": issues, "score": issues * row["feature_weight"]})
    leads.sort(key=lambda r: (-r["score"], -r["feature_weight"], not r["handler_candidates"],
                              r["kind"], r["id"], r["class"] or 0))
    return {
        "evidence_commit": commit, "extra_evidence": extra, "oracle_pins": pins, "unresolved_macros": unresolved,
        "trace_count": len(traces), "traces_with_observations": len(matched),
        "inventory": {kind: {"defined": sum(r["kind"] == kind and bool(r["definitions"]) for r in rows),
                             "observed": sum(r["kind"] == kind and bool(r["issued"]) for r in rows),
                             "never_observed": sum(n for (k, _, _), n in never.items() if k == kind)}
                      for kind in KINDS},
        "issued_but_refused": refused, "issued_but_unserviced": unserviced,
        "non_ok_status": status_only,
        "trace_order_basis": "window order" if args.windows else "first-parent addition date, then directory",
        "unseen_feature_leads": [r for r in leads if not r["issued"]][:40],
        "never_issued": [{"kind": k, "class": o, "feature_inferred": a, "count": n,
                          "user_feature_probe_inferred": next((p for _, _, area, p in FEATURES if area == a),
                                                              UNKNOWN_FEATURE[2])}
                         for (k, o, a), n in never.items()],
        "leads": leads[:40], "rows": rows,
    }


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--revision", default="HEAD", help="Git commit containing committed evidence")
    parser.add_argument("--oracles", type=Path, default=ROOT / "third_party")
    parser.add_argument("--windows", help="Use a committed JSON per-case census in place of raw traces")
    parser.add_argument("--extra-evidence", action="append", default=[], metavar="REVISION:traces/DIRECTORY")
    args = parser.parse_args()
    if args.windows and args.extra_evidence:
        parser.error("--windows and --extra-evidence are mutually exclusive")
    try:
        print(json.dumps(build(args), indent=2))
    except (ValueError, subprocess.CalledProcessError) as error:
        parser.exit(2, f"{error}\n")


if __name__ == "__main__":
    main()
