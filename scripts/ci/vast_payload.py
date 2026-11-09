#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later
"""Thin adapter for the existing disposable-box provision and merge bar.
No provider/CI credentials or network clone machinery belongs here.
"""

import base64
import collections, json, os, re, select, shutil, signal, subprocess, sys, time
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))
import vast_report as local_result_contract

# The current payload has no matrix driver/subrun executor. The trusted Worker
# checks this exact declaration at the selected commit before creating a lease.
PAYLOAD_RELEASE_MATRIX_EXECUTOR = 0

sys.path.insert(0, str(Path(__file__).resolve().parents[1] / "bench"))
import native_xid_guard as xid_guard

STOP = False


class EvidenceError(Exception):
    def __init__(self, name, rows=()):
        self.name = name
        self.rows = list(rows)
        super().__init__(name)


class ProviderFault(EvidenceError):
    """A rented host is unusable before kayfabe tests; never a product FAIL."""


def require_provider_kvm(path):
    if not path.exists():
        raise ProviderFault("provider_kvm_missing")


def provider_fault(text):
    not_ready = re.search(r"^NOT_READY\b", text, re.M)
    failed_driver = re.search(r"^DRIVER_RC=[1-9][0-9]*$", text, re.M)
    unusable = re.search(r"GPU unusable|No devices were found", text, re.I)
    initialization = re.search(r"RmInitAdapter failed|rm_init_adapter failed", text)
    if not_ready and (unusable or (failed_driver and initialization)):
        return "provider_gpu_unavailable"
    return None


def row(name, passed=0, failed=0, skipped=0):
    return {"name": name, "pass": passed, "fail": failed, "skip": skipped}


def only(pattern, text, name, rows=()):
    found = re.findall(pattern, text, re.M)
    if len(found) != 1:
        raise EvidenceError(name, rows)
    return found[0]


def provision_rows(text, rc):
    if re.search(r"^DRIVER_MEDIA_DRIFT reason=[A-Z0-9_]{1,32}$", text, re.M):
        raise EvidenceError("driver_media_drift")
    if (
        rc
        or red_line(text)
        or any(
            len(re.findall("^" + key + "_RC=0$", text, re.M)) != 1
            for key in ["BOX", "DRIVER", "TREE", "LADDER", "FG", "KF3"]
        )
        or re.search(r"^NOT_READY\b", text, re.M)
    ):
        raise EvidenceError("provision")
    only(r"^READY driver=(\d{3}\.\d{1,3}(?:\.\d{1,2})?)$", text, "provision")
    only(r"^EXIT [^\n]+$", text, "provision")
    return [row("provision", 1)]


def merge_rows(text, rc, sha):
    rows = []
    if only(r"^HEAD=([0-9a-f]{40}) CHECKOUT=[^\n]+$", text, "infrastructure") != sha:
        raise EvidenceError("infrastructure")
    # [2026-10-26] The bar no longer runs the GPU-free crate tests (CI's `stable` job does, on every
    # commit). The legacy totals line stays red; the opt-in with-KVM stage (kvm_tests.sh) is graded
    # here, and a request without exactly one passing result is a failure, never a silent skip.
    if re.search(r"^TESTS passed [0-9]+ failed [1-9]", text, re.M):
        raise EvidenceError("kf_tests")
    requested = len(re.findall(r"^KVM_TESTS_REQUESTED=1$", text, re.M))
    results = re.findall(
        r"^KVM_TESTS passed ([0-9]+) failed ([0-9]+) kvm_ran=([0-9]+) kvm_skipped=([0-9]+)$",
        text,
        re.M,
    )
    if re.search(r"^KVM_TESTS_RC=[1-9]", text, re.M):
        raise EvidenceError("kvm_tests", rows)
    if requested or results:
        if requested != 1 or len(results) != 1:
            raise EvidenceError("kvm_tests", rows)
        passed, failed, ran, skipped = map(int, results[0])
        if passed < 1 or failed or ran < 1 or skipped:
            raise EvidenceError("kvm_tests", rows)
        rows.append(row("kvm_tests", passed))
    only(r"^GATES_RC=0$", text, "gates", rows)
    only(r"^V3_GATES_SUMMARY ([^\n]+)$", text, "gates", rows)
    only(r"^V3_GATES_SUMMARY pass=9 fail=0 gate10=(PASS|FALLBACK)$", text, "gates", rows)
    rows.append(row("gates", 9))
    only(r"^KF3_RC=0$", text, "kf3_build", rows)
    rows.append(row("kf3_build", 1))
    only(
        r"^BARE_SUITE_PASS=30 BARE_SUITE_FAIL=0 BARE_SUITE_CRASH=0 ARMS=30$",
        text,
        "raw_suite",
        rows,
    )
    rows.append(row("raw_suite", 30))
    only(r"^FG_RC=0$", text, "thin_suite", rows)
    only(r"^SUITE_RC=0$", text, "thin_suite", rows)
    only(
        r"^FAST_SUITE_PASS=30 FAST_SUITE_FAIL=0 FAST_SUITE_CRASH=0 NOTRUN=0 ARMS=30$",
        text,
        "thin_suite",
        rows,
    )
    rows.append(row("thin_suite", 30))
    only(r"^BIRTH_CENSUS_OK arms=30(?: .*)?$", text, "birth_census", rows)
    rows.append(row("birth_census", 1))
    if rc or red_line(text) or len(re.findall(r"^EXIT rc=0 .+$", text, re.M)) != 1:
        raise EvidenceError("merge_bar", rows)
    return rows


WINDOWS_VERDICT = re.compile(r"^WINDOWS_GATE_(PASS|FAIL|RETRY|REFUSED)\b([^\n]*)$", re.M)
WINDOWS_RETRY_CLASSES = {"PROVIDER", "MEDIA", "OPERATOR", "HOSTREF", "BAR1_EXCEPTION"}


def windows_rows(text, rc, sha, name="windows_gate"):
    """Grade scripts/bench/box/windows_gate.sh. `name` is the row of this run: windows_gate, or
    windows_gate_2 for the release tier's second run on the same box (KF_WINDOWS_RUNS=2).
    THE GATE IS THE GUEST PASSING (owner, 2026-11-01):
    only one WINDOWS_GATE_PASS for this revision with exit 0 is a row that passes. RETRY (and a
    refused precondition) is a host, harness or rental fault: never a pass, never a verdict on the
    guest; its row is named windows_retry_<class> so the runner can say "re-run on a fresh box"."""
    verdicts = WINDOWS_VERDICT.findall(text)
    exits = re.findall(r"^WINDOWS_GATE_EXIT rc=([0-9]+)\b", text, re.M)
    kinds = {kind for kind, _ in verdicts}
    if len(verdicts) == 1 and len(exits) == 1:
        kind, rest = verdicts[0]
        if (
            kind == "PASS"
            and rc == 0
            and exits[0] == "0"
            and re.match(r" rev=" + re.escape(sha[:8]) + r"[0-9a-f]* ", rest)
        ):
            return [row(name, 1)]
        if kind == "FAIL" and rc not in (0, 3):
            raise EvidenceError(name)
    if kinds == {"RETRY"} and len(verdicts) == 1:
        found = re.search(r" class=([A-Z0-9_]+)\b", verdicts[0][1])
        cls = found[1] if found and found[1] in WINDOWS_RETRY_CLASSES else "OPERATOR"
        raise EvidenceError("windows_retry_" + cls.lower())
    if kinds == {"REFUSED"}:
        raise EvidenceError("windows_retry_refused")
    if "FAIL" in kinds:
        raise EvidenceError(name)
    # No verdict, two verdicts, a PASS of another revision or with a non-zero exit: the
    # harness did not end cleanly. Not a pass; a fresh box decides.
    raise EvidenceError("windows_retry_operator")


def windows_requested(environ):
    """The Worker states the tier as KF_MERGE_WINDOWS (ci/vast-broker/src/tier.js): 1 for the master
    gate (profile check), 0 for the die sweep. A box started by a Worker that predates the field
    gets the master gate: a missing statement never makes the required check weaker."""
    value = environ.pop("KF_MERGE_WINDOWS", None)
    if value not in (None, "0", "1"):
        raise EvidenceError("preflight")
    return value != "0", "worker" if value is not None else "default"


RELEASE_DRIVER_FIELDS = ("KF_HOST_DRIVER", "KF_LINUX_GUEST_DRIVER", "KF_WINDOWS_GUEST_DRIVER")


