import json, subprocess, sys, datetime, os, time, re, base64, hashlib, ipaddress
from pathlib import Path

D = Path("/workspace/bw3-bfirst-run")
W = Path("/workspace/bw-native-edge-wt")
L = W / "traces/blackwell_ab_20261023/bfirst_run/owned_instances.json"


def api(args):
    r = subprocess.run(["vastai", *args, "--raw"], text=True, capture_output=True, timeout=60)
    try:
        return json.loads(r.stdout)
    except json.JSONDecodeError:
        try:
            e = json.loads(r.stderr)
        except json.JSONDecodeError:
            e = {}
        msg = str(e.get("msg", "")).lower()
        cause = next(
            (
                word
                for word in (
                    "unavailable",
                    "not available",
                    "not found",
                    "no such",
                    "insufficient",
                    "rate limit",
                    "too many",
                    "verification",
                    "credit",
                    "bid",
                    "price",
                )
                if word in msg
            ),
            "unclassified",
        )
        safe = {
            "command": args[:3],
            "returncode": r.returncode,
            "status_code": e.get("status_code"),
            "cause": cause,
            "stdout_bytes": len(r.stdout),
            "stderr_bytes": len(r.stderr),
        }
        if args[:2] == ["create", "instance"]:
            (D / "create_error_safe.json").write_text(json.dumps(safe, indent=2) + "\n")
        raise RuntimeError(safe) from None


def now():
    return datetime.datetime.now(datetime.timezone.utc)


def save(rows):
    L.parent.mkdir(parents=True, exist_ok=True)
    temporary = L.with_suffix(".tmp")
    temporary.write_text(json.dumps(rows, indent=2) + "\n")
    temporary.replace(L)


sys.path.insert(0, "/workspace/bw-provider-retry-wt/scripts/bench/blackwell")
import provider_retry as P

def arm_watch(row, seconds, kind, rows):
    assert 0 < seconds <= 150 * 60
    iid = int(row["id"])
    key = "auto_destroy" if kind == "auto" else "budget_watch"
    log = open(D / (kind + "_watch_" + str(iid) + ".log"), "x")
    child = subprocess.Popen(
        [sys.executable, str(D / "budget_watch.py"), str(iid), str(seconds)],
        stdout=log, stderr=subprocess.STDOUT, stdin=subprocess.DEVNULL,
        start_new_session=True,
    )
    row[key + "_pid"] = child.pid
    row[key + "_pid_starttime"] = int(
        Path("/proc", str(child.pid), "stat").read_text().rpartition(")")[2].split()[19]
    )
    save(rows)
    marker = D / ("timer_ready_" + str(iid) + "_" + str(child.pid) + ".json")
    for _ in range(50):
        os.kill(child.pid, 0)
        if marker.exists():
            ready = json.loads(marker.read_text())
            assert ready["id"] == iid and ready["pid"] == child.pid
            assert ready["seconds"] == seconds
            assert os.getpgid(child.pid) == child.pid
            return child.pid
        time.sleep(0.1)
    raise RuntimeError("owned relative timer did not signal ready")


