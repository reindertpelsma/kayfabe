import json, subprocess, sys, datetime, os, time, re, base64, hashlib, ipaddress
from concurrent.futures import ThreadPoolExecutor
from pathlib import Path

D = Path("/workspace/bw7-wt/traces/blackwell_native_ref_20261024/BW7/kit")
W = Path("/workspace/bw7-wt")
L = W / "traces/blackwell_native_ref_20261024/BW7/attempt1/owned_instances.json"


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
    rehearsal = json.loads((D / "rehearsal_pass.json").read_text())
    assert rehearsal["tests"] == 3 and rehearsal["passed"] == 3
    assert (
        hashlib.sha256(Path(__file__).read_bytes()).hexdigest()
        == rehearsal["operator_source_sha256"]
    )
    assert (
        hashlib.sha256((D / "guards/console_pin.py").read_bytes()).hexdigest()
        == rehearsal["guard_source_sha256"]
    )
    L.parent.mkdir(parents=True, exist_ok=True)
    temporary = L.with_suffix(".tmp")
    temporary.write_text(json.dumps(rows, indent=2) + "\n")
    temporary.replace(L)


sys.path.insert(0, str(D / "guards"))
import provider_retry as P

sys.path.insert(0, str(D / "guards"))
import console_pin as C
import pin_evidence as PE  # B1g: export console host-key lines
import round_budget as B

sys.path.insert(0, str(D))
import lease_profile as LP
import grant_b1g as GRANT
import dry_loop as DL  # B1f named per-lease profile "7-visible-core deviation"

floor = LP.offer_floor(None)  # default profile


