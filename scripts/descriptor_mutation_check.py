#!/usr/bin/env python3
"""GPU-free production source mutants for the USER descriptor admission contract.

Usage: nice -n 19 flock /tmp/claude-0/kf-local-cargo.lock python3
scripts/descriptor_mutation_check.py OUTDIR TARGET_ROOT [MUTANT ...]

Follows lifecycle_mutation_check.sh/lazy_mutation_check.py: a green baseline,
exact source edits, restoration in finally, compile errors never count as kills.
Each build variant gets a distinct target; every mutant must fail two named tests.
"""
import json
import os
from pathlib import Path
import re
import shutil
import subprocess
import sys

ROOT = Path(__file__).resolve().parents[1]
CH = "crates/kf-chan/src/"
T = CH + "user_turn.rs"
S = CH + "user_storage.rs"
L = CH + "user_lifecycle.rs"
F = CH + "user_stream.rs"

# A format bypass must preserve valid setup classifications. Audit the negative
# assertions themselves, and require healthy Pointer paths to remain green.
FORMAT_KILL_SITES = {
    "an_entry_inspect_entry_refuses_is_refused_by_name_and_never_published": "GP_FORMAT_BAD_RESERVED_BIT",
    "pointer_format_refuses_reserved_controls_and_unrepresentable_end_without_credit": "GP_FORMAT_BAD_WORD",
}
FORMAT_CONTROLS = [
    "pointer_forwards_each_run3440_descriptor_unchanged_without_reading_its_pushbuffer",
    "pointer_forwards_the_largest_representable_descriptor",
    "pointer_zero_length_nops_preserve_sync_level_operand_without_pb_or_budget",
    "pointer_admission_at_each_length_boundary_is_intact_and_constant_work",
]


def audit_format_kills(log, source):
    """Reject setup panics, misplaced markers, and broken valid-entry controls."""
    for name in FORMAT_CONTROLS:
        if not re.search(rf"^test {re.escape(name)} \.\.\. ok$", log, re.M):
            raise ValueError(f"{name}: positive control must pass")
    sites = {}
    for name, marker in FORMAT_KILL_SITES.items():
        function = re.search(rf"^fn {re.escape(name)}\(\).*?(?=^fn |\Z)", source, re.M | re.S)
        assertion = f'expect_err("{marker}")'
        if function is None or function.group().count(assertion) != 1:
            raise ValueError(f"{name}: unique bad-entry assertion missing")
        offset = function.start() + function.group().index(assertion)
        line = source.count("\n", 0, offset) + 1
        block = re.search(rf"^---- {re.escape(name)} stdout ----\n(.*?)(?=^---- |\Z)", log, re.M | re.S)
        path = "crates/kf-chan/tests/user_turn_unit.rs"
        panic = None if block is None else re.search(
            rf"panicked at {re.escape(path)}:{line}:(\d+):\n{re.escape(marker)}:", block.group(1))
        if panic is None:
            raise ValueError(f"{name}: failure must reach its own bad-entry assertion at {path}:{line}")
        sites[name] = {"path": path, "line": line, "column": int(panic.group(1)), "assertion_marker": marker}
    return sites

