#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later
"""Mutation harness for GPU-utilisation reporting (STATUS: LIVE, 2026-11-17).

`report-utilization` (default on) shows the guest the physical GPU's utilisation through one fixed, host-authored, one-way
host read (`docs/design/V3_UTILIZATION_FLAG.md`, OWNER_RULINGS section AV). The claims to protect: the property off is
today's behaviour; the guest controls nothing but asking (no guest byte in a reply, no host call from a request, a bounded
reply, a bounded write); unavailable is never zero; a layout is used only where it was derived. Each mutant is a one-site
edit of production code that would break one of them; each must be killed by at least two independently named failing
tests. A compile error is not a kill. Every file touched is overwritten in place and ALWAYS restored (also on an
interrupt); needs a toolchain and the workspace's dependencies, no GPU.

Equivalent mutants (documented, not run): dropping `gpumon_at`'s matrix cross-check (the generated rows and the matrix agree
at every version today, so no input distinguishes them; `every_row_is_coherent_and_the_matrix_agrees...` pins the data);
`Sampler::period`'s `.max(SAMPLE_PERIOD_MIN)` (both periods already exceed the floor; `however_often_it_is_ticked` kills the
real floor mutant below).

Run: python3 scripts/ci/util_flag_mutants.py [CASE ...]   (exit 0 only if no mutant is weak; `--check` only validates anchors)
"""

import os
import re
import subprocess
import sys

HERE = os.path.dirname(os.path.abspath(__file__))
ROOT = os.path.dirname(os.path.dirname(HERE))
KU = "crates/kf-abi/src/utilization.rs"
RU = "crates/kf-rm/src/utilization.rs"
LIB = "crates/kf-rm/src/lib.rs"
UH = "crates/kf-qemu/src/utilhost.rs"
DEV = "crates/kf-qemu/src/device.rs"
FFI = "crates/kf-qemu/src/ffi_unsafe.rs"
FILES = {}
for rel in (KU, RU, LIB, UH, DEV, FFI):
    with open(os.path.join(ROOT, rel), encoding="utf-8") as f:
        FILES[rel] = f.read()

ABI = [["cargo", "test", "-p", "kf-abi", "--lib", "--", "utilization"]]
RM = [
    ["cargo", "test", "-p", "kf-rm", "--test", "utilization_chain"],
    ["cargo", "test", "-p", "kf-rm", "--test", "utilization_replay"],
    ["cargo", "test", "-p", "kf-rm", "--lib", "--", "utilization"],
]
QEMU = [
    ["cargo", "test", "-p", "kf-qemu", "--lib", "--", "utilhost"],
    ["cargo", "test", "-p", "kf-qemu", "--test", "utilization_use_sites"],
]


BAD = []


def ed(rel, old, new):
    if FILES[rel].count(old) != 1:
        BAD.append((rel, old[:70], FILES[rel].count(old)))
    return rel, FILES[rel].replace(old, new)


