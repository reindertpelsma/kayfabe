#!/usr/bin/env python3
"""GPU-free falsifiers for the simple Vast runner (scripts/ci/vast_simple, stdlib only).

A fake Vast API (http.server on loopback) and a fake SSH transport stand in for the provider and
the box. Time is virtual: the injected sleep advances the injected clock. Mutation testing:
scripts/ci/vast_simple_mutants.py (KF_VAST_SIMPLE_DIR points the tests at a mutated copy).
"""

import http.server
import io
import json
import os
import re
import subprocess
import sys
import tempfile
import threading
import unittest
import urllib.parse
from pathlib import Path

HERE = Path(__file__).resolve().parent
SRC = Path(os.environ.get("KF_VAST_SIMPLE_DIR", HERE / "vast_simple"))
sys.dont_write_bytecode = True
sys.path.insert(0, str(SRC))
DEFAULTS = str(HERE.parents[1] / "ci/vast-broker/defaults.json")

import offers as simple_offers  # noqa: E402
import reaper as simple_reaper  # noqa: E402
import run as simple_run  # noqa: E402
import vast_api  # noqa: E402

SHA = "a" * 40
KEY = "VASTKEY0123456789abcdefSECRETvalue"
NOW = 1_800_000_000


class Clock:
    def __init__(self):
        self.t = float(NOW)

    def time(self):
        return self.t

    def sleep(self, seconds):
        self.t += seconds


def good_offer(oid, machine, price=0.2, **extra):
    storage_cost = 0.1
    offer = {
        "id": oid,
        "machine_id": machine,
        "host_id": 5000 + machine,
        "gpu_arch": "nvidia",
        "compute_cap": 890,
        "gpu_name": "RTX 4070",
        "verified": True,
        "vms_enabled": True,
        "rentable": True,
        "rented": False,
        "num_gpus": 1,
        "disk_space": 500,
        "reliability": 0.99,
        "cpu_cores_effective": 16,
        "cpu_ghz": 3.0,
        "cpu_ram": 64000,
        "dlperf": 30.0,
        "inet_down": 500,
        "inet_up": 200,
        "disk_bw": 500,
        "geolocation": "Texas, US",
        "dph_base": price,
        "storage_cost": storage_cost,
        "dph_total": price + storage_cost * 120 / 720,
        "inet_down_cost": 0.001,
        "inet_up_cost": 0.001,
    }
    offer.update(extra)
    return offer