def release_driver_environment(environ):
    """Map per-leg driver selections, keeping the Windows media and its guest ABI distinct.

    HOST_DRIVER is the existing host installer's selector; KF_HOST_DRIVER is the readiness
    check's selector. Linux guest staging and new Windows media still need an executor.
    """
    names = (*RELEASE_DRIVER_FIELDS, "KF_WINGATE_GUEST_DRIVER", "KF_RELEASE_SUBRUNS", "KF_RELEASE_DRIVER_PINS")
    if not any(name in environ for name in names):
        return {}
    if any(name not in environ for name in (*RELEASE_DRIVER_FIELDS, "KF_RELEASE_SUBRUNS", "KF_RELEASE_DRIVER_PINS")):
        raise EvidenceError("release_retry_matrix_invalid")
    selected = {name: environ[name] for name in names if name in environ}
    for name, value in selected.items():
        if name not in ("KF_RELEASE_SUBRUNS", "KF_RELEASE_DRIVER_PINS") and not re.fullmatch(r"[0-9]{3}\.[0-9]{2,3}(?:\.[0-9]{2})?", value):
            raise EvidenceError("release_retry_matrix_invalid")
    # Same bounded JSON contract as the bootstrap, also checked at the payload boundary.
    try:
        raw = selected["KF_RELEASE_SUBRUNS"]
        plan = json.loads(base64.b64decode(raw + "=" * (-len(raw) % 4), altchars=b"-_", validate=True))
        valid = (re.fullmatch(r"[A-Za-z0-9_-]{1,16384}", raw) is not None and isinstance(plan, dict)
                 and set(plan) == {"status", "subruns", "cross_subrun"}
                 and plan["status"] == "proposed" and isinstance(plan["subruns"], list)
                 and len(plan["subruns"]) <= 32
                 and (plan["cross_subrun"] is None or isinstance(plan["cross_subrun"], dict)))
    except (ValueError, TypeError):
        valid = False
    if not valid:
        raise EvidenceError("release_retry_matrix_invalid")
    pins = local_result_contract.decode_driver_pins(selected["KF_RELEASE_DRIVER_PINS"])
    if pins is None:
        raise EvidenceError("release_retry_matrix_invalid")
    required = {("windows" if name == "KF_WINDOWS_GUEST_DRIVER" else "linux", selected[name]) for name in RELEASE_DRIVER_FIELDS}
    try:
        for run in plan["subruns"]:
            if not isinstance(run, dict) or run.get("os") not in ("linux", "windows") or not isinstance(run.get("drivers"), list):
                raise ValueError()
            for version in run["drivers"]:
                if not isinstance(version, str) or not re.fullmatch(r"[0-9]{3}\.[0-9]{2,3}(?:\.[0-9]{2})?", version):
                    raise ValueError()
                required.add((run["os"], version))
        cross = plan["cross_subrun"]
        if cross is not None:
            if not isinstance(cross, dict) or not isinstance(cross.get("drivers"), dict) or set(cross["drivers"]) != {"host_driver", "linux_guest_driver", "windows_guest_driver"}:
                raise ValueError()
            for axis, version in cross["drivers"].items():
                if not isinstance(version, str) or not re.fullmatch(r"[0-9]{3}\.[0-9]{2,3}(?:\.[0-9]{2})?", version):
                    raise ValueError()
                required.add(("windows" if axis == "windows_guest_driver" else "linux", version))
    except (ValueError, TypeError):
        raise EvidenceError("release_retry_matrix_invalid") from None
    if not required.issubset({(p["os"], p["version"]) for p in pins}):
        raise EvidenceError("release_retry_matrix_invalid")
    selected["HOST_DRIVER"] = selected["KF_HOST_DRIVER"]
    return selected


def require_executable_release(environ):
    """Never turn a proposed matrix into a successful run of today's pinned payload.

    Removal needs an approved matrix, guest staging, profile dispatch, per-subrun receipts
    and result grading. No version fallback is permitted in the meantime.
    """
    selected = release_driver_environment(environ)
    if selected:
        raise EvidenceError("release_retry_matrix_proposed")

WINAPPS_PROFILES = ("small", "mid", "full")
WINAPPS_VERDICT = re.compile(r"^WINAPPS_(PASS|FAIL|RETRY|REFUSED)\b([^\n]*)$", re.M)
# the classes windows_apps.sh may print on a RETRY; anything else is OPERATOR
WINAPPS_RETRY_CLASSES = {"PROVIDER", "MEDIA", "OPERATOR", "HOSTREF", "BAR1_EXCEPTION"}
# Per-profile wall the stage needs after the gate, including 10 min mid cleanup margin (INFERRED from the apps' own timeouts and the guest restart
# budget, docs/design/V3_WINDOWS_APPS_GATE.md section 6; replaced by measurements when a box has run the profile).
WINAPPS_MIN_REMAINING_MS = {"small": 40 * 60000, "mid": 70 * 60000, "full": 200 * 60000}


def winapps_requested(environ, windows):
    """KF_WINAPPS (small, mid, full) asks for the Windows apps stage after the Windows gate. Opt-in: absent means
    no stage, so a Worker that predates it (or has not been told to send it) changes nothing. It needs the
    Windows stage; anything else is refused before any stage runs."""
    value = environ.pop("KF_WINAPPS", None)
    if value is None:
        return None
    if value not in WINAPPS_PROFILES or not windows:
        raise EvidenceError("preflight")
    return value


def winapps_rows(text, rc, sha, profile):
    """Grade scripts/bench/box/windows_apps.sh. THE GATE IS THE GUEST PASSING (owner, 2026-11-01): only one
    WINAPPS_PASS for this revision and this profile with exit 0 is a passing row. RETRY (a host, harness, input or
    rental fault) and a refused precondition are never a pass and never a verdict on the guest; their row is
    named windows_apps_retry_<class> so the runner can say "re-run on a fresh box"."""
    verdicts = WINAPPS_VERDICT.findall(text)
    exits = re.findall(r"^WINAPPS_EXIT rc=([0-9]+)\b", text, re.M)
    kinds = {kind for kind, _ in verdicts}
    if len(verdicts) == 1 and len(exits) == 1:
        kind, rest = verdicts[0]
        if (
            kind == "PASS"
            and rc == 0
            and exits[0] == "0"
            and re.match(r" rev=" + re.escape(sha[:8]) + r"[0-9a-f]* ", rest)
            and (" profile=" + profile + " ") in rest
        ):
            # Like Linux apps: count actual checked apps, not a stage marker. A truncated log,
            # empty expectation, duplicate app or harness row must not become a successful suite.
            grade = re.findall(
                r"^WINAPPS_GRADE kind=PASS expected=([1-9][0-9]{0,3}) pass=([1-9][0-9]{0,3})"
                r" guest_fail=0 boot_fail=0 harness=0 not_run=0 extra=0$", text, re.M
            )
            app_lines = re.findall(r"^WINAPPS_APP .*$", text, re.M)
            passed = re.findall(
                r"^WINAPPS_APP app=([A-Za-z0-9_]{1,48}) class=pass verdict=PASS\*? "
                r"batched=\S+ alone=\S+ secs=\S+ tdr=\S+ wer=\S+ gsp=\S+ note=.*$", text, re.M
            )
            count = re.search(r" apps=([1-9][0-9]{0,3})\b", rest)
            if (len(grade) == 1 and count and grade[0][0] == grade[0][1] == count[1]
                    and len(app_lines) == len(passed) == len(set(passed)) == int(count[1])):
                return [row("windows_apps", int(count[1]))]
            raise EvidenceError("windows_apps_retry_operator")
        if kind == "FAIL" and rc not in (0, 3):
            raise EvidenceError("windows_apps")
    if kinds == {"RETRY"} and len(verdicts) == 1:
        found = re.search(r" class=([A-Z0-9_]+)\b", verdicts[0][1])
        cls = found[1] if found and found[1] in WINAPPS_RETRY_CLASSES else "OPERATOR"
        raise EvidenceError("windows_apps_retry_" + cls.lower())
    if kinds == {"REFUSED"}:
        raise EvidenceError("windows_apps_retry_refused")
    if "FAIL" in kinds:
        raise EvidenceError("windows_apps")
    # No verdict, two verdicts, a PASS of another revision or profile, or a PASS with a non-zero exit: the
    # harness did not end cleanly. Not a pass; a fresh box decides.
    raise EvidenceError("windows_apps_retry_operator")


def tier_requested(environ, windows):
    """The release tier's extras (ci/vast-broker/src/tier.js, 2026-11-05): KF_TIER is informational;
    KF_WINDOWS_RUNS=2 runs the Windows gate a second time on the same box (only with the Windows
    stage); KF_STAGES names extra stages. Only `specviewperf` is a stage slot that can run (with
    KF_SPEC naming its lane, spec_requested); `two_guests` and `trio` are reserved names,
    DESIGN-ONLY and refused. Anything malformed is refused before any stage runs."""
    tier = environ.pop("KF_TIER", None)
    runs = environ.pop("KF_WINDOWS_RUNS", None)
    stages = environ.pop("KF_STAGES", None)
    if tier not in (None, "small", "middle", "large", "adhoc") or runs not in (None, "1", "2"):
        raise EvidenceError("preflight")
    if runs == "2" and not windows:
        raise EvidenceError("preflight")
    names = [] if stages is None else stages.split(":")
    if (
        len(names) != len(set(names))
        or any(n not in STAGE_SLOTS for n in names)
        or any(STAGE_SLOTS[n] == "DESIGN_ONLY" for n in names)
        or (names and tier != "large")
    ):
        raise EvidenceError("preflight")
    return tier or "unknown", 2 if runs == "2" else 1, names


# Stage slots (names reserved in ci/vast-broker/defaults.json stageSlots). PLUMBING = the stage
# runs through scripts/ci/vast_stage_specviewperf.sh; DESIGN_ONLY = refused if requested.
STAGE_SLOTS = {"specviewperf": "PLUMBING", "two_guests": "DESIGN_ONLY", "trio": "DESIGN_ONLY", "llm_parity": "DESIGN_ONLY"}
SPEC_VERDICT = re.compile(r"^SPECVIEWPERF_STAGE_(PASS|FAIL|RETRY)(?: class=([A-Z0-9_]{1,32}))?$", re.M)


