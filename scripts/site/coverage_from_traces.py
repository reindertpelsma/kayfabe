#!/usr/bin/env python3
"""Build site/status/coverage.json: what kayfabe's code has been run on, from evidence in the repo.

STATUS: LIVE, 2026-11-05. Input: the committed trace directories named in SETS. Output: one static
JSON file the status pages (site/status/status.js) read next to the live Worker data. Nothing here
is typed in as a result: every run's verdict, revision and counts are parsed from the verdict lines
of a committed log file (WINDOWS_GATE_PASS|FAIL|RETRY, V3_GATES_SUMMARY, FAST_SUITE_*, the Worker
job log's stage and suite rows). A run whose lines cannot be read unambiguously becomes verdict
UNKNOWN, and the page prints "see evidence" for it; it is never guessed. What this script does
state itself is only the mapping of dies to log files and the reason an uncovered die was not run,
and a test checks each of those against the trace README that states it.

Two sources, kept apart on every cell (docs/design/V3_PLATFORM_TESTING.md section 9):
- worker_reproduced: a run of the CI Worker (the gpu-vast check). Only rows of this kind can ever
  count for the release gate, and only through the Worker's result store, never through this file.
- imported_vast: a direct Vast run whose logs are committed under traces/. Evidence, not a gate pass.

Usage:
    python3 scripts/site/coverage_from_traces.py            # print
    python3 scripts/site/coverage_from_traces.py --write    # update site/status/coverage.json
    python3 scripts/site/coverage_from_traces.py --check    # exit 1 if the committed file is stale
"""

import hashlib
import json
from pathlib import Path
import re
import sys

ROOT = Path(__file__).resolve().parents[2]
TRACES = ROOT / "traces"
OUTPUT = ROOT / "site/status/coverage.json"
SCHEMA = 1

WORKER = "worker_reproduced"
IMPORTED = "imported_vast"
SOURCE_LABELS = {
    WORKER: "CI Worker run (reproduced)",
    IMPORTED: "Direct Vast run, evidence in repo (imported)",
}

# Trace sets. `readme` is the file that states the numbers; `date` is the date it states for the
# runs (a log's own enable_utc date is used when it has one, and a test checks they agree).
SETS = {
    "die_coverage_20261104": {
        "source": IMPORTED,
        "date": "2026-11-04",
        "headline_rev": "0c515b3b",
        "dir": "die_coverage_20261104",
    },
    "gate_windows_ci_20261105": {
        "source": WORKER,
        "date": "2026-11-05",
        "headline_rev": "d74f6063",
        "dir": "gate_windows_ci_20261105",
    },
}

# PLACEHOLDER, no data: traces/trio_vast_20261105 (PR #261) is two guests plus the host on one GPU,
# a different question from the die x platform matrix. It is imported by a later change once its
# README has been read for numbers; until then the generator only warns when the directory exists.
PENDING_SETS = ["trio_vast_20261105"]

DC = "die_coverage_20261104"
WIN = "gate_windows_ci_20261105"

