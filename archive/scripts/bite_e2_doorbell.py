#!/usr/bin/env python3
"""Bite harness for increment **E2** — the usermode doorbell transport.

`docs/design/execution_plane_increments.md` E2 asks for *"a boot in which a guest doorbell
write produces a `DoorbellOutcome`-or-named-`FwdFault`, counted"* with the control *"a
non-doorbell BAR write in the same run produces neither"*. Both guards went green on the
first run, which is exactly when they are worth doubting: a transport test can be green
because the transport works, or because the assertion cannot fail.

So: plant a defect, require the guards to go RED, restore, and report. A guard nobody has
watched fail is not a guard (`only_live_boots_are_proof`, `suspect_the_instrument_first`).

★★★ **Two arms, because the point is WHICH instrument catches WHICH defect.**

  - `device` — `cargo test -p kayfabe-device --test doorbell_aperture`: the transport
    against a *recording* port. It can see routing, widths, apertures, counters and the
    log; its own docs say it witnesses nothing about the core.
  - `shim`   — `cargo test -p kayfabe-qemu-raw --test e2_doorbell`: the composition root,
    where the port really is `SharedDevice::doorbell` over the object model the bridge
    declares into.

A bite only `shim` catches is a bite about the *join*; one only `device` catches is about
the plane. Both numbers are printed. ⊘ Neither is asserted — asserting the split would be
asserting the conclusion.

Usage:
    scripts/bite_e2_doorbell.py [--only N] [--list]

Every bite is applied to the working tree and restored afterwards, and the file's mtime is
touched after restoring: `shutil`/`cp -a` preserve mtime, cargo then serves a stale rlib,
and the next bite is measured against the previous one's binary
(`bite_harness_must_touch_after_restore`).
"""

import argparse
import os
import subprocess
import sys
import time

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
PLANE = os.path.join(ROOT, "crates/kayfabe-device/src/plane.rs")
PORT = os.path.join(ROOT, "crates/kayfabe-device/src/doorbell.rs")
SHIM = os.path.join(ROOT, "crates/kayfabe-qemu-raw/src/shim.rs")

DEVICE_TEST = ["cargo", "test", "-q", "-p", "kayfabe-device", "--test", "doorbell_aperture"]
SHIM_TEST = ["cargo", "test", "-q", "-p", "kayfabe-qemu-raw", "--test", "e2_doorbell"]

RED = "RED"

# ---------------------------------------------------------------------------------------
# The bites. Each is (name, file, anchor, replacement, what it would mean in production).
# ---------------------------------------------------------------------------------------

CLASSIFY = """        if bar == kayfabe_abi::pcibars::bus_bar::REGS as u8
            && let Some(reg) = self.doorbell_reg()
            && off == reg
        {
            return self.ring_doorbell(mask(val, size));
        }"""

BITES = [
    (
        "B1 the classification is deleted — the ring falls through and is DROPPED",
        PLANE,
        CLASSIFY,
        "",
        "the guest's entire submission is counted as an unclaimed write and lost",
    ),
    (
        "B2 the aperture check is dropped — BAR2 offset 0xbb0090 rings too",
        PLANE,
        "        if bar == kayfabe_abi::pcibars::bus_bar::REGS as u8\n"
        "            && let Some(reg) = self.doorbell_reg()",
        "        if let Some(reg) = self.doorbell_reg()",
        "a translated-window access at the same offset is forwarded as a work submission",
    ),
    (
        "B3 the token is not masked to the access width",
        PLANE,
        "            return self.ring_doorbell(mask(val, size));",
        "            return self.ring_doorbell(val);",
        "a byte store rings whatever rubbish the upper 56 bits of the caller's word held",
    ),
    (
        "B4 arrivals are counted only when the core SERVES them",
        PLANE,
        "        self.c.doorbells.fetch_add(1, Ordering::Relaxed);\n        let report = {",
        "        let report = {",
        "a boot in which every ring was refused reports zero rings: the transport looks "
        "untouched and the diagnosis is 'the guest never got there'",
    ),
    (
        "B5 the LAST refusal replaces the first",
        PLANE,
        "            if log.first_refusal.is_none()\n"
        "                && let DoorbellReport::Refused { refusal, .. } = &report",
        "            if let DoorbellReport::Refused { refusal, .. } = &report",
        "a flood of later rings pushes the diagnosis out of the one line a report has room for",
    ),
    (
        "B6 a device reset leaves the previous guest's ring in the log",
        PLANE,
        "        *self.doorbell_log.lock().unwrap_or_else(|e| e.into_inner()) = DoorbellLog::default();",
        "",
        "the next guest's teardown report carries the previous guest's work-submit token",
    ),
    (
        "B7 the doorbell offset stops being derived from the advertised base",
        PORT,
        "pub const USERMODE_DOORBELL_OFF: u64 = 0x90;",
        "pub const USERMODE_DOORBELL_OFF: u64 = 0x80;",
        "the device decodes a different offset from the one it told the driver to map: "
        "every ring is answered with a defaulted zero and vanishes",
    ),
    (
        "B8 the default port becomes a silent SINK instead of a named refusal",
        PORT,
        """    fn ring(&self, token: u64) -> DoorbellReport {
        DoorbellReport::Refused {
            token,
            refusal: DoorbellRefused {
                kind: NO_DOORBELL_PORT_KIND,
                why: String::from(NO_DOORBELL_PORT),
            },
        }
    }""",
        """    fn ring(&self, token: u64) -> DoorbellReport {
        DoorbellReport::Served {
            token,
            proc: 0,
            chan: 0,
            host_token: 0,
            scheduled_now: false,
        }
    }""",
        "a build with no forwarding plane reports every guest submission as SERVED",
    ),
    (
        "B9 the composition root never installs the real port",
        SHIM,
        "        plane.set_doorbell(Box::new(SharedDoorbell(Arc::clone(&device))));",
        "",
        "the archive counts rings and forwards none — the E0 shape, one seam over",
    ),
    (
        "B10 the doorbell rings a SECOND object model, not the bridge's",
        SHIM,
        "        plane.set_doorbell(Box::new(SharedDoorbell(Arc::clone(&device))));",
        """        let second = Arc::new(kayfabe_rt::device::SharedDevice::new(
            kayfabe_core::gpu::Gpu::new(
                Box::new(kayfabe_chips::Ga10xArch::new()),
                isolate_factory(selected_isolate_plane()?)?,
                kayfabe_core::gpa::GpaSpace::new(OBJECT_GPA_WINDOW, OBJECT_GPA_ARENA),
            )
            .expect("realizes"),
            kayfabe_rt::device::LockMode::Sharded,
        ));
        plane.set_doorbell(Box::new(SharedDoorbell(second)));""",
        "★ THE SILENT ONE: every guest ring is routed against a graph nothing declares "
        "into, so `UnknownVchid` is the permanent answer and no behavioural test can tell",
    ),
]


