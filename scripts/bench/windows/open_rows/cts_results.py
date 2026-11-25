# SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later
"""Parsers and the native-vs-guest comparison of the VK-GL-CTS rows (STATUS: DESIGN-ONLY, 2026-11-17; pure functions, GPU-free).

Formats are the ones of the pinned CTS source (CHECKED in framework/qphelper/qpTestLog.c and framework/common/tcuTestSessionExecutor.cpp):
  qpa log      #beginTestCaseResult <path> ... <Result StatusCode="Pass">detail</Result> ... #endTestCaseResult
               #terminateTestCaseResult <Status>   (a case cut short by the crash handler or the watchdog)
  console      "\\nTest case '<path>'..\\n" then "  <Status> (<description>)\\n"
Status names: Pass Fail QualityWarning CompatibilityWarning Pending NotSupported ResourceError InternalError Crash Timeout Waiver DeviceLost ...

The per-case results file is `case<TAB>status` under a `# kfopen-results/1` header. PASS of a row is "the per-case list equals the control":
the same binary built from the same pinned commit, run on native Windows on the same die and driver (docs/design/V3_WINDOWS_OPEN_ROWS.md).
"""
import re
from collections import Counter, namedtuple

from kfopen_common import OpenRowError, scrub_uuids

Case = namedtuple("Case", "name status detail")
OKISH = frozenset({"Pass", "QualityWarning", "CompatibilityWarning", "Waiver"})
BAD = frozenset({"Fail", "Crash", "Timeout", "DeviceLost", "InternalError", "ResourceError", "Pending"})
KNOWN = OKISH | BAD | {"NotSupported", "EnforceDefaultContext", "EnforceDefaultInstance", "CapabilityWarning"}
HEADER = "# kfopen-results/1"

BEGIN_RX = re.compile(r"^#beginTestCaseResult (\S+)\s*$")
END_RX = re.compile(r"^#endTestCaseResult\s*$")
TERM_RX = re.compile(r"^#terminateTestCaseResult (\S+)\s*$")
RESULT_RX = re.compile(r'<Result StatusCode="([A-Za-z]+)"\s*>(.*?)</Result>', re.S)
CON_CASE_RX = re.compile(r"^Test case '([^']+)'\.\.\s*$")
CON_RES_RX = re.compile(r"^  ([A-Za-z]+) \((.*)\)\s*$")


def parse_qpa(text):
    """(cases, inflight): every finished case in file order, and the name of a case that began and never ended (None if none).

    A case that began and has no #end is reported by the caller as Crash or Timeout; the parser only says which one it was."""
    cases, cur, status, detail, buf = [], None, None, "", []
    for line in text.splitlines():
        m = BEGIN_RX.match(line)
        if m:
            if cur is not None:                         # a new case began while one was open: the old one was cut off silently
                cases.append(Case(cur, status or "Crash", detail or "no #endTestCaseResult before the next case"))
            cur, status, detail, buf = m.group(1), None, "", []
            continue
        if cur is None:
            continue
        if END_RX.match(line):
            blob = "\n".join(buf)
            r = RESULT_RX.search(blob)
            cases.append(Case(cur, r.group(1) if r else "InternalError", (r.group(2).strip() if r else "no <Result> element")[:200]))
            cur, status, detail, buf = None, None, "", []
            continue
        t = TERM_RX.match(line)
        if t:
            cases.append(Case(cur, t.group(1), "terminated"))
            cur, status, detail, buf = None, None, "", []
            continue
        buf.append(line)
    return cases, cur


def parse_console(text):
    """Cases from deqp's stdout (the fallback when a qpa file is lost). Same shape as parse_qpa()'s first value."""
    out, cur = [], None
    for line in text.splitlines():
        m = CON_CASE_RX.match(line)
        if m:
            cur = m.group(1)
            continue
        r = CON_RES_RX.match(line)
        if r and cur and r.group(1) in KNOWN:
            out.append(Case(cur, r.group(1), r.group(2)[:200]))
            cur = None
    return out


# ------------------------------------------------------------------------------------------------ results files
def format_results(header, results):
    """header: {key: value}; results: {case: (status, detail)} or {case: status}. Deterministic text, sorted by case name."""
    lines = [HEADER, "# " + " ".join("%s=%s" % (k, header[k]) for k in sorted(header))]
    for name in sorted(results):
        st = results[name]
        lines.append("%s\t%s" % (name, st[0] if isinstance(st, tuple) else st))
    return "\n".join(lines) + "\n"