# The SPECviewperf stage (docs/design/V3_SPEC_GATE.md, DRAFT 2026-11-07, DISABLED BY DEFAULT: the Worker sends
# KF_STAGES=specviewperf with KF_SPEC=lanea|laneb|both only when the owner has enabled `releaseStages` and a lane plan,
# and nothing sends them today). It runs after the Windows gate(s), in the Windows guest the gate passed, on the same
# box. Lane A (energy, creo, blender, enscape) and lane B (medical, maya, catia, solidworks, unreal_engine) are two
# boxes of a die; `both` is one box with all nine and needs a lease longer than the 300-minute box limit allows on most
# hosts (its time check below then fails closed). The lanes and every time constant are one file the lane script
# reads too: scripts/bench/specviewperf/lanes.json.
SPEC_LANES = ("lanea", "laneb", "both")
SPEC_TABLE = "scripts/bench/specviewperf/lanes.json"
SPEC_RETRY_CLASSES = {
    "NOT_BUILT", "MEDIA", "PROVIDER", "TRANSPORT", "TIME", "HOSTREF", "BAR1_EXCEPTION", "OPERATOR", "USAGE",
}
SPEC_ROW = re.compile(
    r"^SPEC_ROW name=([a-z][a-z_]*-[0-9]{2}) result=(PASS|FAIL|NOT_RUN) subtests=[0-9]{1,3}/[0-9]{1,3}"
    r" reason=(?:[A-Z_]{1,16}|-) failed=(?:[A-Za-z0-9_,]{1,120}|-)$",
    re.M,
)
SPEC_LIST = re.compile(r"^SPEC_STAGE_LIST lane=(lanea|laneb|both) n=([0-9]{1,2})$", re.M)
SPEC_EXIT = re.compile(r"^SPECVIEWPERF_STAGE_EXIT rc=([0-9]+)$", re.M)
# The Windows image is installed by the gate with a disk this size (a sparse file: it costs only what the guest
# writes); win_vm.sh's default (80 GB) cannot hold a lane's tree (44 to 50 GB) next to Windows. MEASURED: the
# 2026-11-05 run installed with 220 GB (traces/specviewperf_vast_20261105/README.md).
SPEC_DISK_GB = "220"
# The stage's own deadline is the soft deadline minus this margin (the verdict, the evidence and the deletion of the copy).
SPEC_DEADLINE_MARGIN_S = 120


def spec_requested(environ, windows, stages):
    """KF_SPEC (lanea, laneb, both) names the lane of the SPECviewperf stage. It goes with the stage slot, and the stage
    with it: a lane without the slot, the slot without a lane (it would silently run both) and the stage without the
    Windows gate (it runs in the gate's guest) are refused before any stage runs. Absent and no slot: today's run."""
    value = environ.pop("KF_SPEC", None)
    wanted = "specviewperf" in stages
    if wanted and not windows:
        raise EvidenceError("preflight")
    if value is None:
        if wanted:
            raise EvidenceError("preflight")
        return None
    if value not in SPEC_LANES or not wanted:
        raise EvidenceError("preflight")
    return value


def spec_table(root):
    """The lane table (lanes.json); a missing or malformed file is a preflight failure, before any stage."""
    try:
        table = json.loads(bounded_text(Path(root) / SPEC_TABLE, 65536))
        lanes = table["lanes"]
        assert sorted(lanes) == ["lanea", "laneb"] and all(isinstance(lanes[k], list) and lanes[k] for k in lanes)
        assert set(sum(lanes.values(), [])) == set(table["viewsets"]) and len(sum(lanes.values(), [])) == len(table["viewsets"])
        for key in ("setup_s", "factor", "tail_reserve_s"):
            assert table[key] > 0
        return table
    except (OSError, ValueError, KeyError, TypeError, AssertionError, EvidenceError):
        raise EvidenceError("preflight") from None


def spec_names(table, lane):
    """The viewsets of a lane in run order (`both`: lane A, then lane B)."""
    if lane == "both":
        return list(table["lanes"]["lanea"]) + list(table["lanes"]["laneb"])
    return list(table["lanes"][lane])


def spec_min_remaining_ms(table, lane):
    """The stage does not start when the whole lane cannot plausibly finish: setup, the native walls with a margin and the
    tail (INFERRED constants, lanes.json). A shortfall is `specviewperf_retry_time`, not a verdict on the guest."""
    native = sum(table["viewsets"][n]["native_s"] for n in spec_names(table, lane))
    return int(table["setup_s"] + table["factor"] * native + table["tail_reserve_s"]) * 1000


def specviewperf_rows(text, rc, lane, table):
    """Grade the SPECviewperf stage wrapper. THE GATE IS THE GUEST PASSING: one verdict PASS with exit 0, one exit line
    rc=0, the lane named in the list line, and exactly one PASS row for every viewset of the lane, no more, no fewer.
    Only status lines leave the box (the wrapper keeps the lane's own output, which it never reads for scores, in a
    private file). FAIL is the row `specviewperf` (a viewset failed in the guest, with the passed count beside it).
    RETRY (a missing lane, media, transport, time, a host or harness fault) is `specviewperf_retry_<class>`: never a
    pass. A PASS that is not backed by every row is the harness not ending cleanly: retry, class operator."""
    names = spec_names(table, lane)
    verdicts = SPEC_VERDICT.findall(text)
    if len(verdicts) != 1:
        raise EvidenceError("specviewperf_retry_operator")
    kind, cls = verdicts[0]
    exits = SPEC_EXIT.findall(text)
    rows = SPEC_ROW.findall(text)
    seen = [name for name, _ in rows]
    passed = sum(1 for _, result in rows if result == "PASS")
    partial = [row("specviewperf_passed", passed)] if passed else []
    if (
        kind == "PASS"
        and rc == 0
        and exits == ["0"]
        and SPEC_LIST.findall(text) == [(lane, str(len(names)))]
        and sorted(seen) == sorted(names)
        and passed == len(names)
    ):
        out = [row("specviewperf", len(names))]
        if lane == "both":
            out += [row("specviewperf_" + k, len(table["lanes"][k])) for k in ("lanea", "laneb")]
        else:
            out.append(row("specviewperf_" + lane, len(names)))
        return out
    if kind == "FAIL":
        raise EvidenceError("specviewperf", partial)
    if kind == "RETRY":
        name = "specviewperf_retry_" + (cls.lower() if cls in SPEC_RETRY_CLASSES else "operator")
        raise EvidenceError(name, partial)
    raise EvidenceError("specviewperf_retry_operator", partial)


# Linux apps: the trusted Worker sends KF_LINUX_APPS=1 only for signed middle runs.
# The old source flag remains a legacy owner opt-in for middle/large; its default stays off.
# KF_STAGES never selects this stage. Missing media and incomplete output are retries, not PASS.
APPS_STAGE_ENABLED = False
APPS_TIERS = ("middle", "large")
APPS_RETRY_CLASSES = {"MEDIA", "NOT_BUILT", "TIME", "BOX", "OPERATOR", "NO_LIST", "BAD_LIST", "USAGE"}
APPS_VERDICT = re.compile(r"^APPS_STAGE_(PASS|FAIL|RETRY)(?: class=([A-Z_]{1,32}))?$", re.M)
APPS_ROW = re.compile(
    r"^APP_ROW name=([A-Za-z0-9_]{1,48}) os=linux result=(PASS|FAIL|NOT_RUN)"
    r" secs=(?:[0-9]+|-) guest_xid=(?:[0-9]+|-) note=(.*)$",
    re.M,
)
APPS_LIST_LINE = re.compile(r"^APPS_STAGE_LIST n=([0-9]+) ", re.M)
# Its own wall-clock budget (apps_gate.sh KF_APPS_BUDGET_S) plus margin; INFERRED until a box has run
# the whole gate list under the payload (the measurement is in traces/apps_linux_20261107/).
APPS_MIN_REMAINING_MS = 60 * 60000


def apps_requested(tier):
    """The tier decides, and only when the owner has enabled the stage; never for small or adhoc."""
    return bool(APPS_STAGE_ENABLED) and tier in APPS_TIERS


def linuxapps_requested(environ, tier):
    """Worker-authored explicit middle opt-in; the old source flag remains a legacy owner control."""
    value = environ.pop("KF_LINUX_APPS", None)
    if value is None:
        return apps_requested(tier)
    if value != "1" or tier != "middle":
        raise EvidenceError("preflight")
    return True


# The release tier (`large`) runs the FULL gate list; `middle` runs the same list without the apps that need
# more than a minute in the guest (scripts/apps/gate_list_middle.txt), so it fits the middle tier's budget.
APPS_LISTS = {"large": ("gate_list.txt", "full"), "middle": ("gate_list_middle.txt", "middle")}


def apps_gate_list(root, tier="large"):
    """The gate list of the tier, in run order; one app name per line, `#` comments and blanks ignored."""
    names = [
        s.strip()
        for s in bounded_text(Path(root) / "scripts/apps" / APPS_LISTS[tier][0], 65536).splitlines()
        if s.strip() and not s.strip().startswith("#")
    ]
    if (
        not names
        or len(names) != len(set(names))
        or not all(re.fullmatch(r"[A-Za-z0-9_]{1,48}", n) for n in names)
    ):
        raise EvidenceError("apps_retry_no_list")
    return names


def apps_rows(text, rc, names):
    """Grade scripts/bench/box/apps_gate.sh. THE GATE IS THE GUEST PASSING: one APPS_STAGE_PASS with
    exit 0, one exit line rc=0, and exactly one PASS APP_ROW for every name of the gate list, no more,
    no fewer. A FAIL (a gate app failed in the guest) is the row `apps`, with the passed count beside
    it. RETRY (missing bundle, budget, boot) is `apps_retry_<class>`: not a verdict on the guest."""
    verdicts = APPS_VERDICT.findall(text)
    exits = re.findall(r"^APPS_STAGE_EXIT rc=([0-9]+)$", text, re.M)
    rows = APPS_ROW.findall(text)
    seen = [name for name, _, _ in rows]
    passed = sum(1 for _, result, _ in rows if result == "PASS")
    if len(verdicts) == 1 and len(exits) == 1:
        kind, cls = verdicts[0]
        listed = APPS_LIST_LINE.findall(text)
        if (
            kind == "PASS"
            and rc == 0
            and exits[0] == "0"
            and listed == [str(len(names))]
            and sorted(seen) == sorted(names)
            and passed == len(names)
        ):
            return [row("apps", len(names))]
        if kind == "FAIL" and rc not in (0, 3) and exits[0] != "0":
            raise EvidenceError("apps", [row("apps_passed", passed)] if passed else [])
        if kind == "RETRY":
            # Normalize the legacy Linux missing-bundle spelling to the shared MEDIA retry class.
            cls = "MEDIA" if cls == "NOT_BUILT" else cls
            name = "apps_retry_" + (cls.lower() if cls in APPS_RETRY_CLASSES else "operator")
            raise EvidenceError(name, [row("apps_passed", passed)] if passed else [])
    # No verdict, two verdicts, a PASS with a missing/extra/duplicate/failing app row or a non-zero exit
    # (names are unique, so sorted(seen) == sorted(names) also rules out a duplicate row):
    # the harness did not end cleanly. Not a pass; a fresh box decides.
    raise EvidenceError("apps_retry_operator")


