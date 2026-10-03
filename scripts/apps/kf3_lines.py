#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later
"""kf3_lines.py <kind> [args] — print the kf3 log line(s) of one kind, RENDERED FROM THE RUST SOURCE.

★ 2026-10-03 (review of 5af7e644). The app-matrix classifiers (`loud_verdict.sh`, `boot_gate.sh`,
the hook's per-row counts, `triage.py`) key on the shapes of lines kf3 prints. Their fixtures used
to be hand-typed — and one hand-typed `C′` line (`kf3: REFUSED VasKey(0x1) …`) was not the shape kf3
prints (`kf3: mem t=<s>s REFUSED VasKey(<n>) …`), while the line that actually broke the classifier
(the DELIVERY_UNBUILT boot sentence) was in no fixture at all. This script reads the format strings
and the DELIVERY_UNBUILT constant out of the tree and fills their placeholders, so a fixture IS the
line kf3 prints at this revision.

⊘ CORRECTED 2026-10-03 (review of 9390f51c) — the claim above was stronger than the script. The
first version filled every placeholder with a value that was ALREADY FORMATTED (`"0xc1d0001e"`) and
never read the placeholder's spec, so changing `{:#x}` to `{:x}` in chan.rs left the fixture at
`chan 0xc1d0001e:…` and every test green, while kf3 would print `chan c1d0001e:…`, which
`loud_verdict.sh` and `boot_gate.sh` never match. Now every fill value is a TYPED Rust value (an
integer, a float, a string, a derived-Debug tuple struct such as `VasKey(u64)`) and each placeholder
is formatted BY ITS SPEC, as Rust's `format!` would: `{}` Display, `{:?}` Debug, `{:x}`/`{:#x}`/`{:X}`/
`{:#X}` hex, `{:.N}` fixed-point. A spec this script does not model, a value whose type the spec
cannot format, `{0}`-style indices, a missing or unused named value, a changed positional count, or
`VasKey` losing its derived `Debug` — each exits 2 with the reason, so the fixture test FAILS rather
than test a line kf3 no longer prints. ⚠ What it still cannot see: which VALUE the code passes into a
placeholder (the fixture picks it), and lines it is not asked to render.

Kinds: boot_line, unarmed (all three producers), none, posted, unserviced, not_applied, host_twin,
xid_posted, rc_status [unarmed none [armed]] (the status line's `rc[…]` counters), xid_text
[channels] (the guest Xid-31 text kf3 posts, chan.rs `xid_text`), all (every kind,
`<kind>\\t<line>`). KF3_LINES_ROOT=<tree> renders from another tree (test_verdicts.sh's drift tests).
"""
import os
import re
import sys
from pathlib import Path

ROOT = Path(os.environ.get("KF3_LINES_ROOT") or Path(__file__).resolve().parents[2])
CHAN = ROOT / "crates/kf-qemu/src/chan.rs"
DEVICE = ROOT / "crates/kf-qemu/src/device.rs"
VASMGR = ROOT / "crates/kf-mem/src/vasmgr.rs"
FAULTBUFFER = ROOT / "crates/kf-abi/src/faultbuffer.rs"

LIT = re.compile(r'"((?:[^"\\]|\\.)*)"', re.S)


def die(why):
    print(f"KF3_LINES_DRIFT {why}", file=sys.stderr)
    sys.exit(2)


class Tuple:
    """A Rust tuple struct whose `Debug` is DERIVED: `{:?}` prints `Name(<inner Debug>)`. Built only
    by `derived_tuple()`, which checks the derive in the source first."""

    def __init__(self, name, inner):
        self.name, self.inner = name, inner


def derived_tuple(path, name, field, inner):
    src = path.read_text()
    m = re.search(r"#\[derive\(([^)]*)\)\]\s*pub struct " + re.escape(name) + r"\(pub " + re.escape(field) + r"\);", src)
    if not m or "Debug" not in [d.strip() for d in m.group(1).split(",")]:
        die(f"`pub struct {name}(pub {field});` with a derived Debug not found in {path.relative_to(ROOT)} — "
            f"its `{{:?}}` is no longer `{name}(<n>)`")
    if re.search(r"impl\s+(?:\w+::)*Debug\s+for\s+" + re.escape(name) + r"\b", src):
        die(f"{name} has a hand-written Debug impl in {path.relative_to(ROOT)}")
    return Tuple(name, inner)


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


