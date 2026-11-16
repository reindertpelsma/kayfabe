#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later
"""Fail closed when the Worker's bandwidth table (ci/vast-broker/bandwidth.json) is incomplete, stale or malformed (stdlib only).

STATUS: DESIGN-ONLY, 2026-11-16 (docs/design/V3_WORKER_PROPIMAGES.md, "Bandwidth"). The table is data a person edits; this is the
guard that makes editing it safe. GPU-free tests: scripts/ci/test_bandwidth_check.py; mutants: scripts/ci/bandwidth_mutants.py.

    python3 -B -I scripts/ci/bandwidth_check.py                      check the committed table (exit 1 and one line per problem)
    python3 -B -I scripts/ci/bandwidth_check.py --explain FILE       what the table makes of an instruction file (JSON), the Python
                                                                     twin of src/bandwidth.js (test/bandwidth.test.js compares them)

Checks (one named token per problem, `CODE:where`):
  UNKNOWN_FIELD  a key the format does not have            MISSING   a required field is absent
  CLASS / BASIS  not one of the known values               DATE      not a real date, or in the future
  SOURCE / URL   empty                                     ESTIMATE_NO_NOTE   an ESTIMATE with no note saying what it rests on
  ESTIMATE_STALE an ESTIMATE older than 60 days            BYTES     not a non-negative integer
  SIZE_SOURCE / SIZE_FROM   no size, a size copied from a pin file, or a reference that does not resolve
  WHEN           a condition with an unknown key or value  DUPLICATE_ID
  STAGE_MISSING  a stage of the payload that the table does not name (a new stage cannot be forgotten)
  STAGE_UNKNOWN  a table stage the payload no longer has   EMPTY_STAGE        a stage with neither entries nor a note
  CALIBRATE_MANY more than one entry of a stage takes the calibrated remainder
"""

import argparse
import datetime
import json
from pathlib import Path
import re
import sys

HERE = Path(__file__).resolve().parent
ROOT = HERE.parents[1]
DATA = ROOT / "ci/vast-broker/bandwidth.json"
PROPS = ROOT / "ci/vast-broker/propimages.json"
MATRIX = ROOT / "ci/vast-broker/release-matrix.json"
PAYLOAD = ROOT / "scripts/ci/vast_payload.py"

CLASSES = ("r2-pinned", "r2-listed", "official-url", "package", "repo")
BASES = ("measured", "ESTIMATE")
MAX_ESTIMATE_AGE_DAYS = 60
MAX_BYTES = 1 << 50
TOP_KEYS = {"schema", "status", "margin", "default_images", "fixed", "stages"}
MARGIN_KEYS = {"percent", "basis", "date", "source", "note"}
STAGE_KEYS = {"note", "entries", "upload"}
UPLOAD_KEYS = {"up_bytes", "basis", "date", "source", "note"}
ENTRY_KEYS = {"id", "what", "class", "down_bytes", "size_from", "per_image", "url", "basis", "date", "source", "note", "when", "calibrate"}
ENTRY_REQUIRED = {"id", "what", "class", "basis", "date", "source", "url"}
WHEN_KEYS = {"tier", "winapps", "spec", "linux_apps", "images"}
ID_RE = re.compile(r"[a-z][a-z0-9_]*\.[a-z][a-z0-9_]*")
DATE_RE = re.compile(r"\d{4}-\d{2}-\d{2}")


def _count(n):
    return type(n) is int and 0 <= n <= MAX_BYTES


def _date(text, today):
    if type(text) is not str or not DATE_RE.fullmatch(text):
        return None
    try:
        day = datetime.date.fromisoformat(text)
    except ValueError:
        return None
    return day if day <= today else None