# One entry per run: (set, tag, platform, log file under traces/, optional note). Order inside a die
# is the order the groups are first seen.
NOTE_GA106 = (
    "The FAIL (stage reboot) is classed in the README as a gate-harness defect, not a guest result; "
    "read the evidence."
)
NOTE_GB202 = "The FAIL is a guest bugcheck 0x113 during the idle window; the README gives no cause."
NOTE_BRANCH = "Run on an experimental branch revision, not on the integration revision."
RUNS = [
    # Turing
    (DC, "tu106a1", "linux", f"{DC}/turing/tu106_box59031420/tu106a1.log", "TU106", None),
    (DC, "tu106a1", "windows", f"{DC}/turing/tu106_box59031420/tu106a1_windows.log", "TU106", None),
    (DC, "tu116b1", "linux", f"{DC}/turing/tu116_box59031423/tu116b1.log", "TU116", None),
    (DC, "tu116b1", "windows", f"{DC}/turing/tu116_box59031423/tu116b1_windows.log", "TU116", None),
    (DC, "tu116b2", "windows", f"{DC}/turing/tu116_box59031423/tu116b2_windows.log", "TU116", None),
    # Ampere
    (DC, "dca104a", "linux", f"{DC}/ampere/ga104/dca104a.log", "GA104", None),
    (DC, "dca104a", "windows", f"{DC}/ampere/ga104/dca104a_windows.log", "GA104", None),
    (DC, "dca106a", "linux", f"{DC}/ampere/ga106/dca106a.log", "GA106", None),
    (DC, "dca106a", "windows", f"{DC}/ampere/ga106/dca106a_windows.log", "GA106", NOTE_GA106),
    (DC, "dca106b", "windows", f"{DC}/ampere/ga106/dca106b_windows.log", "GA106", None),
    (WIN, "run41936606326", "linux", f"{WIN}/run41936606326_broker.log", "GA102", None),
    (WIN, "run41936606326", "windows", f"{WIN}/run41936606326_broker.log", "GA102", None),
    # Ada
    (DC, "ada102a", "linux", f"{DC}/ada/ad102/ada102a.log", "AD102", None),
    (DC, "ada102a", "windows", f"{DC}/ada/ad102/ada102a_windows.log", "AD102", None),
    (DC, "ada102b", "windows", f"{DC}/ada/ad102/ada102b_windows.log", "AD102", None),
    (DC, "ada102p", "windows", f"{DC}/ada/ad102/ada102p_windows.log", "AD102", NOTE_BRANCH),
    (DC, "ada102q", "windows", f"{DC}/ada/ad102/ada102q_windows.log", "AD102", NOTE_BRANCH),
    (DC, "ada102r", "windows", f"{DC}/ada/ad102/ada102r_windows.log", "AD102", NOTE_BRANCH),
    (DC, "ada102g", "linux", f"{DC}/ada/followup_54ab3856/ad102/ada102g.log", "AD102", NOTE_BRANCH),
    (DC, "ada102g", "windows", f"{DC}/ada/followup_54ab3856/ad102/ada102g_windows.log", "AD102", NOTE_BRANCH),
    (DC, "ada103a", "linux", f"{DC}/ada/ad103/ada103a.log", "AD103", None),
    (DC, "ada104a", "linux", f"{DC}/ada/ad104/ada104a.log", "AD104", None),
    (DC, "ada104w", "windows", f"{DC}/ada/ad104/ada104w_windows.log", "AD104", None),
    (DC, "ada106a", "linux", f"{DC}/ada/ad106/ada106a.log", "AD106", None),
    # Blackwell
    (DC, "dcgb202", "linux", f"{DC}/blackwell/gb202/merge/dcgb202.log", "GB202", None),
    (DC, "dcgb202", "windows", f"{DC}/blackwell/gb202/windows_dcgb202/dcgb202_windows.log", "GB202", NOTE_GB202),
    (DC, "dcgb202r2", "windows", f"{DC}/blackwell/gb202/windows_dcgb202r2/dcgb202r2_windows.log", "GB202", None),
    (DC, "dcgb202r3", "windows", f"{DC}/blackwell/gb202/windows_dcgb202r3/dcgb202r3_windows.log", "GB202", None),
    (DC, "dcgb205", "linux", f"{DC}/blackwell/gb205/merge/dcgb205.log", "GB205", None),
    (DC, "dcgb205", "windows", f"{DC}/blackwell/gb205/windows_dcgb205/dcgb205_windows.log", "GB205", None),
    (DC, "dcgb205r2", "windows", f"{DC}/blackwell/gb205/windows_dcgb205r2/dcgb205r2_windows.log", "GB205", None),
    (DC, "dcgb206", "linux", f"{DC}/blackwell/gb206/merge/dcgb206.log", "GB206", None),
    (DC, "dcgb206", "windows", f"{DC}/blackwell/gb206/windows_dcgb206/dcgb206_windows.log", "GB206", None),
    (DC, "dcgb206r2", "windows", f"{DC}/blackwell/gb206/windows_dcgb206r2/dcgb206r2_windows.log", "GB206", None),
]