mode = sys.argv[1]
if mode in ("preflight", "rent"):
    L.parent.mkdir(parents=True, exist_ok=True)
    rows = json.loads(L.read_text()) if L.exists() else []
    assert all(
        r.get("destroyed") and r.get("absence_verified_utc") for r in rows
    ), "close every own lease before replacement"
    media = json.loads((D / "media_manifest.json").read_text())
    assert media["all_verified"] and len(media["files"]) == 4
    for f in media["files"]:
        p = D / "media" / f["file"]
        assert p.stat().st_size == f["expected_bytes"]
        with p.open("rb") as src:
            assert hashlib.file_digest(src, "sha256").hexdigest() == f["expected_sha256"]
    sys.path.insert(0, str(W / "scripts/bench/blackwell"))
    from lease_ownership import may_rent

    owned = {
        57099244,
        57107585,
        57108853,
        57117614,
        57132969,
        57142142,
        57143789,
        57148807,
        57273215,
        57280677,
        57285340,
        57296911,
        57302449,
        57310491,
        57324106,
    }
    owned.update(int(r["id"]) for r in rows)
    for topic in (
        "blackwell_validation_20261022",
        "blackwell_validation2_20261022",
        "blackwell_validation3_20261022",
        "blackwell_validation4_20261022",
    ):
        ledger = W / "traces" / topic / "owned_instances.json"
        if ledger.exists():
            owned.update(int(r["id"]) for r in json.loads(ledger.read_text()))
    live = api(["show", "instances"])
    assert may_rent(live, owned), "another own lane lease is active"
    credit = float(api(["show", "user"])["credit"])
    skus = {
        "RTX_5090": "GB202",
        "RTX_5080": "GB203",
        "RTX_5070_Ti": "GB203",
        "RTX_5070": "GB205",
    }
    offers = []
    for sku, die in reversed(list(skus.items())):
        found = api(
            [
                "search",
                "offers",
                f"vms_enabled=true num_gpus=1 gpu_name={sku} disk_space>=180 cpu_cores_effective>=16 cpu_ram>=32 inet_down>200 reliability>=0.94 verified=any",
                "-o",
                "dph_total",
                "--limit",
                "100",
            ]
        )
        for x in found:
            x["candidate_die"] = die
        offers.extend(found)
    safe = [
        {
            k: x.get(k)
            for k in (
                "id",
                "machine_id",
                "gpu_name",
                "candidate_die",
                "verification",
                "dph_total",
                "cpu_cores_effective",
                "cpu_ram",
                "reliability",
                "inet_down",
                "disk_space",
            )
        }
        for x in offers
    ]
    (
        W / "traces/blackwell_ab_20261023/bfirst_run/market_all_rental_preflight.json"
    ).write_text(
        json.dumps(
            {
                "utc": now().isoformat(),
                "credit_usd": credit,
                "offers": safe,
                "authority": "owner22:50 all verified RTX50 series; other requirements unchanged",
            },
            indent=2,
        )
        + "\n"
    )
    if not offers:
        print("NO_ELIGIBLE_OFFERS (no creation attempted)", flush=True)
        sys.exit(75)
    offers = [
        x
        for x in offers
        if x.get("verification") == "verified"
        and float(x.get("cpu_ram", 0)) >= 32768
        and int(x["machine_id"]) not in (145861, 152912)
        and int(x["id"]) != 55276777
    ]
    if not offers:
        print("NO_SUITABLE_REPLACEMENT_OFFERS", flush=True)
        sys.exit(75)
    offers = P.exclude_bad(offers, D / "known_bad_machines.json")
    assert offers, "no verified different-machine offers after known-bad exclusion"
    runtime_spent = 4.556202657379075 + sum(
        r.get("runtime_cost_estimate_usd", 0) for r in rows
    )
    spent = runtime_spent
    minutes = 150
    def fits_budget(candidate):
        price = float(candidate["dph_total"])
        guarded = max(price * 1.15, price + 0.10)
        exposure = guarded * minutes / 60 + 0.50
        return spent + exposure < 7.5 and credit - exposure >= 5
    eligible = [x for x in offers if fits_budget(x)]
    assert eligible, "no approved die fits the fixed150-minute budget"
    offer = min(
        eligible,
        key=lambda x: (
            ["GB202", "GB203", "GB205"].index(x["candidate_die"]),
            float(x["dph_total"]),
        ),
    )
    rate = float(offer["dph_total"])
    rate_guard = max(rate * 1.15, rate + 0.10)
    reservation = rate_guard * minutes / 60 + 0.50
    assert spent + reservation < 7.5 and credit - reservation >= 5
    if mode == "preflight":
        print(
            json.dumps(
                {
                    "credit": credit,
                    "chosen": {
                        k: offer.get(k)
                        for k in (
                            "id",
                            "machine_id",
                            "gpu_name",
                            "candidate_die",
                            "verification",
                            "dph_total",
                            "cpu_cores_effective",
                            "cpu_ram",
                            "reliability",
                            "inet_down",
                        )
                    },
                    "reservation_usd": reservation,
                    "prior_spend_usd": spent,
                    "eligible_verified": len(offers),
                },
                indent=2,
            )
        )
        sys.exit(0)
    source = subprocess.check_output(["git", "rev-parse", "6e3fd3fd6"], cwd=W, text=True).strip()
    if rows:
        assert P.can_replace(
            rows[-1], P.load(D / "known_bad_machines.json", [])
        ), "last cell was not a retired provider failure; no replacement"
    retry_index = P.reserve_retry(D / "stretch.json")
    result = api(
        [
            "create",
            "instance",
            str(offer["id"]),
            "--image",
            "docker.io/vastai/kvm:ubuntu_cli_22.04-2025-05-16",
            "--disk",
            "180",
            "--cancel-unavail",
            "--ssh",
            "--direct",
            "--label",
            "codex-bw3-bfirst-20261023",
        ]
    )
    assert result.get("success")
    iid = int(result["new_contract"])
    row = {
        "id": iid,
        "retry_index": retry_index,
        "offer": offer["id"],
        "machine_id": offer["machine_id"],
        "gpu_offer": offer["gpu_name"],
        "candidate_die": offer["candidate_die"],
        "measurement_date": now().date().isoformat(),
        "quote_dph": rate,
        "verification": offer.get("verification"),
        "reliability": offer.get("reliability"),
        "created_utc": now().isoformat(),
        "credit_before": credit,
        "prior_lane_runtime_usd": runtime_spent,
        "prior_lane_spend_guard_usd": spent,
        "reservation_usd": reservation,
        "budget_stop": 7.5,
        "phase_cap": 8,
        "credit_reserve": 5,
        "auto_destroy_minutes": 150,
        "budget_cutoff_minutes": minutes,
        "guard_rate": rate_guard,
        "product_head": source,
        "measured_candidate": "48b027267a0e05c9ffdb4a049d34f4d719fd1837",
        "destroyed": False,
        "authority": "owner09:45 continuation: verified GB202 preferred, GB203/205 fallback; B48b first, A6e3 optional; phase8 stop7.5 reserve5; fixed150-minute timer",
    }
    rows.append(row)
    save(rows)
    try:
        # REQUIRED FIRST: a ready lease-relative 150-minute exact-own-id timer.
        pid = arm_watch(row, 150 * 60, "auto", rows)
        row.update(
            auto_destroy_requested_minutes=150,
            auto_destroy_minutes=150,
            auto_destroy_pid=pid,
            auto_destroy_verified=True,
            auto_destroy_armed_utc=now().isoformat(),
            auto_destroy_method="lease_relative_exact_id_ready_marker",
        )
        save(rows)
        elapsed = (now() - datetime.datetime.fromisoformat(row["created_utc"])).total_seconds()
        budget_pid = arm_watch(row, max(1, minutes * 60 - elapsed), "budget", rows)
        row.update(budget_watch_pid=budget_pid, budget_watch_verified=True)
        for key in ("auto_destroy_pid", "budget_watch_pid"):
            row[key + "_starttime"] = int(
                Path("/proc", str(row[key]), "stat").read_text().rpartition(")")[2].split()[19]
            )
        save(rows)
    except BaseException:
        subprocess.run(["vastai", "destroy", "instance", str(iid), "-y"], capture_output=True)
        for _ in range(10):
            if not any(int(x["id"]) == iid for x in api(["show", "instances"])):
                break
            time.sleep(3)
        else:
            raise RuntimeError("timer arming failed and API absence unverified")
        row.update(
            destroyed=True,
            absence_verified_utc=now().isoformat(),
            failure="auto-destroy arming failure",
        )
        for key in ("auto_destroy_pid", "budget_watch_pid"):
            pid = row.get(key)
            try:
                actual = int(Path("/proc", str(pid), "stat").read_text()
                             .rpartition(")")[2].split()[19])
                if actual == row.get(key + "_starttime") and os.getpgid(pid) == pid:
                    os.killpg(pid, 15)
            except (ProcessLookupError, FileNotFoundError):
                pass
        row["runtime_cost_estimate_usd"] = (
            now() - datetime.datetime.fromisoformat(row["created_utc"])
        ).total_seconds() / 3600 * row["guard_rate"]
        row["lane_runtime_total_usd"] = (
            row["prior_lane_runtime_usd"] + row["runtime_cost_estimate_usd"]
        )
        save(rows)
        raise
    (D / "own-id").write_text(str(iid) + "\n")
    print(json.dumps(row), flush=True)
