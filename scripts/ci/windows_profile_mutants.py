#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later
"""Mutation harness for the Windows support profile (STATUS: LIVE, 2026-11-15).

The measured Windows profile is the device's default for a guest that DECLARES the Windows driver at fn 1, and
`windows-support=off` is its one opt-out (`docs/design/V3_WINDOWS_SUPPORT_PROFILE.md`, OWNER_RULINGS section AT).
The claim to protect is "a Linux guest, an undeclared guest and a `windows-support=off` VM reach none of it". Each
mutant below is a one-site edit of production code that would break that claim (or the Windows side of it); each
must be killed by at least two independently named failing tests. A compile error is not a kill. Every file touched
is overwritten in place and ALWAYS restored (also on an interrupt); needs a toolchain and the workspace's
dependencies, no GPU.

Equivalent mutants (documented, not run): removing `WindowsOnly` from the software-runlist host link (the link
checks the same identity itself), or the `windows.support()` guards on links already behind `WindowsOnly` (an
inactive profile answers nothing either way).

Run: python3 scripts/ci/windows_profile_mutants.py [CASE ...]   (exit 0 only if no mutant is weak)
"""

import os
import re
import subprocess
import sys

HERE = os.path.dirname(os.path.abspath(__file__))
ROOT = os.path.dirname(os.path.dirname(HERE))
WP = "crates/kf-rm/src/winprofile.rs"
LIB = "crates/kf-rm/src/lib.rs"
CL = "crates/kf-rm/src/chanlink.rs"
POL = "crates/kf-rm/src/rmrpc/policy.rs"
SRP = "crates/kf-rm/src/sw_runlist_probe.rs"
KDISP = "crates/kf-rm/src/display.rs"
CHAN = "crates/kf-qemu/src/chan.rs"
MEM = "crates/kf-qemu/src/mem.rs"
QDISP = "crates/kf-qemu/src/display.rs"
FILES = {}
for rel in (WP, LIB, CL, POL, SRP, KDISP, CHAN, MEM, QDISP):
    with open(os.path.join(ROOT, rel), encoding="utf-8") as f:
        FILES[rel] = f.read()

RM = [
    ["cargo", "test", "-p", "kf-rm", "--test", "windows_profile_chain"],
    ["cargo", "test", "-p", "kf-rm", "--test", "memory_list_chain"],
    ["cargo", "test", "-p", "kf-rm", "--test", "sw_runlist_probe_chain"],
    ["cargo", "test", "-p", "kf-rm", "--lib", "--", "winprofile", "chanlink", "sw_runlist_probe"],
]
QEMU = [
    [
        "cargo", "test", "-p", "kf-qemu", "--lib", "--",
        "windows_birth", "twin_defapi", "mirror_space", "spare", "recycled", "retired", "created_twin",
        "sdr", "caps_page", "declaration", "reseal", "single_page", "lab_override",
    ]
]


def ed(rel, old, new):
    assert FILES[rel].count(old) == 1, (rel, old, FILES[rel].count(old))
    return rel, FILES[rel].replace(old, new)


