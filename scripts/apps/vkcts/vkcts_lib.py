# SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later
"""Pure logic of the open Vulkan CTS row (stdlib only, no I/O beyond reading the files it is given).

STATUS: LIVE, 2026-11-17 (design: docs/design/V3_VULKAN_CTS_ROW.md; tests: scripts/ci/test_vkcts.py; mutants:
scripts/ci/vkcts_mutants.py). Nothing here starts a process or touches a GPU.

What lives here
    pins          load and validate pins.json; compare it with the pinned CTS's own external/fetch_sources.py
    caselist      read and validate the committed case list (area TAB case name)
    qpa           parse deqp-vk's QPA log (and the console totals) into per-case results
    classify      map a CTS status code to one of the classes pass warn not_supported fail crash timeout not_run
    execute       the batch / resume loop that attributes a crash or a hang to the case that was running
    block         the machine-readable per-case results block that the row prints and `diff` reads back
    diff          host-versus-guest comparison of two result sets, the only pass criterion of the row
Everything that comes out of a guest is data: bounded, sanitised and only accepted for case names that are in the case list.
"""
import collections
import hashlib
import json
import re

SCHEMA = "kf-vkcts-pins/1"
BLOCK_VERSION = "v1"
COMMIT_RE = re.compile(r"^[0-9a-f]{40}$")
SHA256_RE = re.compile(r"^[0-9a-f]{64}$")
CASE_RE = re.compile(r"^dEQP-VK(?:\.[A-Za-z0-9_-]{1,120}){1,24}$")
AREA_RE = re.compile(r"^[a-z][a-z0-9_]{1,31}$")
MAX_CASES = 400
MAX_LINE = 65536
MAX_LOG_BYTES = 256 * 1024 * 1024
DETAIL_MAX = 100

# CTS status code (qpTestLog.c s_qpTestResultMap) -> class. A code that is not here is an unknown, which is a failure
# (never a pass): a new CTS status must be classified on purpose.
STATUS_CLASS = {
    "Pass": "pass",
    "QualityWarning": "warn",
    "CompatibilityWarning": "warn",
    "Waiver": "warn",
    "NotSupported": "not_supported",
    "Fail": "fail",
    "ResourceError": "fail",
    "InternalError": "fail",
    "DeviceLost": "fail",
    "Pending": "fail",
    "EnforceDefaultContext": "fail",
    "EnforceDefaultInstance": "fail",
    "Crash": "crash",
    "Timeout": "timeout",
    "NotRun": "not_run",
}
CLASSES = ("pass", "warn", "not_supported", "fail", "crash", "timeout", "not_run")
OK_CLASSES = ("pass", "warn")
BAD_CLASSES = ("fail", "crash", "timeout")


class PinRefused(Exception):
    """A pin is missing, malformed or does not match what was fetched: the row refuses, it never fetches 'anyway'."""


class CaselistError(Exception):
    pass


def classify(status):
    return STATUS_CLASS.get(status, "fail")


def sanitize(text, limit=DETAIL_MAX):
    """Untrusted text -> one short printable line (no tab, no newline, no markers): the only form that reaches a log or a TSV."""
    text = re.sub(r"[^A-Za-z0-9 _.,:;()/=+\-]", "?", str(text))
    text = re.sub(r"\s+", " ", text).strip()
    return text[:limit] or "-"