class FakeVast:
    """Minimal Vast console: bundles, create (PUT asks), inspect, delete, instances-v1."""

    def __init__(self):
        self.offers = [good_offer(11, 1, 0.2), good_offer(12, 2, 0.3)]
        self.instances = {}
        self.requests = []
        self.next_id = 1000
        self.loading_polls = 1
        self.never_running = set()
        self.list_lag = 0
        self.never_absent = False
        self.create_error_with_instance = set()
        self.create_400_body = {}
        self.label_swap = {}
        self.inspect_swap = {}
        self.list_override = None
        self.pages = None
        self.loop_token = False
        self.lock = threading.Lock()
        outer = self

        class Handler(http.server.BaseHTTPRequestHandler):
            def log_message(self, *args):
                pass

            def handle_any(self):
                length = int(self.headers.get("Content-Length") or 0)
                raw = self.rfile.read(length) if length else b""
                body = json.loads(raw) if raw else None
                with outer.lock:
                    status, payload = outer.route(
                        self.command, self.path, body, self.headers.get("Authorization")
                    )
                data = json.dumps(payload).encode()
                self.send_response(status)
                self.send_header("Content-Type", "application/json")
                self.send_header("Content-Length", str(len(data)))
                self.end_headers()
                self.wfile.write(data)

            do_GET = do_PUT = do_POST = do_DELETE = handle_any

        self.httpd = http.server.ThreadingHTTPServer(("127.0.0.1", 0), Handler)
        self.url = "http://127.0.0.1:%d" % self.httpd.server_address[1]
        self.thread = threading.Thread(
            target=self.httpd.serve_forever, kwargs={"poll_interval": 0.01}, daemon=True
        )
        self.thread.start()

    def close(self):
        self.httpd.shutdown()
        self.httpd.server_close()

    def add_instance(self, label, iid=None):
        if iid is None:
            iid = self.next_id
            self.next_id += 1
        self.instances[iid] = {"id": iid, "label": label, "polls": 0, "deleted": False}
        return iid

    def view(self, inst):
        iid = inst["id"]
        running = inst["polls"] > self.loading_polls and iid not in self.never_running
        view = {"id": iid, "label": self.label_swap.get(iid, inst["label"])}
        view["actual_status"] = "running" if running else "loading"
        view["public_ipaddr"] = "203.0.113.%d" % (iid % 250)
        view["ports"] = {"22/tcp": [{"HostIp": "0.0.0.0", "HostPort": str(40000 + iid)}]}
        view["start_date"] = NOW
        return view

    def deleted_calls(self):
        return [int(p.rsplit("/", 2)[-2]) for m, p, _ in self.requests if m == "DELETE"]

    def route(self, method, raw_path, body, auth):
        parsed = urllib.parse.urlparse(raw_path)
        path, query = parsed.path, urllib.parse.parse_qs(parsed.query)
        self.requests.append((method, raw_path, body))
        if auth != "Bearer " + KEY:
            return 401, {"success": False, "msg": "bad key"}
        parts = [p for p in path.split("/") if p]
        if method == "POST" and parts == ["api", "v0", "bundles"]:
            return 200, {"offers": self.offers}
        if method == "PUT" and parts[:3] == ["api", "v0", "asks"]:
            oid = int(parts[3])
            if oid in self.create_400_body:
                return 400, self.create_400_body[oid]
            iid = self.add_instance(body["label"])
            self.instances[iid]["offer"] = oid
            if oid in self.create_error_with_instance:
                return 500, {"success": False, "error": "provider exploded"}
            return 200, {"success": True, "new_contract": iid}
        if method == "GET" and parts[:3] == ["api", "v0", "instances"] and len(parts) == 4:
            inst = self.instances.get(int(parts[3]))
            if inst is None or inst["deleted"]:
                return 200, {"instances": None}
            inst["polls"] += 1
            view = self.view(inst)
            view["label"] = self.inspect_swap.get(inst["id"], view["label"])
            return 200, {"instances": view}
        if method == "PUT" and parts[:4] == ["api", "v0", "instances", "request_logs"]:
            return 200, {"success": False}
        if method == "DELETE" and parts[:3] == ["api", "v0", "instances"]:
            inst = self.instances.get(int(parts[3]))
            if inst is None:
                return 404, {"success": False}
            inst["deleted"] = True
            inst["lag"] = self.list_lag
            return 200, {"success": True}
        if method == "GET" and parts == ["api", "v1", "instances"]:
            return self.list_v1(query)
        return 404, {"success": False, "msg": "no route"}

    def list_v1(self, query):
        if self.list_override is not None:
            return 200, self.list_override
        label = None
        if "select_filters" in query:
            label = json.loads(query["select_filters"][0])["label"]["eq"]
        rows = []
        for inst in self.instances.values():
            if inst["deleted"] and not self.never_absent:
                if inst["lag"] <= 0:
                    continue
                inst["lag"] -= 1
            if label is not None and inst["label"] != label:
                continue
            rows.append(self.view(inst))
        if self.pages:
            token = (query.get("next_token") or ["0"])[0]
            at = int(token)
            chunk = rows[at : at + self.pages]
            more = self.loop_token or at + self.pages < len(rows)
            token = "0" if self.loop_token else str(at + self.pages)
            return 200, {"success": True, "instances": chunk, "next_token": token if more else None}
        return 200, {"success": True, "instances": rows, "next_token": None}


GOOD_RESULT = {
    "verdict": "PASS",
    "suites": [{"name": "fast", "pass": 30, "fail": 0, "skip": 0}],
    "hardware": {
        "gpu_name": "NVIDIA GeForce RTX 4070",
        "compute_cap": 890,
        "driver_version": "580",
    },
}


class FakeSsh:
    def __init__(self, answers=None, payload_rc=0, result=GOOD_RESULT, clone_rc=0, **options):
        self.answers = answers or (lambda host, port: True)
        self.payload_rc = payload_rc
        self.result = result
        self.clone_rc = clone_rc
        self.hang = options.get("hang", False)
        self.boom = options.get("boom")
        self.chatter = options.get("chatter", "payload running\n")
        self.calls = []

    def factory(self, host, port):
        return FakeConn(self, host, port)


class FakeConn:
    def __init__(self, ssh, host, port):
        self.ssh, self.host, self.port = ssh, host, port

    def exec(self, cmd, timeout, sink):
        ssh = self.ssh
        ssh.calls.append((self.host, self.port, cmd, timeout))
        if cmd.strip() == "true":
            return 0 if ssh.answers(self.host, self.port) else 255
        if "git clone" in cmd:
            sink("cloned\n")
            return ssh.clone_rc
        if "vast_payload.py" in cmd:
            if ssh.boom:
                raise ssh.boom
            sink(ssh.chatter)
            return 124 if ssh.hang else ssh.payload_rc
        if "kf-result.json" in cmd:
            if ssh.result is None:
                return 1
            sink(json.dumps(ssh.result) if not isinstance(ssh.result, str) else ssh.result)
            return 0
        return 0


