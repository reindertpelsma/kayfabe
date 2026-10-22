#!/usr/bin/env python3
"""V4 controller-side CI protocol driver. Never runs on the rented box.

Default: read-only real provider preflight. --native-mock drives local workerd
with the accompanying native-smoke.mjs adapter. Owner-authorized real creation
uses smoke-live.py with the supported SSH/onstart contract.
All endpoint/capability/instance values stay in memory or owner-only files.
"""
import argparse, hashlib, json, math, os, re, secrets, socket, subprocess, sys, tempfile, time, urllib.error, urllib.parse, urllib.request
from pathlib import Path


class Stop(Exception):
    pass


def request(url, method="GET", token=None, body=None, gh=None):
    headers = {}
    if token:
        headers["Authorization"] = "Bearer " + token
    if gh:
        headers["X-GH-Token"] = gh
    data = None
    if body is not None:
        data = json.dumps(body, separators=(",", ":")).encode()
        headers["Content-Type"] = "application/json"
    try:
        try:
            response = urllib.request.urlopen(
                urllib.request.Request(url, method=method, headers=headers, data=data), timeout=25
            )
        except urllib.error.HTTPError as error:
            response = error
        raw = response.read(262145)
        if len(raw) > 262144:
            raise Stop("RESPONSE_LIMIT")
        return response.status, json.loads(raw)
    except (urllib.error.URLError, ValueError):
        raise Stop("TRANSPORT_OR_JSON") from None


def configured_key():
    from vastai._base import _resolve_api_key, _APIKEY_SENTINEL

    return _resolve_api_key(_APIKEY_SENTINEL)


def provider(path, method="GET", body=None):
    code, value = request("https://console.vast.ai" + path, method, configured_key(), body)
    if code != 200:
        raise Stop("PROVIDER_UNAVAILABLE")
    return value


def preflight():
    q = {
        "limit": 20,
        "type": "ondemand",
        "verified": {"eq": True},
        "vms_enabled": {"eq": True},
        "rentable": {"eq": True},
        "rented": {"eq": False},
        "num_gpus": {"eq": 1},
        "gpu_name": {"in": ["RTX 4070", "RTX 4070 Super", "RTX 4080", "RTX 4090"]},
        "reliability": {"gte": 0.99},
        "disk_space": {"gte": 120},
        "dph_total": {"lte": 0.5},
        "allocated_storage": 120,
        "order": [["dph_total", "asc"]],
    }
    data = provider("/api/v0/bundles/", "POST", q)
    offers = data.get("offers")
    if not isinstance(offers, list) or len(offers) > 20:
        raise Stop("OFFER_SHAPE")
    # Use exactly the Worker's defensive post-filter, not merely the server query.
    eligible = [
        o
        for o in offers
        if isinstance(o.get("id"), int)
        and o["id"] > 0
        and (
            o.get("verified") is True or "verified" not in o and o.get("verification") == "verified"
        )
        and o.get("vms_enabled") is True
        and o.get("rentable") is True
        and o.get("rented") is False
        and o.get("num_gpus") == 1
        and str(o.get("gpu_name", "")).replace("_", " ").upper()
        in ["RTX 4070", "RTX 4070 SUPER", "RTX 4080", "RTX 4090"]
        and isinstance(o.get("reliability"), (int, float))
        and o["reliability"] >= 0.99
        and isinstance(o.get("disk_space"), (int, float))
        and o["disk_space"] >= 120
        and isinstance(o.get("dph_total"), (int, float))
        and not isinstance(o["dph_total"], bool)
        and math.isfinite(o["dph_total"])
        and 0 <= o["dph_total"] <= 0.5
    ]
    credit = provider("/api/v0/users/current/").get("credit")
    price = min((o["dph_total"] for o in eligible), default=None)
    return {
        "mode": "REAL_READ_ONLY",
        "offers_returned": len(offers),
        "eligible_offers": len(eligible),
        "hourly_cap_usd": 0.5,
        "cheapest_eligible_usd_hour": price,
        "prepaid_credit_field_present": isinstance(credit, (int, float))
        and not isinstance(credit, bool)
        and math.isfinite(credit),
        "reserve_plus_one_21min_run_met": isinstance(credit, (int, float))
        and not isinstance(credit, bool)
        and math.isfinite(credit)
        and credit >= 8 + (price or 0.5) * 21 / 60,
        "rentals_created": 0,
        "estimated_new_spend_usd": 0,
        "state": "UNAVAILABLE" if not eligible else "ELIGIBLE_OFFER_FOUND",
    }


