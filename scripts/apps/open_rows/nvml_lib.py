#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later
"""Logic of the NVML sweep and of the DCGM row: parsers and the host/guest diff (stdlib only; GPU-free tested).

STATUS: LIVE, 2026-11-17 (design: docs/design/V3_OPEN_ROWS_FFMPEG_DCGM.md). Parsers are tested on fixtures written to the formats
the tools document (fake GPU UUID prefix fa4efa4e-fa4e-4fa4-8fa4-fa4efa4e...); native outputs of two Vast boxes are in
traces/open_rows_ffdcgm_20261117/ (when that directory exists). No kayfabe guest has run it.

The rule the row enforces (owner, 2026-11-17): every NVML field, query and nvidia-smi value that the HOST answers with a number must be a
number in the guest too; a guest that answers `[N/A]` / NOT_SUPPORTED where the host answered a number is a FINDING, unless the exact
value is listed in the committed table nvml_expected_not_supported.tsv with a reason. Live counters are equal only within a tolerance
(nvml_field_classes.tsv); static values are equal. A field the driver does not know at all is not listed (the sweep asks the driver).

All records are rows `kind TAB key TAB label TAB status TAB value`:
    kind    field (NVML field value id), query (a typed NVML call), smi (a leaf of `nvidia-smi -q -x`), smiq (nvidia-smi --query-gpu=),
            dcgm (a DCGM field from `dcgmi dmon`), diag (a `dcgmi diag` test of one GPU)
    status  OK | NA | NOT_SUPPORTED | NO_PERMISSION | INVALID_ARGUMENT | UNKNOWN_FIELD | NO_SYMBOL | ERR_<name> | PASS | FAIL | WARN | SKIP
"""
import fnmatch
import json
import re
import xml.etree.ElementTree as ET

KINDS = ("field", "query", "smi", "smiq", "dcgm", "diag")
COLUMNS = ("kind", "key", "label", "status", "value")
CLASSES = ("static", "live", "numeric", "ignore")
NA_TEXT = {"n/a", "[n/a]", "not supported", "[not supported]", "[not found]", "not found", "unknown error", "[unknown error]", "-", ""}
# identity: never recorded as a value, never compared (UUIDs, serials, bus ids differ by construction between host and guest)
IDENTITY = re.compile(r"(^|/)(uuid|serial|serial_number|board_id|minor_number|pci_bus_id|pci_bus|pci_device|pci_domain|pci_device_id|"
                      r"pci_sub_system_id|multigpu_board|gpu_uuid|pci_device_id|domain|bus|device|function|id)$|pci_bus_id|uuid", re.I)


PCI_ADDRESS = re.compile(r"\b[0-9a-fA-F]{4,8}:[0-9a-fA-F]{2}:[0-9a-fA-F]{2}\.[0-7]\b")
UUID_LIKE = re.compile(r"\b(?:GPU|MIG)-[0-9a-fA-F-]{8,}|\b[0-9a-fA-F]{8}-[0-9a-fA-F]{4}-[0-9a-fA-F]{4}-[0-9a-fA-F]{4}-[0-9a-fA-F]{12}\b")


class TableError(ValueError):
    pass


# --------------------------------------------------------------------------------------------------- values and rows
def is_na_text(text):
    return text.strip().lower() in NA_TEXT


_NUM = re.compile(r"^\s*(-?\d+(?:\.\d+)?)(?:\s*[A-Za-z%/]+(?:\s*[A-Za-z%/]+)?)?\s*$")


def number_of(text):
    """The number in a value cell like '43', '43 %', '250.00 W', '0x1f' or '0.5' -> float, else None. 'N/A' is not a number."""
    t = text.strip()
    if re.fullmatch(r"0[xX][0-9a-fA-F]+", t):
        return float(int(t, 16))
    m = _NUM.match(t)
    return float(m[1]) if m else None


def numeric_parts(value):
    """A value is one number or `a=1;b=2` parts: -> {name: float} when EVERY part is a number, else None (empty -> None)."""
    if value is None or value == "" or value == "-":
        return None
    out = {}
    for i, part in enumerate(value.split(";")):
        name, eq, v = part.partition("=")
        if not eq:
            name, v = "v", part
        n = number_of(v)
        if n is None:
            return None
        out[name if eq else ("v" if i == 0 else "v%d" % i)] = n
    return out or None


def row_line(kind, key, label, status, value):
    clean = lambda s: re.sub(r"[\t\r\n]+", " ", str(s))[:300]
    return "\t".join((kind, clean(key), clean(label) or "-", clean(status), clean(value) if value not in (None, "") else "-"))