class Base(unittest.TestCase):
    def setUp(self):
        self.vast = FakeVast()
        self.addCleanup(self.vast.close)
        self.tmp = tempfile.TemporaryDirectory()
        self.addCleanup(self.tmp.cleanup)
        self.out = Path(self.tmp.name) / "out"
        self.clock = Clock()
        self.stdout = io.StringIO()

    def deps(self, ssh=None):
        ssh = ssh or FakeSsh()
        self.ssh = ssh
        return simple_run.Deps(
            transport_factory=ssh.factory,
            clock=self.clock.time,
            sleep=self.clock.sleep,
            stdout=self.stdout,
        )

    def argv(self, *extra, sha=SHA, profile="check"):
        return [
            "--sha", sha, "--profile", profile, "--out", str(self.out),
            "--defaults", DEFAULTS, "--api-base", self.vast.url,
            *extra,
        ]  # fmt: skip

    def go(self, *extra, ssh=None, key=KEY, **kw):
        env = {"VAST_API_KEY": key} if key else {}
        return simple_run.main(self.argv(*extra, **kw), self.deps(ssh), env)

    def created(self):
        return [b for m, p, b in self.vast.requests if m == "PUT" and "/asks/" in p]

    def assert_all_destroyed(self):
        for iid, inst in self.vast.instances.items():
            if inst["label"].startswith("kf-ci-simple-"):
                self.assertTrue(inst["deleted"], "instance %d leaked" % iid)


class OfferRules(unittest.TestCase):
    def cfg(self, bad_machines=(), **kw):
        cfg = simple_offers.make_config(simple_offers.load_defaults(DEFAULTS), bad_machines)
        cfg.update(kw)
        return cfg

    def pick(self, offers, profile="check", **kw):
        return simple_offers.candidates(offers, self.cfg(**kw), profile)

    def test_good_offer_is_allowed_and_classified(self):
        got = self.pick([good_offer(1, 1)])
        self.assertEqual([(o["id"], o["die"]) for o in got], [(1, "AD104")])

    def test_rejections_follow_the_worker_rules(self):
        bad = {
            "unverified": good_offer(1, 1, verified=False),
            "not_vm": good_offer(2, 2, vms_enabled=False),
            "two_gpus": good_offer(3, 3, num_gpus=2),
            "small_disk": good_offer(4, 4, disk_space=100),
            "reliability": good_offer(5, 5, reliability=0.5),
            "slow_net": good_offer(6, 6, inet_down=10),
            "dear": good_offer(7, 7, price=1.5),
            "inet_cost": good_offer(8, 8, inet_down_cost=0.5),
            "cc_old": good_offer(9, 9, compute_cap=700),
            "die": good_offer(10, 10, gpu_name="RTX 4060 Ti", compute_cap=890),
            "nonnumber": good_offer(13, 13, cpu_ghz=True),
        }
        sibling = good_offer(99, 99)
        self.assertEqual([o["id"] for o in self.pick(list(bad.values()) + [sibling])], [99])

    def test_price_must_include_storage(self):
        offer = good_offer(1, 1)
        offer["dph_total"] = offer["dph_base"]
        self.assertEqual(self.pick([offer]), [])

    def test_bad_machine_file_and_defaults_are_skipped(self):
        offers = [good_offer(1, 42636), good_offer(2, 77), good_offer(3, 78)]
        got = self.pick(offers, bad_machines={77})
        self.assertEqual([o["id"] for o in got], [3])

    def test_denied_and_unknown_countries_are_rejected(self):
        for place in ["Shanghai, CN", "Hong Kong", "Beijing, China", "Tehran, IR", "", "nowhere"]:
            self.assertEqual(self.pick([good_offer(1, 1, geolocation=place)]), [], place)
        self.assertEqual(len(self.pick([good_offer(1, 1, geolocation="Berlin, DE")])), 1)

    def test_order_is_die_then_country_then_price(self):
        offers = [
            good_offer(1, 1, 0.1, geolocation="Berlin, DE"),
            good_offer(2, 2, 0.3, geolocation="Texas, US"),
            good_offer(3, 3, 0.2, geolocation="Texas, US"),
        ]
        self.assertEqual([o["id"] for o in self.pick(offers)], [3, 2, 1])

    def test_sweep_uses_the_sweep_dies(self):
        other = good_offer(1, 1, gpu_name="RTX 5090", compute_cap=1200)
        self.assertEqual(self.pick([other], "check"), [])
        self.assertEqual([o["die"] for o in self.pick([other], "sweep")], ["GB202"])


