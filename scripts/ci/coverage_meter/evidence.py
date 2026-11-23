# SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later
"""Decode public interface fields from census and kernel evidence."""

import re
from collections import Counter, defaultdict
from dataclasses import dataclass, field

NUMBER = r"0x[0-9a-fA-F]+"
NUM = rf"(?:{NUMBER}|[0-9]+)"
RPC = re.compile(rf"kf-rm: rpc-trace fn=(76|103) (?:RmControl|RmAlloc) "
                 rf"(?:seq=\d+ )?(?:cmd|class)=({NUMBER})\b.*?\bresult=(none|{NUMBER})\b")
REFUSAL_KEY = rf"fn(\d+)(?:/({NUMBER}))?=({NUMBER})"
REFUSAL = re.compile(rf"kf3: GSP REFUSED {REFUSAL_KEY}\b")
HEARTBEAT = re.compile(r"gsp_refusals\[total=(\d+) distinct=(\d+) \[([^]]*)\]\]")
TALLY = re.compile(rf"{REFUSAL_KEY}x(\d+)\b")
SERVED = re.compile(rf"nvkvm:\s+control ({NUMBER}) result ({NUMBER}) x(\d+)\b")
MISSED = re.compile(rf"nvkvm:\s+unserviced fn (\d+)(?: cmd ({NUMBER}))?")
MISSED_LIST = re.compile(r"\bunserviced=\[((?:fn\d+|0x[0-9a-fA-F]+)(?:,(?:fn\d+|0x[0-9a-fA-F]+))*)\]")
UNSERVED = re.compile(r"kf3: GSP rpc UNSERVICED Unserviced \{ code: (\d+), sequence: \d+")
CODE_COUNT = re.compile(r"^\s*unserviced (\d+) \(code (\d+)\)\s*$", re.I)
FABRICATED = re.compile(rf"kf-rm: (?:WINDOWS PROFILE [A-Z0-9_]+|SW-RUNLIST HOST-OWNED) ({NUMBER})\b.*FABRICATED answer")
GFX_POOL = re.compile(r"kf-rm: EXPERIMENT virtual GFX_POOL_QUERY_SIZE .*result=Ok\(\(\)\)")
CHANNEL_FACTS = re.compile(rf"kf-rm: chanlink: CHAN-FACTS ({NUMBER}:{NUMBER}) .*?\bclass=({NUMBER})\b")
PASSTHROUGH = re.compile(rf"kf3:.*?chan ({NUMBER}:{NUMBER}) BORN Passthrough:")
ESCAPE = re.compile(rf"NVRM:\s*(?:RmIoctl:\s*)?unknown NVRM ioctl command:\s*({NUMBER})\b")
XID = re.compile(r"NVRM:\s*Xid\s*\([^)]*\):\s*(\d+)\b")
FOREIGN = re.compile(rf"ForeignClass \{{ subch: \d+, class: ({NUM}) \}}")
METHOD = re.compile(rf"(?:RefusedMethod|SwMethod|Unclassified|UnboundSubchannel) \{{ "
                    rf"(?:subch: \d+, )?method: ({NUM})\b")

# Select candidate records without printing log bodies. Patterns above validate
# the producer and extract only interface fields.
CANDIDATES = (
    r"rpc-trace|GSP REFUSED|gsp_refusals\[|nvkvm: +control |nvkvm: +unserviced fn "
    r"|unserviced=\[0x|unserviced=\[fn|GSP rpc UNSERVICED|WINDOWS PROFILE"
    r"|BORN Passthrough:|CHAN-FACTS|virtual GFX_POOL_QUERY_SIZE|SW-RUNLIST HOST-OWNED|unknown NVRM ioctl command|NVRM: *Xid|ForeignClass \{|RefusedMethod \{"
    r"|SwMethod \{|Unclassified \{|UnboundSubchannel \{|unserviced [0-9]+ \(code"
)