class Client:
    def __init__(self, bindings, base, identity, evidence):
        self.env = bindings
        self.base = base.rstrip("/")
        self.identity = identity.rstrip("/")
        self.rows = []
        self.evidence = Path(evidence)
        self.prefix = (
            ("/" + bindings["BROKER_PATH_PREFIX"]) if bindings.get("BROKER_PATH_PREFIX") else ""
        )

    def call(self, path, method="GET", token=None, body=None, oidc=True):
        if token is None and oidc:
            code, data = request(self.identity + "/token")
            assert code == 200
            token = data["token"]
        before = time.monotonic()
        code, data = request(
            self.base + self.prefix + path,
            method,
            token,
            body,
            "local-read-only-job-token" if oidc else None,
        )
        self.rows.append(
            {
                "route": path.split("?")[0].split("/")[1],
                "method": method,
                "status": code,
                "response_bytes": len(json.dumps(data).encode()),
                "wall_ms": round((time.monotonic() - before) * 1000, 3),
            }
        )
        return code, data

    def rent(self, tested_sha, pr=7):
        self.runner_nonce = secrets.token_hex(32)
        code, data = self.call(
            "/rent",
            "POST",
            body={"runner_nonce": self.runner_nonce, "tested_sha": tested_sha, "pr_number": pr},
        )
        if code == 200 and (
            data.get("runner_nonce") != self.runner_nonce or data.get("sha") != tested_sha
        ):
            raise Stop("RENT_BINDING")
        return code, data

    def control(self, body):
        code, data = request(
            self.base + "/__v4/control", "POST", self.env["OWNER_STOP_TOKEN"], body
        )
        if code != 200:
            raise Stop("MOCK_CONTROL_FAILED")
        return data

    def scheduled(self):
        start = time.monotonic()
        with urllib.request.urlopen(self.base + "/cdn-cgi/local/scheduled", timeout=25) as response:
            size = len(response.read(64))
            assert response.status == 200
        self.rows.append(
            {
                "route": "scheduled",
                "method": "GET",
                "status": 200,
                "response_bytes": size,
                "wall_ms": round((time.monotonic() - start) * 1000, 3),
            }
        )

    def drain(self, run, cursor):
        result = ""
        for _ in range(10000):
            code, page = self.call("/logs/" + run + "?cursor=" + str(cursor))
            assert code == 200
            result += page["log_increment_sanitised"]
            next_cursor = page["next_cursor"]
            if next_cursor == cursor:
                return next_cursor, result
            assert next_cursor > cursor
            cursor = next_cursor
        raise Stop("CURSOR_LOOP")

    def native_flow(self):
        results = []
        run_base = int(time.time() * 1000)
        for index, (scenario, verdict) in enumerate(
            [("pass-kill", "PASS"), ("pass-cron", "PASS"), ("fail", "FAIL")]
        ):
            run = str(run_base + index) + "-1"
            tested_sha = hashlib.sha256(("synthetic-" + run).encode()).hexdigest()[:40]
            request(
                self.identity + "/select",
                "POST",
                body={"run": run.split("-")[0], "sha": tested_sha},
            )
            self.control({"action": "reset", "offset": 0})
            code, data = self.rent(tested_sha)
            assert code == 200 and data["state"] == "RUNNING"
            cap = self.control({"action": "capability", "run": run})
            if index == 0:
                target = urllib.parse.urlsplit(self.base)
                assert target.hostname in {"127.0.0.1", "localhost"}
                connection = socket.create_connection(
                    (target.hostname, target.port or 80), timeout=3
                )
                upload_path = self.prefix + "/report/" + run
                wire = (
                    "POST "
                    + upload_path
                    + " HTTP/1.1\r\nHost: "
                    + target.netloc
                    + "\r\nAuthorization: Bearer "
                    + cap["token"]
                    + "\r\nContent-Type: application/json\r\nContent-Length: 1000\r\n\r\n{"
                ).encode()
                try:
                    connection.sendall(wire)
                    connection.settimeout(1)
                    try:
                        connection.recv(1, socket.MSG_PEEK)
                        raise Stop("EARLY_UPLOAD_RESPONSE")
                    except socket.timeout:
                        pass
                    self.scheduled()
                    assert self.control({"action": "capability", "run": run})["rate_count"] == 0
                finally:
                    connection.close()
            code, _ = self.call(
                "/report/" + run,
                "POST",
                cap["token"],
                {"seq": 1, "kind": "progress", "text": "before running"},
                oidc=False,
            )
            assert code == 409
            self.control({"action": "running"})
            code, _ = self.call(
                "/report/" + run,
                "POST",
                "bad",
                {"seq": 1, "kind": "result", "result": {"nonce": cap["nonce"], "verdict": "PASS"}},
                oidc=False,
            )
            assert code == 401
            planted = (
                "Bearer "
                + ("FAKE" * 12)
                + "\n"
                + ".".join(["eyJhbGciOiJSUzI1NiJ9", "eyJzdWIiOiJwbGFudGVkIn0", "J" * 80])
                + "\n\x1b[31mcolor\x1b[0m\n::error::x\n##[group]x\n"
                + "Z" * 100000
            )
            chunks = [planted[i : i + 8192] for i in range(0, len(planted), 8192)]
            for seq, text in enumerate(chunks, 1):
                code, _ = self.call(
                    "/report/" + run,
                    "POST",
                    cap["token"],
                    {"seq": seq, "kind": "log", "text": text},
                    oidc=False,
                )
                assert code == 200
            cursor, live_log = self.drain(run, 0)
            code, empty = self.call("/logs/" + run + "?cursor=" + str(cursor))
            assert code == 200 and not empty["log_increment_sanitised"]
            assert (
                "::" not in live_log
                and "##[" not in live_log
                and "\x1b" not in live_log
                and ("FAKE" * 12) not in live_log
                and "Z" * 100000 not in live_log
                and "J" * 80 not in live_log
            )
            code, _ = self.call(
                "/report/" + run,
                "POST",
                cap["token"],
                {
                    "seq": len(chunks) + 1,
                    "kind": "result",
                    "result": {
                        "nonce": cap["nonce"],
                        "verdict": verdict,
                        "runner_nonce": self.runner_nonce,
                        "sha": tested_sha,
                    },
                },
                oidc=False,
            )
            assert code == 200
            code, data = self.call("/status/" + run)
            assert (
                code == 200
                and data["state"] == "COMPLETE"
                and data["verdict"] == verdict
                and data["cleanup_pending"]
            )
            assert self.control({"action": "capability", "run": run})["present"]
            if scenario == "pass-kill":
                self.control({"action": "advance", "offset": 60000})
                code, _ = self.call("/kill/" + run, "POST", cap["token"], {}, oidc=False)
                assert code == 200
                assert not self.control({"action": "capability", "run": run})["present"]
            else:
                self.control({"action": "advance", "offset": 299000})
                self.scheduled()
                assert self.control({"action": "capability", "run": run})["present"]
                self.control({"action": "advance", "offset": 301000})
                self.scheduled()
                assert not self.control({"action": "capability", "run": run})["present"]
            settled = self.control({"action": "capability", "run": run})
            assert settled["settled"]
            results.append(
                {
                    "scenario": scenario,
                    "verdict": verdict,
                    "grace_kept_before_cleanup": True,
                    "mock_provider_absent": True,
                    "ledger_settled": True,
                    "clock": "SYNTHETIC",
                    "live_box": False,
                    "slow_upload_check": "PASS" if index == 0 else "NOT_RUN",
                }
            )
            if index == 0:
                self.evidence.mkdir(parents=True, exist_ok=True)
                (self.evidence / "render-input.json").write_text(json.dumps(data, indent=2) + "\n")
                (self.evidence / "render-log.txt").write_text(live_log)
        code, _ = self.call("/stop-all", "POST", self.env["OWNER_STOP_TOKEN"], oidc=False)
        assert code == 200
        return {
            "mode": "NATIVE_WORKER_MOCK_PROVIDER",
            "results": results,
            "routes": self.rows,
            "rentals_created_real": 0,
            "spend_usd": 0,
        }


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--native-mock", action="store_true")
    parser.add_argument("--vars", type=Path)
    parser.add_argument("--base", default="http://127.0.0.1:18787")
    parser.add_argument("--identity", default="http://127.0.0.1:18788")
    parser.add_argument("--evidence", type=Path, required=True)
    args = parser.parse_args()
    args.evidence.mkdir(parents=True, exist_ok=True)
    try:
        if args.native_mock:
            env = dict(
                line.split("=", 1) for line in args.vars.read_text().splitlines() if "=" in line
            )
            result = Client(env, args.base, args.identity, args.evidence).native_flow()
            # Public render input excludes every provider ID/run and endpoint.
            p = args.evidence / "render-input.json"
            data = json.loads(p.read_text())
            p.write_text(
                json.dumps(
                    {k: v for k, v in data.items() if k not in {"id", "run", "pr"}}, indent=2
                )
                + "\n"
            )
        else:
            result = preflight()
        (
            args.evidence / "native.json" if args.native_mock else args.evidence / "preflight.json"
        ).write_text(json.dumps(result, indent=2) + "\n")
        print(
            json.dumps(
                {k: v for k, v in result.items() if k not in {"routes", "results"}}, sort_keys=True
            )
        )
    except Exception as error:
        # Exceptions can contain request URLs or provider fields. Never print them.
        print(json.dumps({"state": "UNAVAILABLE", "error_type": type(error).__name__}))
        return 1
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