def _provenance(errors, where, row, keys, today):
    """The fields every number carries: basis, date, source, and a note when it is an ESTIMATE."""
    for key in ("basis", "date", "source"):
        if key not in row:
            errors.append("MISSING:%s:%s" % (where, key))
    if row.get("basis") not in BASES and "basis" in row:
        errors.append("BASIS:" + where)
    day = _date(row.get("date"), today) if "date" in row else None
    if "date" in row and day is None:
        errors.append("DATE:" + where)
    if "source" in row and not (type(row["source"]) is str and row["source"].strip()):
        errors.append("SOURCE:" + where)
    if row.get("basis") == "ESTIMATE":
        if not (type(row.get("note")) is str and row["note"].strip()):
            errors.append("ESTIMATE_NO_NOTE:" + where)
        if day is not None and (today - day).days > MAX_ESTIMATE_AGE_DAYS:
            errors.append("ESTIMATE_STALE:" + where)


def resolve_size(entry, props, matrix):
    """(kind, name) of a size_from reference, or None when it does not resolve against the data files."""
    ref = entry.get("size_from")
    if type(ref) is not str or ":" not in ref:
        return None
    kind, name = ref.split(":", 1)
    if kind == "propimage":
        objects = {o["id"]: o for o in props["objects"]}
        if name == "image":
            return (kind, name) if entry.get("per_image") is True else None
        return (kind, name) if name in objects and not entry.get("per_image") else None
    if kind == "driver":
        pins = {p["os"] + "/" + p["version"]: p for p in matrix.get("driver_installers", [])}
        return (kind, name) if name in pins and type(pins[name].get("size")) is int else None
    if kind == "r2":
        return (kind, name) if name and entry.get("down_bytes") is not None else None
    return None


def _entry(errors, stage, entry, seen, props, matrix, today):
    if type(entry) is not dict:
        errors.append("UNKNOWN_FIELD:%s:entry" % stage)
        return
    name = str(entry.get("id", "?"))[:60]
    for key in sorted(set(entry) - ENTRY_KEYS):
        errors.append("UNKNOWN_FIELD:%s:%s" % (name, key))
    for key in sorted(ENTRY_REQUIRED - set(entry)):
        errors.append("MISSING:%s:%s" % (name, key))
    if "id" in entry:
        if not (type(entry["id"]) is str and ID_RE.fullmatch(entry["id"])) or not entry["id"].startswith(stage + "."):
            errors.append("MISSING:%s:id" % name)
        if entry["id"] in seen:
            errors.append("DUPLICATE_ID:" + name)
        seen.add(entry["id"])
    if "class" in entry and entry["class"] not in CLASSES:
        errors.append("CLASS:" + name)
    if "what" in entry and not (type(entry["what"]) is str and entry["what"].strip()):
        errors.append("MISSING:%s:what" % name)
    if "url" in entry and entry["url"] is not None and not (type(entry["url"]) is str and entry["url"].strip()):
        errors.append("URL:" + name)
    if entry.get("url") is None and entry.get("class") not in (None, "package") and "url" in entry:
        errors.append("URL:" + name)
    _provenance(errors, name, entry, ENTRY_KEYS, today)
    ref = entry.get("size_from")
    if "down_bytes" not in entry and ref is None:
        errors.append("SIZE_SOURCE:" + name)
    if "down_bytes" in entry and not _count(entry["down_bytes"]):
        errors.append("BYTES:" + name)
    if ref is not None:
        resolved = resolve_size(entry, props, matrix)
        if resolved is None:
            errors.append("SIZE_FROM:" + name)
        elif resolved[0] in ("propimage", "driver") and "down_bytes" in entry:
            errors.append("SIZE_SOURCE:" + name)          # a pinned size is read from its pin, never copied beside it
    when = entry.get("when")
    if when is not None:
        labels = {i["label"] for i in props["images"]}
        ok = type(when) is dict and set(when) <= WHEN_KEYS and all(type(v) is list and v and all(type(x) is str for x in v) for v in when.values())
        if ok and "images" in when and not set(when["images"]) <= labels:
            ok = False
        if not ok:
            errors.append("WHEN:" + name)
    if "calibrate" in entry and entry["calibrate"] is not True:
        errors.append("MISSING:%s:calibrate" % name)
    if "per_image" in entry and entry["per_image"] is not True:
        errors.append("MISSING:%s:per_image" % name)