CASES = [
    # --- kf-abi: the pure encoders (the guest's reply, the host's reading, the RUSD record)
    ("reduce-never-clamps", ed(KU, "    if pct > 100 {\n        (100, true)\n    } else {", "    if false {\n        (100, true)\n    } else {"), ABI + RM),
    ("reduce-truncates", ed(KU, "let pct = (u64::from(util_x100) + 50) / 100;", "let pct = u64::from(util_x100) / 100;"), ABI + RM),
    ("host-reads-the-oldest-sample", ed(KU, "newest.is_none_or(|(t, _)| ts > t)", "newest.is_none_or(|(t, _)| ts < t)"), ABI + RM),
    ("host-reply-with-another-type-is-read", ed(KU, "        || params.get(off(l.off_type)).copied() != u8::try_from(l.gpumon_type_util).ok()\n        || u64::from(get_u32(params, off(l.off_buf_size))?) != l.buffer_size()", "        || u64::from(get_u32(params, off(l.off_buf_size))?) != l.buffer_size()"), ABI),
    ("reply-carries-a-process-id", ed(KU, "            put_u32(&mut p, e + off(l.eng_off_proc_id), pid)?;\n", "            put_u32(&mut p, e + off(l.eng_off_proc_id), 4242)?;\n"), ABI + RM),
    ("reply-tracker-wrong", ed(KU, "        count % u32::try_from(l.sample_count).ok()?,", "        count,"), ABI),
    ("reply-keeps-the-oldest-72", ed(KU, "let samples = &samples[samples.len().saturating_sub(n)..];", "let samples = &samples[..samples.len().min(n)];"), ABI),
    ("request-ignores-bufsize", ed(KU, "        && get_u32(params, off(l.off_buf_size)).map(u64::from) == Some(l.buffer_size())\n}", "}"), ABI + RM),
    ("request-ignores-type", ed(KU, "    params.len() == l.params_len()\n        && params.get(off(l.off_type)).copied() == u8::try_from(l.gpumon_type_util).ok()\n        && get_u32(params, off(l.off_buf_size)).map(u64::from)", "    params.len() == l.params_len()\n        && get_u32(params, off(l.off_buf_size)).map(u64::from)"), ABI + RM),
    ("init-accepts-an-unaligned-page", ed(KU, "(phys != 0 && phys % PAGE == 0 && phys < (1u64 << 52))", "(phys != 0 && phys < (1u64 << 52))"), ABI + RM),
    ("init-accepts-the-top-of-the-address-space", ed(KU, "(phys != 0 && phys % PAGE == 0 && phys < (1u64 << 52))", "(phys != 0 && phys % PAGE == 0)"), ABI + RM),
    ("init-accepts-a-zero-page", ed(KU, "(phys != 0 && phys % PAGE == 0 && phys < (1u64 << 52))", "(phys % PAGE == 0 && phys < (1u64 << 52))"), ABI + RM),
    ("init-accepts-any-binit", ed(KU, "        _ => None,\n    }\n}\n\n/// Parse a SET_DATA_POLL", "        _ => Some(RusdInit::Unlink),\n    }\n}\n\n/// Parse a SET_DATA_POLL"), ABI + RM),
    ("any-poll-bit-asks-for-the-record", ed(KU, "mask & l.poll_perf != 0", "mask != 0"), ABI + RM),
    ("rusd-timestamp-unchecked", ed(KU, "        || timestamp == l.ts_invalid\n        || timestamp == l.ts_in_progress\n        || timestamp >= SEQ_START", "        || false"), ABI + RM),
    ("rusd-seqlock-begin-is-a-valid-timestamp", ed(KU, "begin: (base + l.util_off_ts, l.ts_in_progress.to_le_bytes()),", "begin: (base + l.util_off_ts, 1u64.to_le_bytes()),"), ABI + RM),
    ("layout-is-a-nearest-neighbour", ed(KU, ".find(|(versions, _)| versions.contains(&version))", ".find(|(versions, _)| versions.iter().any(|v| v.major == version.major))"), ABI + RM),
    ("host-borrows-a-row-outside-the-interval", ed(KU, "        in_interval(&host)\n            .then(|| {", "        true\n            .then(|| {"), ABI + QEMU),
    ("host-request-asks-for-another-type", ed(KU, "*b = u8::try_from(l.gpumon_type_util).unwrap_or(0)", "*b = 1"), ABI + QEMU),
    # --- kf-rm: the state, the sampler, the publisher, the link
    ("bucket-never-limits", ed(RU, "        if self.tokens == 0 {\n            return false;\n        }", "        if false {\n            return false;\n        }"), RM),
    ("sampler-ignores-the-period-and-floor", ed(RU, "if self.last_call_ns != 0 && now.saturating_sub(self.last_call_ns) < need {", "if false && now.saturating_sub(self.last_call_ns) < need {"), RM),
    ("sampler-keeps-a-stalled-sample", ed(RU, "if r.host_timestamp <= self.last_host_ts {", "if false {"), RM),
    ("sampler-always-slow", ed(RU, "        if self.state.interested() {\n            SAMPLE_PERIOD.max(SAMPLE_PERIOD_MIN)", "        if false {\n            SAMPLE_PERIOD.max(SAMPLE_PERIOD_MIN)"), RM),
    ("stale-snapshots-are-served", ed(RU, "(age <= u64::try_from(FRESH_FOR.as_nanos()).unwrap_or(u64::MAX)).then(|| ring.clone())", "Some(ring.clone())"), RM),
    ("ring-grows-without-bound", ed(RU, "        if ring.len() == RING_MAX {\n            ring.remove(0);\n        }\n", ""), RM),
    ("publish-without-a-poll", ed(RU, "if phys == 0 || !self.rusd_wants.load(Ordering::Relaxed) {", "if phys == 0 {"), RM),
    ("publish-every-pass", ed(RU, "if !pending && last != 0 && now.saturating_sub(last) < period {", "if false {"), RM),
    ("publish-timestamps-may-repeat", ed(RU, "let ts = now.max(self.last_ts.load(Ordering::Relaxed).saturating_add(1));", "let ts = now;"), RM),
    ("publish-ignores-the-address-wrap", ed(RU, "let at = |off: u64| phys.checked_add(off);", "let at = |off: u64| Some(phys.wrapping_add(off));"), RM),
    ("publish-continues-after-a-refused-write", ed(RU, "if !write(a1, &w.begin.1) || !write(a2, &w.info.1) || !write(a3, &w.finish.1) {", "let (w1, w2, w3) = (write(a1, &w.begin.1), write(a2, &w.info.1), write(a3, &w.finish.1));\n        if !(w1 && w2 && w3) {"), RM),
    ("unlink-keeps-the-page", ed(RU, "            Some(RusdInit::Unlink) => {\n                self.state.rusd_phys.store(0, Ordering::Relaxed);", "            Some(RusdInit::Unlink) => {"), RM),
    ("a-new-chain-keeps-the-old-link", ed(RU, "        self.rusd_phys.store(0, Ordering::Relaxed);\n        self.rusd_wants.store(false, Ordering::Relaxed);\n        self.publish_pending.store(false, Ordering::Relaxed);\n        *self", "        *self"), RM),
    ("poll-leaves-the-wish-set", ed(RU, "self.state.rusd_wants.store(wants, Ordering::Relaxed);", "if wants {\n            self.state.rusd_wants.store(true, Ordering::Relaxed);\n        }"), RM),
    ("serialized-envelope-is-served", ed(RU, "        if kf_abi::rpc_params_are_serialized(req.rmapi_rpc_flags) {\n            return None;\n        }\n        let size = req.params_size as usize;", "        let size = req.params_size as usize;"), RM),
    ("gpumon-served-without-the-bucket", ed(RU, "        if !self.bucket.take(self.state.now_ns()) {", "        if false {"), RM),
    ("gpumon-reply-echoes-the-guests-bytes", ed(RU, "let reply = ku::encode_gpumon_reply(l, &samples)?;", "let mut reply = ku::encode_gpumon_reply(l, &samples)?;\n        reply[16..64].copy_from_slice(&params[16..64]);"), RM),
    ("link-not-seated", ed(LIB, "    if let Some(seat) = &utilization\n        && let Some(link) = utilization::UtilizationPolicy::new(driver, seat)\n    {", "    if let Some(seat) = &utilization\n        && false\n        && let Some(link) = utilization::UtilizationPolicy::new(driver, seat)\n    {"), RM),
    ("link-seated-after-the-ledger-never-reached", ed(LIB, "    if let Some(seat) = &utilization\n        && let Some(link) = utilization::UtilizationPolicy::new(driver, seat)\n    {\n        chain.push(Box::new(link));\n    }\n", ""), RM),
    # --- kf-qemu: the host read, the property
    ("classify-treats-not-supported-as-transient", ed(UH, "RmError::InsufficientPermissions | RmError::Other(NV_ERR_NOT_SUPPORTED) => {", "RmError::InsufficientPermissions => {"), QEMU),
    ("read-via-sends-guest-shaped-junk", ed(UH, "let mut params = ku::host_request(layout);", "let mut params = vec![0xa5u8; layout.params_len()];"), QEMU),
    ("read-via-ignores-the-host-version", ed(UH, "    let layout = ku::host_layout(version).ok_or_else(|| {", "    let layout = ku::host_layout(DriverVersion { major: 580, minor: 159, patch: 4 }).ok_or_else(|| {"), QEMU),
    ("property-off-still-builds-the-host", ed(UH, "    if !on {\n        return None;\n    }\n    enable(make_host())", "    enable(make_host())"), QEMU),
    ("realize-hardwires-the-property-on", ed(DEV, "crate::utilhost::enable_if(cfg.report_utilization, ||", "crate::utilhost::enable_if(true, ||"), QEMU),
    ("the-seat-ignores-the-runtime", ed(DEV, "let util_seat = util.as_ref().map(crate::utilhost::UtilRuntime::seat);", "let util_seat = None::<kf_rm::utilization::UtilizationSeat>;"), QEMU),
    ("drainer-never-publishes", ed(DEV, "u.state\n                    .publish(&mut |gpa, bytes| self.ram.write_gpa(gpa, bytes));", "let _ = u;"), QEMU),
    ("the-abi-word-is-inverted", ed(FFI, "report_utilization: report_utilization != 0,", "report_utilization: report_utilization == 0,"), QEMU),
    ("sampler-thread-starts-without-a-runtime", ed(FFI, "            if d.util.is_some()\n                && std::thread::Builder::new()\n                    .name(\"kf3-util\".into())", "            if true\n                && std::thread::Builder::new()\n                    .name(\"kf3-util\".into())"), QEMU),
]


if BAD:
    for b in BAD:
        print("ANCHOR does not match exactly once:", b)
    raise SystemExit(2)


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
    args = [a for a in sys.argv[1:] if a != "--check"]
    if "--check" in sys.argv[1:]:
        print("anchors ok: %d mutants" % len(CASES))
        return 0
    want = set(args)
    cases = [c for c in CASES if not want or c[0] in want]
    try:
        base = run({}, ABI + RM + QEMU)
        assert not base, "baseline is not green: %s" % base
        print("baseline: green")
        weak = []
        for name, (rel, text), cmds in cases:
            killers = run({rel: text}, cmds)
            restore()
            print("%-52s killers=%d %s %s" % (name, len(killers), "" if len(killers) >= 2 else " <-- WEAK", killers))
            if len(killers) < 2:
                weak.append(name)
        print("MUTANTS total=%d weak=%d" % (len(cases), len(weak)))
        return 1 if weak else 0
    finally:
        restore()


if __name__ == "__main__":
    sys.exit(main())