# name, file, exact old/new, two required independent killing tests
MUTANTS = [
    ("pointer_admission_keeps_snapshot_budget", T,
     "match inspect_format(self.raw)", "match crate::user_stream::inspect_entry(self.raw, io.max_scan_bytes())",
     ["a_1026596_byte_pointer_descriptor_completes_through_marker_and_get_writeback", "a_large_captured_descriptor_is_not_reloaded_after_guest_mutation"]),
    ("storage_keeps_snapshot_budget", S,
     "crate::user_stream::inspect_format(entry)", "crate::user_stream::inspect_entry(entry, MAX_SOURCE_BYTES)",
     ["user_storage::tests::large_user_descriptors_cross_snapshot_bound_without_changing_real_credit_fields", "user_storage::tests::write_user_gp_stores_each_run3440_descriptor"]),
    ("pointer_format_check_bypassed", T,
     "match inspect_format(self.raw)", "match inspect_format(self.raw).or(Ok::<_, crate::user_stream::Refusal>(Source::Nop))",
     ["an_entry_inspect_entry_refuses_is_refused_by_name_and_never_published", "pointer_format_refuses_reserved_controls_and_unrepresentable_end_without_credit"]),
    ("storage_format_check_removed", S,
     "crate::user_stream::inspect_format(entry)", "Ok::<_, crate::user_stream::Refusal>(crate::user_stream::Source::Nop)",
     ["user_storage::tests::bad_user_descriptor_cannot_modify_private_ring_or_userd", "user_storage::tests::shadow_writer_preserves_opaque_bits_but_cannot_publish_or_forge_progress"]),
    ("scan_budget_removed", T,
     "if d.len_bytes > maximum {", "if false {",
     ["scan_mode_oversized_descriptor_is_refused_as_scan_budget_before_any_read_or_gate", "seeded_window_oversized_descriptor_is_refused_as_scan_budget_before_any_read_or_gate"]),
    ("scan_bound_exclusive", T,
     "if d.len_bytes > maximum {", "if d.len_bytes >= maximum {",
     ["scan_copy_bound_is_inclusive_and_checked_before_io", "window_copy_bound_is_inclusive_and_checked_before_io"]),
    ("large_pointer_attempts_pb_read", T,
     "                self.plan_forward = true;", "                let _ = io.read_pb(self.raw, &GpSegment { gpu_va: 0, len_bytes: 4 })?;\n                self.plan_forward = true;",
     ["pointer_forwards_the_largest_representable_descriptor", "pointer_forwards_each_run3440_descriptor_unchanged_without_reading_its_pushbuffer"]),
    ("pointer_large_counter_dropped", L,
     "self.counts.pointer_large_entries.saturating_add(1)", "self.counts.pointer_large_entries",
     ["large_pointer_counters_aggregate_counts_but_keep_a_vm_maximum", "pointer_forwards_each_run3440_descriptor_unchanged_without_reading_its_pushbuffer"]),
    ("pointer_max_counter_dropped", L,
     "self.counts.pointer_entry_max_bytes.max(d.len_bytes)", "self.counts.pointer_entry_max_bytes",
     ["a_1026596_byte_pointer_descriptor_completes_through_marker_and_get_writeback", "large_pointer_counters_aggregate_counts_but_keep_a_vm_maximum"]),
    ("vm_pointer_maximum_is_summed", L,
     "self.pointer_entry_max_bytes.fetch_max(d.pointer_entry_max_bytes, Relaxed)", "self.pointer_entry_max_bytes.fetch_add(d.pointer_entry_max_bytes, Relaxed)",
     ["large_pointer_counters_aggregate_counts_but_keep_a_vm_maximum", "pointer_maximum_is_not_summed_on_repeated_delta_flushes"]),
    ("scan_refusal_counter_dropped", L,
     "self.counts.refused_scan_budget.saturating_add(1)", "self.counts.refused_scan_budget",
     ["scan_mode_oversized_descriptor_is_refused_as_scan_budget_before_any_read_or_gate", "seeded_window_oversized_descriptor_is_refused_as_scan_budget_before_any_read_or_gate"]),
    ("scan_budget_event_dropped", T,
     "                        io.event(Ev::ScanBudget);\n", "",
     ["scan_mode_oversized_descriptor_is_refused_as_scan_budget_before_any_read_or_gate", "seeded_window_oversized_descriptor_is_refused_as_scan_budget_before_any_read_or_gate"]),
    ("large_counter_stops_relay", T,
     "        self.note_relayed(self.raw);", "        self.note_relayed(self.raw);\n        if self.life.counts().pointer_large_entries > 0 { return Err(\"MUTANT_COUNTER_STOPS\".into()); }",
     ["a_1026596_byte_pointer_descriptor_completes_through_marker_and_get_writeback", "a_large_captured_descriptor_is_not_reloaded_after_guest_mutation"]),
    ("end_at_2_40_admitted", F,
     "end >= 1 << 40", "end > 1 << 40",
     ["pointer_end_address_boundary_is_checked_without_copying_or_credit_on_refusal", "user_storage::tests::write_user_gp_refuses_end_at_2_40"]),
]