# ---------------------------------------------------------------------------------------------------------------- pins
def validate_pins(pins):
    """Raise PinRefused unless the pin document is complete and well formed. Returns the document."""
    if not isinstance(pins, dict) or pins.get("schema") != SCHEMA:
        raise PinRefused("pins: wrong schema")
    cts = pins.get("cts") or {}
    if not COMMIT_RE.match(str(cts.get("commit", ""))):
        raise PinRefused("pins: the CTS pin is not a 40-hex commit: %r" % cts.get("commit"))
    if not str(cts.get("url", "")).startswith("https://github.com/KhronosGroup/"):
        raise PinRefused("pins: the CTS url is not the upstream https origin")
    if not cts.get("tag"):
        raise PinRefused("pins: the CTS tag is empty")
    ext = pins.get("externals_git") or {}
    if not ext:
        raise PinRefused("pins: no git externals")
    for name, row in sorted(ext.items()):
        if not COMMIT_RE.match(str(row.get("commit", ""))):
            raise PinRefused("pins: external %s is not pinned by a 40-hex commit: %r" % (name, row.get("commit")))
        if not str(row.get("url", "")).startswith("https://"):
            raise PinRefused("pins: external %s has no https url" % name)
    files = pins.get("externals_files") or {}
    for name, row in sorted(files.items()):
        if not SHA256_RE.match(str(row.get("sha256", ""))):
            raise PinRefused("pins: external file %s has no sha256 pin" % name)
    if not isinstance(pins.get("build", {}).get("cmake_args"), list) or not pins["build"]["cmake_args"]:
        raise PinRefused("pins: no cmake arguments")
    return pins


def parse_fetch_sources(text):
    """The pinned CTS's own external/fetch_sources.py -> ({baseDir: (url, revision)}, {name: (url, sha256)}).
    Parsed as text (the script is never imported or run here)."""
    git = {}
    for m in re.finditer(r'GitRepo\(\s*"([^"]+)",\s*"[^"]*",\s*"([^"]+)",\s*"([^"]+)"', text):
        git[m.group(3)] = (m.group(1), m.group(2))
    files = {}
    for m in re.finditer(r'SourcePackage\(\s*"([^"]+)",\s*"([0-9a-f]{64})",\s*"([^"]+)"', text):
        files[m.group(3)] = (m.group(1), m.group(2))
    for m in re.finditer(r'SourceFile\(\s*"([^"]+)",\s*"[^"]+",\s*"([0-9a-f]{64})",\s*"([^"]+)"', text):
        files[m.group(3)] = (m.group(1), m.group(2))
    return git, files


def check_fetch_sources(text, pins, resolved_tags=None):
    """The checked-out fetch_sources.py must say exactly what pins.json says, for every external we pin. A revision that is a tag in
    the script (video_generator) is accepted only through `upstream_ref` and the commit pinned here. Raises PinRefused."""
    git, files = parse_fetch_sources(text)
    resolved_tags = resolved_tags or {}
    for name, row in sorted(pins["externals_git"].items()):
        if name not in git:
            raise PinRefused("fetch_sources.py has no git external %s (pinned here)" % name)
        url, rev = git[name]
        if url != row["url"]:
            raise PinRefused("fetch_sources.py: %s comes from %s, pinned %s" % (name, url, row["url"]))
        if COMMIT_RE.match(rev):
            if rev != row["commit"]:
                raise PinRefused("fetch_sources.py: %s revision %s is not the pinned %s" % (name, rev, row["commit"]))
        else:
            if row.get("upstream_ref") != rev:
                raise PinRefused("fetch_sources.py: %s is the ref %r, pins.json names %r" % (name, rev, row.get("upstream_ref")))
            got = resolved_tags.get(name)
            if got is not None and got != row["commit"]:
                raise PinRefused("upstream ref %s of %s now resolves to %s, pinned %s" % (rev, name, got, row["commit"]))
    unpinned = sorted(set(git) - set(pins["externals_git"]) - {"vulkan-validationlayers"})
    if unpinned:
        raise PinRefused("fetch_sources.py has git externals this repository does not pin: %s" % ", ".join(unpinned))
    for name, row in sorted(pins.get("externals_files", {}).items()):
        if name not in files:
            raise PinRefused("fetch_sources.py has no file external %s (pinned here)" % name)
        url, digest = files[name]
        if url != row["url"] or digest != row["sha256"]:
            raise PinRefused("fetch_sources.py: file external %s differs from pins.json" % name)
    return True