def fmt_one(fmt, arg, spec, v):
    """One placeholder, as Rust's `format!` prints `v` under `spec` — or exit 2."""
    where = f"{fmt[:60]!r}…: {{{arg}:{spec}}}"
    if isinstance(v, bool):
        die(f"{where}: a bool fill value is not modelled")
    if spec == "":
        if isinstance(v, (int, str)):
            return str(v)
        die(f"{where}: Display of {type(v).__name__} is not modelled")
    if spec == "?":
        if isinstance(v, int):
            return str(v)
        if isinstance(v, Tuple):
            return f"{v.name}({fmt_one(fmt, arg, '?', v.inner)})"
        die(f"{where}: Debug of {type(v).__name__} is not modelled")
    if spec in ("x", "#x", "X", "#X"):
        if not isinstance(v, int) or v < 0:
            die(f"{where}: hex needs a non-negative integer, the fixture gives {v!r}")
        h = format(v, "x" if "x" in spec else "X")
        return ("0x" if spec.startswith("#") else "") + h
    m = re.fullmatch(r"\.(\d+)", spec)
    if m:
        if not isinstance(v, float):
            die(f"{where}: a precision needs a float, the fixture gives {v!r}")
        return f"{v:.{int(m.group(1))}f}"
    die(f"{where}: format spec {spec!r} is not modelled — teach fmt_one() what Rust prints for it")


def render(fmt, pos=(), **named):
    """`fmt` filled the way Rust fills it: `{}`/`{:spec}` take `pos` in order, `{name}`/`{name:spec}`
    take `named[name]` (inline captures; a name may repeat). `{{`/`}}` are literal braces."""
    out, it, used, i = [], iter(pos), set(), 0
    npos = 0
    while i < len(fmt):
        c = fmt[i]
        if fmt.startswith("{{", i) or fmt.startswith("}}", i):
            out.append(c)
            i += 2
            continue
        if c == "}":
            die(f"{fmt!r}: unmatched '}}' at {i}")
        if c != "{":
            out.append(c)
            i += 1
            continue
        j = fmt.find("}", i)
        if j < 0:
            die(f"{fmt!r}: unterminated placeholder at {i}")
        body = fmt[i + 1:j]
        arg, _, spec = body.partition(":")
        if arg == "":
            npos += 1
            try:
                v = next(it)
            except StopIteration:
                die(f"{fmt!r}: more positional placeholders than the fixture fills ({len(pos)})")
        elif re.fullmatch(r"[A-Za-z_]\w*", arg):
            if arg not in named:
                die(f"{fmt!r}: named placeholder {{{arg}}} has no fixture value")
            v = named[arg]
            used.add(arg)
        else:
            die(f"{fmt!r}: placeholder {{{body}}} (an index or expression) is not modelled")
        out.append(fmt_one(fmt, arg, spec, v))
        i = j + 1
    if npos != len(pos):
        die(f"{fmt!r}: {npos} positional placeholder(s), the fixture fills {len(pos)}")
    if set(named) - used:
        die(f"{fmt!r}: the fixture names {sorted(set(named) - used)}, which the format no longer uses")
    return "".join(out)


def delivery_unbuilt():
    src = FAULTBUFFER.read_text()
    m = re.search(r'pub const DELIVERY_UNBUILT: &str = "((?:[^"\\]|\\.)*)";', src, re.S)
    if not m:
        die("DELIVERY_UNBUILT constant not found in crates/kf-abi/src/faultbuffer.rs")
    return unescape(m.group(1))


CLIENT, HANDLE = 0xC1D0001E, 0xCAF00099


def arg(args, i, default):
    try:
        return int(args[i]) if len(args) > i else default
    except ValueError:
        die(f"argument {args[i]!r} is not an integer")


