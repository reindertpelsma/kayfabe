#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later
"""The uniform guest clock rule (stdlib only; the one place it lives; shell launchers only call it).

STATUS: LIVE as code, 2026-11-16 (GPU-free; no VM was started by it or its tests). Design: docs/design/V3_CLOCK_RULE.md.
Owner ruling 2026-11-16 (docs/OWNER_RULINGS.md, section AU).

The clock depends ONLY on whether the guest has network:

  network OFF (slirp restrict=on, only the ssh forward)  ->  the RECORDED clock: `-rtc base=<recorded>,clock=vm`
  network ON  (restrict=off)                             ->  the REAL clock:     `-rtc base=utc`

An offline image needs no current time; a moving clock only adds ways for it to break (expiring certificates and caches,
timers, update prompts). A fixed clock makes a run reproducible, the way pinning dependency versions does;
reproducible-build practice pins the timestamp for the same reason. A guest with network needs the real time.

The recorded clock is a sidecar `clock.json` next to the image (`record`): the guest's UTC time at the last probe before the
build's shutdown. An offline boot starts at the clock floor and `clock=vm` makes it advance monotonically from there.

THE INVARIANT: a recorded clock is never earlier than the creation time of software loaded into the image.
  clock_floor = max(recorded, max(created_utc of every loaded software) + margin)   (margin default 1 h; no software -> the
  recorded clock). Only SOFTWARE counts: a package installed or loaded into a non-proprietary image that boots offline with a
  recorded clock (the SPECviewperf bundle on the open LTSC image, a guest driver installer). Our scripts and test/input assets
  (repo scripts, harness files, the open test-assets bundle, data files, media) do NOT count and are never passed here. A
  proprietary image uses exactly the clock of its state.json (propimage.py) and does not use the floor.
  created_utc comes from the pin (an item with `loads_software: true` and `created_utc`; for a private R2 object the R2
  LastModified), or `PATH=CREATED_UTC` from the stage. A file's mtime is NOT reliable after a copy or rsync without -t, so it
  is only a fallback, with KF_CLOCK_INPUT_NO_CREATED and a WARN. An EXPLICIT base earlier than a created_utc is refused
  (KF_CLOCK_BEFORE_INPUT).

  clockrule.py rtc --net offline|online --image NAME [--sidecar F] [--loaded-software P[=UTC] ...] [--pins F ...] [--base T] [--strict 0|1]
                   [--phase run|build] [--margin-s N] [--json OUT]
        stdout: the one `-rtc` value. stderr: named lines (KF_CLOCK ..., KF_CLOCK_NO_RECORD ..., KF_CLOCK_REFUSED ...).
        exit 0 ok, 2 refused (named). --json writes the verdict (PASS or WARN) as a report file.
  clockrule.py record --sidecar F --utc T --source S [--tool-rev R] [--input NAME=UTC ...]

Refusals (named): online + a recorded clock is ALWAYS refused (`online_recorded_clock`); offline + the real clock is refused
only with strict on (`offline_real_clock_strict`). Lenient (the default): no sidecar -> the real clock plus exactly one line
`KF_CLOCK_NO_RECORD image=<name> using=real` and verdict WARN. A bad sidecar is refused (`bad_record`), never read as "none".
Phase `build` (the boots that make the record: nothing is recorded yet) runs the real clock without a warning.
"""
import argparse
import datetime as dt
import json
import os
import re
import sys
from pathlib import Path

SCHEMA = 1
STAMP = re.compile(r"\d{4}-\d\d-\d\dT\d\d:\d\d:\d\dZ")
MARGIN_S = 3600  # the default margin after the newest input created_utc
RTC_TAIL = ",driftfix=slew"  # the launchers' drift policy, unchanged
KINDS = ("recorded", "real")


class ClockError(Exception):
    """A named refusal: `reason` is the machine-readable name, printed as KF_CLOCK_REFUSED reason=<reason>."""

    def __init__(self, reason, detail=""):
        super().__init__("%s%s" % (reason, (": " + detail) if detail else ""))
        self.reason = reason
        self.detail = detail


def parse_utc(text):
    if not isinstance(text, str) or not STAMP.fullmatch(text):
        raise ClockError("bad_record", "bad UTC time %r (want YYYY-MM-DDTHH:MM:SSZ)" % (text,))
    return dt.datetime.strptime(text, "%Y-%m-%dT%H:%M:%SZ").replace(tzinfo=dt.timezone.utc)


