#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later
"""Mutation harness for the offer filter 'engines with no derived method table are not offered to the guest'
(STATUS: LIVE, 2026-11-15): crates/kf-chip/src/video_gaps.rs, kf_rm::hostquery::hide_untabled_video and the
policy plumbing, kf-qemu's rmfacts flag reader.

Each mutant is a one-site edit of production code; each must be killed by at least two independently named
failing tests. A compile error is not a kill. Every file touched is overwritten in place and ALWAYS restored
(also on an interrupt); needs a toolchain and the workspace's dependencies.
Run: python3 scripts/ci/hide_untabled_video_mutants.py [CASE ...]   (exit 0 only if no mutant is weak)
"""

import os
import re
import subprocess
import sys

HERE = os.path.dirname(os.path.abspath(__file__))
ROOT = os.path.dirname(os.path.dirname(HERE))
GAPS = "crates/kf-chip/src/video_gaps.rs"
HQ = "crates/kf-rm/src/hostquery.rs"
RMF = "crates/kf-qemu/src/rmfacts.rs"
FILES = {}
for rel in (GAPS, HQ, RMF):
    with open(os.path.join(ROOT, rel), encoding="utf-8") as f:
        FILES[rel] = f.read()

RM = [
    ["cargo", "test", "-p", "kf-rm", "--test", "hide_untabled_video"],
    ["cargo", "test", "-p", "kf-rm", "--test", "host_facts_query_ga106"],
]
CHIP = [["cargo", "test", "-p", "kf-chip", "--lib", "video_gaps"]]
CHAN = [["cargo", "test", "-p", "kf-chan", "--test", "video_class_coverage"]]
QEMU = [["cargo", "test", "-p", "kf-qemu", "--lib", "policy_tests"]]


def ed(rel, old, new):
    assert FILES[rel].count(old) == 1, (rel, old, FILES[rel].count(old))
    return rel, FILES[rel].replace(old, new)


SERVABLE = "if kf_chip::video_gaps::kernel_ring_servable(family, kind) {"
CASES = [
    ("filter-keeps-everything", ed(HQ, SERVABLE, "if true || kf_chip::video_gaps::kernel_ring_servable(family, kind) {"), RM),
    ("filter-inverted", ed(HQ, SERVABLE, "if !kf_chip::video_gaps::kernel_ring_servable(family, kind) {"), RM),
    ("filter-drops-every-video-engine", ed(HQ, SERVABLE, "if false && kf_chip::video_gaps::kernel_ring_servable(family, kind) {"), RM),
    ("ofa-judged-as-a-decoder", ed(HQ, "EngineKind::OpticalFlow(_) => Kind::OpticalFlow,", "EngineKind::OpticalFlow(_) => Kind::VideoDecoder,"), RM),
    ("non-video-engines-removed", ed(HQ, "                _ => return true,", "                _ => return false,"), RM),
    ("policy-ignored", ed(HQ, "if policy.hide_untabled_video {", "if false && policy.hide_untabled_video {"), RM),
    ("policy-always-on", ed(HQ, "if policy.hide_untabled_video {", "if true || policy.hide_untabled_video {"), RM),
    (
        "plain-query-hides",
        ed(HQ, "query_host_facts_with(host, family, HostFactsPolicy::default())", "query_host_facts_with(host, family, HostFactsPolicy { hide_untabled_video: true })"),
        RM,
    ),
    ("empty-class-list-is-servable", ed(GAPS, "    !classes.is_empty() && classes.iter().all(|c| !is_untabled(*c))", "    classes.iter().all(|c| !is_untabled(*c))"), RM + CHIP),
    ("nothing-is-untabled", ed(GAPS, "UNTABLED_VIDEO_CLASSES.iter().any(|&(c, _)| c == class)", "false && UNTABLED_VIDEO_CLASSES.iter().any(|&(c, _)| c == class)"), RM + CHIP),
    ("ofa-gap-entry-misnamed", ed(GAPS, "        0xc9fa,\n", "        0xc9fb,\n"), RM + CHAN),
    (
        "stray-gap-entry",
        ed(GAPS, "pub const UNTABLED_VIDEO_CLASSES: &[(u32, &str)] = &[\n", 'pub const UNTABLED_VIDEO_CLASSES: &[(u32, &str)] = &[\n    (0xc9d1, "x WUKONG_RELAUNCH_TDR_20261115"),\n'),
        RM + CHAN,
    ),
    ("flag-is-not-exactly-one", ed(RMF, 'value.is_some_and(|v| v == "1")', 'value.is_some_and(|v| v != "0")'), QEMU),
    ("flag-is-any-value", ed(RMF, 'value.is_some_and(|v| v == "1")', "value.is_some_and(|_| true)"), QEMU),
    ("flag-never-reaches-the-policy", ed(RMF, "        hide_untabled_video: on,", "        hide_untabled_video: false,"), QEMU),
]


def run(overrides, cmds):
    for rel, text in overrides.items():
        with open(os.path.join(ROOT, rel), "w", encoding="utf-8") as f:
            f.write(text)
    failed = []
    for i, cmd in enumerate(cmds):
        p = subprocess.run(cmd, cwd=ROOT, capture_output=True, text=True)
        out = p.stdout + p.stderr
        if "could not compile" in out:
            raise SystemExit("the mutant does not compile:\n" + out[-800:])
        failed += ["%s:%s" % (cmd[3] + "/" + cmd[5], n) for n in re.findall(r"(?m)^test (\S+) \.\.\. FAILED$", out)]
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
        all_cmds = RM + CHIP + CHAN + QEMU
        base = run({}, all_cmds)
        assert not base, "baseline is not green: %s" % base
        print("baseline: green")
        weak = []
        for name, (rel, text), cmds in cases:
            killers = run({rel: text}, cmds)
            restore()
            print("%-34s killers=%d %s %s" % (name, len(killers), "" if len(killers) >= 2 else " <-- WEAK", killers))
            if len(killers) < 2:
                weak.append(name)
        print("MUTANTS total=%d weak=%d" % (len(cases), len(weak)))
        return 1 if weak else 0
    finally:
        restore()


if __name__ == "__main__":
    sys.exit(main())
