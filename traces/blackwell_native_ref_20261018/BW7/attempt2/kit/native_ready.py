"""Readiness for the one native Windows template; no other path dispatches here."""

import datetime as dt
import ipaddress
import json
from pathlib import Path
import subprocess
import time

import console_pin as C
import native_pin as N
import pin_evidence as PE

TEMPLATE = "183b10af7fd788d08aca02e31811d70d"
COLD_BOOT_SECONDS = 1200


def ready(directory, ledger, save, *, clock=time.monotonic, sleep=time.sleep,
          run=subprocess.run, utc=lambda: dt.datetime.now(dt.timezone.utc)):
    D, L = Path(directory), Path(ledger)
    iid = int((D / "own-id").read_text())
    rows = json.loads(L.read_text())
    row = next(r for r in rows if r["id"] == iid)
    assert row["native_template_hash"] == TEMPLATE
    assert row["no_kayfabe"] is True
    assert row["auto_destroy_verified"] and not row.get("destroyed")
    born = dt.datetime.fromisoformat(row["created_utc"])
    cold_deadline = clock() + max(0, COLD_BOOT_SECONDS - (utc() - born).total_seconds())

    def scan(host, port, timeout):
        result = run(
            ["ssh-keyscan", "-T", str(max(1, int(min(5, timeout)))),
             "-t", "ed25519", "-p", str(port), host],
            capture_output=True, text=True, timeout=timeout,
        )
        fields = [line.split() for line in result.stdout.splitlines()
                  if line and not line.startswith("#")]
        keys = [f[2] for f in fields if len(f) >= 3 and f[1] == "ssh-ed25519"]
        return keys[0] if len(keys) == 1 else None

    first_reachable = first_utc = key = None
    host = port = None
    while clock() < cold_deadline:
        remaining = cold_deadline - clock()
        result = run(["vastai", "show", "instances", "--raw"], capture_output=True,
                     text=True, check=True, timeout=min(45, remaining))
        status = next((x for x in json.loads(result.stdout) if x["id"] == iid), None)
        if status is None:
            raise C.PinFlap("owned native instance disappeared before identity admission")
        row["observed_dph_total"] = float(status["dph_total"])
        assert row["observed_dph_total"] <= 1.0
        assert row["observed_dph_total"] <= row["guard_rate"]
        row["provider_status"] = status.get("actual_status")
        print("NATIVE_PROVIDER", row["provider_status"], flush=True)
        ports = status.get("ports") or {}
        port = int((ports.get("22/tcp") or [{}])[0].get("HostPort") or 0)
        if status.get("public_ipaddr") and port:
            host = str(ipaddress.ip_address(status["public_ipaddr"].strip()))
            assert ipaddress.ip_address(host).is_global
            started, started_utc = clock(), utc()
            try:
                key = scan(host, port, min(10, max(0.01, cold_deadline - clock())))
            except subprocess.SubprocessError:
                key = None
            if key:
                first_reachable, first_utc = started, started_utc
                break
        sleep(min(15, max(0, cold_deadline - clock())))
    if first_reachable is None:
        raise C.PinFlap("native endpoint was not SSH-reachable before cold-boot bound")
    row.update({
        "native_first_reachable_utc": first_utc.isoformat(),
        "native_identity_deadline_utc":
            (first_utc + dt.timedelta(seconds=N.ABSOLUTE_SECONDS)).isoformat(),
        "native_stability_seconds": N.STABILITY_SECONDS,
    })
    save(rows)
    print("NATIVE_FIRST_REACHABLE", row["native_first_reachable_utc"], flush=True)

    def console(timeout):
        result = run(["vastai", "logs", str(iid), "--tail", "20000"],
                     capture_output=True, text=True, check=True, timeout=timeout)
        PE.capture(D, iid, result.stdout)
        return result.stdout

    def record(receipt):
        receipt = {"lease_id": iid, "utc": utc().isoformat(), **receipt}
        with (D / f"console_pin_{iid}.jsonl").open("a") as out:
            out.write(json.dumps(receipt) + "\n")
        print("NATIVE_PIN", receipt["state"], receipt["polls"],
              "stable", round(receipt["stable_seconds"], 1),
              "elapsed", round(receipt["elapsed_since_first_reachable"], 1), flush=True)

    proof = N.settle_pin(
        lambda timeout: scan(host, port, timeout), console,
        first_reachable_at=first_reachable, clock=clock, sleep=sleep, record=record,
    )
    remaining = first_reachable + N.ABSOLUTE_SECONDS - clock()
    if remaining <= 0:
        raise C.PinFlap("native identity proof arrived after fixed deadline")
    alias = f"bw7-native-{iid}"
    (D / "known_hosts").write_text(alias + " ssh-ed25519 " + proof["key"] + "\n")
    (D / "known_hosts_windows").write_text(alias + " ssh-ed25519 " + proof["key"] + "\n")
    conf = (
        f"Host {alias}\n HostName {host}\n Port {port}\n"
        " User vast\n IdentityFile /root/.ssh/id_ed25519\n IdentitiesOnly yes\n"
        " ForwardAgent no\n BatchMode yes\n ConnectTimeout 15\n"
        f" HostKeyAlias {alias}\n UserKnownHostsFile {D}/known_hosts\n"
        "StrictHostKeyChecking yes\n ServerAliveInterval 15\n ServerAliveCountMax 3\n"
    )
    (D / "ssh.conf").write_text(conf)
    (D / "ssh_windows.conf").write_text(conf)
    result = run(["ssh", "-F", str(D / "ssh.conf"), alias, "echo CAPC_SSH_READY"],
                 capture_output=True, text=True, timeout=min(20, remaining))
    if (clock() >= first_reachable + N.ABSOLUTE_SECONDS
            or result.returncode != 0 or "CAPC_SSH_READY" not in result.stdout):
        raise C.PinFlap("console-bound native endpoint did not finish strict SSH before deadline")
    row.update(ssh_ready_utc=utc().isoformat(), ssh_hostkey_fingerprint=proof["fingerprint"],
               console_pin_proof=proof)
    save(rows)
    (D / "alias").write_text(alias)
    print("NATIVE_READINESS_PASS", iid, flush=True)
    return 0
