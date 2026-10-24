import json, subprocess, sys, datetime, os, time, re, base64, hashlib, ipaddress
from concurrent.futures import ThreadPoolExecutor
from pathlib import Path

D = Path("/workspace/bw3-final-run")
W = Path("/workspace/bw-native-edge-wt")
L = W / "traces/blackwell_ab_20261023/UNVERIFIED-HOST_1345/owned_instances.json"


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
    rehearsal=json.loads((D/"rehearsal_pass.json").read_text())
    assert rehearsal["tests"]==3 and rehearsal["passed"]==3
    assert hashlib.sha256(Path(__file__).read_bytes()).hexdigest()==rehearsal["operator_source_sha256"]
    assert hashlib.sha256((W/"scripts/bench/blackwell/console_pin.py").read_bytes()).hexdigest()==rehearsal["guard_source_sha256"]
    L.parent.mkdir(parents=True, exist_ok=True)
    temporary = L.with_suffix(".tmp")
    temporary.write_text(json.dumps(rows, indent=2) + "\n")
    temporary.replace(L)


sys.path.insert(0, "/workspace/bw-provider-retry-wt/scripts/bench/blackwell")
import provider_retry as P
sys.path.insert(0, str(W / "scripts/bench/blackwell"))
import console_pin as C

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
print("UNVERIFIED-HOST",mode,flush=True)
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
        57340681,
        57359730,
        57363603,
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
    skus = {"RTX_5070":"GB205", "RTX_5060_Ti":"GB206", "RTX_5060":"GB206",
            "RTX_5080":"GB203", "RTX_5070_Ti":"GB203", "RTX_5090":"GB202",
            "RTX_5050":"GB207"}
    cache = D / "single_market_1345.json"
    if cache.exists():
        market = json.loads(cache.read_text())
        offers = market["offers"]
    else:
        def search(item):
            sku, die = item
            found = api(["search", "offers",
                f"vms_enabled=true num_gpus=1 gpu_name={sku} disk_space>=180 "
                "cpu_cores_effective>=16 cpu_ram>=32 inet_down>=200 reliability>=0.95 verified=any",
                "-o", "dph_total", "--limit", "100"])
            for offer in found:offer["candidate_die"] = die
            return found
        offers=[]
        with ThreadPoolExecutor(max_workers=7) as pool:
            for found in pool.map(search, skus.items()):offers.extend(found)
        market={"utc":now().isoformat(),"host_verification":"UNVERIFIED-HOST","offers":offers}
        cache.write_text(json.dumps(market)+"\n")
    safe=[{k:x.get(k) for k in ("id","machine_id","gpu_name","candidate_die","verification",
          "dph_total","reliability","inet_down","vms_enabled","cpu_cores_effective","cpu_ram",
          "disk_space","is_vm_deverified")} for x in offers]
    (L.parent/"market.json").write_text(json.dumps({"host_verification":"UNVERIFIED-HOST",
        "utc":market["utc"],"credit_usd":credit,"offers":safe,
        "authority":"controller13:45 non-verified RTX50, one market check, no bad machines"},
        indent=2)+"\n")
    offers=[x for x in offers if x.get("verification") in ("verified","unverified","deverified")
            and float(x.get("reliability",0))>=0.95 and float(x.get("inet_down",0))>=200
            and bool(x.get("vms_enabled")) and not x.get("is_vm_deverified",False)
            and float(x.get("dph_total",99))<=1.10 and float(x.get("cpu_ram",0))>=32768]
    offers=P.exclude_bad(offers,D/"known_bad_machines.json")
    spent=5.243903789283519+sum(r.get("runtime_cost_estimate_usd",0) for r in rows)
    runtime_spent=spent;minutes=150
    def exposure(x):
        rate=float(x["dph_total"])
        return max(rate*1.15,rate+0.10)*2.5+0.50
    eligible=[x for x in offers if spent+exposure(x)<7.5 and credit-exposure(x)>=5]
    if not eligible:
        print("UNVERIFIED-HOST NO_QUALIFYING_OFFER_ONE_MARKET_CHECK",flush=True)
        sys.exit(75)
    offer=min(eligible,key=lambda x:(x.get("verification")!="verified",["GB205","GB206","GB203","GB202","GB207"].index(
        x["candidate_die"]),float(x["dph_total"])))
    rate=float(offer["dph_total"]);rate_guard=max(rate*1.15,rate+0.10)
    reservation=exposure(offer)
    if rows:
        assert P.can_replace(rows[-1],P.load(D/"known_bad_machines.json",[])), "not a provider-only retry"
    state=json.loads((D/"stretch.json").read_text())
    assert state["attempts_used"]<3 and state["provider_retries_used"]<=2
    state["attempts_used"]+=1
    (D/"stretch.json").write_text(json.dumps(state)+"\n")
    retry_index=state["attempts_used"]
    one_time_exception=False
    source=subprocess.check_output(["git","rev-parse","6e3fd3fd6"],cwd=W,text=True).strip()
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
            "codex-bw3-UNVERIFIED-HOST-20261023",
        ]
    )
    if not result.get("success"):
        message = str(result.get("msg", result.get("error", ""))).lower()
        categories = [word for word in ("not found", "not available", "unavailable",
                      "already rented", "no such", "verification", "credit", "price")
                      if word in message]
        (D / "create_refusal_1305.json").write_text(json.dumps({
            "utc":now().isoformat(),"offer_id":int(offer["id"]),
            "machine_id":int(offer["machine_id"]),"success":False,
            "reason_categories":categories,"new_contract_returned":bool(result.get("new_contract")),
            "cancel_unavail":True,"single_recheck_reused":True})+"\n")
        raise RuntimeError("exact approved fallback create refused; see sanitized receipt")
    iid = int(result["new_contract"])
    row = {
        "id": iid,
        "host_verification": "UNVERIFIED-HOST",
        "retry_index": retry_index,
        "one_time_provider_exception": one_time_exception,
        "provider_failure_requires_stop": one_time_exception,
        "offer": offer["id"],
        "machine_id": offer["machine_id"],
        "gpu_offer": offer["gpu_name"],
        "candidate_die": offer["candidate_die"],
        "measurement_date": now().date().isoformat(),
        "quote_dph": rate,
        "verification": offer.get("verification"),
        "verification_last_seen_utc": offer.get("verification_last_seen_utc"),
        "controller_named_fallback": offer.get("controller_named_fallback", False),
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
        "authority": "controller13:45 UNVERIFIED-HOST RTX50 sweep; one market check, reliability>=.95/KVM/price<=1.10/inet>=200, no bad machines, max2 provider retries; same scoped observation; measured GB202/GB206 pages, explicit GB202-to-GB203/205/207 inheritance; B48b then optional measured-page+B1; max8 exact owned-pair records, other faults/errors/health fatal; phase8 stop7.5 reserve5; fixed150-minute timer",
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
    pin_stage_started = pin_factor = pin_deadline = None

    def lease_age():
        return (now() - datetime.datetime.fromisoformat(row["created_utc"])).total_seconds()

    def fail_readiness(reason, status, elapsed):
        safe = {key: status.get(key) for key in (
            "id", "actual_status", "intended_status", "status_msg", "cur_state", "dph_total"
        )}
        receipt = {
            "host_verification":"UNVERIFIED-HOST", "utc": now().isoformat(), "reason": reason,
            "machine_id": row["machine_id"], "status": safe,
            "elapsed_seconds": elapsed, "adaptive_boot_seconds": 1200,
            "first_pinned_banner_seconds": banner_seconds,
            "console_stage_started_elapsed": pin_stage_started,
            "console_factor": pin_factor, "console_deadline_since_create": pin_deadline,
        }
        (D / ("provider_failure_" + str(iid) + ".json")).write_text(
            json.dumps(receipt, indent=2) + "\n"
        )
        print("PROVIDER_RETRYABLE", iid, reason, flush=True)
        sys.exit(76)

    def record_pin(receipt):
        with (D / ("console_pin_" + str(iid) + ".jsonl")).open("a") as out:
            out.write(json.dumps({"host_verification":"UNVERIFIED-HOST","lease_id": iid, "utc": now().isoformat(), **receipt}) + "\n")
        print("CONSOLE_PIN", receipt["state"], receipt["polls"], flush=True)

    def console_text(timeout):
        current = next(x for x in api(["show", "instances"]) if int(x["id"]) == iid)
        reason = P.failure(current, lease_age(), banner_seconds=None)
        if reason:
            fail_readiness(reason, current, lease_age())
        return subprocess.run(
            ["vastai", "logs", str(iid), "--tail", "5000"],
            capture_output=True, text=True, timeout=timeout,
        ).stdout

    while True:
        x = next(x for x in api(["show", "instances"]) if int(x["id"]) == iid)
        print("READINESS", now().isoformat(), x.get("actual_status"), flush=True)
        elapsed = lease_age()
        reason = P.failure(x, elapsed, banner_seconds=None)
        if pin_deadline is not None and elapsed >= pin_deadline:
            reason = reason or "provider_boot_timeout"
        if reason:
            fail_readiness(reason, x, elapsed)
        if (
            x.get("actual_status") == "created"
            and x.get("intended_status") == "stopped"
            and not row.get("explicit_start_requested_utc")
        ):
            subprocess.run(
                ["vastai", "start", "instance", str(iid), "--raw"],
                check=True, capture_output=True, text=True,
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
            scan_start = time.monotonic()
            scan = subprocess.run(
                ["ssh-keyscan", "-T", "5", "-t", "ed25519", "-p", str(port), ip],
                capture_output=True, text=True, timeout=min(10, 1200 - elapsed),
            ).stdout
            scan_seconds = max(0.001, time.monotonic() - scan_start)
            candidates = [l.split() for l in scan.splitlines() if l and not l.startswith("#")]
            for fields in candidates:
                if len(fields) < 3:
                    continue
                if pin_stage_started is None:
                    pin_stage_started = lease_age()
                    pin_factor = P.policy(scan_seconds)["factor"]
                    pin_deadline = min(1200, pin_stage_started + C.stage_seconds(pin_factor))
                try:
                    proof = C.wait_for_pin(
                        fields[2], console_text, elapsed_since_create=lease_age(),
                        factor=pin_factor, stage_started_elapsed=pin_stage_started,
                        clock=time.monotonic, sleep=time.sleep, record=record_pin,
                    )
                except C.PinExpired:
                    fail_readiness("provider_boot_timeout", x, lease_age())
                # A different published pin propagates immediately as a safety refusal.
                fp = proof["fingerprint"]
                (D / "known_hosts").write_text(alias + " " + fields[1] + " " + fields[2] + "\n")
                (D / "ssh.conf").write_text(
                    f"Host {alias} s4-own\n HostName {ip}\n Port {port}\n User root\n IdentityFile /root/.ssh/id_ed25519\n IdentitiesOnly yes\n ForwardAgent no\n BatchMode yes\n ConnectTimeout 80\n ServerAliveInterval 15\n ServerAliveCountMax 3\n HostKeyAlias {alias}\n UserKnownHostsFile {D}/known_hosts\n StrictHostKeyChecking yes\n"
                )
                remaining = min(1200, pin_deadline) - lease_age()
                if remaining <= 0:
                    fail_readiness("provider_boot_timeout", x, lease_age())
                banner_start = time.monotonic()
                try:
                    result = subprocess.run(
                        ["ssh", "-F", str(D / "ssh.conf"), alias, "echo CAPC_SSH_READY"],
                        capture_output=True, text=True, timeout=min(100, remaining),
                    )
                except subprocess.TimeoutExpired:
                    result = subprocess.CompletedProcess([], 255, "", "bounded SSH banner timeout")
                if banner_seconds is None:
                    banner_seconds = max(0.001, time.monotonic() - banner_start)
                if result.returncode == 0 and "CAPC_SSH_READY" in result.stdout:
                    if lease_age() >= min(1200, pin_deadline):
                        fail_readiness("provider_boot_timeout", x, lease_age())
                    (D / "alias").write_text(alias)
                    row.update(
                        ssh_ready_utc=now().isoformat(), ssh_hostkey_fingerprint=fp,
                        observed_dph_total=float(x["dph_total"]),
                        console_pin_proof=proof, console_pin_factor=pin_factor,
                    )
                    rows = json.loads(L.read_text())
                    rows[-1] = row
                    save(rows)
                    assert (
                        row["prior_lane_runtime_usd"]
                        + row["observed_dph_total"] * row["budget_cutoff_minutes"] / 60
                        + 0.50 < 7.5
                        and row["observed_dph_total"] <= 1.10
                    )
                    print("READINESS_PASS", iid, fp, flush=True)
                    sys.exit(0)
                print("PINNED_SSH_RC", result.returncode, flush=True)
        time.sleep(20)
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
