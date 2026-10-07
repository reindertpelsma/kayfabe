#!/usr/bin/env python3
"""Bite harness for the STICKY-ANSWER universe (`kayfabe_device::sticky`).

★ Committed, not run once and thrown away. "Eleven bites fired once" is not "eleven bites
fire": a guard that stops biting is exactly the failure this repository keeps meeting, and
the only way to keep the claim true is to be able to re-measure it in one command.

For each mutation: apply, run the named test target(s), record the failing test names,
restore, and TOUCH the restored file (a `cp`-style restore keeps the old mtime and cargo
serves a stale rlib — memory: bite_harness_must_touch_after_restore).

`--no-fail-fast` is BEFORE `--` on purpose: after `--` libtest rejects it and runs nothing.
Without it cargo stops after the first failing TARGET and manufactures FALSE non-biters.

  ROOT=/path/to/worktree CARGO_TARGET_DIR=/path/to/target scripts/bite_sticky_answer.py

Measured ALL TWELVE FIRING on 2026-08-01 at the tree this file landed in.
"""
import os
import re
import subprocess
import sys
import time

ROOT = os.environ.get(
    "ROOT", os.path.dirname(os.path.dirname(os.path.abspath(__file__))))
ENV = dict(os.environ,
           PATH=os.path.expanduser("~/.cargo/bin") + ":" + os.environ["PATH"])

BITES = [
    # (id, file, old, new, target-args, tests that MUST go red)
    ("B1-guard-not-installed",
     "crates/kayfabe-device/src/lib.rs",
     "    Box::new(sticky::StickyAnswerGuard::new(\n        driver,\n        served_chain(chip, driver, unserviced, fault_buffer),\n    ))",
     "    served_chain(chip, driver, unserviced, fault_buffer)",
     ["-p", "kayfabe-tests", "--test", "sticky_answer"],
     ["the_port_never_lets_the_guest_mark_our_answer_cacheable"]),

    ("B2-rewrite-removed",
     "crates/kayfabe-device/src/sticky.rs",
     "        reply.body[CONTROL_RMCTRL_FLAGS_OFF..CONTROL_RMCTRL_FLAGS_OFF + 4]\n            .copy_from_slice(&0u32.to_le_bytes());\n        reply.body[CONTROL_RMCTRL_ACCESS_RIGHT_OFF..CONTROL_RMCTRL_ACCESS_RIGHT_OFF + 4]\n            .copy_from_slice(&0u32.to_le_bytes());",
     "        // bite: the rewrite removed",
     ["-p", "kayfabe-tests", "--test", "sticky_answer"],
     ["the_port_never_lets_the_guest_mark_our_answer_cacheable",
      "every_served_control_leaves_the_port_non_cacheable",
      "a_gss_legacy_control_answered_ok_is_counted_and_neutralised"]),

    # ★ The defect shape is not "a row was deleted" (the array length makes that a COMPILE
    # error, which is stronger) — it is "an answering policy was ADDED and nobody wrote a
    # row". That is what this mutates.
    ("B3-impl-added-without-a-row",
     "crates/kayfabe-device/src/inert.rs",
     "kayfabe_util::assert_send_sync!(InertPolicy);",
     "kayfabe_util::assert_send_sync!(InertPolicy);\n\n/// bite\n#[derive(Debug, Default)]\npub struct BiteAnswersEverything;\nimpl CommandPolicy for BiteAnswersEverything {\n    fn respond(&mut self, cmd: &RpcCommand) -> Option<Reply> {\n        Some(Reply { rpc_result: 0, body: cmd.payload.clone() })\n    }\n}",
     ["-p", "kayfabe-tests", "--test", "sticky_answer"],
     ["the_universe_of_answering_policies_is_derived_from_the_source"]),

    ("B3b-row-names-a-phantom",
     "crates/kayfabe-device/src/sticky.rs",
     '        name: "InertPolicy",',
     '        name: "InertPolicyThatIsNot",',
     ["-p", "kayfabe-tests", "--test", "sticky_answer"],
     ["the_universe_of_answering_policies_is_derived_from_the_source",
      "every_disposition_row_points_at_the_file_that_implements_it"]),

    ("B4-never-answers-starts-answering",
     "crates/kayfabe-device/src/unserviced.rs",
     "        // ⊘ Always `None`. See this module's docs: recording is not answering.\n        None",
     "        Some(Reply { rpc_result: 0, body: cmd.payload.clone() })",
     ["-p", "kayfabe-tests", "--test", "sticky_answer"],
     ["the_never_answers_rows_answer_nothing_even_for_a_gss_legacy_control"]),

    ("B5-not-a-control-starts-claiming-fn76",
     "crates/kayfabe-device/src/inert.rs",
     "            RpcFunction::InitGspTraceCrashBuffer | RpcFunction::UnloadingGuestDriver",
     "            RpcFunction::InitGspTraceCrashBuffer\n                | RpcFunction::UnloadingGuestDriver\n                | RpcFunction::RmControl",
     ["-p", "kayfabe-tests", "--test", "sticky_answer"],
     ["the_not_a_control_rows_decline_every_control_command"]),

    ("B6-cacheable-mask-narrowed",
     "crates/kayfabe-device/src/sticky.rs",
     "pub const RMCTRL_FLAGS_CACHEABLE_ANY: u32 =\n    RMCTRL_FLAGS_CACHEABLE | RMCTRL_FLAGS_CACHEABLE_BY_INPUT;",
     "pub const RMCTRL_FLAGS_CACHEABLE_ANY: u32 = RMCTRL_FLAGS_CACHEABLE_BY_INPUT;",
     ["-p", "kayfabe-tests", "--test", "sticky_answer"],
     ["the_cacheability_predicate_matches_the_guests_own"]),

    ("B7-branch-a-row-wrong",
     "crates/kayfabe-device/src/sticky.rs",
     "pub const BRANCH_A_CACHEABLE: [u32; 4] = [0x2080_1803, 0x2080_0a36, 0x2080_0a41, 0x2080_0a40];",
     "pub const BRANCH_A_CACHEABLE: [u32; 4] = [0x2080_1803, 0x2080_0a36, 0x2080_0a41, 0x2080_9999];",
     ["-p", "kayfabe-tests", "--test", "sticky_answer"],
     ["branch_a_is_a_subset_of_what_this_port_serves_from_a_constant_row"]),

    ("B8-capture-sweep-reaches-the-controls",
     "tests/tests/sticky_answer.rs",
     "        if req.cmd & 0x0000_8000 != 0 {",
     "        if req.cmd & 0x0000_0800 != 0 {",
     ["-p", "kayfabe-tests", "--test", "sticky_answer"],
     ["no_gss_legacy_control_appears_in_the_cold_boot_capture"]),

    ("B9-guard-counts-refusals",
     "crates/kayfabe-device/src/sticky.rs",
     "        if cmd.function != RpcFunction::RmControl || reply.rpc_result != NV_OK {\n            return Some(reply);\n        }\n        self.inspected = self.inspected.saturating_add(1);",
     "        self.inspected = self.inspected.saturating_add(1);\n        if cmd.function != RpcFunction::RmControl || reply.rpc_result != NV_OK {\n            return Some(reply);\n        }",
     ["-p", "kayfabe-tests", "--test", "sticky_answer"],
     ["a_refusal_crosses_the_guard_unchanged_and_uncounted"]),

    ("B10-short-body-passed-not-refused",
     "crates/kayfabe-device/src/sticky.rs",
     "        if reply.body.len() < CONTROL_HEADER {\n            self.malformed = self.malformed.saturating_add(1);\n            return Some(Reply {\n                rpc_result: kayfabe_abi::NV_ERR_NOT_SUPPORTED,\n                body: Vec::new(),\n            });\n        }",
     "        if reply.body.len() < CONTROL_HEADER {\n            return Some(reply);\n        }",
     ["-p", "kayfabe-tests", "--test", "sticky_answer"],
     ["an_accepted_control_reply_that_cannot_hold_the_header_is_refused"]),

    ("B11-graphpolicy-accepted-path-filters",
     "crates/kayfabe-rmrpc/src/policy.rs",
     "            Ok(_) => Some(Reply {\n                rpc_result: 0, // NV_OK\n                body: cmd.payload.clone(),\n            }),",
     "            Ok(_) => Some(Reply {\n                rpc_result: 0, // NV_OK\n                body: {\n                    let mut b = cmd.payload.clone();\n                    if b.len() >= 32 { b[24..32].fill(0); }\n                    b\n                },\n            }),",
     ["-p", "kayfabe-rmrpc", "--test", "gss_legacy_answer"],
     ["an_accepted_answer_echoes_the_guests_own_cacheability_bits_verbatim"]),
]