def parse_rows(text):
    rows = {}
    for line in text.splitlines():
        cells = line.split("\t")
        if len(cells) != 5 or cells[0] not in KINDS:
            continue
        rows[(cells[0], cells[1])] = dict(zip(COLUMNS, cells))
    return rows


# -------------------------------------------------------------------------------------------- nvidia-smi -q -x
def flatten_smi_xml(text):
    """-> {path: leaf text}. Repeated siblings get [n]. Identity leaves (uuid, serial, bus ids) are returned as '<scrubbed>' (still present,
    so a host/guest diff sees that the guest has the field, but their values are never kept)."""
    root = ET.fromstring(re.sub(r"<!DOCTYPE[^>]*>", "", text))
    out = {}

    def walk(el, path):
        kids = list(el)
        if not kids:
            out[path] = (el.text or "").strip()
            return
        seen = {}
        for k in kids:
            seen[k.tag] = seen.get(k.tag, 0) + 1
        idx = {}
        for k in kids:
            i = idx.get(k.tag, 0)
            idx[k.tag] = i + 1
            walk(k, "%s/%s%s" % (path, k.tag, "[%d]" % i if seen[k.tag] > 1 else ""))
        for name, val in el.attrib.items():
            out["%s/@%s" % (path, name)] = val

    walk(root, root.tag)
    for k in list(out):
        attr = k.rpartition("/@")[2] if "/@" in k else ""
        if IDENTITY.search(k.split("/@")[0].rsplit("/", 1)[-1]) or IDENTITY.search(k) or attr and IDENTITY.search("/" + attr) \
                or PCI_ADDRESS.search(out[k]) or UUID_LIKE.search(out[k]):
            out[k] = "<scrubbed>"
    return out


def smi_rows(flat):
    rows = []
    for k, v in sorted(flat.items()):
        if v == "<scrubbed>":
            rows.append(row_line("smi", k, "-", "OK", "<scrubbed>"))
        elif is_na_text(v) and v.strip():
            rows.append(row_line("smi", k, "-", "NA", v))
        else:
            rows.append(row_line("smi", k, "-", "OK", v))
    return rows


def parse_help_query_gpu(text):
    """Names of nvidia-smi --help-query-gpu: lines like '"timestamp"' (quoted at line start). Only the quoted property names."""
    names = []
    for line in text.splitlines():
        m = re.match(r'^"([a-z0-9_.\[\]]+)"\s*$', line.strip())
        if m and m[1] not in names:
            names.append(m[1])
    return names


def smiq_rows(names, csv_line):
    """names: the fields asked in one call; csv_line: its single `--format=csv,noheader,nounits` output line."""
    cells = [c.strip() for c in csv_line.strip().split(",")]
    if len(cells) != len(names):
        raise ValueError("%d values for %d fields" % (len(cells), len(names)))
    rows = []
    for n, c in zip(names, cells):
        if IDENTITY.search(n) or n in ("serial", "uuid", "pci.bus_id", "pci.bus", "pci.device", "pci.domain", "index"):
            rows.append(row_line("smiq", n, "-", "OK", "<scrubbed>"))
        elif is_na_text(c):
            rows.append(row_line("smiq", n, "-", "NA", c or "-"))
        else:
            rows.append(row_line("smiq", n, "-", "OK", c))
    return rows


# ------------------------------------------------------------------------------ committed tables: classes, expected NS
def load_classes(text):
    out = []
    for line in text.splitlines():
        if not line.strip() or line.startswith("#") or line.startswith("glob\t"):
            continue
        cells = line.split("\t")
        if len(cells) != 3 or cells[1] not in CLASSES:
            raise TableError("bad class line: %r" % line)
        glob, cls, param = cells
        if cls == "live" and not re.fullmatch(r"(abs|rel):\d+(\.\d+)?", param):
            raise TableError("live needs abs:N or rel:F, got %r" % param)
        out.append((glob, cls, param))
    return out


def classify(kind, key, classes):
    """First matching glob over 'kind:key' wins; the default is `numeric` (only that it is a number is asserted)."""
    for glob, cls, param in classes:
        if fnmatch.fnmatchcase("%s:%s" % (kind, key), glob):
            return cls, param
    return "numeric", "-"


def load_expected_ns(text):
    out = []
    for line in text.splitlines():
        if not line.strip() or line.startswith("#") or line.startswith("glob\t"):
            continue
        cells = line.split("\t")
        if len(cells) != 3 or not cells[2].strip():
            raise TableError("bad expected-NOT_SUPPORTED line (glob, group, reason required): %r" % line)
        out.append(tuple(cells))
    return out