def lines(kind, args=()):
    if kind == "boot_line":
        (fmt,) = literals(DEVICE, "kf3: replayable fault buffer registered (", "")
        return [render(fmt, [delivery_unbuilt()], fb=1)]
    if kind == "unarmed":
        out = []
        for fmt in literals(CHAN, "kf3: chan {", " RC-UNARMED: "):
            # the three producers: `{:#x}:{:#x}` + a gpa, `{:#x}:{:#x}` alone, `{client}:{handle}` + why
            if "{why}" in fmt:
                out.append(render(fmt, client=CLIENT, handle=HANDLE, why="notifier read view: refused"))
            elif "{gpa" in fmt:
                out.append(render(fmt, [CLIENT, HANDLE], gpa=0x1F000))
            else:
                out.append(render(fmt, [CLIENT, HANDLE]))
        if len(out) < 3:
            die(f"expected three RC-UNARMED producers in chan.rs, found {len(out)}")
        return out
    if kind == "none":
        return [render(f, [CLIENT, HANDLE]) for f in literals(CHAN, "kf3: chan {", " RC-NONE: ")]
    if kind == "posted":
        (fmt,) = literals(DEVICE, "kf3: RC_TRIGGERED posted: ", "")
        return [render(fmt, [0xB, 0x1, 0x1F, 0x22])]
    if kind == "unserviced":
        (fmt,) = literals(CHAN, "kf3: UNSERVICED-GPU-FAULT guest client {", "")
        # chids: the joined `{c:#x}` strings; the last `{}`: the rate-limit note (empty when none held)
        return [render(fmt, ["0xb 0x8", ""], client=CLIENT, mmu=31)]
    if kind == "not_applied":
        (outer,) = literals(DEVICE, "kf3: mem t={:.3}s REFUSED ", "")
        (inner,) = literals(VASMGR, "{key:?} root {walked_root:#x}: ", " run(s) not applied: ")
        key = derived_tuple(VASMGR, "VasKey", "u64", 0xC1D0001E_CAF00099)
        why = render(inner, [1, "map 0x7c70d8400000+0x10000: Other(31)"], key=key, walked_root=0x201000)
        return [render(outer, [52.101], why=why)]
    if kind == "host_twin":
        (fmt,) = literals(CHAN, "kf3: RC host twin {", "")
        return [render(fmt, [0x22, 0xB, 0x1, 0x1F, 0x1F])]
    if kind == "xid_posted":
        (fmt,) = literals(DEVICE, "kf3: OS_ERROR_LOG posted: ", "")
        return [render(fmt, client=CLIENT, except_type=31, chid=0xB, runlist_id=0, channels=8)]
    if kind == "rc_status":
        # the `rc[…]` counters of kf3's 2 s status line, after a fixed `kf3: family=… phase=…` head
        # (the classifiers grep only the `rc[armed= unarmed= none=` run of it)
        unarmed, none, armed = arg(args, 0, 0), arg(args, 1, 0), arg(args, 2, 8)
        (fmt,) = literals(DEVICE, " views[armed={va} released={vr} refused={vx} held={}] rc[", "")
        frag = render(fmt, [0, armed, unarmed, none, 0, 0, 0, 0, 0, 0, 0], va=0, vr=0, vx=0)
        return ["kf3: family=Ampere phase=Running trapped=1" + frag]
    if kind == "xid_text":
        (fmt,) = literals(CHAN, "kayfabe: unserviced GPU page fault; ", "")
        return [render(fmt, channels=arg(args, 0, 8))]
    die(f"unknown kind {kind!r}")


KINDS = ["boot_line", "unarmed", "none", "posted", "unserviced", "not_applied", "host_twin", "xid_posted",
         "rc_status", "xid_text"]

if __name__ == "__main__":
    k = sys.argv[1] if len(sys.argv) > 1 else "all"
    if k == "all":
        for kind in KINDS:
            for line in lines(kind):
                print(f"{kind}\t{line}")
    else:
        for line in lines(k, sys.argv[2:]):
            print(line)