elif mode == "ready":
    iid = int((D / "own-id").read_text())
    row = next(x for x in json.loads(L.read_text()) if x["id"] == iid)
    assert row["auto_destroy_verified"]
    os.kill(row["auto_destroy_pid"], 0)
    banner_seconds = None
    while True:
        x = next(x for x in api(["show", "instances"]) if int(x["id"]) == iid)
        print("READINESS", now().isoformat(), x.get("actual_status"), flush=True)
        elapsed = (now() - datetime.datetime.fromisoformat(row["created_utc"])).total_seconds()
        reason = P.failure(x, elapsed, banner_seconds=banner_seconds)
        if reason:
            safe = {
                k: x.get(k)
                for k in (
                    "id",
                    "actual_status",
                    "intended_status",
                    "status_msg",
                    "cur_state",
                    "dph_total",
                )
            }
            receipt = {
                "utc": now().isoformat(),
                "reason": reason,
                "machine_id": row["machine_id"],
                "status": safe,
                "elapsed_seconds": elapsed,
                "adaptive_boot_seconds": P.boot_bound(banner_seconds),
                "first_pinned_banner_seconds": banner_seconds,
            }
            (D / ("provider_failure_" + str(iid) + ".json")).write_text(
                json.dumps(receipt, indent=2) + "\n"
            )
            print("PROVIDER_RETRYABLE", iid, reason, flush=True)
            sys.exit(76)
        if (
            x.get("actual_status") == "created"
            and x.get("intended_status") == "stopped"
            and not row.get("explicit_start_requested_utc")
        ):
            subprocess.run(
                ["vastai", "start", "instance", str(iid), "--raw"],
                check=True,
                capture_output=True,
                text=True,
            )
            row["explicit_start_requested_utc"] = now().isoformat()
            rows = json.loads(L.read_text())
            rows[-1] = row
            save(rows)
        ports = x.get("ports") or {}
        port = (ports.get("22/tcp") or [{}])[0].get("HostPort")
        if x.get("public_ipaddr") and port:
            ip = str(ipaddress.ip_address(x["public_ipaddr"].strip()))
            assert ipaddress.ip_address(ip).is_global
            port = int(port)
            alias = "bw-fields-" + str(iid)
            console = subprocess.run(
                ["vastai", "logs", str(iid), "--tail", "5000"],
                capture_output=True,
                text=True,
                timeout=30,
            ).stdout
            scan = subprocess.run(
                ["ssh-keyscan", "-T", "5", "-t", "ed25519", "-p", str(port), ip],
                capture_output=True,
                text=True,
                timeout=10,
            ).stdout
            candidates = [l.split() for l in scan.splitlines() if l and not l.startswith("#")]
            for fields in candidates:
                if len(fields) < 3:
                    continue
                fp = "SHA256:" + base64.b64encode(
                    hashlib.sha256(base64.b64decode(fields[2])).digest()
                ).decode().rstrip("=")
                if fields[2] not in console and fp not in console:
                    continue
                (D / "known_hosts").write_text(alias + " " + fields[1] + " " + fields[2] + "\n")
                (D / "ssh.conf").write_text(
                    f"Host {alias}\n HostName {ip}\n Port {port}\n User root\n IdentityFile /root/.ssh/id_ed25519\n IdentitiesOnly yes\n ForwardAgent no\n BatchMode yes\n ConnectTimeout 80\n ServerAliveInterval 15\n ServerAliveCountMax 3\n HostKeyAlias {alias}\n UserKnownHostsFile {D}/known_hosts\n StrictHostKeyChecking yes\n"
                )
                remaining = (
                    P.boot_bound(banner_seconds)
                    - (now() - datetime.datetime.fromisoformat(row["created_utc"])).total_seconds()
                )
                if remaining <= 0:
                    continue
                banner_start = time.monotonic()
                try:
                    result = subprocess.run(
                        ["ssh", "-F", str(D / "ssh.conf"), alias, "echo CAPC_SSH_READY"],
                        capture_output=True,
                        text=True,
                        timeout=min(100, remaining),
                    )
                except subprocess.TimeoutExpired:
                    result = subprocess.CompletedProcess([], 255, "", "bounded SSH banner timeout")
                if banner_seconds is None:
                    banner_seconds = max(0.001, time.monotonic() - banner_start)
                if result.returncode == 0 and "CAPC_SSH_READY" in result.stdout:
                    elapsed = (
                        now() - datetime.datetime.fromisoformat(row["created_utc"])
                    ).total_seconds()
                    if elapsed >= P.boot_bound(banner_seconds):
                        continue
                    (D / "alias").write_text(alias)
                    row.update(
                        ssh_ready_utc=now().isoformat(),
                        ssh_hostkey_fingerprint=fp,
                        observed_dph_total=float(x["dph_total"]),
                    )
                    rows = json.loads(L.read_text())
                    rows[-1] = row
                    save(rows)
                    assert (
                        row["prior_lane_runtime_usd"]
                        + row["observed_dph_total"] * row["budget_cutoff_minutes"] / 60
                        + 0.50
                        < 7.5
                    )
                    print("READINESS_PASS", iid, fp, flush=True)
                    sys.exit(0)
                print("PINNED_SSH_RC", result.returncode, flush=True)
            if candidates and not any(
                fields[2] in console
                or (
                    "SHA256:"
                    + base64.b64encode(hashlib.sha256(base64.b64decode(fields[2])).digest())
                    .decode()
                    .rstrip("=")
                )
                in console
                for fields in candidates
            ):
                raise RuntimeError("host-key pin absent from authenticated instance console; STOP")
        time.sleep(15)
    raise RuntimeError("provider boot loop exited unexpectedly")