def run(cmd):
    p = subprocess.run(cmd, cwd=ROOT, capture_output=True, text=True)
    return p.returncode, (p.stdout + p.stderr)


def apply_bite(path, anchor, repl):
    with open(path) as f:
        text = f.read()
    n = text.count(anchor)
    if n != 1:
        return None, n
    return text, text.replace(anchor, repl, 1)


def restore(path, text):
    with open(path, "w") as f:
        f.write(text)
    # ★ mtime, explicitly. See the module docstring.
    now = time.time()
    os.utime(path, (now, now))


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--only", type=int, help="run one bite, by 1-based index")
    ap.add_argument("--list", action="store_true")
    args = ap.parse_args()

    if args.list:
        for i, (name, path, _, _, why) in enumerate(BITES, 1):
            print(f"{i:2d}. [{os.path.basename(path)}] {name}\n      would mean: {why}")
        return 0

    print("== sanity: the restored tree must be GREEN before anything is planted")
    for label, cmd in (("device", DEVICE_TEST), ("shim", SHIM_TEST)):
        rc, out = run(cmd)
        if rc != 0:
            print(f"★ the {label} arm is ALREADY RED — fix that before measuring bites")
            print(out[-4000:])
            return 2
    print("   both arms green")

    rows = []
    for i, (name, path, anchor, repl, why) in enumerate(BITES, 1):
        if args.only and args.only != i:
            continue
        original, bitten = apply_bite(path, anchor, repl)
        if original is None:
            print(f"{i:2d}. {name}\n    ★ ANCHOR MATCHED {bitten} TIMES — bite NOT applied")
            rows.append((name, "ANCHOR", "ANCHOR"))
            continue
        try:
            with open(path, "w") as f:
                f.write(bitten)
            now = time.time()
            os.utime(path, (now, now))
            dev_rc, dev_out = run(DEVICE_TEST)
            shim_rc, shim_out = run(SHIM_TEST)
        finally:
            restore(path, original)
        # ⊘ A bite that does not COMPILE is not a caught bite: it is a bite that was never
        # measured, and reporting it as RED is how a harness flatters itself.
        def verdict(rc, out):
            if "error[E" in out or "error: could not compile" in out:
                return "NOCOMPILE"
            return RED if rc != 0 else "green"

        dev = verdict(dev_rc, dev_out)
        shim = verdict(shim_rc, shim_out)
        caught = RED in (dev, shim)
        mark = "✔" if caught else "★ MISSED"
        print(f"{i:2d}. {mark}  device={dev:9s} shim={shim:9s}  {name}")
        if not caught:
            print(f"      would mean in production: {why}")
        rows.append((name, dev, shim))

    fired = sum(1 for _, d, s in rows if RED in (d, s))
    nocompile = sum(1 for _, d, s in rows if "NOCOMPILE" in (d, s) and RED not in (d, s))
    print(f"\n== {fired}/{len(rows)} bites caught  ({nocompile} did not compile)")

    print("== sanity: the restored tree is green again")
    for label, cmd in (("device", DEVICE_TEST), ("shim", SHIM_TEST)):
        rc, _ = run(cmd)
        print(f"   {label}: {'GREEN' if rc == 0 else '★ RED — the restore is broken'}")
        if rc != 0:
            return 2
    return 0 if fired == len(rows) else 1


if __name__ == "__main__":
    sys.exit(main())