def arm_watch(row, seconds, kind, rows):
    assert 0 < seconds <= LP.timer_ceiling_seconds(None)
    iid = int(row["id"])
    key = "auto_destroy" if kind == "auto" else "budget_watch"
    log = open(D / (kind + "_watch_" + str(iid) + ".log"), "x")
    child = subprocess.Popen(
        [sys.executable, str(D / "budget_watch.py"), str(iid), str(seconds)],
        stdout=log,
        stderr=subprocess.STDOUT,
        stdin=subprocess.DEVNULL,
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
print("UNVERIFIED-HOST", mode, flush=True)
if mode in ("preflight", "rent"):
    authority = json.loads((D / "owner_authority.json").read_text())
    assert authority["leases_authorized"] == 1 and authority["record_limit_per_arm"] == 8
    import saved_xid32_observation as SX

    assert authority["die_pairs"] == {k: list(v) for k, v in SX.DIE_PAIRS.items()}
    assert authority["experiment_mode"] == "OBSERVATION" and not authority["attestation"]
    assert authority["no_kayfabe"] and authority["skus"] == {"RTX_5090": "GB202"}
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
    sys.path.insert(0, str(D / "guards"))
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
        57366202,
        57388945,
        57425378,
        57429076,
        57431305,
        57443671,
        57448629,
        57453849,
        57457694,
        57465679,
        57472433,
        57485318,
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
    }
    cache = D / "single_market_BW7_1.json"
    if cache.exists():
        market = json.loads(cache.read_text())
        offers = market["offers"]
    else:

        def search(item):
            sku, die = item
            found = api(
                [
                    "search",
                    "offers",
                    f"vms_enabled=true num_gpus=1 gpu_name={sku} disk_space>=180 "
                    f"cpu_cores_effective>={floor} cpu_ram>=32 inet_down>=200 reliability>=0.95 verified=any",
                    "-o",
                    "dph_total",
                    "--limit",
                    "100",
                ]
            )
            for offer in found:
                offer["candidate_die"] = die
            return found

        offers = []
        with ThreadPoolExecutor(max_workers=7) as pool:
            for found in pool.map(search, skus.items()):
                offers.extend(found)
        market = {
            "utc": now().isoformat(),
            "host_verification": "UNVERIFIED-HOST",
            "offers": offers,
        }
        cache.write_text(json.dumps(market) + "\n")
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
                "reliability",
                "inet_down",
                "vms_enabled",
                "cpu_cores_effective",
                "cpu_ram",
                "disk_space",
                "is_vm_deverified",
                "geolocation",
            )
        }
        for x in offers
    ]
    (L.parent / "market.json").write_text(
        json.dumps(
            {
                "host_verification": "UNVERIFIED-HOST",
                "utc": market["utc"],
                "credit_usd": credit,
                "offers": safe,
                "authority": "controller17:00 one-lease OBSERVATION, no denied countries or bad machines",
            },
            indent=2,
        )
        + "\n"
    )
    offers = [
        x
        for x in offers
        if x.get("verification") in ("verified", "unverified")  # BW4: not deverified
        and float(x.get("reliability", 0)) >= 0.95
        and float(x.get("inet_down", 0)) >= 200
        and bool(x.get("vms_enabled"))
        and not x.get("is_vm_deverified", False)
        and float(x.get("dph_total", 99)) <= 1.00
        and float(x.get("cpu_ram", 0)) >= 32768
        and LP.offer_ok(x, None)
    ]
    offers = [
        x
        for x in offers
        if re.search(r",\s*([A-Z]{2})\s*$", x.get("geolocation", ""))
        and re.search(r",\s*([A-Z]{2})\s*$", x["geolocation"])[1] not in {"CU", "IR", "KP", "SY", "CN", "HK", "MO"}
    ]
    eligible = []
    for candidate in offers:
        try:
            proposed = LP.plan(
                float(candidate["dph_total"]), credit, now(), stop_usd=2.5, cap_usd=3.0, max_seconds=None
            )
        except ValueError:
            continue
        if proposed["seconds"] >= 90 * 60:
            eligible.append(candidate)
    if not eligible:
        print("OBSERVATION NO_QUALIFYING_OFFER_ONE_MARKET_CHECK", flush=True)
        sys.exit(75)
    preferred = [x for x in eligible if int(x["machine_id"]) == 152098]
    eligible = preferred or eligible
    if not eligible:
        print("OBSERVATION NO_QUALIFYING_OFFER_ONE_MARKET_CHECK", flush=True)
        sys.exit(75)
    offer = min(
        eligible,
        key=lambda x: (
            ["GB206", "GB205", "GB203", "GB202", "GB207"].index(x["candidate_die"]),
            x.get("verification") != "verified",
            float(x["dph_total"]),
        ),
    )
    rate = float(offer["dph_total"])
    if mode == "preflight":
        print(
            json.dumps(
                {
                    "selected_offer": offer["id"],
                    "machine": offer["machine_id"],
                    "geolocation": offer["geolocation"],
                    "die": offer["candidate_die"],
                    "plan": LP.plan(rate, credit, now(), stop_usd=2.5, cap_usd=3.0, max_seconds=None),
                    "creates": 0,
                }
            ),
            flush=True,
        )
        sys.exit(0)
    # fix 03:30: every guard/watcher loop ran once on synthetic input, sources unchanged
    DL.require_receipt(D, roots=DL.default_roots(D, L.parent / "run_kit"))
    state = json.loads((D / "stretch.json").read_text())
    assert state["attempts_used"] == 0, "one lease only; no provider retry this round"
    state["attempts_used"] = 1
    (D / "stretch.json").write_text(json.dumps(state) + "\n")
    plan = LP.plan(rate, credit, now(), stop_usd=2.5, cap_usd=3.0, max_seconds=None)
    (D / "round_plan.json").write_text(json.dumps(plan, indent=2) + "\n")
    assert L.read_text().strip() == "[]"
    ready_path = D / "precreate_watch_ready.json"
    assert not ready_path.exists()
    watch_log = (D / "precreate_watch.log").open("x")
    prewatch = subprocess.Popen(
        [
            sys.executable,
            str(D / "guards/precreate_watchdog.py"),
            "--plan",
            str(D / "round_plan.json"),
            "--ledger",
            str(L),
            "--ready",
            str(ready_path),
            "--result",
            str(D / "precreate_watch_result.json"),
        ],
        stdin=subprocess.DEVNULL,
        stdout=watch_log,
        stderr=subprocess.STDOUT,
        start_new_session=True,
    )
    for _ in range(100):
        assert prewatch.poll() is None
        if ready_path.exists():
            break
        time.sleep(0.1)
    assert ready_path.exists(), "precreate watchdog did not arm"
    armed = json.loads(ready_path.read_text())
    assert armed["state"] == "ARMED" and armed["pid"] == prewatch.pid
    assert armed["deadline_utc"] == plan["deadline_utc"]
    assert os.getpgid(prewatch.pid) == prewatch.pid
    assert (
        int(Path("/proc", str(prewatch.pid), "stat").read_text().rpartition(")")[2].split()[19])
        == armed["starttime"]
    )
    assert B.remaining(plan, now()) > 25 * 60
    spent = runtime_spent = 0.0
    minutes = plan["seconds"] / 60
    rate_guard = plan["rate_bound"]
    reservation = plan["reservation_usd"]
    retry_index = 1
    one_time_exception = False
    source = subprocess.check_output(["git", "rev-parse", "6e3fd3fd6"], cwd=W, text=True).strip()
    result = api(
        [
            "create",
            "instance",
            str(offer["id"]),
            "--template_hash",
            "183b10af7fd788d08aca02e31811d70d",
            "--disk",
            "150",
            "--cancel-unavail",
            "--ssh",
            "--direct",
            "--label",
            "codex-bw7-REF1-NATIVE-20261024",
        ]
    )
    if not result.get("success"):
        if not result.get("new_contract"):
            prewatch.terminate()
            prewatch.wait(timeout=5)
        message = str(result.get("msg", result.get("error", ""))).lower()
        categories = [
            word
            for word in (
                "not found",
                "not available",
                "unavailable",
                "already rented",
                "no such",
                "verification",
                "credit",
                "price",
            )
            if word in message
        ]
        (D / "create_refusal_1305.json").write_text(
            json.dumps(
                {
                    "utc": now().isoformat(),
                    "offer_id": int(offer["id"]),
                    "machine_id": int(offer["machine_id"]),
                    "success": False,
                    "reason_categories": categories,
                    "new_contract_returned": bool(result.get("new_contract")),
                    "cancel_unavail": True,
                    "single_recheck_reused": True,
                }
            )
            + "\n"
        )
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
        "budget_stop": 2.5,
        "phase_cap": 3.0,
        "credit_reserve": 8,
        "auto_destroy_minutes": minutes,
        "round_deadline_utc": plan["deadline_utc"],
        "precreate_armed_utc": plan["armed_utc"],
        "experiment_mode": "OBSERVATION",
        "attestation": False,
        "budget_cutoff_minutes": minutes,
        "guard_rate": rate_guard,
        "harness_source": source,
        "no_kayfabe": True,
        "measured_candidate": "stock native Windows 580.88",
        "destroyed": False,
        "lease_profile": "default",
        "authority": "owner BW7 native GB202 reference; one lease stop2.50 cap3.00 reserve8",
    }
    rows.append(row)
    save(rows)
    try:
        assert prewatch.poll() is None
        row.update(
            auto_destroy_requested_minutes=minutes,
            auto_destroy_pid=prewatch.pid,
            auto_destroy_pid_starttime=armed["starttime"],
            auto_destroy_verified=True,
            auto_destroy_armed_utc=plan["armed_utc"],
            auto_destroy_method="precreate_fixed_deadline_watchdog",
        )
        save(rows)
        budget_pid = arm_watch(row, B.remaining(plan, now()), "budget", rows)
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
                actual = int(
                    Path("/proc", str(pid), "stat").read_text().rpartition(")")[2].split()[19]
                )
                if actual == row.get(key + "_starttime") and os.getpgid(pid) == pid:
                    os.killpg(pid, 15)
            except (ProcessLookupError, FileNotFoundError):
                pass
        row["runtime_cost_estimate_usd"] = (
            (now() - datetime.datetime.fromisoformat(row["created_utc"])).total_seconds()
            / 3600
            * row["guard_rate"]
        )
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
        safe = {
            key: status.get(key)
            for key in (
                "id",
                "actual_status",
                "intended_status",
                "status_msg",
                "cur_state",
                "dph_total",
            )
        }
        receipt = {
            "host_verification": "UNVERIFIED-HOST",
            "utc": now().isoformat(),
            "reason": reason,
            "machine_id": row["machine_id"],
            "status": safe,
            "elapsed_seconds": elapsed,
            "adaptive_boot_seconds": 1200,
            "first_pinned_banner_seconds": banner_seconds,
            "console_stage_started_elapsed": pin_stage_started,
            "console_factor": pin_factor,
            "console_deadline_since_create": pin_deadline,
        }
        (D / ("provider_failure_" + str(iid) + ".json")).write_text(
            json.dumps(receipt, indent=2) + "\n"
        )
        print("PROVIDER_RETRYABLE", iid, reason, flush=True)
        sys.exit(76)

    def record_pin(receipt):
        with (D / ("console_pin_" + str(iid) + ".jsonl")).open("a") as out:
            out.write(
                json.dumps(
                    {
                        "host_verification": "UNVERIFIED-HOST",
                        "lease_id": iid,
                        "utc": now().isoformat(),
                        **receipt,
                    }
                )
                + "\n"
            )
        print("CONSOLE_PIN", receipt["state"], receipt["polls"], flush=True)
        evidence = L.parent / "lease_metadata" / ("console_pin_" + str(iid) + ".jsonl")
        evidence.parent.mkdir(parents=True, exist_ok=True)
        evidence.write_text((D / ("console_pin_" + str(iid) + ".jsonl")).read_text())

    def console_text(timeout):
        current = next(x for x in api(["show", "instances"]) if int(x["id"]) == iid)
        reason = P.failure(current, lease_age(), banner_seconds=None)
        if reason:
            fail_readiness(reason, current, lease_age())
        text = subprocess.run(
            ["vastai", "logs", str(iid), "--tail", "5000"],
            capture_output=True,
            text=True,
            timeout=timeout,
        ).stdout
        PE.capture(D, iid, text)
        return text

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
            scan_start = time.monotonic()
            scan = subprocess.run(
                ["ssh-keyscan", "-T", "5", "-t", "ed25519", "-p", str(port), ip],
                capture_output=True,
                text=True,
                timeout=min(10, 1200 - elapsed),
            ).stdout
            scan_seconds = max(0.001, time.monotonic() - scan_start)
            candidates = [l.split() for l in scan.splitlines() if l and not l.startswith("#")]
            def rescan():
                out = subprocess.run(
                    ["ssh-keyscan", "-T", "5", "-t", "ed25519", "-p", str(port), ip],
                    capture_output=True,
                    text=True,
                    timeout=10,
                ).stdout
                rows = [l.split() for l in out.splitlines() if l and not l.startswith("#")]
                keys = [f[2] for f in rows if len(f) >= 3 and f[1] == "ssh-ed25519"]
                return keys[0] if len(keys) == 1 else None

            for fields in candidates:
                if len(fields) < 3:
                    continue
                if pin_stage_started is None:
                    pin_stage_started = lease_age()
                    pin_factor = P.policy(scan_seconds)["factor"]
                    pin_deadline = min(1200, pin_stage_started + C.stage_seconds(pin_factor))
                try:
                    proof = C.settle_pin(
                        rescan,
                        console_text,
                        elapsed_since_create=lease_age(),
                        factor=pin_factor,
                        stage_started_elapsed=pin_stage_started,
                        clock=time.monotonic,
                        sleep=time.sleep,
                        record=record_pin,
                    )
                except C.PinExpired:
                    fail_readiness("provider_boot_timeout", x, lease_age())
                # A different published pin propagates immediately as a safety refusal.
                fp = proof["fingerprint"]
                (D / "known_hosts").write_text(alias + " ssh-ed25519 " + proof["key"] + "\n")
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
                        capture_output=True,
                        text=True,
                        timeout=min(100, remaining),
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
                        ssh_ready_utc=now().isoformat(),
                        ssh_hostkey_fingerprint=fp,
                        observed_dph_total=float(x["dph_total"]),
                        console_pin_proof=proof,
                        console_pin_factor=pin_factor,
                    )
                    rows = json.loads(L.read_text())
                    rows[-1] = row
                    save(rows)
                    assert (
                        row["prior_lane_runtime_usd"]
                        + row["observed_dph_total"] * row["budget_cutoff_minutes"] / 60
                        + 0.50
                        < 2.5
                        and row["observed_dph_total"] <= 1.00
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
    from ref_windows import verify_capture_before_destroy

    if row_needs_capture := r.get("ssh_ready_utc"):
        verify_capture_before_destroy(D / "capture_verified.json")
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