def expected_ns(kind, key, table):
    for glob, group, reason in table:
        if fnmatch.fnmatchcase("%s:%s" % (kind, key), glob):
            return group, reason
    return None


def within(h, g, param):
    mode, _, n = param.partition(":")
    tol = float(n)
    if mode == "abs":
        return abs(h - g) <= tol
    return abs(h - g) <= tol * max(abs(h), abs(g), 1e-9)


# ------------------------------------------------------------------------------------------------- host/guest diff
FINDING_STATUSES = ("FINDING_NA_IN_GUEST", "FINDING_GUEST_MISSING", "FINDING_DIFF_STATIC", "FINDING_DIFF_LIVE", "FINDING_PART_MISSING")


def compare_row(h, g, classes, table):
    """-> (status, detail). h is the host control's row, g the guest's (None when absent). See the module docstring for the rule."""
    kind, key = h["kind"], h["key"]
    cls, param = classify(kind, key, classes)
    if cls == "ignore":
        return "IGNORED", ""
    hn = numeric_parts(h["value"]) if h["status"] == "OK" else None
    gn = numeric_parts(g["value"]) if g is not None and g["status"] == "OK" else None
    if hn is not None:
        if g is None:
            ex = expected_ns(kind, key, table)
            return ("EXPECTED_NS", ex[1]) if ex else ("FINDING_GUEST_MISSING", "host %s, guest has no row" % h["value"])
        if gn is None:
            ex = expected_ns(kind, key, table)
            if ex:
                return "EXPECTED_NS", "%s: %s" % ex
            return "FINDING_NA_IN_GUEST", "host=%s guest=%s/%s" % (h["value"], g["status"], g["value"])
        missing = sorted(set(hn) - set(gn))
        if missing:
            return "FINDING_PART_MISSING", "guest lacks %s" % ",".join(missing)
        if cls == "static":
            bad = [p for p in hn if hn[p] != gn[p]]
            if bad:
                return "FINDING_DIFF_STATIC", "; ".join("%s host=%g guest=%g" % (p, hn[p], gn[p]) for p in bad)
        elif cls == "live":
            bad = [p for p in hn if not within(hn[p], gn[p], param)]
            if bad:
                return "FINDING_DIFF_LIVE", "; ".join("%s host=%g guest=%g tol=%s" % (p, hn[p], gn[p], param) for p in bad)
        ex = expected_ns(kind, key, table)
        return ("OK_SERVED_THOUGH_LISTED", ex[1]) if ex else ("OK", "")
    # the host did not answer a number
    if g is not None and gn is not None:
        return "GUEST_ONLY_NUMBER", "host=%s/%s guest=%s" % (h["status"], h["value"], g["value"])
    if h["status"] == "OK" and g is not None and g["status"] == "OK" and cls == "static" and h["value"] != g["value"]:
        return "TEXT_DIFF", "host=%s guest=%s" % (h["value"], g["value"])
    return "BOTH_NOT_NUMBER", ""


def compare_sweeps(host_rows, guest_rows, classes, table):
    """host_rows, guest_rows: dicts as parse_rows returns. Walks the HOST's rows (the control decides what is owed). -> list of dicts and counts."""
    out, counts = [], {}
    for key in sorted(host_rows):
        st, detail = compare_row(host_rows[key], guest_rows.get(key), classes, table)
        counts[st] = counts.get(st, 0) + 1
        out.append({"kind": key[0], "key": key[1], "status": st, "detail": detail})
    for key in sorted(set(guest_rows) - set(host_rows)):
        counts["GUEST_EXTRA"] = counts.get("GUEST_EXTRA", 0) + 1
    return out, counts


def findings(counts):
    return sum(v for k, v in counts.items() if k in FINDING_STATUSES)


# --------------------------------------------------------------------------------------------- DCGM output parsers
def parse_dcgm_discovery(text):
    """`dcgmi discovery -l`: -> number of GPUs ('N GPUs found.'), or None."""
    m = re.search(r"(\d+)\s+GPUs?\s+found", text)
    return int(m[1]) if m else None


def parse_dcgm_dmon_list(text):
    """`dcgmi dmon -l`: -> [(field id, name)]. Lines are `<name>   <id>` under a `Long Name  Field ID` header (DCGM 4.8, a native run)."""
    out = []
    for line in text.splitlines():
        m = re.match(r"^\s*([A-Za-z][A-Za-z0-9_.]*)\s+(\d+)\s*$", line)
        if m and m[1] != "Field":
            out.append((int(m[2]), m[1]))
    return out


