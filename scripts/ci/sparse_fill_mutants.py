#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later
"""Mutation harness for the `unmapped-reads-zero` property and the refusal of a `0x51` inside a reservation of ours
(STATUS: LIVE, 2026-11-25: the property is on/off/auto and defaults to auto, on for a default-start space and off for a
floor-started one; docs/design/V3_SPARSE_RESIDENCY_RCA.md): kf_host::channel::GuestVaFill, GuestVaFillByStart and
guest_range_plan, kf_qemu::device::Config::guest_va_fill_by_start, UnmappedReadsZero and UNMAPPED_READS_ZERO_DEFAULT, the C
default in kf3.c and the C-to-Rust wiring (source pins in crates/kf-qemu/tests/unmapped_reads_zero_use_sites.rs), the
kf-mem host model's S1/S2 tests, and kf_host::channel::own_reservation_answer / map_pieces (a FIXED map through a
reservation of ours that host RM answers 0x51 is refused by name, never reported as already mapped).

Each mutant is a one-site edit of production code; each must be killed by at least two independently named
failing tests. A compile error is not a kill. Every file touched is overwritten in place and ALWAYS restored
(also on an interrupt). NOT covered GPU-free (needs host RM): alloc_vaspace_with passing the plan's flags to
reserve_va_flags (the source pin covers the text, not the call), and HostRm::map_in calling map_through_reservation
(one source pin, kf-host `map_in_answers_through_the_reservation_seam`); the box run's log lines and verdict cover them.
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
RESV = [["cargo", "test", "-p", "kf-host", "--lib", "split_row_tests"]]


def ed(rel, old, new):
    assert FILES[rel].count(old) == 1, (rel, old, FILES[rel].count(old))
    return rel, FILES[rel].replace(old, new)


CASES = [
    ("zero-fill-without-sparse", ed(CH, "Self::Zero => NVOS32_RESERVE_FLAGS | NVOS32_ALLOC_FLAGS_SPARSE,", "Self::Zero => NVOS32_RESERVE_FLAGS,"), HOST + MEM),
    ("fault-fill-is-sparse", ed(CH, "Self::Fault => NVOS32_RESERVE_FLAGS,", "Self::Fault => NVOS32_RESERVE_FLAGS | NVOS32_ALLOC_FLAGS_SPARSE,"), HOST + MEM),
    ("default-fill-is-zero", ed(CH, "    #[default]\n    Fault,\n    /// SPARSE", "    Fault,\n    #[default]\n    /// SPARSE"), HOST + MEM),
    ("plan-ignores-the-fill", ed(CH, "    let flags = fill.reserve_flags();\n    GUEST_VA_RANGES", "    let flags = { let _ = fill; NVOS32_RESERVE_FLAGS };\n    GUEST_VA_RANGES"), HOST),
    ("plan-drops-the-low-range", ed(CH, "        .chain(start.low_range())\n        .map(move |(lo, hi)| (lo, hi, flags))", "        .map(move |(lo, hi)| (lo, hi, flags))"), HOST),
    ("auto-is-zero-for-a-windows-space", ed(DEV, "                guest_floor: GuestVaFill::Fault,\n            },\n            UnmappedReadsZero::On", "                guest_floor: GuestVaFill::Zero,\n            },\n            UnmappedReadsZero::On"), QEMU + SITES),
    ("auto-is-fault-for-a-linux-space", ed(DEV, "                host_default: GuestVaFill::Zero,\n                guest_floor: GuestVaFill::Fault,", "                host_default: GuestVaFill::Fault,\n                guest_floor: GuestVaFill::Fault,"), QEMU + SITES),
    ("on-is-fault", ed(DEV, "UnmappedReadsZero::On => GuestVaFillByStart::uniform(GuestVaFill::Zero),", "UnmappedReadsZero::On => GuestVaFillByStart::uniform(GuestVaFill::Fault),"), QEMU + SITES),
    ("off-is-zero", ed(DEV, "UnmappedReadsZero::Off => GuestVaFillByStart::uniform(GuestVaFill::Fault),", "UnmappedReadsZero::Off => GuestVaFillByStart::uniform(GuestVaFill::Zero),"), QEMU + SITES),
    ("default-const-on", ed(DEV, "pub const UNMAPPED_READS_ZERO_DEFAULT: UnmappedReadsZero = UnmappedReadsZero::Auto;", "pub const UNMAPPED_READS_ZERO_DEFAULT: UnmappedReadsZero = UnmappedReadsZero::On;"), QEMU + SITES),
    ("c-default-on", ed(KF3C, "unmapped_reads_zero, ON_OFF_AUTO_AUTO),", "unmapped_reads_zero, ON_OFF_AUTO_ON),"), SITES),
    ("abi-word-swaps-on-and-off", ed(DEV, "            1 => Some(Self::On),\n            2 => Some(Self::Off),", "            1 => Some(Self::Off),\n            2 => Some(Self::On),"), QEMU + SITES),
    ("abi-word-unknown-is-auto", ed(DEV, "            2 => Some(Self::Off),\n            _ => None,", "            2 => Some(Self::Off),\n            _ => Some(Self::Auto),"), QEMU + SITES),
    ("linux-space-faults", ed(CH, "    let flags = fill.reserve_flags();\n    GUEST_VA_RANGES", "    let flags = if matches!(start, MirrorVaStart::HostDefault) { NVOS32_RESERVE_FLAGS } else { fill.reserve_flags() };\n    GUEST_VA_RANGES"), HOST + QEMU),
    ("ffi-ignores-the-word", ed(FFI, "crate::device::UnmappedReadsZero::from_abi(unmapped_reads_zero)", "crate::device::UnmappedReadsZero::from_abi(0)"), SITES),
    ("realize-hardwires-fault", ed(DEV, "rm.set_guest_va_fill_by_start(cfg.guest_va_fill_by_start());", "rm.set_guest_va_fill_by_start(kf_host::channel::GuestVaFillByStart::default());"), SITES),
    ("by-start-swapped", ed(CH, "            MirrorVaStart::HostDefault => self.host_default,\n            MirrorVaStart::GuestFloor => self.guest_floor,", "            MirrorVaStart::HostDefault => self.guest_floor,\n            MirrorVaStart::GuestFloor => self.host_default,"), HOST + QEMU),
    ("uniform-leaves-the-floor-at-fault", ed(CH, "            host_default: fill,\n            guest_floor: fill,", "            host_default: fill,\n            guest_floor: GuestVaFill::Fault,"), HOST + QEMU),
    ("reservation-0x51-stays-already-mapped", ed(CH, "            Err(RmError::Other(VA_RESERVED_MAP_REFUSED))\n        }\n        other => other,", "            Err(RmError::Other(crate::VA_ALREADY_MAPPED))\n        }\n        other => other,"), RESV),
    ("pieces-skip-the-reservation-answer", ed(CH, "        if space.is_guest_reservation(h) {\n            map_through_reservation(v, h, p)", "        if space.is_guest_reservation(h) && p.len == u64::MAX {\n            map_through_reservation(v, h, p)"), RESV),
    ("the-space-range-counts-as-a-reservation", ed(CH, "        h_dma != 0 && h_dma != self.range && self.guest.iter().any(|g| g.handle == h_dma)", "        h_dma != 0"), RESV),
    ("reservation-answer-renames-every-refusal", ed(CH, "        Err(RmError::Other(crate::VA_ALREADY_MAPPED)) => {\n            kf_arch::rate_eprintln!(", "        Err(_) => {\n            kf_arch::rate_eprintln!("), RESV),
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
        base = run({}, HOST + MEM + QEMU + SITES + RESV)
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