# Families, dies and the board each die was run on (the README that names it is checked by a test).
# `readme` is the README under traces/ that states the die's SKU and, for an uncovered die, why.
FAMILIES = [
    ("turing", "Turing", [
        ("TU102", None), ("TU104", None), ("TU106", "RTX 2060 SUPER"), ("TU116", "GTX 1660 Ti"),
        ("TU117", None)]),
    ("ampere", "Ampere", [
        ("GA102", "RTX 3090"), ("GA103", None), ("GA104", "RTX 3060 Ti"), ("GA106", "RTX 3060"),
        ("GA107", None)]),
    ("ada", "Ada Lovelace", [
        ("AD102", "RTX 4090"), ("AD103", "RTX 4080 SUPER"), ("AD104", "RTX 4070 SUPER"),
        ("AD106", "RTX 4060 Ti"), ("AD107", None)]),
    ("blackwell", "Blackwell", [
        ("GB202", "RTX 5090"), ("GB203", None), ("GB205", "RTX 5070"), ("GB206", "RTX 5060 Ti"),
        ("GB207", None)]),
]
FAMILY_README = {
    "turing": f"{DC}/turing/README.md",
    "ampere": f"{DC}/ampere/README.md",
    "ada": f"{DC}/ada/README.md",
    "blackwell": f"{DC}/blackwell/README.md",
}
DIE_README = {"GA102": f"{WIN}/README.md"}

# Why a platform of a die has no run, as the README states it (a short quotation; a test checks that
# the quoted `anchor` is in the README, so a reworded README fails the build instead of the page).
OFFER_TURING = ("No Vast VM offer during the lane; offer scans of 2026-11-04 found none "
                "(turing/offer_scans.txt).")
UNCOVERED = {
    "TU102": (OFFER_TURING, "TU102 (RTX 2080 Ti, Titan RTX"),
    "TU104": (OFFER_TURING, "never a TU104 (RTX 2070 SUPER"),
    "TU117": (OFFER_TURING, "TU117 (GTX 1650)"),
    "GA103": ("No KVM offer with a GA103-only board in the searches of 2026-11-04 (00:05Z, 02:08Z).",
              "GA103/GA107"),
    "GA107": ("No KVM offer with a GA107-only board in the searches of 2026-11-04 (00:05Z, 02:08Z).",
              "GA103/GA107"),
    "AD107": ("No vms_enabled offer at 2026-11-04T00:11Z.", "no `vms_enabled` offer"),
    "GB207": ("No offer (zero RTX 5050 offers, with or without vms_enabled, 2026-11-04 00:07Z and "
              "01:52Z).", "zero RTX 5050 offers"),
    "GB203": ("Not repeated at the integration revision; the README says it passed on an earlier "
              "tree (traces/blackwell_win_20261103/).", "not repeated (passed on the earlier tree"),
}
LINUX_ONLY_DIES = {
    "AD103": ("Windows gate not run in that lane (a Linux-only die there).", "not run (Linux-only die)"),
    "AD106": ("Windows gate not run in that lane (a Linux-only die there).", "not run (Linux-only die)"),
}
ALSO = {
    ("TU106", "windows"): ("The README also lists earlier Windows gate PASS runs at other revisions "
                           "(2026-11-03); they are not imported here.", f"traces/{DC}/turing"),
    ("GA102", "windows"): ("Earlier Windows evidence at another revision is named in the Ampere "
                           "README; it is not imported here.", "traces/ampere_hw_20261103"),
}
GB203_SEE = "traces/blackwell_win_20261103"


class ParseError(Exception):
    pass


def read(rel):
    return (TRACES / rel).read_text(errors="replace")