def validate(data, stage_names, props, matrix, today):
    errors = []
    if type(data) is not dict:
        return ["UNKNOWN_FIELD:top"]
    for key in sorted(set(data) - TOP_KEYS):
        errors.append("UNKNOWN_FIELD:" + key)
    for key in sorted(TOP_KEYS - set(data)):
        errors.append("MISSING:top:" + key)
    if errors:
        return errors
    if data["schema"] != "kayfabe.bandwidth/1":
        errors.append("MISSING:top:schema")
    margin = data["margin"]
    if type(margin) is not dict or set(margin) - MARGIN_KEYS:
        errors.append("UNKNOWN_FIELD:margin")
    else:
        if type(margin.get("percent")) not in (int, float) or not 0 <= margin["percent"] <= 100:
            errors.append("BYTES:margin.percent")
        _provenance(errors, "margin", margin, MARGIN_KEYS, today)
    labels = {i["label"] for i in props["images"]}
    if type(data["default_images"]) is not list or not data["default_images"] or not set(data["default_images"]) <= labels:
        errors.append("WHEN:default_images")
    seen = set()
    if type(data["fixed"]) is not list:
        errors.append("UNKNOWN_FIELD:fixed")
    else:
        for entry in data["fixed"]:
            _entry(errors, "fixed", entry, seen, props, matrix, today)
    stages = data["stages"]
    if type(stages) is not dict:
        return errors + ["UNKNOWN_FIELD:stages"]
    for name in sorted(set(stage_names) - set(stages)):
        errors.append("STAGE_MISSING:" + name)
    for name in sorted(set(stages) - set(stage_names)):
        errors.append("STAGE_UNKNOWN:" + name)
    for name, stage in stages.items():
        if type(stage) is not dict:
            errors.append("UNKNOWN_FIELD:" + name)
            continue
        for key in sorted(set(stage) - STAGE_KEYS):
            errors.append("UNKNOWN_FIELD:%s:%s" % (name, key))
        for key in sorted(STAGE_KEYS - set(stage)):
            errors.append("MISSING:%s:%s" % (name, key))
        entries = stage.get("entries", [])
        if type(entries) is not list:
            errors.append("UNKNOWN_FIELD:%s:entries" % name)
            continue
        if not entries and not (type(stage.get("note")) is str and stage["note"].strip()):
            errors.append("EMPTY_STAGE:" + name)
        if sum(1 for e in entries if type(e) is dict and e.get("calibrate")) > 1:
            errors.append("CALIBRATE_MANY:" + name)
        for entry in entries:
            _entry(errors, name, entry, seen, props, matrix, today)
        upload = stage.get("upload")
        if type(upload) is dict:
            for key in sorted(set(upload) - UPLOAD_KEYS):
                errors.append("UNKNOWN_FIELD:%s.upload:%s" % (name, key))
            if not _count(upload.get("up_bytes")):
                errors.append("BYTES:%s.upload" % name)
            _provenance(errors, name + ".upload", upload, UPLOAD_KEYS, today)
        elif "upload" in stage:
            errors.append("UNKNOWN_FIELD:%s.upload" % name)
    return errors


def payload_stage_names(source=None):
    """Every stage the payload can run: the planned ones and the manual-only ones (profile_resources reads the same literals)."""
    sys.path.insert(0, str(HERE))
    try:
        import profile_resources
    finally:
        sys.path.pop(0)
    text = source if source is not None else PAYLOAD.read_text()
    manual, _flags = profile_resources.manual_only(text)
    return profile_resources.stage_names(text) | set(manual)


def load(path=DATA):
    return json.loads(Path(path).read_text())