def head_matches(have, want):
    if not COMMIT_RE.match(str(want)):
        raise PinRefused("pin %r is not a 40-hex commit" % (want,))
    if have != want:
        raise PinRefused("HEAD %s is not the pinned commit %s" % (have, want))
    return True


def content_key(pins):
    """Key of a build on this machine: the source pins and the build configuration, nothing else. Same key, same inputs."""
    doc = {"cts": pins["cts"], "externals_git": pins["externals_git"], "externals_files": pins.get("externals_files", {}),
           "fetch_scripts": pins.get("fetch_scripts", []), "build": pins["build"]}
    return hashlib.sha256(json.dumps(doc, sort_keys=True).encode()).hexdigest()[:16]


# ------------------------------------------------------------------------------------------------------------ caselist
def read_caselist(text):
    """`area TAB case` lines, `#` comments and blank lines ignored -> [(area, case)]. Raises CaselistError on any malformed line."""
    out = []
    for n, raw in enumerate(text.splitlines(), 1):
        line = raw.rstrip("\r")
        if not line.strip() or line.lstrip().startswith("#"):
            continue
        parts = line.split("\t")
        if len(parts) != 2:
            raise CaselistError("line %d: expected `area TAB case`" % n)
        area, case = parts
        if not AREA_RE.match(area):
            raise CaselistError("line %d: bad area %r" % (n, area))
        if not CASE_RE.match(case):
            raise CaselistError("line %d: bad case name %r" % (n, case))
        out.append((area, case))
    return out


def validate_caselist(entries, info_cases=(), required_areas=(), max_cases=MAX_CASES):
    """Duplicate-free, bounded, carries the info cases and at least one case in every required area. Returns the entries."""
    cases = [c for _, c in entries]
    dup = [c for c, n in collections.Counter(cases).items() if n > 1]
    if dup:
        raise CaselistError("duplicate cases: %s" % ", ".join(sorted(dup)[:5]))
    if not entries:
        raise CaselistError("empty case list")
    if len(entries) > max_cases:
        raise CaselistError("%d cases, more than the %d allowed (a few hundred at most)" % (len(entries), max_cases))
    for c in info_cases:
        if c not in cases:
            raise CaselistError("info case %s is missing" % c)
    have = {a for a, _ in entries}
    for a in required_areas:
        if a not in have:
            raise CaselistError("area %s has no case" % a)
    return entries


def caselist_sha256(text):
    return hashlib.sha256(text.encode()).hexdigest()


# ------------------------------------------------------------------------------------------------------------------ QPA
class QpaResult:
    """One case of a QPA log: status code string, detail, whether the block was complete, and (for kept cases) its body."""

    __slots__ = ("case", "status", "detail", "complete", "body")

    def __init__(self, case, status, detail, complete, body=None):
        self.case, self.status, self.detail, self.complete, self.body = case, status, detail, complete, body

    def as_tuple(self):
        return (self.case, self.status, self.detail, self.complete)


_RESULT_RE = re.compile(r'<Result\s+StatusCode="([A-Za-z]{1,40})"\s*>(.*?)(?:</Result>|$)')


def _unescape(text):
    return (text.replace("&lt;", "<").replace("&gt;", ">").replace("&quot;", '"').replace("&apos;", "'").replace("&amp;", "&"))