def sha256(rel):
    return hashlib.sha256((TRACES / rel).read_bytes()).hexdigest()


# --- log parsers -------------------------------------------------------------------------------

WINDOWS_LINE = re.compile(r"(?<![A-Z_])WINDOWS_GATE_(PASS|FAIL|RETRY) (.*)")


def parse_windows(text):
    """The one Windows gate verdict line of a log. Identical repeats are one line; two different
    verdict lines are ambiguous and give UNKNOWN."""
    found = {m.group(0) for m in WINDOWS_LINE.finditer(text)}
    if len(found) != 1:
        return {"verdict": "UNKNOWN"}
    m = WINDOWS_LINE.search(next(iter(found)))
    rest = m.group(2)
    rev = re.search(r"\brev=([0-9a-f]{8})\b", rest)
    gpu = re.search(r'\bgpu="([^"]{1,60})"', rest)
    enable = re.search(r"\benable_utc=(\d{4}-\d\d-\d\d)T", rest)
    stage = re.search(r"\bstage=([a-z_0-9]+)", rest)
    if not rev:
        return {"verdict": "UNKNOWN"}
    out = {
        "verdict": m.group(1),
        "rev": rev.group(1),
        "gpu": gpu.group(1) if gpu else None,
        "date": enable.group(1) if enable else None,
        "stage": stage.group(1) if stage else None,
    }
    resets = re.search(r"\bresets=(\d+)", rest)
    out["resets"] = int(resets.group(1)) if resets else None
    return out


def counter(text, prefix, names):
    m = re.search(r"^" + prefix + r"=(\d+)" + "".join(rf" {n}=(\d+)" for n in names), text, re.M)
    return [int(g) for g in m.groups()] if m else None


def parse_linux_bar(text):
    """The Linux bar of a merge check: v3 gates, bare and thin-guest suites, birth census."""
    head = re.search(r"^HEAD=([0-9a-f]{40})\b", text, re.M)
    gates = re.search(r"^V3_GATES_SUMMARY pass=(\d+) fail=(\d+) gate10=(\w+)", text, re.M)
    rc = re.search(r"^KF3_RC=(\d+)", text, re.M)
    bare = re.search(r"^BARE_SUITE_PASS=(\d+) BARE_SUITE_FAIL=(\d+) BARE_SUITE_CRASH=(\d+) ARMS=(\d+)", text, re.M)
    fast = re.search(
        r"^FAST_SUITE_PASS=(\d+) FAST_SUITE_FAIL=(\d+) FAST_SUITE_CRASH=(\d+) NOTRUN=(\d+) ARMS=(\d+)",
        text, re.M)
    census = re.search(r"^BIRTH_CENSUS_OK arms=(\d+)", text, re.M)
    if not (head and gates and rc and bare and fast):
        return {"verdict": "UNKNOWN"}
    g = [int(x) for x in gates.groups()[:2]]
    b = [int(x) for x in bare.groups()]
    f = [int(x) for x in fast.groups()]
    ok = (
        g[1] == 0 and g[0] == 9 and int(rc.group(1)) == 0
        and b[0] == b[3] and b[1] == 0 and b[2] == 0
        and f[0] == f[4] and f[1] == 0 and f[2] == 0 and f[3] == 0
        and census is not None
    )
    failed = g[1] > 0 or b[1] > 0 or b[2] > 0 or f[1] > 0 or f[2] > 0 or f[3] > 0
    detail = "v3 gates {}/{}, bare suite {}/{}, thin guest {}/{}, birth census {}".format(
        g[0], g[0] + g[1], b[0], b[3], f[0], f[4], "OK" if census else "missing")
    return {
        "verdict": "PASS" if ok else "FAIL" if failed else "UNKNOWN",
        "rev": head.group(1)[:8],
        "rev_full": head.group(1),
        "detail": detail,
    }


