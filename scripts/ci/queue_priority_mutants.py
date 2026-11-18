#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later
"""Mutation harness for the queue-priority link (STATUS: LIVE, 2026-11-18): crates/kf-rm/src/queuepriority.rs
(SET_SCHED_POLICY 0x20801115 and the two SET_INTERLEAVE_LEVEL controls, accepted as per-VM virtual state, never
forwarded) and its seat in kf_rm::served_chain (docs/design/V3_QUEUE_PRIORITY.md).

Each mutant is a one-site edit of production code; each must be killed by at least two independently named
failing tests. A compile error is not a kill. Every file touched is overwritten in place and ALWAYS restored
(also on an interrupt); needs a toolchain and the workspace's dependencies.
Run: python3 scripts/ci/queue_priority_mutants.py [CASE ...]   (exit 0 only if no mutant is weak)
"""

import os
import re
import subprocess
import sys

HERE = os.path.dirname(os.path.abspath(__file__))
ROOT = os.path.dirname(os.path.dirname(HERE))
QP = "crates/kf-rm/src/queuepriority.rs"
LIB = "crates/kf-rm/src/lib.rs"
FILES = {}
for rel in (QP, LIB):
    with open(os.path.join(ROOT, rel), encoding="utf-8") as f:
        FILES[rel] = f.read()

UNIT = ["cargo", "test", "-p", "kf-rm", "--lib", "queuepriority"]
REPLAY = ["cargo", "test", "-p", "kf-rm", "--test", "queue_priority"]
ALL = [UNIT, REPLAY]


def ed(rel, *pairs):
    text = FILES[rel]
    for old, new in pairs:
        assert text.count(old) == 1, (rel, old, text.count(old))
        text = text.replace(old, new)
    return rel, text


CLAIMS = "SET_SCHED_POLICY | TSG_SET_INTERLEAVE_LEVEL | CHANNEL_SET_INTERLEAVE_LEVEL"
CASES = [
    ("sched-policy-unclaimed", ed(QP, (CLAIMS, "TSG_SET_INTERLEAVE_LEVEL | CHANNEL_SET_INTERLEAVE_LEVEL")), ALL),
    ("claims-everything", ed(QP, ("        matches!(\n            cmd,\n            " + CLAIMS, "        cmd != 0 || matches!(\n            cmd,\n            " + CLAIMS)), ALL),
    ("claims-make-realtime", ed(QP, (CLAIMS, CLAIMS + " | 0xa06c_0110")), ALL),
    ("restore-flag-not-read", ed(QP, ("if flags & FLAGS_RESTORE != 0 {", "if false {")), ALL),
    ("restore-is-flag-equality", ed(QP, ("if flags & FLAGS_RESTORE != 0 {", "if flags == FLAGS_RESTORE {")), ALL),
    ("restore-keeps-policy", ed(QP, ("                self.policy = SchedPolicy::Default;\n                bump(&mut self.policy_restores);", "                bump(&mut self.policy_restores);")), ALL),
    ("policy-range-open", ed(QP, ("            _ => None,\n        }\n    }\n}\n\n/// ★ The link.", "            _ => Some(SchedPolicy::Default),\n        }\n    }\n}\n\n/// ★ The link.")), ALL),
    ("wddm-policy-dropped", ed(QP, ("2 => Some(SchedPolicy::ChannelInterleavedWddm),", "2 => None,")), ALL),
    ("level-range-open", ed(QP, ("level <= INTERLEAVE_LEVEL_MAX =>", "level <= INTERLEAVE_LEVEL_MAX + 1 =>")), ALL),
    ("level-not-recorded", ed(QP, ("                self.last_level = Some(level);\n", "")), ALL),
    ("declared-size-unchecked", ed(QP, ("if req.params_size as usize != expected {", "if false {")), ALL),
    ("serialized-envelope-decoded", ed(QP, ("if kf_abi::rpc_params_are_serialized(req.rmapi_rpc_flags) {", "if false {")), ALL),
    ("other-functions-claimed", ed(QP, ("if cmd.function != RpcFunction::RmControl {", "if false {")), ALL),
    ("status-word-left-stale", ed(QP, ("            w.copy_from_slice(&NV_OK.to_le_bytes());", "            let _ = w;")), ALL),
    ("params-not-echoed", ed(QP, ("        let mut body = cmd.payload.clone();", "        let mut body = vec![0u8; cmd.payload.len()];")), ALL),
    ("refusals-uncounted", ed(QP, ("            bump(&mut this.refused);\n", "")), ALL),
    ("counter-wraps", ed(QP, ("*n = n.checked_add(1).unwrap_or(u64::MAX);", "*n = n.wrapping_add(1);")), ALL),
    (
        "link-not-seated-in-the-chain",
        ed(
            LIB,
            ("chain.extend::<[Box<dyn kf_gsp::CommandPolicy>; 10]>([", "chain.extend::<[Box<dyn kf_gsp::CommandPolicy>; 9]>(["),
            ("        Box::new(queuepriority::QueuePriorityPolicy::new(driver)),\n", ""),
        ),
        ALL,
    ),
]

FAIL = re.compile(r"(?m)^test (\S+) \.\.\. FAILED$")


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
        failed += ["%s:%s" % (cmd[5] if cmd[4] == "--lib" else cmd[5], n) for n in FAIL.findall(out)]
        if p.returncode != 0 and not FAIL.search(out):
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
        base = run({}, ALL)
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
