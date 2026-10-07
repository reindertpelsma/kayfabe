#!/usr/bin/env python3
"""The `GspRmAlloc` bite harness -- remove each fix, WATCH its test go red, restore.

The rung this covers is the one a LIVE BOOT measured
(`docs/design/boot_measured_2026_08_01.md`): a stock 580.159.04 driver was refused
`0x56` for every class `rpcRmApiAlloc_GSP` asked for. Serving them took an ABI table
row, a capability row, a policy, an `Arch`, an isolate factory, and -- found by the
FIRST boot of the fix, not by any test -- a transport correction.

Every one of those is a place where a plausible-looking edit reproduces the wall or
something worse. This file un-does each in the tree, compiles, runs the ONE test that
names it, and reports what happened.

WHY IT IS COMMITTED RATHER THAN RUN ONCE. Same argument as `bite_promote_ctx.py`: "the
bites fired once" is a claim about a tree that has since moved. This is re-runnable at
any revision.

★ THREE FAILURE MODES IT REPORTS RATHER THAN HIDES, because each looks like success from
a distance:
  - PATTERN NOT UNIQUE -- the fix moved or was reformatted, so the bite was never applied
    and the test's green says nothing. NOT a pass.
  - DID NOT COMPILE -- the removal was rejected by the compiler rather than by the test.
    Inconclusive: the test never ran.
  - NON-BITER -- the test passed WITHOUT the fix. That is the finding this exists for.

★★ The file is rewritten and its mtime bumped on both the plant and the restore. `cp -a`
and `shutil.move` preserve mtimes and cargo then serves a stale rlib, which manufactures
false non-biters (memory: `bite_harness_must_touch_after_restore`).

★ Every bite is a REAL DEFECT SHAPE. B1 is the `alloc_params` row a reader would most
plausibly write for a class whose params "declare nothing" (the blanket default, which
would silently answer for EVERY class); B3 is the transport bug the boot actually found;
B7 is the `GraphPolicy`-instead-of-`ObjectPolicy` install, which is the mistake the
composable/claiming split exists to prevent and which nothing else in the tree would
catch; B8 plants a MOCK where the shipped `Arch` goes.

Usage:  python3 scripts/bite_gsp_rm_alloc.py     (from anywhere; paths are repo-relative)
Exit:   0 if every bite fired, 1 otherwise.
"""

import os
import subprocess
import sys
import time

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
ENV = dict(os.environ)

VERS = "crates/kayfabe-abi/src/versions.rs"
CAPS = "crates/kayfabe-abi/src/capability.rs"
RMRPC = "crates/kayfabe-rmrpc/src/lib.rs"
POLICY = "crates/kayfabe-rmrpc/src/policy.rs"
ELEM = "crates/kayfabe-gsp/src/element.rs"
RPC = "crates/kayfabe-gsp/src/rpc.rs"
DEVLIB = "crates/kayfabe-device/src/lib.rs"
GA10X = "crates/kayfabe-chips/src/ga10x.rs"
ISO = "crates/kayfabe-isolate/src/lib.rs"