class HappyPath(Base):
    def test_pass_exits_zero_and_destroys(self):
        self.assertEqual(self.go(), 0, self.stdout.getvalue())
        self.assertEqual(len(self.created()), 1)
        self.assert_all_destroyed()
        summary = json.loads((self.out / "summary.json").read_text())
        self.assertEqual((summary["verdict"], summary["sha"]), ("PASS", SHA))

    def test_create_body_matches_the_worker(self):
        self.go()
        (body,) = self.created()
        self.assertTrue(body["vm"])
        self.assertEqual(body["image"], "docker.io/vastai/kvm:ubuntu_cli_22.04-2025-05-16")
        self.assertEqual((body["runtype"], body["disk"]), ("ssh_direct", 120))
        self.assertEqual(body["target_state"], "running")
        self.assertTrue(re.fullmatch(r"kf-ci-simple-\d{10}-[0-9a-f]{8}", body["label"]))
        self.assertNotIn("KF_", json.dumps(body))
        self.assertIn("PUT /api/v0/asks/11/", [m + " " + p for m, p, _ in self.vast.requests])

    def test_cheapest_offer_is_rented(self):
        self.go()
        self.assertEqual(self.vast.instances[1000]["offer"], 11)

    def test_clone_checks_out_the_exact_sha_then_runs_the_payload(self):
        self.go()
        cmds = [c for _, _, c, _ in self.ssh.calls]
        clone = next(c for c in cmds if "git clone" in c)
        self.assertIn("--filter=blob:none", clone)
        self.assertIn(SHA, clone)
        self.assertIn("rev-parse HEAD", clone)
        payload = next(c for c in cmds if "vast_payload.py" in c)
        self.assertRegex(payload, SHA + r" [0-9a-f]{32}")
        self.assertLess(cmds.index(clone), cmds.index(payload))

    def test_ssh_goes_to_the_direct_ip_and_port(self):
        self.go()
        hosts = {(h, p) for h, p, _, _ in self.ssh.calls}
        self.assertEqual(hosts, {("203.0.113.%d" % (1000 % 250), 41000)})

    def test_bad_machine_file_is_honoured(self):
        path = Path(self.tmp.name) / "bad.txt"
        path.write_text("# machines\n1\n")
        self.assertEqual(self.go("--bad-machines", str(path)), 0)
        self.assertEqual(self.vast.instances[1000]["offer"], 12)

    def test_sweep_profile_runs(self):
        self.vast.offers = [good_offer(21, 3, gpu_name="RTX 5090", compute_cap=1200)]
        self.assertEqual(self.go(profile="sweep"), 0, self.stdout.getvalue())

    def test_bad_sha_or_profile_calls_nothing(self):
        self.assertEqual(self.go(sha="abc"), 2)
        self.assertEqual(self.go(profile="everything"), 2)
        self.assertEqual(self.vast.requests, [])

    def test_denied_country_offer_is_never_rented(self):
        self.vast.offers = [good_offer(11, 1, geolocation="Shanghai, CN")]
        self.assertEqual(self.go(), 2)
        self.assertEqual(self.created(), [])

    def test_unverified_offer_is_never_rented(self):
        self.vast.offers = [good_offer(11, 1, 0.1, verified=False), good_offer(12, 2, 0.3)]
        self.assertEqual(self.go(), 0, self.stdout.getvalue())
        self.assertEqual(self.vast.instances[1000]["offer"], 12)

    def test_no_offer_means_unavailable_and_no_create(self):
        self.vast.offers = []
        self.assertEqual(self.go(), 2)
        self.assertEqual(self.created(), [])


class BootFailures(Base):
    def test_boot_never_answers_destroys_and_retries_the_next_machine(self):
        dead = 40000 + 1000
        ssh = FakeSsh(answers=lambda host, port: port != dead)
        self.assertEqual(self.go(ssh=ssh), 0, self.stdout.getvalue())
        self.assertEqual([self.vast.instances[i]["offer"] for i in (1000, 1001)], [11, 12])
        self.assertLess(self.vast.deleted_calls().index(1000), len(self.created()) + 1)
        self.assert_all_destroyed()
        first_delete = next(i for i, r in enumerate(self.vast.requests) if r[0] == "DELETE")
        creates = [i for i, r in enumerate(self.vast.requests) if "/asks/" in r[1]]
        second_create = creates[1]
        self.assertLess(first_delete, second_create)

    def test_stuck_instance_is_destroyed_before_the_retry_is_rented(self):
        self.vast.never_running = {1000}
        self.go()
        order = [r[0] + (":asks" if "/asks/" in r[1] else "") for r in self.vast.requests]
        creates = [i for i, o in enumerate(order) if o == "PUT:asks"]
        self.assertLess(order.index("DELETE"), creates[1])

    def test_boot_deadline_is_fifteen_minutes(self):
        ssh = FakeSsh(answers=lambda host, port: False)
        start = self.clock.t
        self.assertEqual(self.go(ssh=ssh), 2)
        self.assertGreaterEqual(self.clock.t - start, 2 * 15 * 60)
        self.assertLess(self.clock.t - start, 2 * 15 * 60 + 600)

    def test_both_boots_failing_is_unavailable_with_two_creates_only(self):
        self.vast.offers.append(good_offer(13, 3, 0.4))
        ssh = FakeSsh(answers=lambda host, port: False)
        self.assertEqual(self.go(ssh=ssh), 2)
        self.assertEqual(len(self.created()), 2)
        self.assert_all_destroyed()

    def test_provider_stuck_loading_is_a_boot_failure(self):
        self.vast.never_running = {1000}
        self.assertEqual(self.go(), 0, self.stdout.getvalue())
        self.assertEqual(len(self.created()), 2)
        self.assert_all_destroyed()

    def test_create_error_that_made_an_instance_is_reconciled_and_destroyed(self):
        self.vast.create_error_with_instance = {11}
        self.assertEqual(self.go(), 0, self.stdout.getvalue())
        self.assertTrue(self.vast.instances[1000]["deleted"])
        self.assert_all_destroyed()

    def test_create_refusal_moves_on_to_the_next_offer(self):
        self.vast.create_400_body = {11: {"success": False, "msg": "no longer available"}}
        self.assertEqual(self.go(), 0, self.stdout.getvalue())
        self.assertEqual(self.vast.instances[1000]["offer"], 12)