# The Windows stage needs about an hour on a fresh box (install 19-31 min, gate 27 min, MEASURED
# traces/windows_gate_20261101/, traces/blackwell_hw_20261103/); it does not start with less than
# this left before the soft deadline, and that shortfall is a RETRY, not a verdict.
WINDOWS_MIN_REMAINING_MS = 75 * 60000
# The second Windows run reuses the installed image (install=once). MEASURED second runs on
# 2026-11-04 at rev 0c515b3b: 906 s (AD102 ada102b) and 1795 s (TU116 tu116b2),
# traces/die_coverage_20261104/; 45 minutes is that plus margin (INFERRED, not measured).
WINDOWS_REPEAT_MIN_REMAINING_MS = 45 * 60000


def ladder_rows(text, rc, mode, sha):
    name = "cuda_host" if mode == "host" else "cuda_fat"
    if mode not in {"host", "guest"} or rc:
        raise EvidenceError(name)
    only(
        r"^CUDA_LADDER_STARTED=.+ mode="
        + mode
        + r" reps=1 rungs=cup2,cup3,cup8,cup8bench rev="
        + re.escape(sha[:8])
        + r"$",
        text,
        name,
    )
    only(r"^CUDA_LADDER_DONE=.+$", text, name)
    lines = [s for s in text.splitlines() if s.startswith("CL_ROW ")]
    seen = set()
    if len(lines) != 4:
        raise EvidenceError(name)
    for line in lines:
        match = re.match(
            r"^CL_ROW mode="
            + mode
            + r" rung=(cup2|cup3|cup8|cup8bench) rep=1 verdict=PASS "
            + ("rc=0" if mode == "host" else "boot_rc=0")
            + r"(?: |$)",
            line,
        )
        if not match or match[1] in seen:
            raise EvidenceError(name)
        seen.add(match[1])
    return [row(name, 4)]


def red_line(line):
    return bool(
        re.search(
            r"^(?:BOX|DRIVER|TREE|LADDER|FG|KF3|GATES|SUITE|CARGO|KVM_TESTS)_RC=[1-9][0-9]*\b",
            line,
            re.M,
        )
        or re.search(r"^(?:BARE|FAST)_CELL_ARM\b.* verdict=(?!PASS\b)[A-Z_]+\b", line, re.M)
        or re.search(r"^V3_GATES_SUMMARY\b.* fail=[1-9]", line, re.M)
        or re.search(r"^TESTS passed [0-9]+ failed [1-9]", line, re.M)
        or re.search(r"^KVM_TESTS passed [0-9]+ failed [1-9]", line, re.M)
        or re.search(r"^KVM_TESTS passed .* kvm_skipped=[1-9]", line, re.M)
        or re.search(r"^CL_ROW .* verdict=(?!PASS\b)[A-Z_]+\b", line, re.M)
    )


FAIL_LINE = re.compile(
    r"^(?:error(?:\[E\d+\])?:|NOT_READY\b|KF_[A-Z_]*(?:REFUSED|FAIL)\b|"
    r"[^\n]*(?:\bFATAL\b|\bpanicked at\b|BENCH_TREE_FAILED|\bFAILED\b|NEW HOST Xid))"
)
ANSI = re.compile(r"\x1b\[[0-?]*[ -/]*[@-~]|\x1b[ -/]*[@-~]")
SUMMARY_BEGIN = "KF_FINAL_SUMMARY_BEGIN"
SUMMARY_END = "KF_FINAL_SUMMARY_END"
SUMMARY_LIMIT = 4096
LINE_LIMIT = 240
# Records of the stages of the current run, in order; the FINAL SUMMARY is built from them.
STAGE_RECORDS = []
PROV_DIR = "/root/prov"
# provision_full.sh writes one log per step; its RC line names the step whose log has the cause.
PROVISION_STEP_LOGS = {
    "BOX": "box.log",
    "DRIVER": "driver.log",
    "TREE": "tree.log",
    "FG": "fg.log",
    "KF3": "kf3.log",
}


def printable(line, limit=LINE_LIMIT):
    line = ANSI.sub("", line)
    line = "".join(c if " " <= c <= "~" else " " if c == "\t" else "" for c in line)
    line = line.strip()
    return line if len(line) <= limit else line[: limit - 3] + "..."


def read_lines(path, limit=2 * 1024 * 1024):
    try:
        with open(path, "rb") as source:
            data = source.read(limit)
    except OSError:
        return []
    return [ANSI.sub("", s) for s in data.decode("utf-8", errors="replace").splitlines()]


def is_failing(line):
    return bool(red_line(line) or FAIL_LINE.search(line))


def first_failing_line(paths):
    """First red line of the stage's logs, in the order given; else the last line printed."""
    last = ""
    for path in paths:
        lines = read_lines(path)
        for line in lines:
            if is_failing(line):
                return printable(line)
        for line in reversed(lines):
            if line.strip() and not last:
                last = line
    return "no red line found; last line: " + printable(last) if last else "no output found"


# Test names. libtest's terse mode prints `<name> --- FAILED`, its verbose mode `test <name> ... FAILED`,
# and both end with a `failures:` list of indented names. Measured 2026-10-26 (gate run 40263112941):
# the Worker/runner scrubber blanks any run of 32+ [A-Za-z0-9_-] characters, so a 60-character test
# name reached the log as `[redacted] --- FAILED`.
TEST_NAME = r"[A-Za-z_][A-Za-z0-9_]*(?:::[A-Za-z_][A-Za-z0-9_]*)*"
FAILED_TEST = re.compile(r"^(?:test )?(" + TEST_NAME + r")(?: \.\.\.| ---) FAILED$")
FAILURE_ITEM = re.compile(r"^    (" + TEST_NAME + r")$")
PANIC = re.compile(r"\bpanicked at\b")
FAILED_TEST_LIMIT = 20
# A snake_case test name: lowercase words, at least three, each at most 20 characters, at most 128
# characters in all and at most one digit in five. Hex, base64url, mixed-case, hyphenated and
# digit-heavy strings are not. ci/vast-broker (sanitize.js, runner.py scrub) uses the same grammar to
# decide which 32+ character runs a scrubber may keep; scrub-vectors.json pins both sides.
SNAKE_NAME = re.compile(r"[a-z][a-z0-9]{0,19}(?:_[a-z0-9]{1,20}){2,}")
LONG_RUN = re.compile(r"[A-Za-z0-9_-]{32,}")
FOLD = 30


def is_test_identifier(run):
    return (
        len(run) <= 128
        and SNAKE_NAME.fullmatch(run) is not None
        and sum(c.isdigit() for c in run) * 5 <= len(run)
    )


def fold_identifiers(text):
    """Split test-name-shaped 32+ character runs into pieces of at most 30 joined by "~".

    A scrubber that has not been redeployed still blanks the long form; the pieces stay readable.
    Anything that is not test-name shaped is left whole, so it still reaches the scrubber whole.
    """
    return LONG_RUN.sub(
        lambda m: (
            "~".join(m[0][i : i + FOLD] for i in range(0, len(m[0]), FOLD))
            if is_test_identifier(m[0])
            else m[0]
        ),
        text,
    )


def failed_tests(paths, limit=FAILED_TEST_LIMIT):
    """(names, total): the failed test names in the logs, in order, de-duplicated, at most `limit`."""
    seen = []
    for path in paths:
        state = None  # "head" after `failures:`, "list" while its indented names continue
        for line in read_lines(path):
            if line == "failures:":
                state = "head"
                continue
            item = FAILURE_ITEM.match(line) if state else None
            state = "list" if item else None
            found = FAILED_TEST.match(line) or item
            if found and found[1] not in seen:
                seen.append(found[1])
    return seen[:limit], len(seen)


def first_panic(paths):
    """The first panic line of the logs and the message line after it, bounded; "" if none."""
    for path in paths:
        lines = read_lines(path)
        for i, line in enumerate(lines):
            if PANIC.search(line):
                following = lines[i + 1].strip() if i + 1 < len(lines) else ""
                return printable(line.strip() + (" | " + following if following else ""))
    return ""


def step_cause(root, first):
    """The last lines of the provisioning step log named by a failing RC line."""
    match = re.match(r"(BOX|DRIVER|TREE|FG|KF3)_RC=", first)
    if not match:
        return ""
    lines = [s for s in read_lines(Path(root) / PROVISION_STEP_LOGS[match[1]]) if s.strip()]
    return " | ".join(printable(s, 100) for s in lines[-3:])