def parse_worker_log(text, platform):
    """The Worker job log of a gpu-vast run: stage rows, suite rows, and the Windows gate line."""
    stages = {m.group(1): (int(m.group(2)), m.group(3))
              for m in re.finditer(r"stage name=([a-z_0-9]+) rc=(-?\d+) duration_ms=\d+ result=(\w+)", text)}
    suites = {m.group(1): (int(m.group(2)), int(m.group(3)))
              for m in re.finditer(r"suite name=([a-z_0-9]+) pass=(\d+) fail=(\d+) skip=\d+", text)}
    final = re.search(r"KF_FINAL_SUMMARY_BEGIN verdict=(\w+) stages=(\d+)", text)
    if platform == "windows":
        out = parse_windows(text)
        want = stages.get("windows_gate")
        if out["verdict"] == "PASS" and not (want and want[1] == "PASS" and final
                                             and final.group(1) == "PASS"):
            out["verdict"] = "UNKNOWN"
        return out
    linux = ["provision", "cuda_host", "merge_bar", "cuda_guest"]
    rows = ["gates", "raw_suite", "thin_suite"]
    if not (all(s in stages for s in linux) and all(r in suites for r in rows) and final):
        return {"verdict": "UNKNOWN"}
    win = parse_windows(text)
    ok = (all(stages[s][1] == "PASS" for s in linux) and all(suites[r][1] == 0 for r in rows)
          and final.group(1) == "PASS")
    failed = any(stages[s][1] == "FAIL" for s in linux) or any(suites[r][1] for r in rows)
    return {
        "verdict": "PASS" if ok else "FAIL" if failed else "UNKNOWN",
        "rev": win.get("rev"),
        "detail": "Linux stages provision, cuda_host, merge_bar, cuda_guest PASS; suite rows "
                  "gates {}, raw {}, thin guest {} with fail=0".format(
                      *[suites[r][0] for r in rows]),
    }


def worker_facts(rel):
    """The runner's own verdict fields, from the job log next to the broker log (GitHub's status job)."""
    status = (TRACES / rel).parent / "run41936606326_gate_status.log"
    text = status.read_text(errors="replace") if status.exists() else ""
    get = lambda key: (re.search(rf"^\s*{key}: (\S+)", text, re.M) or [None, None])[1]
    return {"verdict": get("BROKER_VERDICT"), "verified": get("BROKER_VERIFIED"), "state": get("BROKER_STATE")}


# --- assembly ----------------------------------------------------------------------------------

def run_record(spec):
    set_id, tag, platform, rel, die, note = spec
    info = SETS[set_id]
    text = read(rel)
    if set_id == WIN:
        parsed = parse_worker_log(text, platform)
        facts = worker_facts(rel)
        if facts["verdict"] != "PASS" or facts["verified"] != "1":
            parsed["verdict"] = "UNKNOWN"
    elif platform == "windows":
        parsed = parse_windows(text)
    else:
        parsed = parse_linux_bar(text)
    verdict = parsed["verdict"]
    if verdict in ("PASS", "FAIL", "RETRY") and parsed.get("rev") is None:
        verdict = "UNKNOWN"
    return {
        "die": die, "platform": platform, "tag": tag, "set": set_id, "source": info["source"],
        "verdict": verdict, "rev": parsed.get("rev"), "rev_full": parsed.get("rev_full"),
        "date": parsed.get("date") or info["date"], "detail": parsed.get("detail"),
        "stage": parsed.get("stage"), "gpu": parsed.get("gpu"), "note": note, "file": rel,
    }