CASES = [
    # --- the witness (kf-rm winprofile.rs)
    ("active-ignores-the-property", ed(WP, "self.support && self.declared.load(Ordering::Acquire)", "self.declared.load(Ordering::Acquire)"), RM),
    ("active-ignores-the-declaration", ed(WP, "self.support && self.declared.load(Ordering::Acquire)", "self.support"), RM),
    ("wrapper-answers-always", ed(WP, "        if self.profile.active() {\n            self.inner.respond(cmd)\n        } else {\n            None\n        }", "        self.inner.respond(cmd)"), RM),
    ("wrapper-holds-always", ed(WP, "self.profile.active() && self.inner.holds_for_refresh(cmd)", "self.inner.holds_for_refresh(cmd)"), RM),
    ("observer-reads-every-command", ed(WP, "        if cmd.function == RpcFunction::SetGuestSystemInfo {\n            self.profile", "        if true {\n            self.profile"), RM),
    ("everything-declares-windows", ed(WP, "kf_abi::hoststub::cell(version).is_some_and(|c| c.matches_identity(payload))", "kf_abi::hoststub::cell(version).is_some()"), RM),
    # --- the chain (kf-rm lib.rs)
    ("observer-not-seated", ed(LIB, "    chain.push(Box::new(kf_gsp::Observing(Box::new(\n        winprofile::IdentityObserver::new(driver.driver_version(), windows.clone()),\n    ))));\n", ""), RM),
    ("host-stub-built-on-an-off-vm", ed(LIB, "    if windows.support() {\n        chain.push(Box::new(hoststub::HostStubPolicy::new(driver)));", "    if true {\n        chain.push(Box::new(hoststub::HostStubPolicy::new(driver)));"), RM),
    ("preempt-bind-ungated", ed(LIB, "        chain.push(Box::new(winprofile::WindowsOnly::new(\n            preempt_bind_probe::PreemptBindProbe::new(driver),\n            windows.clone(),\n        )));", "        chain.push(Box::new(preempt_bind_probe::PreemptBindProbe::new(driver)));"), RM),
    ("zcull-bind-ungated", ed(LIB, "        chain.push(Box::new(winprofile::WindowsOnly::new(\n            preempt_bind_probe::ZcullBindProbe::new(driver),\n            windows.clone(),\n        )));", "        chain.push(Box::new(preempt_bind_probe::ZcullBindProbe::new(driver)));"), RM),
    ("display-ctrl-ungated", ed(LIB, "        chain.push(Box::new(winprofile::WindowsOnly::new(\n            display_ctrl_probe::DisplayCtrlProbe::new(driver),\n            windows.clone(),\n        )));", "        chain.push(Box::new(display_ctrl_probe::DisplayCtrlProbe::new(driver)));"), RM),
    ("gfx-pool-ungated", ed(LIB, "        chain.push(Box::new(winprofile::WindowsOnly::new(\n            gfxpool_probe::GfxPoolProbe { driver, served: 0 },\n            windows.clone(),\n        )));", "        chain.push(Box::new(gfxpool_probe::GfxPoolProbe { driver, served: 0 }));"), RM),
    ("memory-list-always-active", ed(POL, ".is_some_and(crate::winprofile::WindowsProfile::active)", ".is_some()"), RM),
    ("sw-runlist-probe-sees-every-guest", ed(SRP, "if !sw_runlist::is_probe_class(req.class) || !self.profile.active() {", "if !sw_runlist::is_probe_class(req.class) {"), RM),
    ("imp-always-true", ed(KDISP, "g.report_imp_enabled(self.windows.as_ref().is_some_and(|w| w.active()));", "g.report_imp_enabled(true);"), RM),
    ("imp-follows-the-property-not-the-declaration", ed(KDISP, "self.windows.as_ref().is_some_and(|w| w.active())", "self.windows.as_ref().is_some_and(|w| w.support())"), RM),
    # --- the channel link (kf-rm chanlink.rs)
    ("identity-ignores-the-property", ed(CL, "self.windows_identity = self.windows_support\n                    && kf_abi::hoststub::cell(", "self.windows_identity = true\n                    && kf_abi::hoststub::cell("), RM),
    ("every-fn1-is-windows", ed(CL, ".is_some_and(|c| c.matches_identity(&cmd.payload));\n                None", ".is_some();\n                None"), RM),
    ("async-preempt-ungated", ed(CL, "                    self.windows_identity,\n                    d.runlist_preempt_event,", "                    true,\n                    d.runlist_preempt_event,"), RM),
    ("alloc-always-carries-the-windows-fact", ed(CL, "            user_work: user_work.is_ok(),\n            windows: self.windows_identity,", "            user_work: user_work.is_ok(),\n            windows: true,"), RM),
    ("user-work-ignores-the-identity", ed(CL, "            flag: self.windows_support,\n            windows: self.windows_identity,", "            flag: self.windows_support,\n            windows: true,"), RM),
    # --- the channel plane (kf-qemu chan.rs)
    ("birth-kernel-gr-always", ed(CHAN, "            kernel_gr_ce: windows,", "            kernel_gr_ce: true,"), QEMU),
    ("birth-gr-work-always", ed(CHAN, "            gr_work: windows,", "            gr_work: true,"), QEMU),
    ("birth-deferred-api-always", ed(CHAN, "            deferred_api: windows,", "            deferred_api: true,"), QEMU),
    ("birth-ce-relay-always", ed(CHAN, "            ce_relay: windows,", "            ce_relay: true,"), QEMU),
    ("birth-host-owned-scheduling-always", ed(CHAN, "            scheduled_at_birth: windows,", "            scheduled_at_birth: true,"), QEMU),
    ("birth-reads-user-work-not-the-identity", ed(CHAN, "        WindowsBirth::of(a.windows)", "        WindowsBirth::of(a.user_work)"), QEMU),
    ("kernel-gr-for-a-user-channel", ed(CHAN, "        kernel_work && engine == kf_abi::submit::ENGINE_TYPE_GRAPHICS && self.kernel_gr_ce", "        engine == kf_abi::submit::ENGINE_TYPE_GRAPHICS && self.kernel_gr_ce"), QEMU),
    ("kernel-gr-for-any-engine", ed(CHAN, "        kernel_work && engine == kf_abi::submit::ENGINE_TYPE_GRAPHICS && self.kernel_gr_ce", "        kernel_work && self.kernel_gr_ce"), QEMU),
    ("gr-work-without-the-ring", ed(CHAN, "        kernel_gr && self.gr_work", "        self.gr_work"), QEMU),
    ("ce-relay-for-the-gr-tier", ed(CHAN, "        !gr_tier && self.ce_relay && copy_engine && advanced", "        self.ce_relay && copy_engine && advanced"), QEMU),
    ("ce-relay-without-an-advance", ed(CHAN, "        !gr_tier && self.ce_relay && copy_engine && advanced", "        !gr_tier && self.ce_relay && copy_engine"), QEMU),
    ("ce-relay-for-any-engine", ed(CHAN, "        !gr_tier && self.ce_relay && copy_engine && advanced", "        !gr_tier && self.ce_relay && advanced"), QEMU),
    ("twin-defapi-without-the-identity", ed(CHAN, "    if windows && user_work {\n        TwinDefapiPlan::AuthorHostObject", "    if user_work {\n        TwinDefapiPlan::AuthorHostObject"), QEMU),
    # --- the memory plane (kf-qemu mem.rs)
    ("mirror-start-always-the-floor", ed(MEM, "    MirrorVaStart::select(profile.active())", "    MirrorVaStart::select(true)"), QEMU),
    ("spare-taken-whatever-its-start", ed(MEM, "spares.iter().rposition(|s| s.start == start)?", "spares.iter().rposition(|_| true)?"), QEMU),
    ("retired-space-forgets-its-start", ed(MEM, "        space: mi.space,\n        start: mi.start,", "        space: mi.space,\n        start: MirrorVaStart::HostDefault,"), QEMU),
    ("recycled-spare-forgets-its-start", ed(MEM, "twin_record(key, sp.space, sp.start, sp.negctl_window, sp.ram_obj)", "twin_record(key, sp.space, MirrorVaStart::HostDefault, sp.negctl_window, sp.ram_obj)"), QEMU),
    ("prewarm-ignores-the-start", ed(MEM, "    let space = host.alloc_space(start)?;", "    let space = host.alloc_space(MirrorVaStart::HostDefault)?;"), QEMU),
    ("created-twin-forgets-its-start", ed(MEM, "    twin_record(key, space, start, fb.map(|b| (b, store_len)), ram_obj)", "    twin_record(key, space, MirrorVaStart::HostDefault, fb.map(|b| (b, store_len)), ram_obj)"), QEMU),
    # --- the display plane (kf-qemu display.rs)
    ("sdr-path-ignores-the-declaration", ed(QDISP, "    if profile_active { built } else { None }", "    built"), QEMU),
    ("sdr-wanted-ignores-the-profile", ed(QDISP, "    forced || profile_active", "    forced"), QEMU),
    ("sdr-wanted-always", ed(QDISP, "    forced || profile_active", "    true"), QEMU),
    ("caps-sync-republishes-every-time", ed(QDISP, "        if self.shown_sdr.swap(want, Ordering::AcqRel) == want {\n            return;\n        }", "        self.shown_sdr.store(want, Ordering::Release);"), QEMU),
    ("caps-sync-stores-nothing", ed(QDISP, "                store(to.base + u64::from(o), b);", "                let _ = (to.base, b);"), QEMU),
    ("caps-sdr-page-never-shown", ed(QDISP, "            (Some(p), true) => p,", "            (Some(_), true) => &self.base,"), QEMU),
    ("color-decoder-mirror-off", ed(QDISP, "    lut_mirror: true,\n    ilut_offset_256: true,\n};", "    lut_mirror: false,\n    ilut_offset_256: true,\n};"), QEMU),
]


def label(cmd):
    t = cmd[cmd.index("-p") + 1]
    if "--test" in cmd:
        return t + "/" + cmd[cmd.index("--test") + 1]
    return t + "/lib"


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
        names = re.findall(r"(?m)^test (\S+) \.\.\. FAILED$", out)
        failed += ["%s:%s" % (label(cmd), n) for n in names]
        if p.returncode != 0 and not names:
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
        base = run({}, RM + QEMU)
        assert not base, "baseline is not green: %s" % base
        print("baseline: green")
        weak = []
        for name, (rel, text), cmds in cases:
            killers = run({rel: text}, cmds)
            restore()
            print("%-46s killers=%d %s %s" % (name, len(killers), "" if len(killers) >= 2 else " <-- WEAK", killers))
            if len(killers) < 2:
                weak.append(name)
        print("MUTANTS total=%d weak=%d" % (len(cases), len(weak)))
        return 1 if weak else 0
    finally:
        restore()


if __name__ == "__main__":
    sys.exit(main())