def run(target):
    p = subprocess.run(["cargo", "test", *target, "--no-fail-fast"],
                       cwd=ROOT, env=ENV, capture_output=True, text=True)
    out = p.stdout + p.stderr
    failed = set(re.findall(r"^test (\S+) \.\.\. FAILED$", out, re.M))
    compile_err = "error[" in out or re.search(r"^error: could not compile", out, re.M)
    return failed, compile_err, out


def main():
    results = []
    for (bid, rel, old, new, target, expect) in BITES:
        path = os.path.join(ROOT, rel)
        src = open(path).read()
        if old not in src:
            results.append((bid, "PATTERN-NOT-FOUND", set(), expect))
            print(f"{bid}: PATTERN NOT FOUND in {rel}", flush=True)
            continue
        open(path, "w").write(src.replace(old, new, 1))
        try:
            failed, cerr, out = run(target)
        finally:
            open(path, "w").write(src)
            os.utime(path, (time.time(), time.time()))
        missing = [t for t in expect if t not in failed]
        status = "BITES" if not missing and not cerr else (
            "COMPILE-ERROR" if cerr else "NON-BITER")
        results.append((bid, status, failed, expect))
        print(f"{bid}: {status}  failed={sorted(failed)}", flush=True)
        if status != "BITES":
            tail = "\n".join(out.splitlines()[-25:])
            print(f"---- {bid} tail ----\n{tail}\n--------", flush=True)

    print("\n===== BITE LEDGER =====")
    ok = True
    for bid, status, failed, expect in results:
        print(f"{status:16} {bid}")
        ok = ok and status == "BITES"
    # restore-verification: everything must be green again
    failed, cerr, _ = run(["-p", "kayfabe-tests", "--test", "sticky_answer"])
    f2, c2, _ = run(["-p", "kayfabe-rmrpc", "--test", "gss_legacy_answer"])
    print(f"restored: sticky_answer failed={sorted(failed)} compile_err={cerr}")
    print(f"restored: gss_legacy_answer failed={sorted(f2)} compile_err={c2}")
    ok = ok and not failed and not cerr and not f2 and not c2
    print("ALL BITES FIRED" if ok else "SOME BITES DID NOT FIRE")
    sys.exit(0 if ok else 1)


main()