elif mode == "destroy":
    iid = int((D / "own-id").read_text())
    rows = json.loads(L.read_text())
    r = next(x for x in rows if x["id"] == iid)
    live = api(["show", "instances"])
    entry = next((x for x in live if int(x["id"]) == iid), None)
    if entry:
        r["observed_dph_total"] = float(entry["dph_total"])
    if entry:
        subprocess.run(
            ["vastai", "destroy", "instance", str(iid), "-y"],
            check=True,
            capture_output=True,
            text=True,
        )
    for _ in range(10):
        if not any(int(x["id"]) == iid for x in api(["show", "instances"])):
            break
        time.sleep(3)
    else:
        raise RuntimeError("API absence not verified; keep timer armed")
    r.update(destroyed=True, absence_verified_utc=now().isoformat())
    r["elapsed_hours_until_absence"] = (
        now() - datetime.datetime.fromisoformat(r["created_utc"])
    ).total_seconds() / 3600
    r["runtime_cost_estimate_usd"] = r["elapsed_hours_until_absence"] * r.get(
        "observed_dph_total", r["quote_dph"]
    )
    r["lane_runtime_total_usd"] = r["prior_lane_runtime_usd"] + r["runtime_cost_estimate_usd"]
    r["credit_after_observed"] = float(api(["show", "user"])["credit"])
    for key in ("auto_destroy_pid", "budget_watch_pid"):
        pid = r.get(key)
        try:
            actual = int(Path("/proc", str(pid), "stat").read_text().rpartition(")")[2].split()[19])
            if actual == r[key + "_starttime"] and os.getpgid(pid) == pid:
                os.killpg(pid, 15)
        except (ProcessLookupError, FileNotFoundError):
            pass
    r["auto_destroy_cancelled_after_absence"] = True
    r["budget_watch_cancelled_after_absence"] = True
    save(rows)
    print(json.dumps(r), flush=True)