def fmt_utc(t):
    return t.astimezone(dt.timezone.utc).strftime("%Y-%m-%dT%H:%M:%SZ")


def assert_rule(net, kind, strict=False):
    """THE assert. net: 'online'|'offline'; kind: 'recorded'|'real'. Online + recorded is always refused; offline + real
    is refused only when strict."""
    if net not in ("online", "offline"):
        raise ClockError("bad_net", "net must be online or offline, got %r" % (net,))
    if kind not in KINDS:
        raise ClockError("bad_kind", "clock kind must be recorded or real, got %r" % (kind,))
    if net == "online" and kind == "recorded":
        raise ClockError("online_recorded_clock", "a guest with network needs the real clock")
    if net == "offline" and kind == "real" and strict:
        raise ClockError("offline_real_clock_strict", "strict: an offline guest must run the recorded clock")


def read_sidecar(path):
    """-> (the recorded UTC datetime, the build's inputs [{name, created_utc}]) or (None, []) when there is no sidecar.
    A present but unreadable one is refused."""
    p = Path(path)
    if not p.exists():
        return None, []
    try:
        doc = json.loads(p.read_text())
    except (OSError, ValueError) as e:
        raise ClockError("bad_record", "%s: %s" % (p, e))
    if not isinstance(doc, dict) or doc.get("schema") != SCHEMA or not isinstance(doc.get("source"), str):
        raise ClockError("bad_record", "%s: not a schema-%d clock record" % (p, SCHEMA))
    inputs = doc.get("inputs", [])
    if not isinstance(inputs, list) or any(not isinstance(i, dict) or not isinstance(i.get("name"), str) for i in inputs):
        raise ClockError("bad_record", "%s: inputs must be a list of {name, created_utc}" % p)
    return parse_utc(doc.get("recorded_utc")), [
        {"name": i["name"], "created": parse_utc(i.get("created_utc")), "source": "record"} for i in inputs]


def pin_created(doc, name):
    """The `created_utc` a pin file gives for the software called `name`: a dict anywhere in the JSON with `loads_software` true
    whose name, file, ref or dest basename is `name`. Items without loads_software (assets, scripts, media) are ignored. None
    when no such pin names it or it says null. Two different values for one name are refused (`bad_pin`)."""
    found = set()

    def walk(node):
        if isinstance(node, dict):
            names = {str(node.get(k)).rsplit("/", 1)[-1] for k in ("name", "file", "ref", "dest") if node.get(k)}
            if node.get("loads_software") is True and name in names and node.get("created_utc") is not None:
                found.add(node["created_utc"])
            for v in node.values():
                walk(v)
        elif isinstance(node, list):
            for v in node:
                walk(v)

    walk(doc)
    if len(found) > 1:
        raise ClockError("bad_pin", "%s has several created_utc values in the pins: %s" % (name, sorted(found)))
    return parse_utc(found.pop()) if found else None


def resolve_inputs(specs, pins=()):
    """specs: the LOADED SOFTWARE, each 'PATH' or 'PATH=CREATED_UTC'. Returns (inputs, lines, warn). Each input is {name, created, source} with source
    arg | pin | mtime. Order of authority: the stated created_utc, the pin's created_utc, then the file's mtime with a WARN
    (KF_CLOCK_INPUT_NO_CREATED): an mtime is not reliable after a copy or rsync without -t, so it is a fallback, never a
    source. A path that does not exist and has no created_utc is skipped with KF_CLOCK_SOFTWARE_MISSING."""
    docs = []
    for pf in pins:
        try:
            docs.append(json.loads(Path(pf).read_text()))
        except (OSError, ValueError) as e:
            raise ClockError("bad_pin", "%s: %s" % (pf, e))
    out, lines, warn = [], [], False
    for spec in specs:
        path, _, stated = spec.partition("=")
        name = Path(path).name
        created, source = None, None
        if stated:
            created, source = parse_utc(stated), "arg"
        else:
            vals = {pin_created(d, name) for d in docs} - {None}  # only items with loads_software: true
            if len(vals) > 1:
                raise ClockError("bad_pin", "%s has different created_utc values in the pin files" % name)
            if vals:
                created, source = vals.pop(), "pin"
        if created is None:
            p = Path(path)
            if not p.exists():
                lines.append("KF_CLOCK_SOFTWARE_MISSING path=%s" % path)
                continue
            m = max([p.stat().st_mtime] if not p.is_dir() else [q.stat().st_mtime for q in p.rglob("*") if q.is_file()] or [0])
            created, source, warn = dt.datetime.fromtimestamp(int(m) + (m != int(m)), dt.timezone.utc), "mtime", True
            lines.append("KF_CLOCK_INPUT_NO_CREATED input=%s using=mtime" % name)
        out.append({"name": name, "created": created, "source": source})
    return out, lines, warn


