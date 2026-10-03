#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later
"""kf3_lines.py <kind> — print the kf3 log line(s) of one kind, RENDERED FROM THE RUST SOURCE.

★ 2026-10-03 (review of 5af7e644). The app-matrix classifiers (`loud_verdict.sh`, `boot_gate.sh`,
the hook's per-row counts, `triage.py`) key on the shapes of lines kf3 prints. Their fixtures used
to be hand-typed — and one hand-typed `C′` line (`kf3: REFUSED VasKey(0x1) …`) was not the shape kf3
prints (`kf3: mem t=<s>s REFUSED VasKey(<n>) …`), while the line that actually broke the classifier
(the DELIVERY_UNBUILT boot sentence) was in no fixture at all. This script reads the format strings
and the DELIVERY_UNBUILT constant out of the tree and fills their placeholders, so a fixture IS the
line kf3 prints at this revision. A format that changes its placeholder count, or disappears, exits
2 with the reason — the fixture test then fails instead of testing a line kf3 no longer prints.

Kinds: boot_line, unarmed (all three producers), none, posted, unserviced, not_applied, host_twin,
xid_posted, all (every kind, `<kind>\\t<line>`).
"""
import re
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
CHAN = ROOT / "crates/kf-qemu/src/chan.rs"
DEVICE = ROOT / "crates/kf-qemu/src/device.rs"
VASMGR = ROOT / "crates/kf-mem/src/vasmgr.rs"
FAULTBUFFER = ROOT / "crates/kf-abi/src/faultbuffer.rs"

LIT = re.compile(r'"((?:[^"\\]|\\.)*)"', re.S)
PH = re.compile(r"\{[^{}]*\}")


def die(why):
    print(f"KF3_LINES_DRIFT {why}", file=sys.stderr)
    sys.exit(2)


def unescape(body):
    body = re.sub(r"\\\n\s*", "", body)  # a `\` line continuation drops the newline and indent
    return body.replace('\\"', '"').replace("\\\\", "\\").replace("\\n", "\n")


def literals(path, prefix, marker):
    """Every string literal in `path` that starts with `prefix` and contains `marker`. Each is
    parsed from its own opening quote (found as `"` + prefix), never by pairing every quote in the
    file — a `'"'` char literal or a quote in a comment cannot shift it."""
    src = path.read_text()
    out = []
    for m in re.finditer(re.escape('"' + prefix), src):
        lit = LIT.match(src, m.start())
        if lit:
            out.append(unescape(lit.group(1)))
    out = [s for s in out if marker in s]
    if not out:
        die(f"no literal starting {prefix!r} containing {marker!r} in {path.relative_to(ROOT)}")
    return out


def render(fmt, values):
    holes = PH.findall(fmt)
    if len(holes) != len(values):
        die(f"{fmt!r}: {len(holes)} placeholder(s), the fixture fills {len(values)}")
    it = iter(values)
    return PH.sub(lambda _m: str(next(it)), fmt)


def delivery_unbuilt():
    src = FAULTBUFFER.read_text()
    m = re.search(r'pub const DELIVERY_UNBUILT: &str = "((?:[^"\\]|\\.)*)";', src, re.S)
    if not m:
        die("DELIVERY_UNBUILT constant not found in crates/kf-abi/src/faultbuffer.rs")
    return unescape(m.group(1))


def lines(kind):
    if kind == "boot_line":
        (fmt,) = literals(DEVICE, "kf3: replayable fault buffer registered (", "")
        return [render(fmt, [1, delivery_unbuilt()])]
    if kind == "unarmed":
        out = []
        for fmt in literals(CHAN, "kf3: chan {", " RC-UNARMED: "):
            n = len(PH.findall(fmt))
            # client, handle, then (if any) one more: a gpa or the reason
            fill = ["0xc1d0001e", "0xcaf00099"] + (["notifier read view: refused" if fmt.endswith("{why}")
                                                    else "0x1f000"] * (n - 2))
            out.append(render(fmt, fill))
        if len(out) < 3:
            die(f"expected three RC-UNARMED producers in chan.rs, found {len(out)}")
        return out
    if kind == "none":
        return [render(f, ["0xc1d0001e", "0xcaf00099"]) for f in literals(CHAN, "kf3: chan {", " RC-NONE: ")]
    if kind == "posted":
        (fmt,) = literals(DEVICE, "kf3: RC_TRIGGERED posted: ", "")
        return [render(fmt, ["0xb", "0x1", "0x1f", "0x22"])]
    if kind == "unserviced":
        (fmt,) = literals(CHAN, "kf3: UNSERVICED-GPU-FAULT guest client {", "")
        return [render(fmt, ["0xc1d0001e", "0xb 0x8", 31, 31, ""])]
    if kind == "not_applied":
        (outer,) = literals(DEVICE, "kf3: mem t={:.3}s REFUSED ", "")
        (inner,) = literals(VASMGR, "{key:?} root {walked_root:#x}: ", " run(s) not applied: ")
        why = render(inner, ["VasKey(18446744069414584321)", "0x201000", 1,
                             "map 0x7c70d8400000+0x10000: Other(31)"])
        return [render(outer, ["52.101", why])]
    if kind == "host_twin":
        (fmt,) = literals(CHAN, "kf3: RC host twin {", "")
        return [render(fmt, ["0x22", "0xb", "0x1", "0x1f", 31])]
    if kind == "xid_posted":
        (fmt,) = literals(DEVICE, "kf3: OS_ERROR_LOG posted: ", "")
        return [render(fmt, ["0xc1d0001e", 31, "0xb", 0, 8])]
    die(f"unknown kind {kind!r}")


KINDS = ["boot_line", "unarmed", "none", "posted", "unserviced", "not_applied", "host_twin", "xid_posted"]

if __name__ == "__main__":
    k = sys.argv[1] if len(sys.argv) > 1 else "all"
    if k == "all":
        for kind in KINDS:
            for line in lines(kind):
                print(f"{kind}\t{line}")
    else:
        for line in lines(k):
            print(line)
