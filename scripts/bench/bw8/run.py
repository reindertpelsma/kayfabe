"""Fresh precreate gate and exactly one explicitly granted rebuilt-parent probe lease."""

import argparse
import datetime as dt
import hashlib
import json
from pathlib import Path
import shutil
import sys

from backend import backend_type, load, source_lock
from execution import fit_plan, require_offer
from rehearse import complete_receipt

FIELDS = (
    "id machine_id host_id gpu_name geolocation verification vms_enabled "
    "is_vm_deverified num_gpus dph_base dph_total storage_cost inet_down_cost "
    "inet_up_cost disk_space cpu_ram cpu_cores_effective reliability inet_down"
).split()


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("config", type=Path)
    parser.add_argument("--execute", action="store_true")
    args = parser.parse_args()
    config = json.loads(args.config.read_text())
    source_lock(json.loads(Path(config["source_locks"]).read_text()))
    source_lock(config["grant_locks"])
    receipt = json.loads(Path(config["rehearsal"]).read_text())
    if not complete_receipt(receipt):
        raise ValueError("complete inherited rehearsal not green")
    source_lock(receipt["sources"])
    now = dt.datetime.now(dt.timezone.utc)
    age = (now - dt.datetime.fromisoformat(receipt["utc"])).total_seconds()
    if not 0 <= age <= 3600:
        raise ValueError("fresh rehearsal receipt expired")
    adapter = json.loads(Path(config["adapter_rehearsal"]).read_text())
    if adapter.get("passed") is not True:
        raise ValueError("adapter rehearsal not green")
    source_lock(adapter["sources"])
    mutants = json.loads(Path(config["mutants"]).read_text())
    if not all(len(v["killers"]) >= 2 for v in mutants.values()):
        raise ValueError("mutation qualification incomplete")
    if shutil.disk_usage("/").free < 3 * 1024**3:
        raise ValueError("local disk stop")
    if Path(config["evidence"]).exists():
        raise ValueError("one lease only; no reuse")
    sys.path.insert(0, config["operator"])
    provider = load("bw8_provider", Path(config["operator"]) / "operator_backend.py")
    lifecycle = load("bw8_lifecycle", Path(config["operator"]) / "lifecycle.py")
    offers = provider.api(
        [
            "search",
            "offers",
            "verified=true vms_enabled=true num_gpus=1 gpu_name=RTX_5090 "
            "disk_space>=180 cpu_cores_effective>=8 cpu_ram>=32 reliability>=0.95 inet_down>=200",
            "--storage",
            "180",
        ]
    )
    selected = next((r for r in offers if r["id"] == config["offer_id"]), None)
    if selected is None:
        raise ValueError("selected exact offer disappeared; no create")
    selected = {k: selected.get(k) for k in FIELDS}
    if (selected["machine_id"], selected["host_id"]) != tuple(config["machine_host"]):
        raise ValueError("selected offer machine/host changed")
    rate = require_offer(selected, set(config["blacklisted"]))
    credit = float(provider.api(["show", "user"])["credit"])
    live = provider.api(["show", "instances"])
    if any(
        r["id"] in config["owned_ids"] or str(r.get("label", "")).startswith("codex-bw8-")
        for r in live
    ):
        raise ValueError("prior own lease is live")
    transfer = 12 * selected["inet_down_cost"] + selected["inet_up_cost"]
    deadline = dt.datetime.fromisoformat(config["hard_deadline_utc"])
    plan = fit_plan(
        hourly=rate,
        credit=credit,
        now=dt.datetime.now(dt.timezone.utc),
        transfer=transfer,
        deadline=deadline,
        estimated_seconds=config["estimated_seconds"],
        shared_remaining=config["shared_remaining"],
    )
    config["plan"] = plan
    Path(config["runtime_config"]).write_text(json.dumps(config, indent=2) + "\n")
    summary = dict(
        utc=provider.utc(),
        scope="rebuilt-parent probe",
        offer=selected,
        plan=plan,
        authority_sha256=config["grant_locks"],
        falsifiers=config["falsifiers"],
        parent_identity="rebuilt, not equal to BW6 sealed parent",
        creates=0,
    )
    provider.save(Path(config["preregistration"]), summary)
    with Path(config["report"]).open("a") as stream:
        stream.write("\nPrecreate rebuilt-parent plan: `" + json.dumps(summary) + "`.\n")
    print(
        json.dumps(
            dict(
                state="PRECREATE_GREEN",
                offer=selected["id"],
                machine=selected["machine_id"],
                requested_180gb_rate=rate,
                deadline=plan["deadline_utc"],
                seconds=plan["seconds"],
                reservation_usd=plan["reservation_usd"],
                creates=0,
            )
        ),
        flush=True,
    )
    if args.execute:
        source_lock(json.loads(Path(config["source_locks"]).read_text()))
        source_lock(config["grant_locks"])
        source_lock(adapter["sources"])
        b = backend_type(provider)(selected, plan, credit, config)
        try:
            result = lifecycle.execute(b, plan)
            provider.save(Path(config["evidence"]) / "lifecycle_result.json", result)
        except BaseException as error:
            provider.save(
                Path(config["evidence"]) / "stop.json",
                dict(utc=provider.utc(), kind=type(error).__name__, reason=str(error), retry=False),
            )
            raise


if __name__ == "__main__":
    main()
