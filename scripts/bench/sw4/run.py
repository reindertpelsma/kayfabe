"""Prepare or execute the independently authorized single SW4 Linux lease."""

import argparse
import datetime as dt
import hashlib
import json
import math
from pathlib import Path
import shutil
import subprocess
import tarfile

import lifecycle
import offer_selector
import operator_backend as O
import round_budget as B

PRIVATE = Path("/data/sw4_private_capture")
PRODUCTION = "714ec29d3fdca0a013d9d478755346b0f1af6fa7"
PREDECESSOR = "0c326f0a93334b54d92f12c9eba82877f44aa41a"


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--execute", action="store_true")
    args = parser.parse_args()
    now = dt.datetime.now(dt.timezone.utc)
    grant = json.loads(Path("/root/night/SW4_GRANT.json").read_text())
    control = json.loads(Path("/root/night/COORD_CONTROL.json").read_text())
    assert grant["status"] == "AUTHORIZED" and grant["lane"] == "SW4"
    assert grant["rental_authorized"] is True and grant["max_leases"] == 1
    assert grant["source"] == PRODUCTION and grant["retry_after_refusal"] is False
    assert control["status"] == "ACTIVE" and control["allocations"]["SW4"]["authorized"]
    assert now < dt.datetime.fromisoformat(grant["latest_create_at"])
    assert now < dt.datetime.fromisoformat(control["no_new_work_after"])
    assert shutil.disk_usage("/").free >= 3 * 1024**3
    assert not O.EVIDENCE.exists(), "one lease only; no reuse or retry"
    lifecycle.source_lock(json.loads((PRIVATE / "predecessor_source_locks.json").read_text()))
    # Every required rehearsal is fresh and explicit; no inherited BW8 result is consulted.
    for name, count in (("final_operator_tests.log", 134), ("payload_rehearsal_complete.log", 140)):
        text = (PRIVATE / name).read_text()
        assert f"Ran {count} tests" in text and text.rstrip().endswith("OK")
    for name in ("sw4_mutants.json", "selector_mutants.json", "binding_mutants.json"):
        assert all(len(row["killers"]) >= 2 for row in json.loads((PRIVATE / name).read_text()))
    offers = O.api(
        [
            "search",
            "offers",
            "verified=true vms_enabled=true num_gpus=1 gpu_name=RTX_5060_Ti",
            "--storage",
            "180",
        ]
    )
    fields = (
        "id",
        "machine_id",
        "host_id",
        "geolocation",
        "verification",
        "vms_enabled",
        "is_vm_deverified",
        "num_gpus",
        "gpu_name",
        "gpu_ram",
        "dph_total",
        "storage_cost",
        "storage_total_cost",
        "inet_down_cost",
        "inet_up_cost",
        "disk_space",
        "cpu_cores_effective",
        "cpu_ram",
    )
    candidates = [
        {key: row.get(key) for key in fields}
        for row in offers
        if row["id"] in grant["candidate_offers"]
    ]
    O.save(PRIVATE / "requested_storage_candidates.json", {"utc": O.utc(), "rows": candidates})
    audit = json.loads((PRIVATE / "blacklist_audit.json").read_text())
    lifecycle.source_lock({row["path"]: row["sha256"] for row in audit})
    bad = {
        row[key]
        for item in audit
        for row in item["rows"]
        for key in ("machine_id", "host_id")
        if key in row
    }
    by_id = {row["id"]: row for row in candidates}
    selected = None
    for iid in grant["candidate_offers"]:
        if iid not in by_id:
            continue
        candidate = by_id[iid]
        # A blacklist hit is a lane refusal, never permission to choose a replacement.
        assert candidate["machine_id"] not in bad and candidate["host_id"] not in bad
        offer_selector.require_offer(candidate, bad)
        assert candidate["disk_space"] >= 180
        assert candidate["cpu_cores_effective"] >= 8 and candidate["cpu_ram"] >= 16 * 1024
        selected = candidate
        break
    assert selected is not None, "no fresh authorized candidate; no create"
    down, up = selected["inet_down_cost"], selected["inet_up_cost"]
    assert all(
        type(rate) in (float, int) and math.isfinite(rate) and rate >= 0 for rate in (down, up)
    )
    credit = float(O.api(["show", "user"])["credit"])
    transfer = 12 * down + up
    plan = B.plan(
        selected["dph_total"], credit, dt.datetime.now(dt.timezone.utc), transfer_usd=transfer
    )
    operator_deadline = dt.datetime.fromisoformat(control["lanes"]["codex-SW4"]["deadline"])
    assert dt.datetime.fromisoformat(plan["deadline_utc"]) <= operator_deadline
    assert plan["reservation_usd"] <= grant["stop_usd"] < grant["hard_cap_usd"]
    assert control["spent_or_retained_usd"] + grant["hard_cap_usd"] <= control["new_spend_usd_cap"]
    current = O.api(["show", "instances"])
    assert not any(row.get("label", "").startswith("codex-sw4-") for row in current)
    shutil.copyfile("/workspace/sw3-run/instruments.tar", O.HERE / "instruments.tar")
    derived = PRIVATE / "derived-source"
    for name in ("crates/kf-chip/src/die.rs", "crates/kf-chip/src/lib.rs"):
        path = derived / name
        path.parent.mkdir(parents=True, exist_ok=True)
        path.write_bytes(
            subprocess.check_output(
                [
                    "git",
                    "-C",
                    "/workspace/kayfabe",
                    "show",
                    PREDECESSOR + ":" + name,
                ]
            )
        )
    with tarfile.open(O.HERE / "derived.tar", "w") as archive:
        for path in derived.rglob("*.rs"):
            archive.add(
                path, arcname="derived-source/" + str(path.relative_to(derived)), recursive=False
            )
    locks = {
        str(path): hashlib.sha256(path.read_bytes()).hexdigest()
        for path in O.HERE.iterdir()
        if path.suffix in (".py", ".sh", ".tar")
    }
    O.save(PRIVATE / "sw4_source_locks.json", locks)
    production_hashes = {}
    for name in (
        "crates/kayfabe-rm-ladder/src/main.rs",
        "scripts/fastguest/fast_suite.sh",
        "scripts/fastguest/run_fast_guest.sh",
        "scripts/fastguest/build_fast_guest.sh",
        "scripts/bench/build_kf3.sh",
        "scripts/bench/v3_gates.sh",
    ):
        data = subprocess.check_output(
            [
                "git",
                "-C",
                "/workspace/kayfabe",
                "show",
                PRODUCTION + ":" + name,
            ]
        )
        production_hashes[name] = hashlib.sha256(data).hexdigest()
    registration = {
        "lane": "SW4",
        "utc": O.utc(),
        "offer": selected,
        "plan": plan,
        "production_source": PRODUCTION,
        "production_hashes": production_hashes,
        "derived_die_source": PREDECESSOR,
        "allowed_actual_dies": ["GB206", "GB207"],
        "marketing_is_not_die": True,
        "native_driver": "595.91.07 open",
        "guest_driver": "595.91.07 open, host-mode Linux",
        "native_hypothesis": "Actual bound GB206/GB207 passes gates9/9+gate10/USER and bare30/30.",
        "native_falsifier": "Binding/refusal, any gate/bare failure, or unmatched Xid excludes guest.",
        "guest_hypothesis": "After native pass, exact production source passes fast30/30.",
        "guest_falsifier": "Any failure/crash/unrun, unmatched Xid, or missed fixed180s oracle.",
        "xid_policy": "Exact source-derived native/guest type/channel/VA/client records only.",
        "flags": {"KF_DEVICE": "kf3", "KF_FROM_HOST": "1", "KF3_FB_MB": "8192"},
        "framebuffer": "16GB marketed VRAM; 8192MiB store preserves >=2GiB host headroom.",
        "bar1": "Observe256MiB host limit; run+mark; no resize; inherited guest128MiB.",
        "transfer_bound": {"download_bytes": 12_000_000_000, "upload_bytes": 1_000_000_000},
        "native_failure": "EXCLUDED/UNRUN guest; no kayfabe attribution",
        "max_leases": 1,
        "retry_after_real_refusal": False,
    }
    O.save(PRIVATE / "preregistration.json", registration)
    with O.REPORT.open("a") as stream:
        stream.write(
            "\nPreregistration before create: `" + json.dumps(registration, sort_keys=True) + "`.\n"
        )
    print(
        json.dumps(
            {
                "state": "PREREGISTERED",
                "offer": selected["id"],
                "rate_requested_180GiB": selected["dph_total"],
                "transfer_allocation_usd": transfer,
                "plan": plan,
            },
            indent=2,
        ),
        flush=True,
    )
    if args.execute:
        lifecycle.source_lock(locks)
        assert dt.datetime.now(dt.timezone.utc) < dt.datetime.fromisoformat(
            grant["latest_create_at"]
        )
        backend = O.Backend(selected, plan, credit)
        try:
            lifecycle.execute(backend, plan)
        except Exception as error:
            O.save(
                PRIVATE / "STOP.json",
                {
                    "utc": O.utc(),
                    "kind": type(error).__name__,
                    "reason": str(error),
                    "retry": False,
                },
            )
            print("SW4_STOP " + str(error), flush=True)
            raise SystemExit(1)


if __name__ == "__main__":
    main()
