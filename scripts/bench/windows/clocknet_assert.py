#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later
"""The clock/network safety assert for every Windows guest launch (stdlib only).

STATUS: LIVE, 2026-11-16 (owner rule, 2026-11-16: "VMs with internet must have a valid clock. The clock can only be faked if
internet is off. Clock fake forces internet off is an assert."). GPU-free tests: scripts/ci/test_clocknet_assert.py; mutants:
scripts/ci/clocknet_assert_mutants.py.

One rule, two checks (distinct codes, so a test can pin each one):

  FAKE_CLOCK_NEEDS_OFFLINE     the guest clock is faked (a recorded or pinned RTC base, or a VM-time clock that does not follow the
                               host) and the network is not provably off.
  INTERNET_NEEDS_REAL_CLOCK    the network is not provably off and the RTC base is not the host's real UTC now (a stale base,
                               a clock that does not follow the host).

"Provably off" means: no NIC at all (`-nic none`, or `-nodefaults` with no network backend), or every backend is QEMU user
networking with `restrict=on`. Anything else (a default NIC, `restrict=off`, a missing `restrict`, tap, socket, bridge, an
unparsable option) counts as internet. A real clock with the network off is fine; so is a real clock with internet.

Use:
  assert_clock_net(Clock(...), Net(...), who)        the plan (what a launcher decided)
  assert_argv(argv, who)                              the product (the QEMU argv that is about to start): parsed, never trusted
  clocknet_assert.py argv -- QEMU ARGS...             shell launchers (exit 3 and `CLOCKNET_REFUSED <CODE>` on stderr)
  clocknet_assert.py line 'QEMU ARGS ...'             the same for a quoted command line (propimage_run.sh)
  clocknet_assert.py file PATH                        the same for a saved command line (last_cmd.txt, with an optional `CMD: ` prefix)
"""
import argparse
import dataclasses
import datetime as dt
import shlex
import sys
from pathlib import Path

REAL_TOLERANCE_S = 300      # a base written out explicitly counts as "now" within this many seconds
CODE_FAKE = "FAKE_CLOCK_NEEDS_OFFLINE"
CODE_REAL = "INTERNET_NEEDS_REAL_CLOCK"
RESTRICT_ON = ("on", "yes", "y", "true")


class ClockNetViolation(ValueError):
    """A launch that breaks the clock/network rule. `code` is one of the two upper-case tokens above."""

    def __init__(self, code, detail=""):
        super().__init__("%s: %s" % (code, detail) if detail else code)
        self.code, self.detail = code, detail


@dataclasses.dataclass(frozen=True)
class Clock:
    faked: bool                      # the launcher pins or records the guest clock
    base: object = None              # datetime (UTC) of an explicit RTC base, None = the host's real clock
    vm_clock: bool = False           # -rtc clock=vm: the guest clock does not follow the host


@dataclasses.dataclass(frozen=True)
class Net:
    off: bool                        # provably no route out of the guest (see the module docstring)
    why: str = ""


def utc_now():
    return dt.datetime.now(dt.timezone.utc)


def real_utc_now(clock, now=None):
    """True when the guest clock starts at the host's real UTC now and follows the host."""
    if clock.vm_clock:
        return False
    if clock.base is None:
        return True
    base = clock.base if clock.base.tzinfo else clock.base.replace(tzinfo=dt.timezone.utc)
    return abs((now or utc_now()) - base).total_seconds() <= REAL_TOLERANCE_S


def assert_clock_net(clock, net, who="launch", now=None):
    """Raise ClockNetViolation unless (clock faked -> network off) and (network not off -> clock real UTC now)."""
    if clock.faked and not net.off:
        raise ClockNetViolation(CODE_FAKE, "%s: the guest clock is faked but the network is not off (%s)" % (who, net.why or "internet"))
    if not net.off and not real_utc_now(clock, now):
        raise ClockNetViolation(CODE_REAL, "%s: the guest has internet but its clock is not the real UTC now" % who)
    return clock, net