def parse_qpa(lines, keep_bodies=(), max_body=2 * 1024 * 1024, default_body=8192):
    """Stream a QPA log (an iterable of text lines) -> list of QpaResult in log order, plus the open case name if the log ends
    inside a case. A case that ends with #terminateTestCaseResult carries that status (Crash or Timeout); a case that is
    opened and never closed (the process died) is reported incomplete with status Crash and is also returned as `open_case`.
    A forged or repeated marker is data: the caller only accepts names from its own case list."""
    results, cur, status, detail, body, size, cap = [], None, None, "", None, 0, 0
    keep = set(keep_bodies)
    open_case = None
    for raw in lines:
        line = raw[:MAX_LINE].rstrip("\r\n")
        if line.startswith("#beginTestCaseResult "):
            if cur is not None:  # the previous block never closed: the process died or the log was cut
                results.append(QpaResult(cur, "Crash", "incomplete: next case began", False, body))
            cur = line[len("#beginTestCaseResult "):].strip()
            status, detail, body, size = None, "", [], 0
            cap = max_body if cur in keep else default_body
            continue
        if cur is None:
            continue
        if line.startswith("#endTestCaseResult"):
            if status is None:
                results.append(QpaResult(cur, "Crash", "no Result element", False, body))
            else:
                results.append(QpaResult(cur, status, detail, True, body))
            cur = None
            continue
        if line.startswith("#terminateTestCaseResult"):
            word = line[len("#terminateTestCaseResult"):].strip()
            results.append(QpaResult(cur, word if word in ("Crash", "Timeout") else "Crash", "terminated: %s" % word[:20], False, body))
            cur = None
            continue
        m = _RESULT_RE.search(line)
        if m:
            status, detail = m.group(1), _unescape(m.group(2))
        if body is not None and size < cap:
            body.append(line)
            size += len(line) + 1
    if cur is not None:
        results.append(QpaResult(cur, "Crash", "incomplete: log ends inside the case", False, body))
        open_case = cur
    return results, open_case


def parse_totals(text):
    """The console's `Test run totals` block -> {passed, failed, not_supported, warnings, waived, executed} or None."""
    out = {}
    for key, label in (("passed", "Passed"), ("failed", "Failed"), ("not_supported", "Not supported"), ("warnings", "Warnings"),
                       ("waived", "Waived")):
        m = re.search(r"^\s*%s:\s+(\d+)/(\d+)" % label, text, re.M)
        if not m:
            return None
        out[key] = int(m.group(1))
        out["executed"] = int(m.group(2))
    return out


def totals_agree(results, totals):
    """Cross-check of the QPA against the console totals of the same run: a truncated or edited QPA does not agree."""
    if totals is None:
        return None
    c = collections.Counter(classify(r.status) for r in results)
    return (c["pass"] == totals["passed"] and c["not_supported"] == totals["not_supported"] and c["warn"] == totals["warnings"] + totals["waived"]
            and c["fail"] + c["crash"] + c["timeout"] == totals["failed"] and len(results) == totals["executed"])


# --------------------------------------------------------------------------------------------------------- device info
_DEV_NAME = re.compile(r"deviceName\s*[=:]\s*\"?([^\"\n<&]{1,128})")
_DEV_TYPE = re.compile(r"deviceType\s*[=:]\s*(?:VK_PHYSICAL_DEVICE_TYPE_)?([A-Z_]{3,24})")
_DEV_VENDOR = re.compile(r"vendorID\s*[=:]\s*(0x[0-9a-fA-F]+|\d+)")
_DEV_API = re.compile(r"apiVersion\s*[=:]\s*([0-9a-fx.]+)")
_DEV_DRIVER = re.compile(r"driverVersion\s*[=:]\s*([0-9a-fx.]+)")
_EXT = re.compile(r"extensionName\s*[=:]\s*\"?(VK_[A-Za-z0-9_]+)")
GPU_TYPES = ("DISCRETE_GPU", "INTEGRATED_GPU", "VIRTUAL_GPU")


def device_info(property_lines, extension_lines):
    """The device the CTS ran on, from the bodies of dEQP-VK.info.device_properties and dEQP-VK.info.device_extensions."""
    text = "\n".join(property_lines or [])
    info = {}
    for key, rx in (("name", _DEV_NAME), ("type", _DEV_TYPE), ("vendor", _DEV_VENDOR), ("api", _DEV_API), ("driver", _DEV_DRIVER)):
        m = rx.search(text)
        info[key] = sanitize(_unescape(m.group(1)), 64).strip() if m else "-"
    info["extensions"] = sorted(set(_EXT.findall("\n".join(extension_lines or []))))
    return info


