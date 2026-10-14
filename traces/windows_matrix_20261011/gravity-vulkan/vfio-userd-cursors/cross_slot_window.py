#!/usr/bin/env python3
"""Report same-page raw slot observations without assigning new-handle ownership."""
import gzip
import importlib.util
import json
from pathlib import Path
import tempfile


def main():
    here = Path(__file__).resolve().parent
    spec = importlib.util.spec_from_file_location("samples", here / "summarize_samples.py")
    samples = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(samples)
    spec = importlib.util.spec_from_file_location("allocation_audit", here.parent / "vfio-ring-p0/audit_allocations.py")
    audit = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(audit)
    with tempfile.TemporaryDirectory(prefix="kf-cross-slot-") as scratch:
        scratch = Path(scratch)
        for name in ("userd-samples", "gsp"):
            data = gzip.open(here / "reviewed-result" / (name + ".jsonl.gz"), "rb").read(64 * 1024 * 1024 + 1)
            if len(data) > 64 * 1024 * 1024:
                raise ValueError("bounded export exceeded")
            (scratch / (name + ".jsonl")).write_bytes(data)
        summary = samples.summarize(scratch / "userd-samples.jsonl")
        trace = audit.decode.parse_jsonl(scratch / "gsp.jsonl")
        allocations = audit.audit(trace)
        rows = [json.loads(line) for line in (scratch / "userd-samples.jsonl").read_text().splitlines()]
    if len(summary["registrations"]) != 2 or len(allocations["fifo_registrations"]) != 2:
        raise ValueError("expected this run's two registrations")
    first, second = summary["registrations"]
    old = first["memory_candidate"]
    new = second["memory_candidate"]
    reg = second["registration"]
    if old["gpa"] != new["gpa"] or old["offset"] != new["offset"] or old["generation"] != new["generation"]:
        raise ValueError("not the same observed physical page")
    observed = reg["observed_ns"]
    window = []
    for row in rows:
        if row["kind"] != "sample" or row["offset"] != old["offset"]:
            continue
        if not -12_000_000 <= row["sampled_ns"] - observed <= 22_000_000:
            continue
        candidate_matches = any(all(c[k] == old[k] for k in ("generation", "client", "handle", "allocation_serial"))
                                for c in row["candidates"])
        if not candidate_matches:
            raise ValueError("window lost original candidate association")
        fields = [f for f in row["fields"] if f["slot"] in (first["registration"]["slot"], reg["slot"])]
        second_fields = [f for f in row["second"] if f["slot"] in (first["registration"]["slot"], reg["slot"])]
        window.append(dict(sequence=row["sequence"], relative_us=(row["sampled_ns"] - observed) / 1000,
                           fields=fields, second=second_fields, stable=fields == second_fields,
                           candidate_handle=old["handle"], new_handle_associated=False))
    return dict(schema="kayfabe-native-userd-cross-slot-window/1", complete=False,
                validated_sample_footer=summary["footer"], validated_gsp_records=len(trace["records"]),
                old_registration=first["registration"], new_registration=reg,
                original_candidate=old, later_candidate=new, window=window,
                limitation="Raw fields of the original candidate's page. No new-handle ownership, atomic trigger, exact initial GPU cursor, or retirement proof.",
                inference="Zero fields followed by lower progressing indices support a new-ring reset hypothesis; no universal reset/preserve rule is established.")


if __name__ == "__main__":
    print(json.dumps(main(), indent=2))