def summary_lines(result, records):
    stage_rows, fail_rows, suite_rows = [], [], []
    for record in records:
        if "rc" not in record:
            stage_rows.append("stage name=" + record["name"] + " result=NOT_RUN")
            continue
        text = (
            "stage name="
            + record["name"]
            + " rc="
            + str(record["rc"])
            + " duration_ms="
            + str(record["duration_ms"])
            + " result="
            + record["result"]
        )
        if record.get("reason"):
            text += " reason=" + record["reason"]
        stage_rows.append(text)
        if record.get("first_fail"):
            fail_rows.append(
                "first_fail stage="
                + record["name"]
                + " line="
                + fold_identifiers(record["first_fail"])
            )
        if record.get("panic"):
            fail_rows.append(
                "panic stage=" + record["name"] + " line=" + fold_identifiers(record["panic"])
            )
        for name in record.get("failed_tests", []):
            fail_rows.append(
                "failed_test stage=" + record["name"] + " name=" + fold_identifiers(printable(name))
            )
        more = record.get("failed_total", 0) - len(record.get("failed_tests", []))
        if more > 0:
            fail_rows.append("failed_tests_more stage=%s count=%d" % (record["name"], more))
        if record.get("cause"):
            fail_rows.append("cause stage=" + record["name"] + " log_tail=" + record["cause"])
    for suite in result["suites"]:
        text = "suite name=%s pass=%d fail=%d skip=%d" % (
            suite["name"],
            suite["pass"],
            suite["fail"],
            suite["skip"],
        )
        suite_rows.append((suite["fail"] > 0, text))
    return stage_rows, suite_rows, fail_rows


def final_summary(result, records):
    """One bounded block; it is printed last so a tail-limited log always shows it."""
    begin = "%s verdict=%s stages=%d" % (SUMMARY_BEGIN, result["verdict"], len(records))
    stage_rows, suite_rows, fail_rows = summary_lines(result, records)
    budget = SUMMARY_LIMIT - len(begin) - len(SUMMARY_END) - 2 - 64
    keep = {}
    failed = {
        i
        for i, record in enumerate(records)
        if record.get("result") in ("FAIL", "TIMEOUT", "PROVIDER_FAULT")
    }
    priority = (
        [("stage", i, s) for i, s in enumerate(stage_rows) if i in failed]
        + [("fail", i, s) for i, s in enumerate(fail_rows)]
        + [("suite", i, s) for i, (bad, s) in enumerate(suite_rows) if bad]
        + [("stage", i, s) for i, s in enumerate(stage_rows) if i not in failed]
        + [("suite", i, s) for i, (bad, s) in enumerate(suite_rows) if not bad]
    )
    dropped = 0
    for kind, index, text in priority:
        text = printable(text, LINE_LIMIT + 40)
        if len(text) + 1 > budget:
            dropped += 1
            continue
        budget -= len(text) + 1
        keep[(kind, index)] = text
    lines = [begin]
    for kind in ("stage", "suite", "fail"):
        lines += [keep[k] for k in sorted(keep) if k[0] == kind]
    if dropped:
        lines.append("truncated lines=%d" % dropped)
    lines.append(SUMMARY_END)
    return "\n".join(lines)


def print_final_summary(result):
    print(final_summary(result, STAGE_RECORDS), flush=True)


def watch_delta(text, rc, *, failed_provider=False):
    """Validate the existing watcher's instrument, process status and exact delta."""
    if int(only(r"^\[xid-watch\] host_dmesg_lines=([0-9]+) .*$", text, "host_xid")) < 2:
        raise EvidenceError("host_xid")
    before = int(only(r"^\[xid-watch\] xid_before=([0-9]+)$", text, "host_xid"))
    after = int(only(r"^\[xid-watch\] xid_after=([0-9]+)$", text, "host_xid"))
    only(r"^\[xid-watch\] --- command output begins ---$", text, "host_xid")
    command_rc = int(
        only(r"^\[xid-watch\] --- command output ends \(rc=([0-9]+)\) ---$", text, "host_xid")
    )
    # Only an already-proved provider fault may have a failed wrapped command.
    # Its wrapper must preserve that exact exit and positively prove a clean log.
    # Instrument failure, an Xid stop and timeout are never this exception.
    clean_failure = failed_provider and command_rc == rc and rc not in (0, 6, 7, 124)
    if command_rc and not clean_failure:
        raise EvidenceError("host_xid")
    if (rc == 0 or clean_failure) and before == after and "HOST Xid CLEAN" in text:
        return []
    if rc != 6:
        raise EvidenceError("host_xid")
    suffix = text.split("[xid-watch] --- command output ends ", 1)[1]
    if suffix.count("NEW HOST Xid ACROSS THE COMMAND") != 1:
        raise EvidenceError("host_xid")
    delta = [
        s[4:]
        for s in suffix.split("NEW HOST Xid ACROSS THE COMMAND", 1)[1].split("\n")
        if s.startswith("    ")
    ]
    if not delta or after - before != len(delta):
        raise EvidenceError("host_xid")
    return delta


def watched_command_output(text):
    """Only the command's own interval may supply fault source/identity proof."""
    begin = "[xid-watch] --- command output begins ---\n"
    end = "[xid-watch] --- command output ends "
    if text.count(begin) != 1 or text.count(end) != 1:
        raise EvidenceError("host_xid")
    return text.split(begin, 1)[1].split(end, 1)[0]


def xid_ok(text, rc, arm, arm_output=""):
    """Admission delegates to the single S4 exact-source implementation."""
    try:
        delta = watch_delta(text, rc)
        if arm not in ("--defer-liveness", "--missing-page-fault"):
            return not delta
        journal = "\n".join(delta)
        if len(xid_guard.xid_lines(journal)) != len(delta):
            return False
        xid_guard.approve_native_arm(journal, arm, watched_command_output(text))
        return True
    except (EvidenceError, xid_guard.GuardRefused, ValueError, TypeError):
        return False


def _stop(child):
    # Every child here was started in its own session by this function's caller.
    if child.poll() is not None:
        return
    try:
        os.killpg(child.pid, signal.SIGTERM)
    except ProcessLookupError:
        return
    try:
        child.wait(timeout=5)
    except subprocess.TimeoutExpired:
        try:
            os.killpg(child.pid, signal.SIGKILL)
        except ProcessLookupError:
            pass
        child.wait()


def run_bounded(command, soft_deadline_ms, output, progress_files):
    if STOP or int(time.time() * 1000) >= soft_deadline_ms:
        return 124
    output = Path(output)
    output.parent.mkdir(parents=True, exist_ok=True)
    positions = {}
    pending = {}
    failure = False
    provider_text = ""
    provider_seen = False
    provider_command = (
        len(command) == 6
        and command[0] == "bash"
        and Path(command[1]).name == "host_xid_watch.sh"
        and command[3:5] == ["--", "bash"]
        and Path(command[5]).name == "provision_full.sh"
    )
    child = None

    def forward(path):
        nonlocal failure, provider_text, provider_seen
        try:
            with open(path, "rb") as log:
                log.seek(positions.get(str(path), 0))
                data = log.read(65536)
                positions[str(path)] = log.tell()
        except FileNotFoundError:
            return
        if not data:
            return
        decoded = data.decode("utf-8", errors="replace")
        print(decoded, end="", flush=True)
        if provider_command:
            provider_text = (provider_text + decoded)[-65536:]
            # provision_full exits after driver status 8, but diagnostics and the
            # watcher receipt can arrive in later chunks. Classification is final
            # only after stage_grade checks the complete receipt and fault text.
            provider_seen = (
                provider_seen
                or provider_fault(provider_text) is not None
                or re.search(r"^DRIVER_RC=8$", provider_text, re.M) is not None
            )
        value = pending.get(str(path), "") + decoded
        lines = value.split("\n")
        pending[str(path)] = lines.pop()[-65536:]
        if any(red_line(line) for line in lines):
            failure = True

    try:
        with output.open("wb") as log:
            child = subprocess.Popen(
                command, stdout=log, stderr=subprocess.STDOUT, start_new_session=True
            )
            while True:
                for path in [output, *progress_files]:
                    forward(path)
                if STOP or int(time.time() * 1000) >= soft_deadline_ms:
                    _stop(child)
                    return 124
                # A known failed provider provision must finish its watcher receipt.
                # Deadline, stop, and explicit Xid failure still stop immediately;
                # stage_grade still requires complete, clean watcher evidence.
                if (failure and not provider_seen) or (
                    os.environ.get("KF_PAYLOAD_XID_FAILED")
                    and Path(os.environ["KF_PAYLOAD_XID_FAILED"]).exists()
                ):
                    _stop(child)
                    return 1
                rc = child.poll()
                if rc is not None:
                    for path in [output, *progress_files]:
                        forward(path)
                    return rc if rc else int(failure)
                time.sleep(0.05)
    finally:
        if child is not None:
            _stop(child)


def bounded_text(path, limit=4 * 1024 * 1024):
    with open(path, "rb") as source:
        data = source.read(limit + 1)
    if len(data) > limit:
        raise EvidenceError("infrastructure")
    return data.decode("utf-8", errors="strict")


def prepare_checkout(root, sha):
    def git(*args):
        return subprocess.check_output(
            ["git", "-C", str(root), *args], text=True, stderr=subprocess.DEVNULL
        ).strip()

    if git("rev-parse", "HEAD") != sha:
        raise EvidenceError("infrastructure")
    # Existing helpers fetch a branch. Point those reads at this already approved
    # local checkout, so their unmodified flows cannot select different remote code.
    branch = "kf-ci-approved"
    git("remote", "set-url", "origin", str(root))
    git("update-ref", "refs/heads/" + branch, sha)
    git("update-ref", "refs/remotes/origin/" + branch, sha)
    return branch


def hardware():
    try:
        lines = (
            subprocess.check_output(
                [
                    "nvidia-smi",
                    "--query-gpu=name,compute_cap,driver_version",
                    "--format=csv,noheader",
                ],
                text=True,
            )
            .strip()
            .splitlines()
        )
        if len(lines) != 1:
            raise ValueError()
        name, cc, driver = [x.strip() for x in lines[0].split(",")]
        if not re.fullmatch(r"[A-Za-z0-9_. -]{1,100}", name) or not re.fullmatch(
            r"\d{3}\.\d{1,3}(?:\.\d{1,2})?", driver
        ):
            raise ValueError()
        cap = int(round(float(cc) * 100))
        return {"gpu_name": name, "compute_cap": cap, "driver_version": driver}
    except (subprocess.SubprocessError, ValueError, OSError):
        return {"gpu_name": "unknown", "compute_cap": 0, "driver_version": "unknown"}


