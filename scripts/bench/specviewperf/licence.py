#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later
"""The licence readout of the SPECviewperf stage's guest, as pure functions (stdlib only, GPU-free).

STATUS: CODE, 2026-11-17 (claude/spec-online-ltsc-20261117; docs/design/V3_SPEC_GATE.md, section "Guest network and licence
probe"). GPU-free tests only (scripts/ci/test_spec_online_ltsc.py, scripts/ci/spec_stage_mutants.py); no box has run it.

The Windows evaluation guest needs the network and the real time to complete its own start-up validation. The stage probes the
guest at boot, about every 15 minutes and at the end with scripts/bench/windows/spec/licence_probe.ps1 (read-only), and this
module turns the raw readout into two kinds of status line:

  state     SPEC_LICENCE_STATE at=<label> up_min=<n> status=<text> remaining=<text> expiry=<text> wlms=<s> sppsvc=<s> w32time=<s> net=<n>
  shutdown  SPEC_GUEST_SHUTDOWN_LICENCE viewset=<w> event=1074 time=<utc> process=<exe> text=<reason and comment>

Only state text leaves the raw readout: a product key, an activation id, the computer name and an account (DOMAIN\\user) never
appear in either line (a line of the readout is read by its name and only the named fields are kept; every value is cut to a
safe alphabet, a key-shaped group is removed, and an event keeps the basename of its process, not its path). The raw readout
stays in the stage's private directory.

Event 1074 is the System log's "a process initiated a shutdown or restart". A guest that was taken down by the Windows licensing
service leaves one: the detector accepts an event whose process is wlms or whose reason or comment names a licence, an
evaluation, wlms or an activation, and which is not older than the previous boot's start (`since`, the guest's own clock: with a
recorded clock every boot starts at the same time, so there is no upper bound).
"""

import argparse
import datetime
import re
import sys
from pathlib import Path

UNSAFE = re.compile(r"[^A-Za-z0-9 .,:;()/+_-]")
KEYLIKE = re.compile(r"\b[A-Z0-9]{5}(?:-[A-Z0-9]{5}){2,4}\b")
GUIDLIKE = re.compile(r"\b[0-9a-fA-F]{8}-[0-9a-fA-F]{4}-[0-9a-fA-F]{4}-[0-9a-fA-F]{4}-[0-9a-fA-F]{12}\b")
ACCOUNTLIKE = re.compile(r"(?:NT AUTHORITY|[A-Za-z0-9_.-]+)\\[A-Za-z0-9_.$-]+")  # DOMAIN\\user, MACHINE\\user
LABEL = re.compile(r"[A-Za-z0-9_]{1,40}")
WORKLOAD = re.compile(r"[a-z][a-z_]*-[0-9]{2}")
STAMP = re.compile(r"(\d{4})-(\d\d)-(\d\d)T(\d\d):(\d\d):(\d\d)")
LICENCE_WORDS = re.compile(r"licen[cs]|evaluation|wlms|activation", re.I)
SERVICES = {"wlms": "wlms", "sppsvc": "sppsvc", "W32Time": "w32time"}


def clean(value, limit):
    """A value of the readout cut to the status alphabet: no key-shaped or id-shaped group, no other character, spaces as `_`,
    at most `limit` characters. Empty is `-`."""
    text = ACCOUNTLIKE.sub(" ", KEYLIKE.sub(" ", GUIDLIKE.sub(" ", str(value))))
    text = " ".join(UNSAFE.sub(" ", text).split())
    text = text[:limit].strip().replace(" ", "_")
    return text or "-"


def stamp(text):
    """A UTC datetime from the front of an ISO stamp (any number of fraction digits), or None."""
    m = STAMP.match(str(text).strip())
    if not m:
        return None
    try:
        return datetime.datetime(*(int(g) for g in m.groups()))
    except ValueError:
        return None


def blocks(text):
    """-> (the records {LIC_NAME: rest} with the services under `services` and every event 1074 under LIC_EVT1074, the
    indented blocks {LIC_NAME: [lines]}). A block starts at a LIC_<X>_BEGIN line and runs to the next LIC_ line."""
    records, blk, cur = {"services": {}}, {}, None
    for raw in text.splitlines():
        line = raw.rstrip("\r")
        if line.startswith("LIC_"):
            token, _, rest = line.partition(" ")
            cur = None
            if token.endswith("_BEGIN"):
                cur = token[: -len("_BEGIN")]
                blk[cur] = []
            elif token == "LIC_SERVICE":
                name, _, tail = rest.partition(" ")
                if name:
                    records["services"][name] = tail
            elif token == "LIC_EVT1074":
                records.setdefault(token, []).append(rest)
            else:
                records[token] = rest
        elif cur is not None:
            blk[cur].append(line)
    return records, blk


def field(rest, name):
    """`name=value` or `name='value'` out of a record."""
    m = re.search(r"(?:^|\s)%s=(?:'([^']*)'|(\S*))" % re.escape(name), rest)
    return (m.group(1) if m.group(1) is not None else m.group(2)) if m else None