class Verdicts(Base):
    def test_payload_failure_is_nonzero_and_still_destroyed(self):
        failing = dict(GOOD_RESULT, verdict="FAIL")
        self.assertEqual(self.go(ssh=FakeSsh(payload_rc=1, result=failing)), 1)
        self.assert_all_destroyed()

    def test_nonzero_exit_overrides_a_claimed_pass(self):
        self.assertEqual(self.go(ssh=FakeSsh(payload_rc=3)), 1)

    def not_a_pass(self, *results):
        for result in results:
            self.vast.instances.clear()
            self.assertEqual(self.go(ssh=FakeSsh(result=result)), 1, result)
            self.assert_all_destroyed()

    def suites(self, **row):
        return dict(
            GOOD_RESULT, suites=[dict({"name": "x", "pass": 5, "fail": 0, "skip": 0}, **row)]
        )

    def test_skips_are_not_a_pass(self):
        self.not_a_pass(self.suites(skip=1))

    def test_suite_failures_are_not_a_pass(self):
        self.not_a_pass(self.suites(fail=1))

    def test_zero_passes_are_not_a_pass(self):
        self.not_a_pass(self.suites(**{"pass": 0}), dict(GOOD_RESULT, suites=[]))

    def test_unknown_hardware_is_not_a_pass(self):
        self.not_a_pass(dict(GOOD_RESULT, hardware={"gpu_name": "unknown", "compute_cap": 890}))

    def test_low_compute_cap_is_not_a_pass(self):
        self.not_a_pass(dict(GOOD_RESULT, hardware={"gpu_name": "x", "compute_cap": 700}))

    def test_verdict_text_must_be_exactly_pass(self):
        self.not_a_pass(dict(GOOD_RESULT, verdict="pass"), dict(GOOD_RESULT, verdict="PASS "))

    def test_missing_or_invalid_result_is_a_failure(self):
        for result in (None, "not json", "[]"):
            self.assertEqual(self.go(ssh=FakeSsh(result=result)), 1, result)
        self.assert_all_destroyed()

    def test_clone_failure_is_a_failure_and_destroyed(self):
        self.assertEqual(self.go(ssh=FakeSsh(clone_rc=128)), 1)
        self.assert_all_destroyed()

    def test_payload_timeout_is_bounded_by_the_hard_deadline(self):
        self.assertEqual(self.go(ssh=FakeSsh(hang=True, payload_rc=124)), 1)
        payload = next(c for _, _, c, _ in self.ssh.calls if "vast_payload.py" in c)
        seconds = int(re.search(r"--kill-after=30s (\d+)s", payload).group(1))
        self.assertTrue(0 < seconds <= (180 - 10) * 60, seconds)
        self.assert_all_destroyed()

    def test_transport_exception_is_unavailable_and_still_destroyed(self):
        self.assertEqual(self.go(ssh=FakeSsh(boom=RuntimeError("boom"))), 2)
        self.assert_all_destroyed()

    def test_interrupt_still_destroys(self):
        with self.assertRaises(KeyboardInterrupt):
            self.go(ssh=FakeSsh(boom=KeyboardInterrupt()))
        self.assert_all_destroyed()

    def test_log_is_bounded(self):
        chatter = "x" * 3000 + "\n"
        ssh = FakeSsh(chatter=chatter * 5000)
        self.assertEqual(self.go(ssh=ssh), 0)
        self.assertLess(len(self.stdout.getvalue()), simple_run.LOG_CAP + 100_000)
        self.assertLess((self.out / "box.log").stat().st_size, simple_run.LOG_CAP + 100_000)


