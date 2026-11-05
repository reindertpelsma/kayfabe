"""One pre-SSH allocation stretch; native/guest failures never enter this loop."""

from pathlib import Path
import datetime
import json
import subprocess
import sys

D = Path(__file__).resolve().parent
W = Path("/workspace/bw-native-edge-wt")
T = W / "traces/blackwell_provider_validation_20261023"
sys.path.insert(0, "/workspace/bw-provider-retry-wt/scripts/bench/blackwell")
import provider_retry as P


def ops(mode, name):
    with (D / name).open("w") as out:
        result = subprocess.run(
            [sys.executable, str(D / "ops.py"), mode], stdout=out, stderr=subprocess.STDOUT
        )
    return result.returncode


def absent(iid):
    rows = json.loads(
        subprocess.check_output(
            ["vastai", "show", "instances", "--raw"],
            text=True,
            stderr=subprocess.DEVNULL,
            timeout=45,
        )
    )
    return not any(int(row["id"]) == iid for row in rows)


def destroy(iid):
    assert int((D / "own-id").read_text()) == iid
    assert ops("destroy", "destroy_" + str(iid) + ".txt") == 0
    assert absent(iid)


while True:
    retry = json.loads((D / "stretch.json").read_text())["retries_used"] + 1
    if retry > 2:
        raise SystemExit("provider retries exhausted; no live lease remains")
    if ops("rent", "rent_" + str(retry) + ".txt"):
        raise SystemExit("offer/create/budget refusal; no automatic continuation")
    iid = int((D / "own-id").read_text())
    print("OWN_REPLACEMENT_CREATED", iid, "retry", retry, flush=True)
    code = ops("ready", "ready_" + str(iid) + ".txt")
    if code == 0:
        print("OWN_STRETCH_READY", iid, flush=True)
        break
    if code != 76:
        destroy(iid)
        raise SystemExit("startup guard refusal; destroyed own id, no retry")
    rows = json.loads((T / "owned_instances.json").read_text())
    row = next(r for r in rows if int(r["id"]) == iid)
    proof = json.loads((D / ("provider_failure_" + str(iid) + ".json")).read_text())
    assert proof["status"]["id"] == iid and proof["machine_id"] == row["machine_id"]
    P.retire(
        row,
        {int(r["id"]) for r in rows},
        proof["reason"],
        D / "known_bad_machines.json",
        destroy,
        absent,
        datetime.datetime.now(datetime.timezone.utc).isoformat(),
    )
    (T / ("provider_failure_" + str(iid) + ".json")).write_text(json.dumps(proof, indent=2) + "\n")
    (T / "known_bad_machines.json").write_text((D / "known_bad_machines.json").read_text())
    print("OWN_PROVIDER_RETIRED_API_ABSENT", iid, "excluded_machine", row["machine_id"], flush=True)