def first_match(lines, pattern):
    for line in lines:
        m = re.match(pattern, line.strip())
        if m:
            return m.group(1)
    return None


def state_line(raw, label, net):
    """The SPEC_LICENCE_STATE line of one readout. An empty or unreadable readout is `status=unavailable`."""
    label = label if LABEL.fullmatch(label or "") else "probe"
    net = net if net in ("online", "offline") else "unknown"
    records, blk = blocks(raw or "")
    if "LIC_DONE" not in records and "LIC_TIME" not in records:
        return "SPEC_LICENCE_STATE at=%s status=unavailable net=%s" % (label, net)
    now = stamp(field(records.get("LIC_TIME", ""), "utc") or "")
    boot = stamp(field(records.get("LIC_OS", ""), "lastboot") or "")
    up = str(max(0, int((now - boot).total_seconds() // 60))) if now and boot else "-"
    dlv = blk.get("LIC_DLV", [])
    status = first_match(dlv, r"License Status:\s*(.+)$")
    remaining = first_match(dlv, r"(?:Time remaining|Timebased activation expiration):\s*(.+)$")
    expiry = " ".join(line.strip() for line in blk.get("LIC_XPR", []) if line.strip())
    services = records.get("services", {})
    fields = [
        ("up_min", up),
        ("status", clean(status or "-", 48)),
        ("remaining", clean(remaining or "-", 48)),
        ("expiry", clean(expiry or "-", 120)),
    ]
    for svc, key in SERVICES.items():
        fields.append((key, clean(field(services.get(svc, ""), "status") or "absent", 24) if svc in services else "absent"))
    return "SPEC_LICENCE_STATE at=%s %s net=%s" % (label, " ".join("%s=%s" % kv for kv in fields), net)


def last_boot(raw):
    """The guest's own start time of the booted system (ISO, seconds), or ''."""
    records, _ = blocks(raw or "")
    boot = stamp(field(records.get("LIC_OS", ""), "lastboot") or "")
    return boot.strftime("%Y-%m-%dT%H:%M:%SZ") if boot else ""


def events_1074(raw):
    """-> [{time, process, reason, comment, type}] of the readout, the strings still raw (cleaned when printed)."""
    records, _ = blocks(raw or "")
    out = []
    for rest in records.get("LIC_EVT1074", []):
        when = stamp(field(rest, "time") or "")
        if when is None:
            continue
        # property 0 of event 1074 is "<path of the exe> (<COMPUTER NAME>)": the basename of the path, never the computer
        process = (field(rest, "process") or "").replace("\\", "/").rsplit("/", 1)[-1].split(" ")[0]
        out.append({"time": when, "process": process, "reason": field(rest, "reason") or "",
                    "comment": field(rest, "comment") or "", "type": field(rest, "type") or ""})
    return out


def shutdown_line(raw, viewset, since=None):
    """The SPEC_GUEST_SHUTDOWN_LICENCE line for the newest licence-related event 1074 that is not older than `since` (an ISO
    stamp of the guest's clock: the previous boot's start), or None. A readout without a 1074 of a licence is None: the guest
    went down for another reason, which the row keeps as a bare GUEST."""
    if not WORKLOAD.fullmatch(viewset or ""):
        return None
    floor = stamp(since) if since else None
    best = None
    for ev in events_1074(raw):
        if floor is not None and ev["time"] < floor:
            continue
        text = " ".join(x for x in (ev["reason"], ev["comment"]) if x)
        if not (re.search(r"wlms", ev["process"], re.I) or LICENCE_WORDS.search(text)):
            continue
        if best is None or ev["time"] >= best["time"]:
            best = ev
    if best is None:
        return None
    text = " ".join(x for x in (best["type"], best["reason"], best["comment"]) if x)
    return "SPEC_GUEST_SHUTDOWN_LICENCE viewset=%s event=1074 time=%s process=%s text=%s" % (
        viewset, best["time"].strftime("%Y-%m-%dT%H:%M:%SZ"), clean(best["process"], 40), clean(text, 240))


def main(argv=None):
    ap = argparse.ArgumentParser(prog="licence.py")
    sub = ap.add_subparsers(dest="cmd", required=True)
    p = sub.add_parser("state")
    p.add_argument("--raw", required=True)
    p.add_argument("--label", required=True)
    p.add_argument("--net", default="")
    p = sub.add_parser("last-boot")
    p.add_argument("--raw", required=True)
    p = sub.add_parser("shutdown")
    p.add_argument("--raw", required=True)
    p.add_argument("--viewset", required=True)
    p.add_argument("--since", default="")
    a = ap.parse_args(argv)

    def text(path):
        try:
            return Path(path).read_text(errors="replace")
        except OSError:
            return ""

    if a.cmd == "state":
        print(state_line(text(a.raw), a.label, a.net))
    elif a.cmd == "last-boot":
        print(last_boot(text(a.raw)))
    else:
        line = shutdown_line(text(a.raw), a.viewset, a.since or None)
        if line is None:
            return 1
        print(line)
    return 0


if __name__ == "__main__":
    sys.exit(main())