def rpc_key(fn, detail):
    if detail:
        value = int(detail, 16)
        if fn == "76":
            return "control", value >> 16, value
        if fn == "103":
            return "class", None, value
    return "rpc", None, int(fn)


def outcome(status):
    return "unserviced" if status == "none" else ("refused" if int(status, 16) else "serviced")


@dataclass
class Census:
    events: dict = field(default_factory=lambda: defaultdict(Counter))
    statuses: dict = field(default_factory=lambda: defaultdict(Counter))
    legacy: dict = field(default_factory=lambda: defaultdict(Counter))
    summaries: dict = field(default_factory=lambda: defaultdict(Counter))

    def event(self, key, result, count=1):
        self.events[key][result] += count

    def snapshot(self, key, result, count=1):
        row = self.summaries[key]
        row[result] = max(row[result], count)

    def rows(self):
        for key in self.events.keys() | self.summaries.keys() | self.statuses.keys() | self.legacy.keys():
            events = self.events[key]
            snapshots = self.summaries[key]
            counts = events | snapshots | self.legacy[key]
            if self.statuses[key]:
                counts["non_ok_status"] = sum(self.statuses[key].values())
            # The FSM posts a non-OK status for unserviced requests too. The
            # status ledger alone cannot distinguish them. Keep its own count.
            success = max(counts["serviced"], counts["fabricated"], counts["passthrough"])
            failed = max(counts["refused"] + counts["unserviced"], counts["non_ok_status"])
            yield key, counts, max(success + failed, counts["observed"])


def parse(lines):
    census = Census()
    channel_classes = {}
    for line in lines:
        if m := RPC.search(line):
            fn, detail, status = m.groups()
            census.event(rpc_key(fn, detail), outcome(status))
        elif m := SERVED.search(line):
            detail, status, count = m.groups()
            census.legacy[rpc_key("76", detail)][outcome(status)] += int(count)
        elif m := REFUSAL.search(line):
            fn, detail, status = m.groups()
            counts = census.statuses[rpc_key(fn, detail)]
            counts[status] = max(1, counts[status])
        elif m := HEARTBEAT.search(line):
            for fn, detail, status, count in TALLY.findall(m[3]):
                counts = census.statuses[rpc_key(fn, detail)]
                counts[status] = max(counts[status], int(count))
        elif m := MISSED.search(line):
            census.snapshot(rpc_key(*m.groups()), "unserviced")
        elif m := UNSERVED.search(line):
            census.event(rpc_key(m[1], None), "unserviced")
        elif m := CODE_COUNT.search(line):
            census.snapshot(rpc_key(m[2], None), "unserviced", int(m[1]))
        elif m := FABRICATED.search(line):
            census.event(rpc_key("76", m[1]), "fabricated")
        elif GFX_POOL.search(line):
            # gfxpool_probe prints a command name in place of the numeric ID.
            census.event(rpc_key("76", "0x2080121f"), "fabricated")
        elif m := CHANNEL_FACTS.search(line):
            key = "class", None, int(m[2], 16)
            channel_classes[m[1]] = key
            census.snapshot(key, "observed")
        elif m := ESCAPE.search(line):
            census.event(("host_escape", None, int(m[1], 16)), "refused")
        elif "BORN Passthrough:" in line and "kf3:" in line:
            birth = PASSTHROUGH.search(line)
            key = channel_classes.pop(birth[1], None) if birth else None
            census.event(key or ("route", None, 0), "passthrough")
        elif m := XID.search(line):
            census.event(("xid", None, int(m[1])), "observed")
        elif m := FOREIGN.search(line):
            census.event(("class", None, int(m[1], 0)), "refused")
        elif m := METHOD.search(line):
            census.event(("method", None, int(m[1], 0)), "refused")

        if m := MISSED_LIST.search(line):
            for item in m[1].split(","):
                key = rpc_key(item[2:], None) if item.startswith("fn") else rpc_key("76", item)
                census.snapshot(key, "unserviced")
    return census