class Teardown(Base):
    def test_absence_is_verified_through_instances_v1(self):
        self.go()
        listing = [p for m, p, _ in self.vast.requests if "/api/v1/instances" in p]
        self.assertTrue(listing)
        self.assertLess(
            max(i for i, r in enumerate(self.vast.requests) if r[0] == "DELETE"),
            max(i for i, r in enumerate(self.vast.requests) if "/api/v1/instances" in r[1]),
        )

    def test_lagging_listing_is_waited_out(self):
        self.vast.list_lag = 3
        self.assertEqual(self.go(), 0, self.stdout.getvalue())

    def test_instance_that_never_disappears_is_a_leak_exit_three(self):
        self.vast.never_absent = True
        self.assertEqual(self.go(), 3)
        self.assertIn("LEAK", self.stdout.getvalue())

    def test_foreign_label_is_never_deleted_by_a_run(self):
        mine = self.vast.add_instance("owner-manual-box", 7)
        look = self.vast.add_instance("kf-ci-worker-run-9", 8)
        self.assertEqual(self.go(), 0)
        self.assertEqual(self.vast.deleted_calls(), [1000])
        self.assertFalse(
            self.vast.instances[mine]["deleted"] or self.vast.instances[look]["deleted"]
        )

    def test_returned_foreign_label_is_not_deleted_and_is_a_leak(self):
        self.vast.label_swap = {1000: "owner-manual-box"}
        self.assertEqual(self.go(), 3)
        self.assertEqual(self.vast.deleted_calls(), [])

    def test_swapped_to_another_runs_label_is_a_leak_and_not_deleted(self):
        self.vast.inspect_swap = {1000: "kf-ci-simple-1700000000-aaaaaaaa"}
        self.assertEqual(self.go(), 3)
        self.assertEqual(self.vast.deleted_calls(), [])

    def test_listing_error_during_teardown_is_a_leak(self):
        self.vast.list_override = {"success": False, "instances": []}
        self.assertEqual(self.go(), 3)

    def test_api_destroy_refuses_foreign_even_when_the_caller_names_it(self):
        self.vast.add_instance("owner-manual-box", 7)
        api = vast_api.VastApi(KEY, self.vast.url)
        with self.assertRaises(vast_api.ApiError):
            api.destroy(7, "owner-manual-box")
        self.assertEqual(self.vast.deleted_calls(), [])

    def test_api_destroy_refuses_a_different_simple_label(self):
        self.vast.add_instance("kf-ci-simple-1700000000-aaaaaaaa", 7)
        api = vast_api.VastApi(KEY, self.vast.url)
        with self.assertRaises(vast_api.ApiError):
            api.destroy(7, "kf-ci-simple-1700000001-bbbbbbbb")
        self.assertEqual(self.vast.deleted_calls(), [])

    def test_api_destroy_refuses_other_labels(self):
        self.vast.add_instance("owner-manual-box", 7)
        self.vast.add_instance("kf-ci-simple", 8)
        api = vast_api.VastApi(KEY, self.vast.url)
        for iid in (7, 8):
            with self.assertRaises(vast_api.ApiError):
                api.destroy(iid)
        self.assertEqual(self.vast.deleted_calls(), [])


class Secrets(Base):
    def everything(self):
        text = self.stdout.getvalue()
        for path in self.out.glob("*"):
            text += path.read_text()
        return text

    def test_key_never_appears_in_logs_or_files(self):
        ssh = FakeSsh(chatter="the box echoes " + KEY + " and Bearer " + KEY + "\n")
        self.vast.create_400_body = {11: {"success": False, "msg": "bad " + KEY}}
        self.assertEqual(self.go(ssh=ssh), 0)
        self.assertNotIn(KEY, self.everything())
        self.assertNotIn("SECRETvalue", self.everything())

    def test_key_is_not_in_any_ssh_command_or_request_body(self):
        self.go()
        self.assertNotIn(KEY, json.dumps([c for _, _, c, _ in self.ssh.calls]))
        self.assertNotIn(KEY, json.dumps([b for _, _, b in self.vast.requests]))

    def test_error_text_from_the_provider_is_not_echoed_with_the_key(self):
        self.vast.create_error_with_instance = {11}
        self.vast.offers = [self.vast.offers[0]]
        self.assertEqual(self.go(), 2)
        self.assertNotIn(KEY, self.everything())

    def test_missing_or_malformed_key_is_refused_before_any_request(self):
        for key in (None, "", "short", "has space in it 1234567890"):
            self.stdout.truncate(0)
            self.assertEqual(self.go(key=key), 2)
            self.assertNotIn("space in it", self.stdout.getvalue())
        self.assertEqual(self.vast.requests, [])

    def test_the_runner_refuses_a_foreign_api_base_before_any_request(self):
        self.assertEqual(self.go("--api-base", "https://evil.invalid"), 2)
        self.assertIn("configuration refused", self.stdout.getvalue())
        self.assertEqual(self.vast.requests, [])

    def test_box_log_file_has_no_key(self):
        self.go(ssh=FakeSsh(chatter="leak " + KEY + "\n"))
        self.assertIn("leak ***", (self.out / "box.log").read_text())

    def test_api_base_is_https_vast_or_loopback_only(self):
        for base in (
            "http://evil.example",
            "https://evil.example",
            "http://127.0.0.1.evil.example",
        ):
            with self.assertRaises(vast_api.ApiError):
                vast_api.VastApi(KEY, base)
        vast_api.VastApi(KEY, "https://console.vast.ai")