def device_is_gpu(info):
    """The row's identity assertion: the CTS must have run on a real GPU, never on a software rasteriser (lavapipe/llvmpipe)."""
    if info.get("type") not in GPU_TYPES:
        return False
    return not re.search(r"llvmpipe|lavapipe|swiftshader|softpipe", info.get("name", ""), re.I)


# ------------------------------------------------------------------------------------------------------------ execute
class BatchOutcome:
    """What one deqp-vk process did: results parsed from its QPA (QpaResult list), the case that was running when it ended (or None),
    the exit code, and whether the harness killed it (a hang) rather than the process dying by itself."""

    def __init__(self, results, open_case, rc, killed):
        self.results, self.open_case, self.rc, self.killed = results, open_case, rc, killed


def execute(cases, run_batch, batch_size, budget_s, clock, max_idle_restarts=6, keep_bodies=()):
    """The batch / resume loop. `run_batch(batch)` runs one deqp-vk over the cases of `batch` and returns a BatchOutcome.
    Returns (results, bodies, notes): results is an ordered {case: (status, detail)} for EVERY case of `cases`: a case that was
    never reached is NotRun (budget, restarts) and a case the binary does not know is NotRun too.
    Rules: a case that was running when the process died is Crash (Timeout when the harness killed it) and the loop continues after
    it in a new process; a clean exit that leaves cases without a result marks them NotRun; `max_idle_restarts` consecutive batches
    without any result stop the run (a wedged device must not burn the budget); the budget is checked before every batch."""
    results = collections.OrderedDict()
    bodies, notes = {}, []
    remaining = list(cases)
    wanted = set(cases)
    start, idle, batches = clock(), 0, 0
    reason = None
    while remaining:
        if clock() - start >= budget_s:
            reason = "budget"
            break
        batch = remaining[:batch_size]
        out = run_batch(batch)
        batches += 1
        inbatch = set(batch)
        got = 0
        for r in out.results:
            if r.case in inbatch and r.case not in results:
                status, detail = r.status, r.detail
                if not r.complete and status == "Crash" and out.killed and r.case == out.open_case:
                    status, detail = "Timeout", "killed by the harness"
                elif not r.complete and status == "Crash":
                    detail = "process ended inside the case rc=%s" % out.rc
                results[r.case] = (status, sanitize(detail))
                got += 1
                if r.body is not None and (r.case in keep_bodies or classify(status) not in OK_CLASSES):
                    bodies[r.case] = r.body
            elif r.case not in wanted:
                notes.append("unexpected case in log")
        if out.open_case and out.open_case in inbatch and out.open_case not in results:
            results[out.open_case] = ("Timeout", "killed by the harness") if out.killed else ("Crash", "process ended inside the case rc=%s" % out.rc)
            got += 1
        left = [c for c in batch if c not in results]
        if left and out.rc == 0 and not out.killed:
            for c in left:
                results[c] = ("NotRun", "no result from a clean run (unknown case?)")
            left = []
        remaining = [c for c in remaining if c not in results]
        if got:
            idle = 0
        else:
            idle += 1
            if idle > max_idle_restarts:
                reason = "idle"
                break
    for c in remaining:
        results[c] = ("NotRun", "not reached: %s" % (reason or "run ended"))
    ordered = collections.OrderedDict((c, results[c]) for c in cases)
    notes.append("batches=%d" % batches)
    if reason:
        notes.append("stopped=%s" % reason)
    return ordered, bodies, notes


# ---------------------------------------------------------------------------------------------------------------- block
def case_rows(results, areas):
    """[(case, status, class, area, detail)] in the order of `results`."""
    return [(c, s, classify(s), areas.get(c, "-"), d) for c, (s, d) in results.items()]


