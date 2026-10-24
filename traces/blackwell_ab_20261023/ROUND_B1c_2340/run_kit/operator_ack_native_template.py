from pathlib import Path
import json, sys, datetime

sys.path.insert(0, "/root/sweep2/guard/blackwell")
import phase_xid_guard as G

r = Path("/root/prov")
assert not (r / "XID_STOP").exists()
allowed = json.loads((r / "native_receipt.json").read_text())["approved_native_xid_lines"]
assert tuple(G.N.xid_lines(G.N.kernel_snapshot())) == tuple(allowed)
raw_summary = [
    l
    for l in (r / "sweep_bare595.log").read_text().splitlines()
    if l.startswith("BARE_SUITE_PASS=")
]
fuzz = json.loads((r / "native_fuzz_findings.json").read_text())
gr_summary = [
    l
    for l in (r / "sweep_native_grtier.log").read_text().splitlines()
    if l.startswith("GR_TIER_VERDICT=")
]
rows = [
    x
    for x in Path("/workspace/bench/cl_capc_native595_host.out").read_text().splitlines()
    if x.startswith("CL_ROW ")
]
# All native benchmark verdicts are findings in B1b; fault history remains exact.
j = {
    "utc": datetime.datetime.now(datetime.timezone.utc).isoformat(),
    "backup_kind": "bounded text exported to operator workspace before live B",
    "publication_deferred_until_live_complete": True,
    "local_export_receipt_sha256": "__LOCAL_EXPORT_RECEIPT_SHA256__",
    "raw_summary": raw_summary,
    "cuda_rows": rows,
    "gr_summary": gr_summary,
    "fuzz": fuzz,
    "native_faults": len(allowed),
    "exact_ordered_fault_match": True,
    "native_verdicts_are_a_gate": False,
    "experiment_mode": "OBSERVATION",
    "attestation": False,
}
(r / "N0_EXPORT_ACK").write_text(json.dumps(j) + "\n")
print(json.dumps(j))