# (name, file, old, new, test-filter)
BITES = [
    # ── The ABI table: the two classes the boot asked for ───────────────────────────
    ("B1 the subdevice/event row deleted -- the wall, exactly as the boot found it",
     VERS,
     "            classes::NV20_SUBDEVICE_0 | classes::NV01_EVENT_KERNEL_CALLBACK_EX => {\n"
     "                Some(AllocParams::NoDeclaredFacts)\n"
     "            }",
     "            classes::NV20_SUBDEVICE_0 => Some(AllocParams::NoDeclaredFacts),",
     "the_four_classes_the_boot_asked_for_are_all_served"),

    ("B2 ★★★ the blanket default -- every unmapped class answered NoDeclaredFacts",
     VERS,
     "            _ => None,\n        }\n    }\n\n    /// Which params shape a **control command** carries",
     "            _ => Some(AllocParams::NoDeclaredFacts),\n        }\n    }\n\n"
     "    /// Which params shape a **control command** carries",
     "an_allowlisted_but_unmapped_class_is_refused_as_unmapped_not_as_unpermitted"),

    # ── The transport correction the first boot found ───────────────────────────────
    ("B3 ★★★ the alloc decoded from the DECLARED body -- the boot's own second wall",
     RMRPC,
     "        RpcFunction::RmAlloc => translate_alloc(abi, guest_os, cmd.wire_body()),",
     "        RpcFunction::RmAlloc => translate_alloc(abi, guest_os, &cmd.payload),",
     "an_allocs_params_live_past_the_declared_length_and_are_still_served"),

    # ★★ B4 was a NON-BITER on its first run, and the bite was right while the TEST was
    # wrong -- `suspect_the_instrument_first`. It pointed at
    # `a_run_sized_exactly_to_its_message_decodes_and_one_byte_less_does_not`, where the
    # run is EITHER exactly `msg_len` OR exactly one element, so `run.len()` and
    # `elements * element_size_min` agree in both arms and dropping one of them changes
    # nothing. Only a run LONGER than the guest's element count separates them, and
    # nothing built one until this bite said so.
    ("B4 ★★ the delivered run bounded by the CALLER's buffer instead of the guest's elements",
     ELEM,
     "        .min(len.elements() as usize * len.element_size_min() as usize);",
     "        .min(usize::MAX);",
     "the_delivered_run_stops_at_the_guests_element_count_not_the_callers_buffer"),

    ("B5 ★★★ a REPLY sized from the delivered run -- the unclamped-reply bug class",
     RPC,
     "        let mut payload = vec![0u8; self.payload.len()];",
     "        let mut payload = vec![0u8; self.wire_body().len()];",
     "a_reply_is_sized_by_the_declared_length_never_by_the_delivered_run"),

    # ── The capability boundary ─────────────────────────────────────────────────────
    ("B6 the capability gate stops running before the params table",
     RMRPC,
     "    if let AllocPermit::Denied(denial) = abi.capabilities().alloc_class(class) {",
     "    if let AllocPermit::Denied(denial) = abi.capabilities().alloc_class(class)\n"
     "        && false\n    {",
     "a_class_this_port_does_not_model_is_still_refused_by_name"),

    # ── The chain: what the link may claim ──────────────────────────────────────────
    ("B7 ★★★ the object link claims EVERY function (the `GraphPolicy` install)",
     POLICY,
     "        if !ObjectPolicy::claims(cmd.function) {\n            return None;\n        }",
     "        if !ObjectPolicy::claims(cmd.function) && false {\n            return None;\n        }",
     "the_object_link_does_not_silence_the_unserviced_ledger"),

    ("B8 the object link installed AFTER the recorders, where nothing can reach it",
     DEVLIB,
     "    links.extend(objects);\n"
     "    links.extend::<Vec<Box<dyn kayfabe_gsp::CommandPolicy>>>(vec![",
     "    links.extend::<Vec<Box<dyn kayfabe_gsp::CommandPolicy>>>(vec![",
     "the_four_classes_the_boot_asked_for_are_all_served"),

    # ── The three ports this stage did NOT build, asserted to refuse ────────────────
    ("B9 ★★★ the shipped Arch classifies a real class id as Unknown (the MockArch shape)",
     GA10X,
     "            nv::NV20_SUBDEVICE_0 => ObjectKind::Subdevice,",
     "",
     "the_shipped_arch_classifies_the_real_wire_class_ids"),

    ("B10 ★★★ the unbuilt GMMU answers with plausible geometry instead of refusing",
     GA10X,
     "    fn levels(&self) -> u8 {\n        0\n    }",
     "    fn levels(&self) -> u8 {\n        5\n    }",
     "the_shipped_arch_refuses_every_data_plane_seam"),

    # ★★ B11 was a NON-BITER on its first run, and the bite was the defect: it edited a
    # DOC COMMENT and left `is_retired`'s body alone, so nothing was un-done and the green
    # said nothing. That is exactly the "PATTERN NOT UNIQUE" failure mode wearing a
    # different hat -- a bite that compiles and changes no behaviour is indistinguishable
    # from a passing test. It now plants the real thing.
    ("B11 the stillborn isolate is born LIVE -- a verb becomes issuable in a build with no plane",
     ISO,
     "    fn is_retired(&self) -> bool {\n        true\n    }\n}",
     "    fn is_retired(&self) -> bool {\n        false\n    }\n}",
     "the_shipped_isolate_factory_can_never_issue_a_verb"),

    # ── The client-root normalisation ───────────────────────────────────────────────
    ("B12 the client root left at the wire's 0/0, so its children cannot resolve",
     RMRPC,
     "        let root = HObject(h.client);",
     "        let root = HObject(h.parent);",
     "the_client_root_is_normalised_so_the_device_beneath_it_resolves"),
]

# The tree's own green, re-run after every restore. Deliberately the whole boot sequence:
# it is the one that drives all four classes through the port's real chain.
SANITY = "the_four_classes_the_boot_asked_for_are_all_served"

# Two test targets are in play: most bites name a test in `tests/tests/gsp_rm_alloc.rs`,
# and B4 names one in `tests/tests/gsp_boot.rs`. ★ Derived from the filter rather than
# listed beside it, so a bite cannot silently be run against the wrong binary.
IN_GSP_BOOT: set[str] = set()


def run(filt):
    target = "gsp_boot" if filt in IN_GSP_BOOT else "gsp_rm_alloc"
    p = subprocess.run(
        ["cargo", "test", "-p", "kayfabe-tests", "--test", target,
         "--no-fail-fast", "--", "--exact", filt],
        cwd=ROOT, env=ENV, capture_output=True, text=True)
    return p.returncode, p.stdout + p.stderr


def main():
    results = []
    for name, rel, old, new, filt in BITES:
        path = os.path.join(ROOT, rel)
        src = open(path).read()
        if src.count(old) != 1:
            results.append((name, "★ PATTERN NOT UNIQUE (%d hits) -- bite not applied" % src.count(old)))
            continue
        backup = src
        open(path, "w").write(src.replace(old, new))
        os.utime(path, None)
        time.sleep(0.05)
        rc, out = run(filt)
        open(path, "w").write(backup)
        os.utime(path, None)
        time.sleep(0.05)
        if "error[E" in out or "error: could not compile" in out:
            results.append((name, "★ DID NOT COMPILE -- inconclusive"))
        elif "0 passed; 0 failed" in out:
            results.append((name, "★ TEST DID NOT RUN -- filter matched nothing"))
        elif rc != 0:
            results.append((name, "BITES (test went RED)"))
        else:
            results.append((name, "★★★ NON-BITER -- the test passed WITHOUT the fix"))
    rc, _ = run(SANITY)
    print("\n=== GSP_RM_ALLOC BITE LEDGER ===")
    for n, r in results:
        print(f"  {r:<50} {n}")
    print(f"\nrestored tree sanity check ({SANITY}): {'GREEN' if rc == 0 else 'RED -- RESTORE FAILED'}")
    bad = [n for n, r in results if not r.startswith("BITES")]
    print(f"\n{len(results) - len(bad)}/{len(results)} bites fired")
    sys.exit(1 if bad or rc != 0 else 0)


main()