def digest(rows):
    """What host and guest must agree on: the class of every case. Details never enter it (they carry device specific text)."""
    h = hashlib.sha256()
    for case, _status, cls, _area, _detail in sorted(rows):
        h.update(("%s\t%s\n" % (case, cls)).encode())
    return h.hexdigest()[:16]


def render_block(side, rows):
    out = ["VKCTS_RESULTS_BEGIN %s side=%s" % (BLOCK_VERSION, side), "#case\tstatus\tclass\tarea\tdetail"]
    for case, status, cls, area, detail in rows:
        out.append("%s\t%s\t%s\t%s\t%s" % (case, status, cls, area, sanitize(detail)))
    out.append("VKCTS_RESULTS_END n=%d digest=%s" % (len(rows), digest(rows)))
    return "\n".join(out) + "\n"


def parse_block(text, expected_cases=None):
    """The first complete results block of `text` -> (side, rows, declared digest). Bounded and strict: a missing END, a count
    or digest that does not match, an unknown status or class, or a case outside `expected_cases` is a CaselistError."""
    rows, side, begun, ended = [], None, False, None
    for raw in text.splitlines():
        line = raw[:MAX_LINE]
        if not begun:
            m = re.match(r"VKCTS_RESULTS_BEGIN (\S+) side=(\S+)\s*$", line)
            if m and m.group(1) == BLOCK_VERSION:
                begun, side = True, m.group(2)
            continue
        if line.startswith("VKCTS_RESULTS_END"):
            m = re.match(r"VKCTS_RESULTS_END n=(\d+) digest=([0-9a-f]{16})\s*$", line)
            if not m:
                raise CaselistError("malformed END line")
            ended = (int(m.group(1)), m.group(2))
            break
        if line.startswith("#"):
            continue
        parts = line.split("\t")
        if len(parts) != 5:
            raise CaselistError("malformed row: %r" % line[:80])
        case, status, cls, area, detail = parts
        if not CASE_RE.match(case):
            raise CaselistError("bad case name in block: %r" % case[:80])
        if STATUS_CLASS.get(status) is None or classify(status) != cls:
            raise CaselistError("status/class mismatch for %s: %s/%s" % (case, status, cls))
        if expected_cases is not None and case not in expected_cases:
            raise CaselistError("case %s is not in the case list" % case)
        rows.append((case, status, cls, area, detail))
    if not begun:
        raise CaselistError("no results block")
    if ended is None:
        raise CaselistError("results block is not closed")
    if ended[0] != len(rows) or ended[1] != digest(rows):
        raise CaselistError("results block count or digest does not match its rows")
    if len({r[0] for r in rows}) != len(rows):
        raise CaselistError("duplicate case in block")
    return side, rows, ended[1]


def summarize(rows):
    """Counts per class and per area: {'total': {...}, 'areas': {area: {...}}}."""
    total = collections.Counter(r[2] for r in rows)
    areas = collections.OrderedDict()
    for _c, _s, cls, area, _d in rows:
        areas.setdefault(area, collections.Counter())[cls] += 1
    return {"total": {k: total.get(k, 0) for k in CLASSES}, "areas": {a: {k: v.get(k, 0) for k in CLASSES} for a, v in areas.items()}}


def summary_line(side, rows, info, gpu_verdict):
    s = summarize(rows)["total"]
    return ("VKCTS_SUMMARY side=%s cases=%d pass=%d warn=%d not_supported=%d fail=%d crash=%d timeout=%d not_run=%d device=%s type=%s gpu_use=%s"
            % (side, len(rows), s["pass"], s["warn"], s["not_supported"], s["fail"], s["crash"], s["timeout"], s["not_run"],
               sanitize(info.get("name", "-"), 48).replace(" ", "_"), info.get("type", "-"), gpu_verdict))