def edits():
    # Move exactly the production pre-read block after the gate's decision. A seeded
    # Window now publishes the gate first; an unseeded Scan reads before refusing.
    src = (ROOT / T).read_text()
    start = src.index("                if let Source::Segment(d) = desc {")
    end = src.index("                if restart {", start)
    block = src[start:end]
    late = block.replace("= desc {", "= self.desc.ok_or(\"USER_RING_GP_FORMAT\")? {")
    return [("scan_budget_after_gate", T, [(block, ""),
             ("                self.life.note_pb_read(&permit, words.len() * 4, vram);", late + "                self.life.note_pb_read(&permit, words.len() * 4, vram);")],
             ["scan_mode_oversized_descriptor_is_refused_as_scan_budget_before_any_read_or_gate", "seeded_window_oversized_descriptor_is_refused_as_scan_budget_before_any_read_or_gate"])]


def run(name, out, targets):
    if shutil.disk_usage("/").free < 6 * 1024**3:
        raise RuntimeError("STOP: less than 6 GiB free on /")
    target = targets / name
    if target.exists():
        raise RuntimeError(f"STOP: build variant target already exists: {target}")
    env = {**os.environ, "CARGO_TARGET_DIR": str(target), "CARGO_BUILD_JOBS": "2", "KAYFABE_NO_KVM": "1"}
    log = out / (name + ".log")
    with log.open("w") as stream:
        result = subprocess.run(["cargo", "test", "--locked", "-j2", "-p", "kf-chan", "--lib", "--test", "user_turn_unit", "--no-fail-fast"],
                                cwd=ROOT, env=env, stdout=stream, stderr=subprocess.STDOUT, timeout=600)
    text = log.read_text()
    if re.search(r"^error(?:\[|:.*(?:compil|build))", text, re.M) or "could not compile" in text:
        raise RuntimeError(f"{name}: compile error is not a mutant kill; see {log}")
    failed = sorted(set(re.findall(r"^test (\S+) \.\.\. FAILED$", text, re.M)))
    return result.returncode, failed


def main():
    if len(sys.argv) < 3:
        raise SystemExit(__doc__)
    out, targets = (Path(p).resolve() for p in sys.argv[1:3])
    out.mkdir(parents=True, exist_ok=True)
    targets.mkdir(parents=True, exist_ok=True)
    selected = set(sys.argv[3:])
    cases = [(n, p, [(a, b)], k) for n, p, a, b, k in MUTANTS] + edits()
    if selected - {n for n, *_ in cases}:
        raise RuntimeError("unknown mutant name")
    cases = [case for case in cases if not selected or case[0] in selected]
    # Validate all anchors before any mutation or build.
    for name, path, replacements, _ in cases:
        src = (ROOT / path).read_text()
        for old, new in replacements:
            if src.count(old) != 1:
                raise RuntimeError(f"{name}: expected one source anchor, found {src.count(old)}")
            src = src.replace(old, new)
    rc, failed = run("baseline", out, targets)
    if rc or failed:
        raise RuntimeError("unmutated baseline must pass")
    records = []
    for name, path, replacements, killers in cases:
        file = ROOT / path
        original = file.read_bytes()
        try:
            src = original.decode()
            for old, new in replacements:
                src = src.replace(old, new)
            file.write_text(src)
            rc, failed = run(name, out, targets)
        finally:
            file.write_bytes(original)
        good = rc != 0 and len(failed) >= 2 and set(killers) <= set(failed)
        record = {"mutant": name, "failed": failed, "required_killers": killers, "killed": good}
        if name == "pointer_format_check_bypassed":
            record["kill_sites"] = audit_format_kills(
                (out / (name + ".log")).read_text(), (ROOT / "crates/kf-chan/tests/user_turn_unit.rs").read_text())
            record["positive_controls"] = FORMAT_CONTROLS
        records.append(record)
        (out / "results.json").write_text(json.dumps(records, indent=2) + "\n")
        print(f"{name}: {'KILLED' if good else 'SURVIVED/THIN'} ({len(failed)} tests)", flush=True)
        if not good:
            raise RuntimeError(f"{name}: both named killers must fail")
    return 0


if __name__ == "__main__":
    sys.exit(main())