class SshArgv(unittest.TestCase):
    def argv(self):
        return simple_run.SshTransport("203.0.113.5", 41234, "/k/id", "root").argv("echo hi")

    def test_direct_address_user_and_one_quoted_command(self):
        argv = self.argv()
        self.assertEqual(argv[:3], ["ssh", "-p", "41234"])
        self.assertEqual(argv[-2:], ["root@203.0.113.5", "bash -c 'echo hi'"])

    def test_agent_forwarding_is_off(self):
        argv = self.argv()
        self.assertIn("ForwardAgent=no", argv)
        self.assertNotIn("-A", argv)

    def test_never_interactive_and_only_the_named_identity(self):
        argv = self.argv()
        self.assertIn("BatchMode=yes", argv)
        self.assertIn("IdentitiesOnly=yes", argv)
        self.assertEqual(argv[argv.index("-i") + 1], "/k/id")

    def test_option_set_is_exact(self):
        argv = self.argv()
        opts = sorted(argv[i + 1] for i, a in enumerate(argv) if a == "-o")
        want = [
            "BatchMode=yes", "ConnectTimeout=15", "ForwardAgent=no", "IdentitiesOnly=yes",
            "LogLevel=ERROR", "ServerAliveInterval=30", "StrictHostKeyChecking=no",
            "UserKnownHostsFile=/dev/null",
        ]  # fmt: skip
        self.assertEqual(opts, want)

    def test_the_provider_key_is_unknown_to_the_transport(self):
        self.assertNotIn(KEY, " ".join(self.argv()))


class CloneScript(unittest.TestCase):
    def make_repo(self, root):
        repo = Path(root) / "src.git"
        run = lambda *a, **k: subprocess.run(a, check=True, capture_output=True, **k)  # noqa: E731
        run("git", "init", "-q", str(repo))
        (repo / "f").write_text("x")
        env = dict(os.environ, GIT_AUTHOR_NAME="a", GIT_AUTHOR_EMAIL="a@b", GIT_COMMITTER_NAME="a")
        env["GIT_COMMITTER_EMAIL"] = "a@b"
        run("git", "-C", str(repo), "add", "f")
        run("git", "-C", str(repo), "commit", "-q", "-m", "m", env=env)
        sha = subprocess.check_output(["git", "-C", str(repo), "rev-parse", "HEAD"], text=True)
        return repo, sha.strip()

    def test_script_checks_out_and_verifies_the_sha(self):
        with tempfile.TemporaryDirectory() as root:
            repo, sha = self.make_repo(root)
            dest = Path(root) / "dest"
            cmd = simple_run.clone_command("file://" + str(repo), sha, str(dest))
            self.assertEqual(subprocess.run(["bash", "-c", cmd]).returncode, 0)
            head = subprocess.check_output(["git", "-C", str(dest), "rev-parse", "HEAD"], text=True)
            self.assertEqual(head.strip(), sha)

    def test_script_fails_on_an_unknown_sha(self):
        with tempfile.TemporaryDirectory() as root:
            repo, _ = self.make_repo(root)
            cmd = simple_run.clone_command("file://" + str(repo), "b" * 40, str(Path(root) / "d"))
            self.assertNotEqual(
                subprocess.run(["bash", "-c", cmd], capture_output=True).returncode, 0
            )

    def test_inputs_are_validated_not_quoted_blindly(self):
        for sha in ("main", "a" * 39, "a" * 40 + ";id"):
            with self.assertRaises(ValueError):
                simple_run.clone_command("https://github.com/o/r.git", sha)


