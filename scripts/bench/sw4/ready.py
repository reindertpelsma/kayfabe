import datetime as dt
import ipaddress
import json
from pathlib import Path
import subprocess
import sys
import time
import console_pin as C


import provider_retry as P

D = Path(__file__).resolve().parent
E = Path((D / "current").read_text())
row = json.loads((E / "owned.json").read_text())[0]
iid = row["id"]


def now():
    return dt.datetime.now(dt.timezone.utc)


def age():
    return (now() - dt.datetime.fromisoformat(row["created_utc"])).total_seconds()


def api(args):
    r = subprocess.run(
        ["vastai", *args, "--raw"], capture_output=True, text=True, timeout=60, check=True
    )
    return json.loads(r.stdout)


def status():
    return next(r for r in api(["show", "instances"]) if r["id"] == iid)


def record(receipt):
    with (E / "console_pin.jsonl").open("a") as out:
        out.write(json.dumps({"lease_id": iid, "utc": now().isoformat(), **receipt}) + "\n")
    print("CONSOLE_PIN", receipt["state"], receipt["polls"], flush=True)


def console(timeout):
    x = status()
    if P.failure(x, age(), banner_seconds=None):
        raise RuntimeError("provider failure while waiting for console")
    return subprocess.run(
        ["vastai", "logs", str(iid), "--tail", "5000"],
        capture_output=True,
        text=True,
        timeout=timeout,
    ).stdout


subprocess.run(
    ["vastai", "attach", "ssh", str(iid), "/root/.ssh/id_ed25519.pub"],
    capture_output=True,
    check=True,
    timeout=60,
)
stage_start = factor = deadline = None
while True:
    x = status()
    print("READINESS", now().isoformat(), x.get("actual_status"), flush=True)
    failure = P.failure(x, age(), banner_seconds=None)
    if failure or (deadline is not None and age() >= deadline):
        reason = failure or "provider_boot_timeout"
        (E / "provider_failure.json").write_text(
            json.dumps(
                {
                    "id": iid,
                    "machine_id": row["machine_id"],
                    "reason": reason,
                    "utc": now().isoformat(),
                    "ssh_ready": False,
                    "status": {
                        k: x.get(k) for k in ("actual_status", "intended_status", "status_msg")
                    },
                },
                indent=2,
            )
            + "\n"
        )
        raise SystemExit(76)
    if x.get("actual_status") == "created" and x.get("intended_status") == "stopped":
        api(["start", "instance", str(iid)])
    port = ((x.get("ports") or {}).get("22/tcp") or [{}])[0].get("HostPort")
    if x.get("public_ipaddr") and port:
        ip = str(ipaddress.ip_address(x["public_ipaddr"]))
        assert ipaddress.ip_address(ip).is_global
        port = int(port)
        assert 0 < port < 65536
        begin = time.monotonic()
        scan = subprocess.run(
            ["ssh-keyscan", "-T", "5", "-t", "ed25519", "-p", str(port), ip],
            capture_output=True,
            text=True,
            timeout=min(10, 1200 - age()),
        )
        scan_seconds = max(0.001, time.monotonic() - begin)
        for fields in [s.split() for s in scan.stdout.splitlines() if not s.startswith("#")]:
            if len(fields) != 3:
                continue
            if stage_start is None:
                stage_start = age()
                factor = P.policy(scan_seconds)["factor"]
                deadline = min(1200, stage_start + C.stage_seconds(factor))
            try:
                proof = C.wait_for_pin(
                    fields[2],
                    console,
                    elapsed_since_create=age(),
                    factor=factor,
                    stage_started_elapsed=stage_start,
                    record=record,
                )
            except C.PinMismatch:
                (E / "identity_refusal.json").write_text(
                    json.dumps(
                        {"id": iid, "utc": now().isoformat(), "reason": "published_pin_mismatch"}
                    )
                    + "\n"
                )
                raise
            except C.PinExpired:
                (E / "identity_refusal.json").write_text(
                    json.dumps(
                        {"id": iid, "utc": now().isoformat(), "reason": "console_pin_absent"}
                    )
                    + "\n"
                )
                raise
            alias = "sw4-" + str(iid)
            (E / "known_hosts").write_text(alias + " " + fields[1] + " " + fields[2] + "\n")
            conf = (
                f"Host sw4-own\n HostName {ip}\n Port {port}\n User root\n"
                " IdentityFile /root/.ssh/id_ed25519\n IdentitiesOnly yes\n"
                " ForwardAgent no\n BatchMode yes\n ConnectTimeout 80\n"
                " ServerAliveInterval 15\n ServerAliveCountMax 3\n"
                f" HostKeyAlias {alias}\n UserKnownHostsFile {E}/known_hosts\n"
                " StrictHostKeyChecking yes\n"
            )
            (E / "ssh.conf").write_text(conf)
            result = subprocess.run(
                ["ssh", "-F", str(E / "ssh.conf"), "sw4-own", "echo SW4_SSH_READY"],
                capture_output=True,
                text=True,
                timeout=min(100, deadline - age()),
            )
            if result.returncode == 0 and "SW4_SSH_READY" in result.stdout:
                assert age() < deadline
                row.update(
                    ssh_ready_utc=now().isoformat(),
                    console_pin_proof=proof,
                    observed_dph_total=float(x["dph_total"]),
                )
                (E / "owned.json").write_text(json.dumps([row], indent=2) + "\n")
                print("READINESS_PASS", iid, proof["fingerprint"], flush=True)
                raise SystemExit(0)
            print("PINNED_SSH_RC", result.returncode, flush=True)
    time.sleep(20)