# Host claims for the public setup picker (2026-11-06). CLAIMS: read from this box's own /proc and
# from receipts of its own provisioning; the Worker checks their shape (vast_report.clean_claims,
# results.js cleanClaims) and no verdict reads them. The guest OS versions are the harness's own
# images: the fat Linux guest is the Ubuntu noble cloud image (provision_bench_tree.sh) and the
# Windows guest is Windows 11 Enterprise LTSC 2024, build 26100, with the 580.88 driver
# (win_vm.sh, windows_gate.sh); test_vast_payload_claims.py checks each against those files.
LINUX_GUEST_OS = "ubuntu-24.04"
WINDOWS_GUEST_OS = "11-ltsc-2024-26100"
WINDOWS_GUEST_DRIVER = "580.88"
CPU_VENDORS = {"GenuineIntel": "intel", "AuthenticAMD": "amd"}


def cpu_claims(cpuinfo="/proc/cpuinfo", machine=None):
    """cpu_vendor (amd|intel|arm|other), cpu_model (sanitised, bounded) and cpu_arch of this box.
    Nothing here is a serial, id or address: only the vendor string and the model name line."""
    import platform

    arch = (machine or platform.machine() or "").lower()
    arch = {"amd64": "x86_64", "arm64": "aarch64"}.get(arch, arch)
    vendor = model = None
    try:
        text = Path(cpuinfo).read_bytes()[:262144].decode("utf-8", "replace")
    except OSError:
        text = ""
    for line in text.splitlines():
        key, _, value = line.partition(":")
        key, value = key.strip().lower(), value.strip()
        if key == "vendor_id" and vendor is None and value:
            vendor = CPU_VENDORS.get(value, "other")
        elif key == "model name" and model is None and value:
            model = re.sub(r"[^A-Za-z0-9 ().@+/_-]", " ", value)
            model = " ".join(model.split())[:64].strip()
    if arch == "aarch64":
        vendor = "arm"
    return {"cpu_vendor": vendor, "cpu_model": model or None, "cpu_arch": arch or None}


def guest_driver_receipt(bench):
    """The guest driver the fat guest was provisioned with (provision_bench_tree.sh writes
    guest.qcow2.STAGED with version=<modinfo version of the guest's nvidia module>)."""
    try:
        text = bounded_text(Path(bench) / "guest.qcow2.STAGED", 4096)
    except (OSError, EvidenceError, UnicodeError):
        return None
    for line in text.splitlines():
        if line.startswith("version="):
            return line[len("version="):].strip()
    return None


def host_claims(windows, bench=Path("/workspace/bench"), cpuinfo="/proc/cpuinfo", machine=None):
    """The claims block of the result: only the fields that have a value."""
    raw = dict(cpu_claims(cpuinfo, machine))
    raw["linux_os_version"] = LINUX_GUEST_OS
    raw["linux_driver"] = guest_driver_receipt(bench)
    if windows:
        raw["windows_os_version"] = WINDOWS_GUEST_OS
        raw["windows_driver"] = WINDOWS_GUEST_DRIVER
    return {k: v for k, v in local_result_contract.clean_claims(raw).items() if v is not None}


def refresh_claims(result, windows):
    """Record the host claims in the result. A claim is never worth a failed run: any error leaves
    the result as it was."""
    try:
        claims = host_claims(windows)
    except (OSError, ValueError, UnicodeError, EvidenceError):
        return
    if claims:
        result["host_claims"] = claims


def persist_result(destination, result):
    destination = Path(destination)
    temporary = destination.with_suffix(".tmp")
    snapshot = dict(result)
    state_path = os.environ.get("KF_PAYLOAD_DIAGNOSTIC_STATE")
    if state_path and Path(state_path).is_file():
        events = json.loads(bounded_text(state_path, 65536))["events"]
        local_result_contract.validate_diagnostics(events)
        if events:
            snapshot["diagnostics"] = events
    temporary.write_text(json.dumps(snapshot, separators=(",", ":")) + "\n")
    temporary.replace(destination)


def execute_stages(stages, soft, result, destination, runner=run_bounded):
    """Persist the finished prefix even when a later suite times out or fails."""
    records = STAGE_RECORDS
    result["host_reference"] = "MISSING"
    records[:] = [dict(name=stage["name"]) for stage in stages]
    persist_result(destination, result)
    for stage, record in zip(stages, records):
        name = stage["name"]
        if name == "cuda_host":
            result["host_reference"] = "NOT_GREEN"
            persist_result(destination, result)
        print("KF_PAYLOAD_STAGE_START name=" + name, flush=True)
        start = time.monotonic()
        need = stage.get("min_remaining_ms")
        # Too little time left to finish: not run (rc 125), and not a verdict on the guest.
        short = need is not None and soft - int(time.time() * 1000) < need
        rc = 125 if short else runner(stage["command"], soft, stage["output"], stage["progress"])
        elapsed = int((time.monotonic() - start) * 1000)
        record.update(rc=rc, duration_ms=elapsed, result="PASS")
        print(
            "KF_PAYLOAD_STAGE_END name=" + name + " rc=" + str(rc) + " duration_ms=" + str(elapsed),
            flush=True,
        )
        try:
            if short:
                raise EvidenceError(stage["short_row"])
            rows = stage["grade"](rc)
            if rc == 124:
                raise EvidenceError("timeout", rows)
            if name == "cuda_host":
                result["host_reference"] = "GREEN"
            result["suites"].extend(rows)
            persist_result(destination, result)
        except EvidenceError as error:
            result["suites"].extend(error.rows)
            failure = "timeout" if rc == 124 else error.name
            result["suites"].append(row(failure, failed=1))
            append_arm_failure(result)
            verdict = "PROVIDER_FAULT" if isinstance(error, ProviderFault) else "FAIL"
            result["verdict"] = verdict
            persist_result(destination, result)
            record.update(result="TIMEOUT" if rc == 124 else verdict, reason=failure)
            files = [*stage["progress"], stage["output"], *stage.get("detail", [])]
            record["first_fail"] = first_failing_line(files)
            record["failed_tests"], record["failed_total"] = failed_tests(files)
            record["panic"] = first_panic(files)
            if name == "provision":
                record["cause"] = step_cause(PROV_DIR, record["first_fail"])
            print("KF_PAYLOAD_FAIL suite=" + failure, flush=True)
            print_final_summary(result)
            return 1
    result["verdict"] = "PASS"
    persist_result(destination, result)
    print_final_summary(result)
    return 0


def arm_diagnostic(arm, text, rc, admitted, mode="native"):
    """Send a bounded head/tail through the existing reporter, even on refusal."""
    destination = os.environ.get("KF_PAYLOAD_LOG")
    state_path = os.environ.get("KF_PAYLOAD_DIAGNOSTIC_STATE")
    if not destination or not state_path:
        return
    if not re.fullmatch(r"--[a-z0-9-]{1,40}|guest", arm) or mode not in ("native", "guest"):
        raise EvidenceError("diagnostics")
    path = Path(state_path)
    state = json.loads(bounded_text(path, 65536)) if path.exists() else {"bytes": 0, "events": []}
    if (
        set(state) != {"bytes", "events"}
        or type(state["bytes"]) is not int
        or not 0 <= state["bytes"] <= 1048576
        or not isinstance(state["events"], list)
        or len(state["events"]) >= 128
    ):
        raise EvidenceError("diagnostics")
    raw = text.encode("utf-8")
    if len(raw) > 32768:
        raw = raw[:16384] + b"\n[arm diagnostic truncated; head and tail retained]\n" + raw[-16384:]
    content = raw.decode("utf-8", errors="replace")
    frame = (
        "KF_ARM_DIAG_BEGIN arm="
        + arm
        + " wrapper_rc="
        + str(rc)
        + " admitted="
        + str(int(admitted))
        + "\n"
        + content
        + "\nKF_ARM_DIAG_END arm="
        + arm
        + "\n"
    ).encode("utf-8")
    limit = 1048576 - (65536 if admitted else 0)
    remaining = max(0, limit - state["bytes"])
    frame = frame[:remaining].decode("utf-8", errors="ignore").encode("utf-8")
    descriptor = os.open(destination, os.O_WRONLY | os.O_APPEND | os.O_CREAT | os.O_NOFOLLOW, 0o600)
    with os.fdopen(descriptor, "ab") as output:
        output.write(frame)
    state["bytes"] += len(frame)
    item = local_result_contract.diagnostic_record(arm, mode, rc, bool(admitted), text)
    state["events"] = local_result_contract.compact_diagnostics([*state["events"], item])
    temporary = path.with_suffix(".next")
    temporary.write_text(json.dumps(state) + "\n")
    temporary.replace(path)


def append_arm_failure(result):
    path = os.environ.get("KF_PAYLOAD_DIAGNOSTIC_STATE")
    if not path or not Path(path).is_file():
        return
    events = json.loads(bounded_text(path, 65536))["events"]
    if not isinstance(events, list) or len(events) > 128:
        raise EvidenceError("diagnostics")
    for event in reversed(events):
        if event.get("admitted") is False:
            arm = event.get("arm", "")
            if not re.fullmatch(r"--[a-z0-9-]{1,40}|guest", arm):
                raise EvidenceError("diagnostics")
            prefix = "guest_" if event.get("mode") == "guest" else "arm_"
            name = prefix + arm.removeprefix("--").replace("-", "_")
            if name not in {r["name"] for r in result["suites"]}:
                result["suites"].append(row(name, failed=1))
            break


def xid_record_key(line):
    if line.count("NVRM: Xid") != 1:
        raise EvidenceError("host_xid")
    return line[line.index("NVRM: Xid") :]