def groups_for(records):
    """Group runs of one cell by (source, revision); the set's headline revision comes first."""
    groups = {}
    for r in records:
        key = (r["source"], r["rev"])
        g = groups.setdefault(key, {
            "source": r["source"], "revision": r["rev"], "revision_full": r["rev_full"],
            "date": r["date"], "n": 0, "pass": 0, "fail": 0, "retry": 0, "unknown": 0,
            "tags": [], "detail": r["detail"], "notes": [],
            "evidence": str(Path("traces") / SETS[r["set"]]["dir"]),
            "headline": r["rev"] == SETS[r["set"]]["headline_rev"],
        })
        g["n"] += 1
        bucket = {"PASS": "pass", "FAIL": "fail", "RETRY": "retry"}.get(r["verdict"], "unknown")
        g[bucket] += 1
        g["tags"].append(r["tag"])
        g["revision_full"] = g["revision_full"] or r["rev_full"]
        if r["date"] != g["date"]:
            g["date"] = max(g["date"], r["date"])
        if r["note"] and r["note"] not in g["notes"]:
            g["notes"].append(r["note"])
        if r["verdict"] == "FAIL" and r["stage"]:
            g["notes"].append("FAIL at stage " + r["stage"] + ".")
        if r["set"] == WIN:
            g["run"] = {"label": "gpu-vast run 41936606326", "path": "actions/runs/41936606326"}
            g["profile"] = "check"
            g["notes"] = ["Profile check (master PR gate). Read from the committed job logs, not "
                          "from the Worker's result store."]
    out = sorted(groups.values(), key=lambda g: (not g["headline"], g["date"], g["revision"] or ""))
    for g in out:
        g["notes"] = list(dict.fromkeys(g["notes"]))
    return out


def uncovered_cell(reason, see=None):
    cell = {"state": "not_covered", "reason": reason, "groups": []}
    if see:
        cell["see"] = see
    return cell


def build():
    by_cell = {}
    inputs = {}
    for spec in RUNS:
        rec = run_record(spec)
        by_cell.setdefault((rec["die"], rec["platform"]), []).append(rec)
        inputs[spec[3]] = sha256(spec[3])
    families = []
    for fid, fname, dies in FAMILIES:
        readme = FAMILY_README[fid]
        inputs[readme] = sha256(readme)
        out_dies = []
        for die, sku in dies:
            cells = {}
            for platform in ("linux", "windows"):
                recs = by_cell.get((die, platform))
                if recs:
                    cell = {"state": "covered", "groups": groups_for(recs)}
                elif die in UNCOVERED:
                    cell = uncovered_cell(UNCOVERED[die][0], GB203_SEE if die == "GB203" else None)
                elif die in LINUX_ONLY_DIES and platform == "windows":
                    cell = uncovered_cell(LINUX_ONLY_DIES[die][0])
                else:
                    cell = uncovered_cell("No run in the imported evidence.")
                if (die, platform) in ALSO:
                    cell["also"], cell["also_see"] = ALSO[(die, platform)]
                cells[platform] = cell
            entry = {"id": die, "board": sku, "cells": cells}
            if die in DIE_README:
                inputs[DIE_README[die]] = sha256(DIE_README[die])
            out_dies.append(entry)
        families.append({"id": fid, "name": fname, "dies": out_dies})
    return {
        "schema": SCHEMA,
        "note": "Generated by scripts/site/coverage_from_traces.py from committed traces; do not edit.",
        "sources": SOURCE_LABELS,
        "gate_rule": "Only runs reproduced by the CI Worker count for the release gate, and only "
                     "through its result store. Imported evidence never does.",
        "families": families,
        "inputs": dict(sorted(inputs.items())),
    }


def warnings():
    return [f"traces/{p} exists and is NOT imported (placeholder; read its README for numbers first)"
            for p in PENDING_SETS if (TRACES / p).exists()]


def serialize(data):
    return json.dumps(data, indent=1, sort_keys=False) + "\n"


def main(argv):
    text = serialize(build())
    for line in warnings():
        print("warning: " + line, file=sys.stderr)
    if "--write" in argv:
        OUTPUT.write_text(text)
    elif "--check" in argv:
        if not OUTPUT.exists() or OUTPUT.read_text() != text:
            print("site/status/coverage.json is stale: run scripts/site/coverage_from_traces.py --write",
                  file=sys.stderr)
            return 1
    else:
        sys.stdout.write(text)
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