# ---------------------------------------------------------------- reading a QEMU argv
def _opts(text):
    """`a=b,c=d,flag` -> ([positional], {key: value}); `,,` is an escaped comma (QEMU)."""
    parts, cur, i = [], "", 0
    while i < len(text):
        if text[i] == "," and text[i + 1:i + 2] == ",":
            cur, i = cur + ",", i + 2
        elif text[i] == ",":
            parts.append(cur)
            cur, i = "", i + 1
        else:
            cur, i = cur + text[i], i + 1
    parts.append(cur)
    pos, kv = [], {}
    for p in parts:
        if "=" in p:
            k, _, v = p.partition("=")
            kv[k.strip()] = v.strip()
        elif p:
            pos.append(p.strip())
    return pos, kv


def _values(argv, *names):
    out = []
    for i, a in enumerate(argv[:-1]):
        if a.lstrip("-") in names and a.startswith("-"):
            out.append(argv[i + 1])
    return out


def clock_from_argv(argv):
    """The guest clock a QEMU argv asks for. No -rtc: QEMU's default, base=utc clock=host (real)."""
    rtc = _values(argv, "rtc")
    if not rtc:
        return Clock(faked=False)
    _, kv = _opts(rtc[-1])
    base = kv.get("base", "utc").lower()
    vm = kv.get("clock", "host").lower() not in ("host", "rt")
    if base in ("utc", "localtime"):
        return Clock(faked=vm, vm_clock=vm)
    for fmt in ("%Y-%m-%dT%H:%M:%S", "%Y-%m-%d"):
        try:
            return Clock(faked=True, base=dt.datetime.strptime(kv["base"], fmt).replace(tzinfo=dt.timezone.utc), vm_clock=vm)
        except ValueError:
            continue
    return Clock(faked=True, base=dt.datetime(1970, 1, 1, tzinfo=dt.timezone.utc), vm_clock=vm)   # unparsable: faked and far from now


def net_from_argv(argv):
    """Provably off or not (see the module docstring)."""
    backends, nic_only = [], False
    saw_none = False
    for flag in ("netdev", "nic", "net"):
        for value in _values(argv, flag):
            pos, kv = _opts(value)
            kind = (pos[0] if pos else kv.get("type", "")).lower()
            if kind == "none":
                saw_none = True
            elif flag == "net" and kind == "nic":
                nic_only = True                              # a NIC; its backend is a separate -net entry (or none: not provable)
            else:
                backends.append((kind, kv))
    if not backends and nic_only:
        return Net(False, "-net nic without a backend")
    if not backends:
        nodefaults = any(a.lstrip("-") == "nodefaults" for a in argv if a.startswith("-"))
        if saw_none or nodefaults:
            return Net(True, "no NIC")
        return Net(False, "QEMU adds a default user NIC")
    for kind, kv in backends:
        if kind != "user":
            return Net(False, "backend %s" % (kind or "?"))
        if kv.get("restrict", "off").lower() not in RESTRICT_ON:
            return Net(False, "user networking without restrict=on")
    return Net(True, "restrict=on")


def assert_argv(argv, who="launch", now=None):
    """Check the argv that is about to be exec'd. -> (clock, net)."""
    argv = [str(a) for a in argv]
    return assert_clock_net(clock_from_argv(argv), net_from_argv(argv), who, now)


def command_words(text):
    text = text.strip()
    if text.startswith("CMD:"):
        text = text[4:].strip()
    return shlex.split(text)


def main(argv=None):
    ap = argparse.ArgumentParser(description=__doc__.split("\n\n")[0])
    sub = ap.add_subparsers(dest="cmd", required=True)
    p = sub.add_parser("argv")
    p.add_argument("args", nargs=argparse.REMAINDER)
    p = sub.add_parser("line")
    p.add_argument("text")
    p = sub.add_parser("file")
    p.add_argument("path")
    for q in sub.choices.values():
        q.add_argument("--who", default="shell launcher")
    a = ap.parse_args(argv)
    try:
        if a.cmd == "argv":
            words = [w for w in a.args if w != "--"] if a.args[:1] == ["--"] else a.args
        elif a.cmd == "line":
            words = command_words(a.text)
        else:
            words = command_words(Path(a.path).read_text().splitlines()[0])
        assert_argv(words, a.who)
    except ClockNetViolation as exc:
        print("CLOCKNET_REFUSED " + str(exc), file=sys.stderr)
        return 3
    except (OSError, ValueError, IndexError) as exc:
        print("CLOCKNET_REFUSED UNREADABLE: " + str(exc), file=sys.stderr)
        return 3
    print("CLOCKNET_OK")
    return 0


if __name__ == "__main__":
    sys.exit(main())