def retain_native_control(arm, text):
    directory = os.environ.get("KF_PAYLOAD_CONTROL_DIR")
    receipt = os.environ.get("KF_PAYLOAD_NATIVE_RECEIPT")
    if not directory or not receipt or arm not in ("--defer-liveness", "--missing-page-fault"):
        return
    directory = Path(directory)
    directory.mkdir(parents=True, exist_ok=True)
    (directory / (arm.removeprefix("--") + ".log")).write_text(watched_command_output(text))
    defer = directory / "defer-liveness.log"
    missing = directory / "missing-page-fault.log"
    if defer.is_file() and missing.is_file():
        lines = xid_guard.approve_native(
            xid_guard.kernel_snapshot(), bounded_text(defer), bounded_text(missing)
        )
        persist_result(receipt, {"approved_native_xid_lines": list(lines)})


def guard_command(arm, log_path, rc):
    text = bounded_text(log_path)
    ok = xid_ok(text, rc, arm, text)
    if ok:
        try:
            retain_native_control(arm, text)
        except (xid_guard.GuardRefused, EvidenceError, OSError, ValueError) as error:
            text += "\nKF_NATIVE_RECEIPT_REFUSED reason=" + str(error) + "\n"
            ok = False
    arm_diagnostic(arm, text, rc, ok)
    if ok:
        delta = watch_delta(text, rc)
        if delta:
            with open(os.environ["KF_PAYLOAD_XID_ALLOWED"], "a") as approved:
                for line in delta:
                    approved.write(json.dumps(xid_record_key(line)) + "\n")
    else:
        Path(os.environ["KF_PAYLOAD_XID_FAILED"]).touch()
        print("KF_XID_REFUSED arm=" + arm, flush=True)
    return 0 if ok else 6


def guest_command(qlog, arguments):
    """Keep the caller's timeout; S4 owns and validates the actual QEMU child."""
    qlog = Path(qlog)
    arm = "guest"
    admitted = False
    serial = None
    error = ""
    rc = 6
    try:
        binary = Path(arguments[0])
        argv = arguments[1:]
        arm = xid_guard.guest_launch_arm(argv)
        serials = [
            argv[i + 1].split("path=", 1)[1]
            for i, arg in enumerate(argv[:-1])
            if arg == "-chardev" and argv[i + 1].startswith("file,id=kfcon0,path=")
        ]
        if len(serials) != 1 or not qlog.is_file():
            raise EvidenceError("host_xid")
        serial = Path(serials[0])
        root = Path(os.environ["KF_PAYLOAD_ROOT"])
        revision = subprocess.check_output(
            ["git", "-C", str(root), "rev-parse", "HEAD"], text=True
        ).strip()
        receipt = Path(os.environ["KF_PAYLOAD_NATIVE_RECEIPT"])
        before = json.loads(xid_guard.bounded(receipt))
        baseline = before.get("accepted_xid_lines", before["approved_native_xid_lines"])
        result = xid_guard.supervise_guest(binary, argv, receipt, revision, root, serial, qlog)
        if result != 0:
            raise EvidenceError("host_xid")
        state = json.loads(xid_guard.bounded(receipt))
        accepted = state.get("accepted_xid_lines", state["approved_native_xid_lines"])
        if accepted[: len(baseline)] != baseline:
            raise EvidenceError("host_xid")
        with open(os.environ["KF_PAYLOAD_XID_ALLOWED"], "a") as approved:
            for line in accepted[len(baseline) :]:
                approved.write(json.dumps(xid_record_key(line)) + "\n")
        rc = 0
        admitted = True
    except (
        EvidenceError,
        xid_guard.GuardRefused,
        OSError,
        ValueError,
        KeyError,
        subprocess.SubprocessError,
    ) as failure:
        error = "KF_GUEST_XID_REFUSED reason=" + str(failure) + "\n"
        Path(os.environ["KF_PAYLOAD_XID_FAILED"]).touch()
    finally:
        parts = [error]
        for label, path in (("SERIAL", serial), ("QEMU", qlog)):
            if path is not None and path.is_file():
                with path.open("rb") as source:
                    size = path.stat().st_size
                    head = source.read(16384)
                    source.seek(max(0, size - 16384))
                    tail = source.read(16384) if size > 16384 else b""
                parts.append(label + "\n" + (head + tail).decode("utf-8", errors="replace"))
        arm_diagnostic(arm, "\n".join(parts), rc, admitted, mode="guest")
    return rc


def watched(command, root, name):
    return ["bash", str(root / "scripts/bench/host_xid_watch.sh"), name, "--", *command]


def stage_grade(root, tag, log, grade, rc):
    if rc not in (0, 6):
        try:
            return grade(rc)
        except ProviderFault:
            # Provider recovery must never bypass the unmatched-Xid safety stop.
            if watch_delta(bounded_text(log), rc, failed_provider=True):
                raise EvidenceError("host_xid") from None
            raise
    text = bounded_text(log)
    try:
        delta = [xid_record_key(line) for line in watch_delta(text, rc)]
        allowed_path = Path(os.environ["KF_PAYLOAD_XID_ALLOWED"])
        allowed = [xid_record_key(json.loads(s)) for s in allowed_path.read_text().splitlines()]
        if collections.Counter(delta) != collections.Counter(allowed):
            raise EvidenceError("host_xid")
        allowed_path.write_text("")
    except (EvidenceError, ValueError, OSError):
        # Parse completed rows before appending the failed Xid gate.
        try:
            rows = grade(1)
        except EvidenceError as error:
            rows = error.rows
        raise EvidenceError("host_xid", rows) from None
    return grade(0)