class Reaper(Base):
    def setUp(self):
        super().setUp()
        self.deps_r = simple_reaper.Deps(
            clock=self.clock.time, sleep=self.clock.sleep, stdout=self.stdout
        )

    def label(self, age_s, tag="aaaaaaaa"):
        return "kf-ci-simple-%d-%s" % (NOW - age_s, tag)

    def reap(self, *extra):
        argv = ["--defaults", DEFAULTS, "--api-base", self.vast.url, *extra]
        return simple_reaper.main(argv, self.deps_r, {"VAST_API_KEY": KEY})

    def test_deletes_only_old_simple_instances(self):
        old = self.vast.add_instance(self.label(20000), 1)
        young = self.vast.add_instance(self.label(600, "bbbbbbbb"), 2)
        foreign = self.vast.add_instance("owner-manual-box", 3)
        worker = self.vast.add_instance("kf-ci-" + str(NOW - 99999) + "-aaaaaaaa", 4)
        self.assertEqual(self.reap(), 0, self.stdout.getvalue())
        self.assertEqual(self.vast.deleted_calls(), [old])
        for iid in (young, foreign, worker):
            self.assertFalse(self.vast.instances[iid]["deleted"])

    def test_threshold_is_hard_plus_margin(self):
        edge = self.vast.add_instance(self.label((180 + 30) * 60 - 5), 1)
        self.reap()
        self.assertFalse(self.vast.instances[edge]["deleted"])
        self.vast.instances[edge]["label"] = self.label((180 + 30) * 60 + 5)
        self.reap()
        self.assertTrue(self.vast.instances[edge]["deleted"])

    def test_dry_run_deletes_nothing(self):
        self.vast.add_instance(self.label(99999), 1)
        self.assertEqual(self.reap("--dry-run"), 0)
        self.assertEqual(self.vast.deleted_calls(), [])

    def fail_closed(self, *listings):
        self.vast.add_instance(self.label(99999), 1)
        for listing in listings:
            self.vast.list_override = listing
            self.assertEqual(self.reap(), 2, listing)
        self.assertEqual(self.vast.deleted_calls(), [])

    ROW = {"id": 1, "label": "kf-ci-simple-1799900000-aaaaaaaa"}

    def test_malformed_listing_shapes_fail_closed(self):
        self.fail_closed([], {"success": True}, {"success": True, "instances": {"1": self.ROW}})

    def test_unsuccessful_listing_fails_closed(self):
        self.fail_closed({"success": False, "instances": [self.ROW]})

    def test_malformed_rows_fail_closed(self):
        row = self.ROW
        self.fail_closed(
            {"success": True, "instances": [dict(row, id="1")]},
            {"success": True, "instances": [dict(row, id=True)]},
            {"success": True, "instances": [row, "junk"]},
            {"success": True, "instances": [dict(row, label=5)]},
        )

    def test_duplicate_rows_fail_closed(self):
        self.fail_closed({"success": True, "instances": [self.ROW, dict(self.ROW)]})

    def test_reaper_refuses_without_a_usable_key(self):
        for key in (None, "short"):
            argv = ["--defaults", DEFAULTS, "--api-base", self.vast.url]
            self.assertEqual(simple_reaper.main(argv, self.deps_r, {"VAST_API_KEY": key}), 2)
        self.assertEqual(self.vast.requests, [])

    def test_foreign_only_listing_exits_zero(self):
        self.vast.add_instance("owner-manual-box", 1)
        self.vast.add_instance(None, 2)
        self.assertEqual(self.reap(), 0, self.stdout.getvalue())
        self.assertEqual(self.vast.deleted_calls(), [])

    def test_pagination_is_followed_and_looping_tokens_fail_closed(self):
        ids = [self.vast.add_instance(self.label(99999, "%08d" % n), 100 + n) for n in range(5)]
        self.vast.pages = 2
        self.assertEqual(self.reap(), 0, self.stdout.getvalue())
        self.assertEqual(sorted(self.vast.deleted_calls()), ids)

    def test_looping_page_tokens_fail_closed(self):
        self.vast.add_instance(self.label(99999), 1)
        self.vast.add_instance(self.label(99999, "cccccccc"), 2)
        self.vast.pages = 1
        self.vast.loop_token = True
        self.assertEqual(self.reap(), 2)
        self.assertEqual(self.vast.deleted_calls(), [])

    def test_unparseable_simple_label_is_reported_not_deleted(self):
        odd = self.vast.add_instance("kf-ci-simple-banana", 1)
        old = self.vast.add_instance(self.label(99999), 2)
        self.assertEqual(self.reap(), 3)
        self.assertFalse(self.vast.instances[odd]["deleted"])
        self.assertTrue(self.vast.instances[old]["deleted"])

    def test_undeletable_instance_exits_three(self):
        self.vast.add_instance(self.label(99999), 1)
        self.vast.never_absent = True
        self.assertEqual(self.reap(), 3)


if __name__ == "__main__":
    unittest.main()
