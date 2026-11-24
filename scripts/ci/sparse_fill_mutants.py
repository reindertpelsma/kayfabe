#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later
"""Mutation harness for the `unmapped-reads-zero` property (STATUS: LIVE, 2026-11-24: the property defaults to ON for every
guest and profile; docs/design/V3_SPARSE_RESIDENCY_RCA.md): kf_host::channel::GuestVaFill and guest_range_plan,
kf_qemu::device::Config::guest_va_fill and UNMAPPED_READS_ZERO_DEFAULT, the C default in kf3.c and the C-to-Rust wiring
(source pins in crates/kf-qemu/tests/unmapped_reads_zero_use_sites.rs), and the kf-mem host model's S1/S2 tests.

Each mutant is a one-site edit of production code; each must be killed by at least two independently named
failing tests. A compile error is not a kill. Every file touched is overwritten in place and ALWAYS restored
(also on an interrupt). NOT covered GPU-free (needs host RM): alloc_vaspace_with passing the plan's flags to
reserve_va_flags (the source pin covers the text, not the call); the box run's log line and verdict cover them.
Run: python3 scripts/ci/sparse_fill_mutants.py [CASE ...]   (exit 0 only if no mutant is weak)
"""

import os
import re
import subprocess
import sys

HERE = os.path.dirname(os.path.abspath(__file__))
ROOT = os.path.dirname(os.path.dirname(HERE))
CH = "crates/kf-host/src/channel.rs"
DEV = "crates/kf-qemu/src/device.rs"
FFI = "crates/kf-qemu/src/ffi_unsafe.rs"
KF3C = "qemu/hw/misc/kf3/kf3.c"
FILES = {}
for rel in (CH, DEV, FFI, KF3C):
    with open(os.path.join(ROOT, rel), encoding="utf-8") as f:
        FILES[rel] = f.read()

HOST = [["cargo", "test", "-p", "kf-host", "--lib", "guest_va_fill"]]
MEM = [["cargo", "test", "-p", "kf-mem", "--lib", "sim::sparse"]]
QEMU = [["cargo", "test", "-p", "kf-qemu", "--lib", "config_tests"]]
SITES = [["cargo", "test", "-p", "kf-qemu", "--test", "unmapped_reads_zero_use_sites"]]


def ed(rel, old, new):
    assert FILES[rel].count(old) == 1, (rel, old, FILES[rel].count(old))
    return rel, FILES[rel].replace(old, new)


CASES = [
    ("zero-fill-without-sparse", ed(CH, "Self::Zero => NVOS32_RESERVE_FLAGS | NVOS32_ALLOC_FLAGS_SPARSE,", "Self::Zero => NVOS32_RESERVE_FLAGS,"), HOST + MEM),
    ("fault-fill-is-sparse", ed(CH, "Self::Fault => NVOS32_RESERVE_FLAGS,", "Self::Fault => NVOS32_RESERVE_FLAGS | NVOS32_ALLOC_FLAGS_SPARSE,"), HOST + MEM),
    ("default-fill-is-zero", ed(CH, "    #[default]\n    Fault,\n    /// SPARSE", "    Fault,\n    #[default]\n    /// SPARSE"), HOST + MEM),
    ("plan-ignores-the-fill", ed(CH, "    let flags = fill.reserve_flags();\n    GUEST_VA_RANGES", "    let flags = { let _ = fill; NVOS32_RESERVE_FLAGS };\n    GUEST_VA_RANGES"), HOST),
    ("plan-drops-the-low-range", ed(CH, "        .chain(start.low_range())\n        .map(move |(lo, hi)| (lo, hi, flags))", "        .map(move |(lo, hi)| (lo, hi, flags))"), HOST),
    ("config-ignores-the-property", ed(DEV, "        if self.unmapped_reads_zero {\n            kf_host::channel::GuestVaFill::Zero", "        if false && self.unmapped_reads_zero {\n            kf_host::channel::GuestVaFill::Zero"), QEMU),
    ("config-always-zero", ed(DEV, "        if self.unmapped_reads_zero {\n            kf_host::channel::GuestVaFill::Zero", "        if true || self.unmapped_reads_zero {\n            kf_host::channel::GuestVaFill::Zero"), QEMU),
    ("default-const-off", ed(DEV, "pub const UNMAPPED_READS_ZERO_DEFAULT: bool = true;", "pub const UNMAPPED_READS_ZERO_DEFAULT: bool = false;"), QEMU + SITES),
    ("c-default-off", ed(KF3C, "unmapped_reads_zero, true),", "unmapped_reads_zero, false),"), SITES),
    ("linux-space-faults", ed(CH, "    let flags = fill.reserve_flags();\n    GUEST_VA_RANGES", "    let flags = if matches!(start, MirrorVaStart::HostDefault) { NVOS32_RESERVE_FLAGS } else { fill.reserve_flags() };\n    GUEST_VA_RANGES"), HOST + QEMU),
    ("ffi-inverts-the-word", ed(FFI, "unmapped_reads_zero: unmapped_reads_zero != 0,", "unmapped_reads_zero: unmapped_reads_zero == 0,"), SITES),
    ("realize-hardwires-fault", ed(DEV, "rm.set_guest_va_fill(cfg.guest_va_fill());", "rm.set_guest_va_fill(kf_host::channel::GuestVaFill::Fault);"), SITES),
]


def run(overrides, cmds):
    for rel, text in overrides.items():
        with open(os.path.join(ROOT, rel), "w", encoding="utf-8") as f:
            f.write(text)
    failed = []
    for cmd in cmds:
        p = subprocess.run(cmd, cwd=ROOT, capture_output=True, text=True)
        out = p.stdout + p.stderr
        if "could not compile" in out:
            raise SystemExit("the mutant does not compile:\n" + out[-800:])
        failed += ["%s:%s" % (cmd[3], n) for n in re.findall(r"(?m)^test (\S+) \.\.\. FAILED$", out)]
        if p.returncode != 0 and not re.search(r"(?m)^test (\S+) \.\.\. FAILED$", out):
            raise SystemExit("cargo failed without a failing test:\n" + out[-800:])
    return sorted(set(failed))


def restore():
    for rel, text in FILES.items():
        with open(os.path.join(ROOT, rel), "w", encoding="utf-8") as f:
            f.write(text)


def main():
    want = set(sys.argv[1:])
    cases = [c for c in CASES if not want or c[0] in want]
    try:
        base = run({}, HOST + MEM + QEMU)
        assert not base, "baseline is not green: %s" % base
        print("baseline: green")
        weak = []
        for name, (rel, text), cmds in cases:
            killers = run({rel: text}, cmds)
            restore()
            print("%-30s killers=%d %s %s" % (name, len(killers), "" if len(killers) >= 2 else " <-- WEAK", killers))
            if len(killers) < 2:
                weak.append(name)
        print("MUTANTS total=%d weak=%d" % (len(cases), len(weak)))
        return 1 if weak else 0
    finally:
        restore()


if __name__ == "__main__":
    sys.exit(main())