def floor_for(base, inputs, margin_s=MARGIN_S, refuse=True):
    """The invariant: the recorded clock is ALWAYS later than the creation of every input the image or the run loads.
    -> (clock_floor = max(base, max(created) + margin), the name of the input that set the floor or None).
    `base` itself earlier than an input's created_utc is refused (KF_CLOCK_BEFORE_INPUT): that clock names a time at which
    the input did not exist yet. refuse=False (the sidecar's own clock) raises the floor instead of refusing."""
    for i in inputs:
        if refuse and base < i["created"]:
            raise ClockError("KF_CLOCK_BEFORE_INPUT", "input=%s created=%s base=%s" % (i["name"], fmt_utc(i["created"]), fmt_utc(base)))
    delta = dt.timedelta(seconds=margin_s)
    best, who = base, None
    for i in inputs:
        if i["created"] + delta > best:
            best, who = i["created"] + delta, i["name"]
    return best, who


def clock_for(net, image, sidecar=None, loaded_software=(), strict=False, phase="run", base=None, pins=(), margin_s=MARGIN_S):
    """The decision for one boot. Returns a dict: kind ('recorded'|'real'), rtc (the -rtc value), verdict (PASS|WARN),
    lines (named lines for stderr), recorded_utc, base_utc (the effective floor), floor_input. Raises ClockError.
    base: an explicit YYYY-MM-DDTHH:MM:SSZ override (WINVM_RTC_BASE); it is a recorded clock, so the assert covers it.
    loaded_software: PATH or PATH=CREATED_UTC of each software package the stage loads into the image (never scripts or
    assets); pins: pin files whose items carry loads_software and created_utc."""
    if not (isinstance(image, str) and re.fullmatch(r"[A-Za-z0-9_.-]{1,64}", image)):
        raise ClockError("bad_image", "image name must be 1-64 of [A-Za-z0-9_.-], got %r" % (image,))
    if net not in ("online", "offline"):
        raise ClockError("bad_net", "net must be online or offline, got %r" % (net,))
    if phase not in ("run", "build"):
        raise ClockError("bad_phase", "phase must be run or build, got %r" % (phase,))
    if not isinstance(margin_s, int) or not 0 <= margin_s <= 30 * 86400:
        raise ClockError("bad_margin", "margin must be 0..2592000 seconds")
    out = {"schema": SCHEMA, "image": image, "net": net, "phase": phase, "strict": bool(strict), "lines": [],
           "recorded_utc": None, "base_utc": None, "floor_input": None, "inputs": [], "margin_s": margin_s, "verdict": "PASS"}
    recorded, build_inputs = None, []
    if base is not None:
        recorded = parse_utc(base)
        out["source"] = "override"
    elif net == "offline" and phase == "run":
        recorded, build_inputs = read_sidecar(sidecar) if sidecar else (None, [])
        out["source"] = "sidecar" if recorded else "none"
    else:
        out["source"] = "real_clock_by_rule" if net == "online" else "build"
    kind = "recorded" if recorded is not None else "real"
    assert_rule(net, kind, strict and phase == "run" and base is None)
    if kind == "recorded":
        run_inputs, lines, warn = resolve_inputs(loaded_software, pins)
        inputs = build_inputs + run_inputs
        start, who = floor_for(recorded, inputs, margin_s, refuse=base is not None)
        out["recorded_utc"], out["base_utc"], out["floor_input"] = fmt_utc(recorded), fmt_utc(start), who
        out["inputs"] = [{"name": i["name"], "created_utc": fmt_utc(i["created"]), "source": i["source"]} for i in inputs]
        out["rtc"] = "base=%s,clock=vm%s" % (start.strftime("%Y-%m-%dT%H:%M:%S"), RTC_TAIL)
        out["lines"] += lines + ["KF_CLOCK image=%s net=%s using=recorded base=%s recorded=%s floor_input=%s inputs=%d" % (
            image, net, out["base_utc"], out["recorded_utc"], who or "-", len(inputs))]
        if warn:
            out["verdict"] = "WARN"
    else:
        out["rtc"] = "base=utc" + RTC_TAIL
        if net == "offline" and phase == "run":  # lenient fallback: loud, once, named, a WARN
            out["verdict"] = "WARN"
            out["lines"].append("KF_CLOCK_NO_RECORD image=%s using=real" % image)
        else:
            out["lines"].append("KF_CLOCK image=%s net=%s using=real phase=%s" % (image, net, phase))
    out["kind"] = kind
    return out