# ---------------------------------------------------------------- the twin of src/bandwidth.js
def planned(instruction, only=()):
    """The payload's own plan for an instruction dict (vast_payload.plan_stage_names)."""
    sys.path.insert(0, str(HERE))
    try:
        import payload_instruction
        import vast_payload
    finally:
        sys.path.pop(0)
    inst = payload_instruction.parse(json.dumps(instruction).encode())
    return vast_payload.plan_stage_names(inst, tuple(only))


def _matches(when, ctx):
    if when is None:
        return True
    for key, allowed in when.items():
        if key == "images":
            if not any(label in allowed for label in ctx["images"]):
                return False
        elif ctx.get(key) not in allowed:
            return False
    return True


def explain(instruction, tier, data, props, matrix, only=(), skip=(), r2_sizes=None, repo_bytes=None):
    """[{id, stage, label, down, up}] for an instruction: the same selection rule as estimateBandwidth in src/bandwidth.js."""
    r2_sizes = r2_sizes or {}
    plan = planned(instruction, only)
    chosen = [n for n in plan if (not only or n in only) and n not in skip]
    sent = [i["label"] for i in instruction.get("propimages", [])]
    images = sent or (list(data["default_images"]) if "propimage_apps" in chosen else [])
    ctx = {"tier": tier or instruction["tier"], "winapps": instruction.get("winapps"), "spec": instruction.get("spec"),
           "linux_apps": instruction.get("linux_apps"), "images": images}
    objects = {o["id"]: o for o in props["objects"]}
    pins = {p["os"] + "/" + p["version"]: p for p in matrix.get("driver_installers", [])}
    items = []

    def add(stage, entry):
        if not _matches(entry.get("when"), ctx):
            return
        for label in (images if entry.get("per_image") else [None]):
            ref = entry.get("size_from")
            if ref is None:
                size = entry["down_bytes"]
            else:
                kind, name = ref.split(":", 1)
                if kind == "propimage":
                    size = objects[next(i["object"] for i in props["images"] if i["label"] == label) if name == "image" else name]["size"]
                elif kind == "driver":
                    size = pins[name]["size"]
                else:
                    size = r2_sizes.get(name, entry["down_bytes"])
            if entry["id"] == "fixed.repo_clone" and repo_bytes is not None:
                size = repo_bytes
            items.append({"id": entry["id"], "stage": stage, "label": label, "down": size, "up": 0})

    for entry in data["fixed"]:
        add("fixed", entry)
    for name in chosen:
        for entry in data["stages"][name]["entries"]:
            add(name, entry)
        items.append({"id": name + ".upload", "stage": name, "label": None, "down": 0, "up": data["stages"][name]["upload"]["up_bytes"]})
    return items


def main(argv=None):
    parser = argparse.ArgumentParser(description=__doc__.split("\n")[0])
    parser.add_argument("--file", default=str(DATA))
    parser.add_argument("--today", default=None, help="YYYY-MM-DD (default: today, UTC)")
    parser.add_argument("--explain", metavar="INSTRUCTION")
    parser.add_argument("--tier", default=None)
    parser.add_argument("--only", action="append", default=[])
    parser.add_argument("--skip", action="append", default=[])
    args = parser.parse_args(argv)
    props, matrix = json.loads(PROPS.read_text()), json.loads(MATRIX.read_text())
    data = load(args.file)
    if args.explain:
        items = explain(json.loads(Path(args.explain).read_text()), args.tier, data, props, matrix, args.only, args.skip)
        print(json.dumps(items, sort_keys=True))
        return 0
    today = datetime.date.fromisoformat(args.today) if args.today else datetime.datetime.now(datetime.timezone.utc).date()
    errors = validate(data, payload_stage_names(), props, matrix, today)
    for error in errors:
        print(error)
    print("bandwidth table: " + ("FAIL" if errors else "PASS"))
    return 1 if errors else 0


if __name__ == "__main__":
    sys.exit(main())