def main():
    global STOP
    if len(sys.argv) == 5 and sys.argv[1] == "--grade-xid":
        return guard_command(sys.argv[2], Path(sys.argv[3]), int(sys.argv[4]))
    if len(sys.argv) > 3 and sys.argv[1] == "--guest-exec":
        return guest_command(sys.argv[2], sys.argv[3:])
    sha = sys.argv[1] if len(sys.argv) > 1 else ""
    nonce = sys.argv[2] if len(sys.argv) > 2 else ""
    if not re.fullmatch("[0-9a-f]{40}", sha) or not re.fullmatch("[0-9a-f]{32}", nonce):
        return 2
    root = Path(__file__).resolve().parents[2]
    bench = Path("/workspace/bench")
    evidence = Path("/workspace/kf-ci-evidence")
    evidence.mkdir(parents=True, exist_ok=True)
    destination = Path("/workspace/kf-result.json")
    result = {"verdict": "FAIL", "suites": [], "hardware": hardware(), "host_reference": "MISSING"}
    STAGE_RECORDS.clear()

    def terminate(*_):
        global STOP
        STOP = True

    old_term = signal.signal(signal.SIGTERM, terminate)
    try:
        soft = int(os.environ["KF_SOFT_DEADLINE_MS"])
        # Popped before any stage runs: merge_check.sh must not run the gate inside merge_bar.
        windows, windows_source = windows_requested(os.environ)
        tier, windows_runs, extra_stages = tier_requested(os.environ, windows)
        selected_drivers = release_driver_environment(os.environ)
        require_executable_release(os.environ)
        os.environ.update(selected_drivers)

        winapps = winapps_requested(os.environ, windows)
        linuxapps = linuxapps_requested(os.environ, tier)

        spec = spec_requested(os.environ, windows, extra_stages)
        spec_tbl = spec_table(root) if spec else None
        refresh_claims(result, windows)
        print(
            "KF_PAYLOAD_TIER windows="
            + str(int(windows))
            + " source="
            + windows_source
            + " tier="
            + tier
            + " windows_runs="
            + str(windows_runs)
            + " stages="
            + (":".join(extra_stages) or "none")
            + " spec="
            + (spec or "none")
            + " winapps="
            + (winapps or "none"),
            flush=True,
        )
        subprocess.run(["df", "-h", "/"], check=True)
        free = shutil.disk_usage("/").free
        print("KF_PREFLIGHT_FREE_BYTES=" + str(free), flush=True)
        if free < 60000000000:
            raise EvidenceError("preflight")
        require_provider_kvm(Path("/dev/kvm"))
        if int(time.time() * 1000) >= soft:
            raise EvidenceError("timeout")
        branch = prepare_checkout(root, sha)
        link = Path("/root/kayfabe")
        if link.exists() and link.resolve() != root:
            raise EvidenceError("preflight")
        if not link.exists():
            link.symlink_to(root, target_is_directory=True)
        shutil.copyfile(root / "scripts/bench/provision_box.sh", "/root/provision_box.sh")
        os.environ.update(
            KF_PROVISION_PROFILE="ci",
            KAYFABE_BRANCH=branch,
            KAYFABE_REPO=str(root),
            BENCH_DIR=str(bench),
            KF_DEVICE="kf3",
            CARGO_TARGET_DIR=str(root / "target"),
            CARGO_BUILD_JOBS=str(min(8, os.cpu_count() or 2)),
            # Quiet only: cargo's own progress and colour. Lint level and RUSTFLAGS are not touched.
            CARGO_TERM_QUIET="true",
            CARGO_TERM_COLOR="never",
            CARGO_TERM_PROGRESS_WHEN="never",
            BASH_ENV=str(root / "scripts/ci/vast_payload_hooks.sh"),
            KF_PAYLOAD_ROOT=str(root),
            KF_PAYLOAD_XID_ALLOWED=str(evidence / "xid-allowed.jsonl"),
            KF_PAYLOAD_XID_FAILED=str(evidence / "xid-failed"),
            KF_PAYLOAD_LOG="/workspace/kf-ci-output.log",
            KF_PAYLOAD_DIAGNOSTIC_STATE=str(evidence / "arm-diagnostics.json"),
            KF_PAYLOAD_CONTROL_DIR=str(evidence / "native-controls"),
            KF_PAYLOAD_NATIVE_RECEIPT=str(evidence / "native-receipt.json"),
        )
        if spec:
            # the gate installs Windows on a disk that can hold a lane's tree; an explicit WINVM_DISK_GB wins
            os.environ.setdefault("WINVM_DISK_GB", SPEC_DISK_GB)
        Path(os.environ["KF_PAYLOAD_DIAGNOSTIC_STATE"]).write_text(
            json.dumps({"bytes": 0, "events": []}) + "\n"
        )
        Path(os.environ["KF_PAYLOAD_XID_ALLOWED"]).write_text("")
        Path(os.environ["KF_PAYLOAD_XID_FAILED"]).unlink(missing_ok=True)
        os.environ["PATH"] = str(Path.home() / ".cargo/bin") + ":" + os.environ["PATH"]
        tag = "ci" + nonce[:16]

        def make_stage(name, command, output, progress, grade, detail=()):
            watch_tag = tag + "_" + name
            watcher = bench / ("xid_" + watch_tag + ".log")
            return dict(
                name=name,
                command=watched(command, root, watch_tag),
                output=output,
                progress=progress,
                # Logs read only for the FINAL SUMMARY's first failing line.
                detail=[*detail, watcher],
                grade=lambda rc: stage_grade(root, watch_tag, watcher, grade, rc),
            )

        def provision_grade(rc):
            log = bounded_text("/root/prov/prov.log")
            if re.search(r"^DRIVER_MEDIA_DRIFT reason=[A-Z0-9_]{1,32}$", log, re.M):
                provision_rows(log, rc)
            fault = provider_fault(log)
            if fault:
                raise ProviderFault(fault)
            refresh_claims(result, windows)  # the guest receipt exists once provisioning ran
            result["hardware"] = hardware()
            rows = provision_rows(log, rc)
            tags = {
                s
                for s in (root / "tools/drivermatrix/tags.txt").read_text().splitlines()
                if s and not s.startswith("#")
            }
            if (
                result["hardware"]["driver_version"] not in tags
                or result["hardware"]["compute_cap"] < 750
            ):
                raise EvidenceError("preflight", rows)
            return rows

        stages = [
            make_stage(
                "provision",
                ["bash", str(root / "scripts/bench/box/provision_full.sh")],
                evidence / "provision.run",
                [Path("/root/prov/prov.log")],
                provision_grade,
                [Path("/root/prov") / x for x in PROVISION_STEP_LOGS.values()],
            )
        ]
        for mode in ["host"]:
            path = bench / ("cl_" + tag + "_" + mode + ".out")
            stages.append(
                make_stage(
                    "cuda_" + mode,
                    [
                        "bash",
                        str(root / "scripts/bench/cuda_ladder.sh"),
                        mode,
                        tag,
                        "1",
                        "cup2,cup3,cup8,cup8bench",
                    ],
                    evidence / ("cuda_" + mode + ".run"),
                    [path],
                    lambda rc, mode=mode, path=path: ladder_rows(bounded_text(path), rc, mode, sha),
                )
            )
        merge = Path("/root/prov") / (tag + ".log")
        stages.append(
            make_stage(
                "merge_bar",
                ["bash", str(root / "scripts/bench/box/merge_check.sh"), sha, tag],
                evidence / "merge.run",
                [merge, Path("/root/prov") / (tag + "_host.log"), bench / (tag + "_suite.out")],
                lambda rc: merge_rows(bounded_text(merge), rc, sha),
                [
                    Path("/root/prov") / (tag + "_" + x)
                    for x in [
                        "tests.log",
                        "gates.log",
                        "kf3.log",
                        "client.log",
                        "fg.log",
                        "suite.run",
                        "census_selftest.log",
                        "births.log",
                    ]
                ],
            )
        )
        path = bench / ("cl_" + tag + "_guest.out")
        stages.append(
            make_stage(
                "cuda_guest",
                [
                    "bash",
                    str(root / "scripts/bench/cuda_ladder.sh"),
                    "guest",
                    tag,
                    "1",
                    "cup2,cup3,cup8,cup8bench",
                ],
                evidence / "cuda_guest.run",
                [path],
                lambda rc: ladder_rows(bounded_text(path), rc, "guest", sha),
            )
        )
        if linuxapps:
            # After the last Linux guest stage, before Windows: the app matrix over the gate list. It
            # boots the fat guest itself (apps_matrix.sh), one boot at a time, under its own budget.
            output_apps = evidence / "apps.run"
            names = apps_gate_list(root, tier)
            matrix_log = Path(PROV_DIR) / (tag + "_apps_matrix.log")
            stage = make_stage(
                "apps",
                [
                    "bash",
                    str(root / "scripts/bench/box/apps_gate.sh"),
                    tag,
                    PROV_DIR,
                    APPS_LISTS[tier][1],
                ],
                output_apps,
                [matrix_log],
                lambda rc, names=names, output=output_apps: apps_rows(bounded_text(output), rc, names),
                [matrix_log],
            )
            stage.update(min_remaining_ms=APPS_MIN_REMAINING_MS + (
                WINDOWS_MIN_REMAINING_MS + WINAPPS_MIN_REMAINING_MS[winapps] if windows and winapps else 0
            ), short_row="apps_retry_time")
            stages.append(stage)
        if windows:
            # Last, after every Linux guest has stopped: windows_gate.sh owns the GPU from here.
            output = evidence / "windows.run"
            stage = make_stage(
                "windows_gate",
                ["bash", str(root / "scripts/bench/box/windows_gate.sh"), tag, PROV_DIR],
                output,
                [],
                lambda rc: windows_rows(bounded_text(output), rc, sha),
                [Path(PROV_DIR) / (tag + "_windows.log")],
            )
            stage.update(min_remaining_ms=WINDOWS_MIN_REMAINING_MS + (
                WINAPPS_MIN_REMAINING_MS[winapps] if winapps else 0
            ), short_row="windows_retry_time")
            stages.append(stage)
        if windows and winapps:
            # The Windows apps stage (docs/design/V3_WINDOWS_APPS_GATE.md): the Windows app matrix, in a fresh guest
            # that is a child of the gate's own overlay (same tag), on the same box. It runs only after a green
            # gate (a first red cancels later work). Opt-in: the Worker sends KF_WINAPPS.
            output_apps = evidence / "windows_apps.run"
            stage = make_stage(
                "windows_apps",
                [
                    "env",
                    "KF_WINAPPS_PROFILE=" + winapps,
                    "bash",
                    str(root / "scripts/bench/box/windows_apps.sh"),
                    tag,
                    PROV_DIR,
                ],
                output_apps,
                [],
                lambda rc, output=output_apps: winapps_rows(bounded_text(output), rc, sha, winapps),
                [Path(PROV_DIR) / (tag + "_winapps.log")],
            )
            stage.update(
                min_remaining_ms=WINAPPS_MIN_REMAINING_MS[winapps], short_row="windows_apps_retry_time"
            )
            stages.append(stage)
        if windows and windows_runs == 2:
            # The release tier's second Windows run when no second machine offers the die.
            output2 = evidence / "windows_2.run"
            stage = make_stage(
                "windows_gate_2",
                ["bash", str(root / "scripts/bench/box/windows_gate.sh"), tag + "w2", PROV_DIR],
                output2,
                [],
                lambda rc: windows_rows(bounded_text(output2), rc, sha, "windows_gate_2"),
                [Path(PROV_DIR) / (tag + "w2_windows.log")],
            )
            stage.update(
                min_remaining_ms=WINDOWS_REPEAT_MIN_REMAINING_MS, short_row="windows_retry_time"
            )
            stages.append(stage)
        if "specviewperf" in extra_stages:
            # DISABLED BY DEFAULT (docs/design/V3_SPEC_GATE.md): the wrapper runs scripts/bench/box/specviewperf_stage.sh,
            # which fetches the private copy through the Worker's presigned /bench-data route and runs the lane's
            # viewsets in the Windows guest the gate passed. Last, after every Windows run. Its own deadline is the soft
            # deadline minus a margin; too little time for the whole lane is specviewperf_retry_time, never a start.
            output3 = evidence / "specviewperf.run"
            stage = make_stage(
                "specviewperf",
                [
                    "env",
                    "KF_SPEC_LANE=" + spec,
                    "KF_SPEC_DEADLINE_EPOCH=" + str(soft // 1000 - SPEC_DEADLINE_MARGIN_S),
                    "bash",
                    str(root / "scripts/ci/vast_stage_specviewperf.sh"),
                    tag,
                    PROV_DIR,
                ],
                output3,
                [],
                lambda rc: specviewperf_rows(bounded_text(output3), rc, spec, spec_tbl),
                [Path(PROV_DIR) / (tag + "_spec") / "stage.log"],
            )
            stage.update(
                min_remaining_ms=spec_min_remaining_ms(spec_tbl, spec), short_row="specviewperf_retry_time"
            )
            stages.append(stage)
        return execute_stages(stages, soft, result, destination)
    except EvidenceError as error:
        if isinstance(error, ProviderFault):
            result["verdict"] = "PROVIDER_FAULT"
        result["suites"].extend(error.rows)
        result["suites"].append(row(error.name, failed=1))
        persist_result(destination, result)
        print("KF_PAYLOAD_FAIL suite=" + error.name, flush=True)
        print_final_summary(result)
        return 1
    except (KeyError, ValueError, OSError, subprocess.SubprocessError, UnicodeError):
        result["suites"].append(row("infrastructure", failed=1))
        persist_result(destination, result)
        print("KF_PAYLOAD_FAIL suite=infrastructure", flush=True)
        print_final_summary(result)
        return 1
    finally:
        signal.signal(signal.SIGTERM, old_term)


if __name__ == "__main__":
    raise SystemExit(main())