def record(sidecar, utc, source, tool_rev="", inputs=(), margin_s=MARGIN_S):
    """Write the sidecar atomically. utc is the guest's own UTC time at the last probe before the build's shutdown.
    inputs: NAME=CREATED_UTC of the software the build loaded. A guest time earlier than an input's creation is refused
    (KF_CLOCK_BEFORE_INPUT); floor_input names the software whose created_utc + margin lies after utc (it sets the floor), or null."""
    recorded = parse_utc(utc)
    if not source or not isinstance(source, str):
        raise ClockError("bad_record", "a record needs a source")
    ins = []
    for spec in inputs:
        name, _, created = spec.partition("=")
        if not name or not created:
            raise ClockError("bad_record", "an input is NAME=CREATED_UTC, got %r" % spec)
        ins.append({"name": name, "created": parse_utc(created), "source": "arg"})
    _, who = floor_for(recorded, ins, margin_s)
    doc = {"schema": SCHEMA, "recorded_utc": utc, "source": source, "tool_rev": tool_rev or "unknown",
           "margin_s": margin_s, "floor_input": who,
           "inputs": [{"name": i["name"], "created_utc": fmt_utc(i["created"])} for i in ins]}
    p = Path(sidecar)
    tmp = p.with_name(p.name + ".tmp")
    tmp.write_text(json.dumps(doc, indent=2, sort_keys=True) + "\n")
    os.replace(tmp, p)
    return doc


def main(argv=None):
    ap = argparse.ArgumentParser(description=__doc__.split("\n")[0])
    sub = ap.add_subparsers(dest="cmd", required=True)
    r = sub.add_parser("rtc")
    r.add_argument("--net", required=True)
    r.add_argument("--image", required=True)
    r.add_argument("--sidecar")
    r.add_argument("--loaded-software", nargs="*", default=[], help="PATH or PATH=CREATED_UTC of each software package loaded (not scripts or assets)")
    r.add_argument("--pins", nargs="*", default=[], help="pin files whose items carry created_utc")
    r.add_argument("--margin-s", type=int, default=MARGIN_S)
    r.add_argument("--base", default="")
    r.add_argument("--strict", default="0", choices=("0", "1"))
    r.add_argument("--phase", default="run")
    r.add_argument("--json")
    c = sub.add_parser("record")
    c.add_argument("--sidecar", required=True)
    c.add_argument("--utc", required=True)
    c.add_argument("--source", required=True)
    c.add_argument("--tool-rev", default="")
    c.add_argument("--input", action="append", default=[], help="NAME=CREATED_UTC of software the build loaded")
    c.add_argument("--margin-s", type=int, default=MARGIN_S)
    a = ap.parse_args(argv)
    if a.cmd == "rtc" and a.base and re.fullmatch(r"\d{4}-\d\d-\d\dT\d\d:\d\d:\d\d", a.base):
        a.base += "Z"  # QEMU's base=... is UTC
    try:
        if a.cmd == "record":
            doc = record(a.sidecar, a.utc, a.source, a.tool_rev, a.input, a.margin_s)
            print("KF_CLOCK_RECORDED sidecar=%s utc=%s source=%s" % (a.sidecar, doc["recorded_utc"], doc["source"]), file=sys.stderr)
            return 0
        res = clock_for(a.net, a.image, a.sidecar, [s for s in a.loaded_software if s], a.strict == "1", a.phase, a.base or None, a.pins, a.margin_s)
    except ClockError as e:
        print("KF_CLOCK_REFUSED reason=%s %s" % (e.reason, e.detail), file=sys.stderr)  # reason KF_CLOCK_BEFORE_INPUT is a name of its own
        return 2
    for line in res["lines"]:
        print(line, file=sys.stderr)
    if a.json:
        Path(a.json).write_text(json.dumps({k: v for k, v in res.items() if k != "lines"}, indent=2, sort_keys=True) + "\n")
    print(res["rtc"])
    return 0


if __name__ == "__main__":
    sys.exit(main())