DCGM_NA = {"n/a", "blank", "not", "found", "-", "[n/a]"}


def parse_dcgm_dmon(text, tags):
    """`dcgmi dmon -e ids -c N`: -> {tag: [raw cell per sample]} for GPU 0. `tags` name the columns in the order of -e. The header lines
    (`#Entity 50 150 ...` and the units row `ID C W`) are skipped; a data row is `GPU 0 <one cell per field>`, an unavailable value
    is printed `N/A`."""
    series = {t: [] for t in tags}
    for line in text.splitlines():
        m = re.match(r"^\s*GPU\s+(\d+)\s+(.*)$", line)
        if not m or m[1] != "0":
            continue
        cells = m[2].split()
        if len(cells) < len(tags):
            continue
        for t, c in zip(tags, cells):
            series[t].append(c)
    return series


def dmon_error(text):
    """The refusal `dcgmi dmon` prints when a watch cannot be set (a whole group fails with it): -> (code, message) or None."""
    m = re.search(r"Error[^\n]*?Result:\s*(-?\d+):\s*([^\n]*)", text)
    return (int(m[1]), m[2].strip()[:120]) if m else None


def dmon_error_row(tag, text):
    err = dmon_error(text)
    if err is None:
        return row_line("dcgm", tag, "-", "ERR_NO_TABLE", (text.strip().splitlines() or ["no output"])[-1][:100])
    code, msg = err
    return row_line("dcgm", tag, "-", "NOT_SUPPORTED" if "not supported" in msg.lower() else "ERR_%d" % code, "%d: %s" % (code, msg))


def dmon_rows(series):
    rows = []
    for tag, cells in series.items():
        real = [c for c in cells if c.lower() not in DCGM_NA]
        if not cells:
            rows.append(row_line("dcgm", tag, "-", "NA", "no sample"))
        elif real:
            rows.append(row_line("dcgm", tag, "-", "OK", real[-1]))
        else:
            rows.append(row_line("dcgm", tag, "-", "NA", "N/A"))
    return rows


def parse_diag_json(text):
    """`dcgmi diag -r N -j`: -> [(category, test name, gpu id, status, info)]. Tolerant of both layouts DCGM has used: a test is a dict with
    `name` and `results` (list of dicts with `status`, `gpu_id` or `entity_id`, `info`/`warnings`)."""
    try:
        doc = json.loads(text)
    except ValueError:
        i = text.find("{")
        try:
            doc = json.loads(text[i:]) if i >= 0 else None
        except ValueError:
            doc = None
    out = []

    def walk(node, category):
        if isinstance(node, dict):
            if "results" in node and "name" in node and isinstance(node["results"], list):
                for r in node["results"]:
                    if isinstance(r, dict) and "status" in r:
                        gpu = r.get("gpu_id", r.get("entity_id", "-"))
                        info = r.get("info", r.get("warnings", ""))
                        if isinstance(info, (list, dict)):
                            info = json.dumps(info)[:160]
                        out.append((category, str(node["name"]), str(gpu), str(r["status"]), str(info)[:160]))
                return
            cat = node.get("category", category)
            for v in node.values():
                walk(v, cat)
        elif isinstance(node, list):
            for v in node:
                walk(v, category)

    walk(doc, "-")
    return out


def diag_rows(tests):
    rows = []
    for cat, name, gpu, status, info in tests:
        st = status.strip().upper()
        st = {"PASS": "PASS", "FAIL": "FAIL", "WARN": "WARN", "SKIP": "SKIP", "SKIPPED": "SKIP", "NOT RUN": "SKIP"}.get(st, "FAIL" if "FAIL" in st else st)
        rows.append(row_line("diag", "%s/%s/gpu%s" % (cat, name, gpu), "-", st, info or "-"))
    return rows


def compare_diag(host_rows, guest_rows):
    """A test the host PASSES must not FAIL in the guest; every other pair is listed. -> list of (key, host status, guest status, verdict)."""
    out = []
    for key, h in sorted(host_rows.items()):
        if key[0] != "diag":
            continue
        g = guest_rows.get(key)
        gs = g["status"] if g else "MISSING"
        if h["status"] == "PASS" and gs in ("FAIL", "MISSING"):
            v = "FINDING_DIAG_FAIL_IN_GUEST"
        elif h["status"] == gs:
            v = "AGREE"
        else:
            v = "DIFFER"
        out.append((key[1], h["status"], gs, v))
    return out