# ----------------------------------------------------------------------------------------------------------------- diff
CANDIDATES = ("GUEST_ONLY_FAIL", "GUEST_LOSES_SUPPORT", "GUEST_FAIL_HOST_NS", "GUEST_GAINS_SUPPORT")
CATEGORIES = ("SAME_OK", "SAME_NOT_SUPPORTED", "BOTH_FAIL", "BOTH_FAIL_DIFFERENT") + CANDIDATES + (
    "HOST_ONLY_FAIL", "HOST_FAIL_GUEST_NOT_SUPPORTED", "INCOMPLETE")


def categorize(h, g):
    """The comparison table. h and g are classes of the same case on the host and in the guest. The pass criterion of the row is
    the absence of CANDIDATES; a case that fails on both sides is not kayfabe's (the host run is the control)."""
    if h == "not_run" or g == "not_run":
        return "INCOMPLETE"
    if h in OK_CLASSES and g in OK_CLASSES:
        return "SAME_OK"
    if h == "not_supported" and g == "not_supported":
        return "SAME_NOT_SUPPORTED"
    if h in BAD_CLASSES and g in BAD_CLASSES:
        return "BOTH_FAIL" if h == g else "BOTH_FAIL_DIFFERENT"
    if h in OK_CLASSES:
        return "GUEST_ONLY_FAIL" if g in BAD_CLASSES else "GUEST_LOSES_SUPPORT"
    if h == "not_supported":
        return "GUEST_GAINS_SUPPORT" if g in OK_CLASSES else "GUEST_FAIL_HOST_NS"
    return "HOST_ONLY_FAIL" if g in OK_CLASSES else "HOST_FAIL_GUEST_NOT_SUPPORTED"


def diff_rows(host_rows, guest_rows):
    """Compare two result sets (lists of 5-tuples). A case present on one side only is `not_run` on the other.
    Returns (rows, summary): rows are (case, area, host_status, guest_status, category) in host order, then guest-only cases."""
    h = {r[0]: r for r in host_rows}
    g = {r[0]: r for r in guest_rows}
    order = list(h) + [c for c in g if c not in h]
    rows, count, per_area = [], collections.Counter(), collections.OrderedDict()
    for case in order:
        hr, gr = h.get(case), g.get(case)
        hs, gs = (hr[1] if hr else "NotRun"), (gr[1] if gr else "NotRun")
        area = (hr or gr)[3]
        cat = categorize(classify(hs), classify(gs))
        rows.append((case, area, hs, gs, cat))
        count[cat] += 1
        per_area.setdefault(area, collections.Counter())[cat] += 1
    cand = sum(count[c] for c in CANDIDATES)
    summary = {"cases": len(rows), "categories": {k: count.get(k, 0) for k in CATEGORIES}, "candidates": cand,
               "areas": {a: dict(v) for a, v in per_area.items()}}
    return rows, summary


def diff_lines(rows, summary, host_digest="-", guest_digest="-"):
    """The text of the machine-readable diff: a header, one TSV row per case that is not SAME_*, and the summary line."""
    out = ["#case\tarea\thost\tguest\tcategory"]
    for case, area, hs, gs, cat in rows:
        if not cat.startswith("SAME_"):
            out.append("%s\t%s\t%s\t%s\t%s" % (case, area, hs, gs, cat))
    cats = summary["categories"]
    out.append("VKCTS_DIFF cases=%d candidates=%d %s host_digest=%s guest_digest=%s" % (
        summary["cases"], summary["candidates"], " ".join("%s=%d" % (k.lower(), cats[k]) for k in CATEGORIES), host_digest, guest_digest))
    return out


def extension_diff(host_info, guest_info):
    """Extensions the host exposes and the guest does not (and the reverse): the usual root cause of GUEST_LOSES_SUPPORT."""
    h, g = set(host_info.get("extensions", [])), set(guest_info.get("extensions", []))
    return sorted(h - g), sorted(g - h)