def format_details(results):
    """Fail details, UUID-scrubbed and cut: evidence for a person, never compared."""
    rows = ["%s\t%s\t%s" % (n, st, scrub_uuids(d).replace("\t", " ").replace("\n", " ")[:200])
            for n, (st, d) in sorted(results.items()) if st not in ("Pass",) and d]
    return "\n".join(rows) + ("\n" if rows else "")


def read_results(text):
    """(header dict, {case: status}). Refuses a file that is not a kfopen results file, a duplicate case or an unknown status shape."""
    lines = text.splitlines()
    if not lines or lines[0] != HEADER:
        raise OpenRowError("RESULTS_FORMAT not a kfopen-results/1 file")
    header = {}
    for kv in (lines[1][2:].split() if len(lines) > 1 and lines[1].startswith("# ") else []):
        k, _, v = kv.partition("=")
        header[k] = v
    out = {}
    for line in lines[2:]:
        if not line.strip() or line.startswith("#"):
            continue
        name, sep, status = line.partition("\t")
        if not sep or not re.fullmatch(r"[A-Za-z]+", status.strip()):
            raise OpenRowError("RESULTS_FORMAT bad line %r" % line[:60])
        if name in out:
            raise OpenRowError("RESULTS_DUPLICATE %s" % name)
        out[name] = status.strip()
    return header, out


def summarize(results, depth=3):
    """({status: count}, {group: {status: count}}) with the group being the first `depth` dotted parts of the case path minus the case itself."""
    total, groups = Counter(), {}
    for name, st in results.items():
        status = st[0] if isinstance(st, tuple) else st
        total[status] += 1
        g = ".".join(name.split(".")[:depth])
        groups.setdefault(g, Counter())[status] += 1
    return dict(total), {g: dict(c) for g, c in sorted(groups.items())}


# ------------------------------------------------------------------------------------------------ native vs guest
def classify(native, guest):
    if native == guest:
        return "EQUAL"
    if native in OKISH and guest == "NotSupported":
        return "GUEST_LOSES_FEATURE"
    if native in OKISH and guest in BAD:
        return "GUEST_REGRESSION"
    if native == "NotSupported" and guest in OKISH:
        return "GUEST_GAINS_FEATURE"
    if native in BAD and guest in OKISH:
        return "NATIVE_ONLY_FAIL"
    if native in BAD and guest in BAD:
        return "DIFFERENT_FAILURE"
    if native in OKISH and guest in OKISH:
        return "QUALITY_DIFF"
    return "OTHER_DIFF"


CANDIDATE_CLASSES = ("GUEST_REGRESSION", "GUEST_LOSES_FEATURE")


def diff_results(native, guest):
    """The comparison the rows are judged by. `native` is the control, `guest` the run under test; both {case: status}.

    verdict: PASS (same cases, same statuses) | FAIL (same cases, some differ) | INCOMPLETE (a case is missing on one side: no verdict)."""
    only_n = sorted(set(native) - set(guest))
    only_g = sorted(set(guest) - set(native))
    rows, counts = [], Counter()
    for name in sorted(set(native) & set(guest)):
        k = classify(native[name], guest[name])
        counts[k] += 1
        if k != "EQUAL":
            rows.append((name, native[name], guest[name], k))
    if only_n or only_g:
        verdict = "INCOMPLETE"
    elif rows:
        verdict = "FAIL"
    else:
        verdict = "PASS"
    return {"verdict": verdict, "counts": dict(counts), "rows": rows, "missing_in_guest": only_n, "missing_in_native": only_g,
            "candidates": [r for r in rows if r[3] in CANDIDATE_CLASSES], "cases": len(set(native) | set(guest))}


def format_diff(d):
    lines = ["# kfopen-diff/1 verdict=%s cases=%d" % (d["verdict"], d["cases"]),
             "# counts " + " ".join("%s=%d" % kv for kv in sorted(d["counts"].items()))]
    lines += ["%s\tnative=%s\tguest=%s\t%s" % r for r in d["rows"]]
    lines += ["%s\tMISSING_IN_GUEST" % n for n in d["missing_in_guest"]]
    lines += ["%s\tMISSING_IN_NATIVE" % n for n in d["missing_in_native"]]
    return "\n".join(lines) + "\n"
