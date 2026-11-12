"""Contract test against the Worker's golden fixture (tests/fixtures/hosts-wire.json, copied byte for byte from
ci/vast-broker/test/fixtures/hosts-wire.json of the Worker PR) plus the round-2 interop fixes.

PENDING (not in the fixture yet; W1 is adding them, shapes as the coordinator specified, re-copy the fixture when
it lands): frame `ready`; hello.enforced; heartbeat.idle_since (epoch ms); seed_register.warnings; welcome
.commands_until_ms; cmd.commands_until_ms; offer.run {nonce, runner_nonce, sha}; assignment.seed.
FALSIFIER: a frame the daemon builds has a key or type the fixture's valid frames do not have (outside the pending
list); the daemon fails to parse a valid Worker frame or accepts an invalid offer from the fixture.
"""

import json
import os
import re
import sys
import tempfile
import threading
import time
import unittest
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))

from kfhost import config, frames, jobs, probes, profile  # noqa: E402
from kfhost.daemon import Daemon, parse_frame  # noqa: E402
from kfhost.fetch import FetchError  # noqa: E402
from kfhost.profile import Refused  # noqa: E402
from kfhost.test_kfhost import (Base, Closed, FakeConn, FakeProbes, FakeRunner, FakeSys, REPO, SHA, cfg_for,  # noqa: E402
                                offer, welcome)

FIX = json.loads((Path(__file__).with_name("tests") / "fixtures" / "hosts-wire.json").read_text())
D2W, W2D, CONST = FIX["daemon_to_worker"], FIX["worker_to_daemon"], FIX["constants"]
PENDING_KEYS = {"hello": {"enforced"}, "heartbeat": {"idle_since"}, "seed_register": {"warnings"}}
PENDING_PREFIXES = {"ready"}


def kind(v):
    return "bool" if isinstance(v, bool) else "num" if isinstance(v, (int, float)) else type(v).__name__


def check_shape(case, frame):
    """frame's keys and value kinds must appear among the fixture's valid frames of its type."""
    t = frame["type"]
    if t in PENDING_PREFIXES:
        case.assertEqual(sorted(set(frame) - {"v"}), ["offer_id", "type"])
        case.assertIn(frame.get("v", 1), (1,))
        case.assertRegex(frame["offer_id"], CONST["offer_id_regex"])
        return
    valid = D2W[t]["valid"]
    case.assertIn(frame.get("v", 1), (1,))  # `v` is optional and must be 1
    allowed = {k for f in valid for k in f} | PENDING_KEYS.get(t, set()) | {"v"}
    required = set.intersection(*(set(f) for f in valid)) - {"v"}
    case.assertLessEqual(set(frame), allowed, (t, sorted(set(frame) - allowed)))
    case.assertTrue(required <= set(frame) | {"v"}, (t, required - set(frame)))
    for k, v in frame.items():
        if k == "v" or k in PENDING_KEYS.get(t, set()):
            continue
        seen = {kind(f[k]) for f in valid if k in f}
        case.assertIn(kind(v), seen, (t, k, v))


class FixtureConstants(unittest.TestCase):
    def test_constants_match(self):
        self.assertEqual(config.PUBLIC_REPO, CONST["canonical_repo"])
        self.assertEqual(list(profile.STAGES), CONST["stage_names"])
        self.assertRegex("a" * 64, CONST["tree_key_regex"])
        self.assertEqual(CONST["deadline_field"], "deadline_ms")
        self.assertEqual(CONST["upgrade_route"], "/hosts/<box_id>/ws")
        self.assertEqual(config.load.__module__, "kfhost.config")
        with tempfile.TemporaryDirectory() as t:
            self.assertEqual(cfg_for(t, "standard", box_id="x")["ws_path"], CONST["upgrade_route"].replace("<box_id>", "x"))
        self.assertEqual(CONST["close_codes"]["bad_token"], 4401)


class ServerFramesParse(Base):
    def cfg(self):
        return cfg_for(self.tmp.name, "standard")

    def test_every_valid_server_frame_parses(self):
        for name, spec in W2D.items():
            for f in spec["valid"]:
                msg = parse_frame(json.dumps(f))
                self.assertEqual(msg, f, name)
        for f in W2D["offer"]["valid"]:
            got = profile.check_offer("standard", dict(f), self.cfg(), int(time.time() * 1000))
            self.assertEqual(got["deadline_ms"], f["deadline_ms"])
            if "seed" in f["assignment"]:  # borrowed never consumes an artifact from other boxes
                with self.assertRaises(Refused):
                    profile.check_offer("borrowed", dict(f), self.cfg(), int(time.time() * 1000))
            else:
                profile.check_offer("borrowed", dict(f), self.cfg(), int(time.time() * 1000))

    def test_every_invalid_offer_is_refused(self):
        for case in W2D["offer"]["invalid"]:
            with self.assertRaises(Refused, msg=case["why"]):
                profile.check_offer("standard", dict(case["frame"]), self.cfg(), int(time.time() * 1000))

    def test_server_frames_dispatch_without_error(self):
        d, conn, _ = self.serve([welcome()] + [dict(f) for n in ("welcome", "cancel", "seed_ack", "error") for f in W2D[n]["valid"]]
                                + [None, Closed(4401)])
        d.log = lambda m: None
        d.run_forever(max_sessions=1)
        self.assertEqual(d.profile, "owned")  # the last fixture welcome (owned, commands_until_ms null) is the one applied

    def test_cmd_frames_from_the_fixture_are_refused_cleanly_without_the_flags(self):
        d, conn, _ = self.serve([welcome("standard")] + [dict(f) for f in W2D["cmd"]["valid"]] + [lambda c: d.join(), None, Closed(4401)],
                                ceiling="standard")
        d.log = lambda m: None
        d.run_forever(max_sessions=1)
        done = [m for m in conn.sent if m["type"] == "cmd_done"]
        self.assertEqual([m["rc"] for m in done], [126, 126])
        for m in conn.sent:
            check_shape(self, m) if m["type"] in D2W or m["type"] in PENDING_PREFIXES else None


class DaemonFramesMatchFixture(Base):
    def collect(self, d, conn):
        return [m for m in conn.sent]

    def test_hello_heartbeat_idle_and_busy(self):
        d, conn, _ = self.serve([welcome("owned")] + [None] * 40 + [Closed(4401)], ceiling="owned",
                                prober=FakeProbes(outs={"nvidia-smi": "NVIDIA GeForce RTX 4070, 8.9, 12282, 580.65.06, 0x278410DE\n"},
                                                  existing={"/dev/kvm"}))
        d.run_forever(max_sessions=1)
        hello = conn.sent[0]
        check_shape(self, hello)
        caps = hello["capabilities"]
        fixture_caps = set(D2W["hello"]["valid"][0]["capabilities"])
        self.assertLessEqual(set(caps), fixture_caps | {"die"})
        self.assertEqual(caps["compute_cap"], "8.9")
        self.assertRegex(caps["pci_device_id"], CONST["pci_device_id_regex"])
        self.assertTrue(all(v is not None for v in caps.values()))
        self.assertRegex(hello["box_id"], r"^[a-z0-9][a-z0-9-]{0,39}$")
        self.assertTrue(len(hello["enforced"]) <= 8 and all(isinstance(v, bool) for v in hello["enforced"].values()))
        beats = [m for m in conn.sent if m["type"] == "heartbeat"]
        self.assertTrue(beats)
        for b in beats:
            check_shape(self, b)
        self.assertIsInstance(beats[0]["idle_since"], int)
        self.assertGreater(beats[0]["idle_since"], 10**12)  # epoch MILLISECONDS
        self.assertGreater(beats[0]["ts"], 10**9)
        self.assertLess(beats[0]["ts"], 10**11)  # ts stays in seconds

    def test_busy_heartbeat_has_run_id_and_no_idle_since(self):
        f = frames.heartbeat(1800000000, "busy", run_id="123-1", idle_since_ms=5)
        check_shape(self, f)
        self.assertNotIn("idle_since", f)

    def test_ack_ready_nack_builders(self):
        for f in (frames.ack("0123456789abcdef"), frames.ready("0123456789abcdef"), frames.nack("0123456789abcdef", "x" * 500)):
            check_shape(self, f)
        self.assertEqual(len(frames.nack("0123456789abcdef", "x" * 500)["reason"]), 200)

    def test_real_run_sends_ack_ready_in_order(self):
        d, conn, _ = self.serve([welcome(), offer(), lambda c: [threading.Event().wait(0.3)], Closed(4401)])
        d.run_forever(max_sessions=1)
        d.join()
        seq = [m["type"] for m in conn.sent if m["type"] in ("ack", "ready", "nack")]
        self.assertEqual(seq, ["ack", "ready"])
        for m in conn.sent:
            if m["type"] in ("ack", "ready"):
                check_shape(self, m)

    def test_seed_register_builder(self):
        art = {"name": "kf3.tar", "sha256": "d" * 64, "size": 1234}
        f = frames.seed_register("0123456789abcdef", art, "/ab12cd34/kf3.tar", ["w"] * 9)
        check_shape(self, f)
        self.assertLessEqual(len(f["warnings"]), 5)
        self.assertEqual(f["path"], D2W["seed_register"]["valid"][0]["path"].replace("kf3.tar", "kf3.tar"))
        for bad_art, path in (({**art, "size": 0}, "/ab12cd34/kf3.tar"), ({**art, "name": "-x"}, "/ab12cd34/-x"),
                              ({**art, "name": "a..b"}, "/ab12cd34/a..b"), ({**art, "name": ".hid"}, "/ab12cd34/.hid"),
                              (art, "/ab12cd34567/kf3.tar"), (art, "/ab12cd34/other.tar"), ({**art, "sha256": "zz"}, "/ab12cd34/kf3.tar")):
            with self.assertRaises(ValueError):
                frames.seed_register("0123456789abcdef", bad_art, path)

    def test_cmd_output_and_done_builders_and_pacing(self):
        for f in (frames.cmd_output("fedcba9876543210", 0, "stdout", "hi\n"),
                  frames.cmd_output("fedcba9876543210", 1, "stderr", "", True), frames.cmd_done("fedcba9876543210", 0)):
            check_shape(self, f)
        code = "import sys\nfor _ in range(6):\n sys.stdout.write('x'*4096);sys.stdout.flush()"
        d, conn, _ = self.serve([welcome(), {"type": "cmd", "cmd_id": "00000000000000c1", "argv": [sys.executable, "-c", code]},
                                 lambda c: d.join(), None, Closed(4401)])
        d.cmd_interval = 0.05
        start = time.monotonic()
        d.run_forever(max_sessions=1)
        out = [m for m in conn.sent if m["type"] == "cmd_output"]
        self.assertTrue(all(len(m["data"]) <= 4096 for m in out))
        self.assertGreaterEqual(time.monotonic() - start, 0.05 * (len(out) - 1) * 0.8)  # paced, not a burst
        for m in out:
            check_shape(self, m)

    def test_hello_frame_invalid_cases_cannot_be_built(self):
        with tempfile.TemporaryDirectory() as t:
            for bad in ("Rack_1", "-rack", "a" * 41, "Rack"):
                with self.assertRaises(config.ConfigError):
                    cfg_for(t, "standard", box_id=bad)


class OfferRunSectionAndResult(Base):
    def test_offer_run_section_validation(self):
        cfg, now = cfg_for(self.tmp.name), int(time.time() * 1000)
        for run in ({"nonce": "c" * 31, "runner_nonce": "d" * 64}, {"nonce": "c" * 32, "runner_nonce": "d" * 63},
                    {"nonce": "c" * 32, "runner_nonce": "d" * 64, "sha": "b" * 40}, {"nonce": "c" * 32, "runner_nonce": "d" * 64, "x": 1},
                    {"nonce": "c" * 32, "runner_nonce": "d" * 64, "request_id": "other"}, "x"):
            with self.assertRaises(Refused):
                profile.check_offer("standard", offer(run=run), cfg, now)
        profile.check_offer("standard", offer(run={"nonce": "c" * 32, "runner_nonce": "d" * 64, "sha": SHA, "request_id": "req-1"}), cfg, now)

    def test_offer_request_id_and_run_id_formats(self):
        cfg, now = cfg_for(self.tmp.name), int(time.time() * 1000)
        for kw in ({"request_id": ""}, {"request_id": "x" * 65}, {"run_id": "r1"}, {"run_id": "0-1"}, {"offer_id": "o1"}):
            with self.assertRaises(Refused):
                profile.check_offer("standard", offer(**kw), cfg, now)

    def test_seconds_looking_and_past_deadline_refused(self):
        cfg, now = cfg_for(self.tmp.name), int(time.time() * 1000)
        for dl in (1800000000, now - 1):
            with self.assertRaises(Refused):
                profile.check_offer("standard", offer(deadline_ms=dl), cfg, now)
        o = offer()
        o["deadline"] = o["deadline_ms"] // 1000  # the old seconds field is no longer an accepted key
        with self.assertRaises(Refused):
            profile.check_offer("standard", o, cfg, now)

    def test_result_has_exactly_the_workers_key_set(self):
        cfg = cfg_for(self.tmp.name, "standard")
        r = jobs.JobRunner(cfg, {})
        r.hardware_fn = lambda: {"gpu_name": "G", "compute_cap": 890, "driver_version": "580.1"}
        ctx = jobs.JobContext(offer(), "standard", cfg, None, lambda: False)
        res = r.result(ctx, 0)
        self.assertEqual(sorted(res), ["hardware", "nonce", "runner_nonce", "sha", "suites", "verdict"])
        self.assertRegex(res["nonce"], r"^[0-9a-f]{32}$")
        self.assertRegex(res["runner_nonce"], r"^[0-9a-f]{64}$")
        self.assertRegex(res["sha"], r"^[0-9a-f]{40}$")
        self.assertEqual(r.result(ctx, 1)["verdict"], "FAIL")
        no_run = offer()
        del no_run["run"]
        self.assertIsNone(r.result(jobs.JobContext(no_run, "standard", cfg, None, lambda: False), 0))

    def test_hardware_helper_is_the_vast_payloads(self):
        r = jobs.JobRunner(cfg_for(self.tmp.name), {})
        hw = r.hardware()
        self.assertEqual(sorted(hw), ["compute_cap", "driver_version", "gpu_name"])  # unknown on a GPU-less machine

    def test_run_report_carries_tree_key_mismatch_only_there(self):
        posted = []

        class Mod:
            @staticmethod
            def build_report(lines, result):
                return {"schema": 1}

            class Transport:
                def __init__(s, token, access=None):
                    posted.append(("token", token))

                def post(s, url, body):
                    posted.append((url, json.loads(body)))
                    return 200, {}

        cfg = cfg_for(self.tmp.name)
        o = offer()
        self.assertTrue(jobs.post_run_report(cfg, o, [], {"verdict": "PASS"}, True, module=Mod))
        self.assertEqual(posted[1][0], "https://w.example/run-report/123-1")
        self.assertIs(posted[1][1]["tree_key_mismatch"], True)
        posted.clear()
        jobs.post_run_report(cfg, o, [], {"verdict": "PASS"}, False, module=Mod)
        self.assertNotIn("tree_key_mismatch", posted[1][1])

    def test_run_report_builder_smoke_with_the_real_module(self):
        try:
            mod = jobs.load_run_report_module()
            doc = mod.build_report(["hello"], {"verdict": "FAIL", "suites": [], "hardware": {"gpu_name": "unknown", "compute_cap": 0, "driver_version": "unknown"}})
        except Exception as error:  # noqa: BLE001
            self.fail("real vast_run_report.build_report rejected a minimal input: %r" % error)
        self.assertIsInstance(doc, dict)


class SeedConsumption(Base):
    SEED = {"url": "https://1-2-3-4.sslip.io/ab12cd34/k.tar", "fallback_url": "https://1-2-3-4.traefik.me/ab12cd34/k.tar",
            "sha256": "e" * 64, "size": 5, "name": "k.tar"}

    def runner(self, rcs):
        cfg = cfg_for(self.tmp.name, "standard")
        logs, calls = [], []
        r = jobs.JobRunner(cfg, {})
        ctx = jobs.JobContext(offer(assignment=dict(offer()["assignment"], seed=self.SEED)), "standard", cfg, None, lambda: False)
        ctx.log = logs.append

        class Pol:
            def fetch(s, url, sha, size, out, source="worker"):
                calls.append((url, sha, size, source))
                return rcs.pop(0)

        ctx.downloads = Pol()
        return r, ctx, logs, calls

    def test_primary_then_fallback_with_pinned_sha_and_size(self):
        r, ctx, logs, calls = self.runner([1, 0])
        r.consume_seed(ctx, self.tmp.name)
        self.assertEqual([c[0] for c in calls], [self.SEED["url"], self.SEED["fallback_url"]])
        self.assertTrue(all(c[1:3] == ("e" * 64, 5) for c in calls))
        self.assertTrue(ctx.seed_path.endswith("/seed/k.tar"))
        self.assertEqual(r.child_env(ctx, self.tmp.name)["KF_SEED_FILE"], ctx.seed_path)

    def test_all_failed_builds_locally(self):
        r, ctx, logs, calls = self.runner([1, 1])
        r.consume_seed(ctx, self.tmp.name)
        self.assertFalse(getattr(ctx, "seed_path", None))
        self.assertTrue(any("building locally" in l for l in logs))

    def test_invalid_seed_and_borrowed_refused(self):
        cfg, now = cfg_for(self.tmp.name), int(time.time() * 1000)
        for bad in ({**self.SEED, "size": 0}, {**self.SEED, "sha256": "x"}, {**self.SEED, "name": "../x"}, {**self.SEED, "extra": 1},
                    {**self.SEED, "url": "ftp://x/y"}):
            with self.assertRaises(Refused):
                profile.check_offer("standard", offer(assignment=dict(offer()["assignment"], seed=bad)), cfg, now)
        with self.assertRaises(Refused):
            profile.check_offer("borrowed", offer(assignment=dict(offer()["assignment"], seed=self.SEED)), cfg, now)


class CmdWindowAndLifecycle(Base):
    def run_cmd(self, extra, w=None):
        c = {"type": "cmd", "cmd_id": "00000000000000c1", "argv": ["true"], **extra}
        d, conn, _ = self.serve([w or welcome("owned"), c, lambda cc: d.join(), None, Closed(4401)], ceiling="owned")
        d.run_forever(max_sessions=1)
        return [m["rc"] for m in conn.sent if m["type"] == "cmd_done"]

    def test_commands_until_in_the_cmd_frame_wins(self):
        future, past = int(time.time() * 1000) + 60000, int(time.time() * 1000) - 1
        self.assertEqual(self.run_cmd({"commands_until_ms": future}), [0])
        self.assertEqual(self.run_cmd({"commands_until_ms": past}), [126])
        w = welcome("owned")
        del w["commands_until_ms"]
        self.assertEqual(self.run_cmd({}, w), [126])
        self.assertEqual(self.run_cmd({"commands_until_ms": future}, w), [0])

    def test_cmd_id_must_be_16_hex(self):
        d, conn, _ = self.serve([welcome("owned"), {"type": "cmd", "cmd_id": "c1", "argv": ["true"]}, None, Closed(4401)], ceiling="owned")
        d.run_forever(max_sessions=1)
        self.assertEqual([m for m in conn.sent if m["type"].startswith("cmd")], [])

    def test_cancel_during_fetch_aborts_before_the_run(self):
        gate = threading.Event()
        b = self

        class SlowFetch(FakeRunner):
            def prepare(s, offer_, profile_=None):
                gate.wait(5)
                return tempfile.mkdtemp()

        self.factory = lambda cfg, caps: self.runners.append(SlowFetch(cfg, caps)) or self.runners[-1]
        d, conn, _ = self.serve([welcome(), offer(), {"type": "cancel", "offer_id": "0123456789abcdef"},
                                 lambda c: gate.set(), lambda c: [threading.Event().wait(0.3)], None, Closed(4401)])
        d.run_forever(max_sessions=1)
        d.join()
        types = [m["type"] for m in conn.sent if m["type"] in ("ack", "ready", "nack")]
        self.assertEqual(types, ["ack", "nack"])
        self.assertFalse(self.runners[0].ran)

    def test_deadline_passing_during_fetch_aborts_before_the_run(self):
        class SlowFetch(FakeRunner):
            def prepare(s, offer_, profile_=None):
                time.sleep(0.4)
                return tempfile.mkdtemp()

        self.factory = lambda cfg, caps: self.runners.append(SlowFetch(cfg, caps)) or self.runners[-1]
        d, conn, _ = self.serve([welcome(), offer(deadline_ms=int(time.time() * 1000) + 150), lambda c: [threading.Event().wait(0.8)],
                                 None, Closed(4401)])
        d.run_forever(max_sessions=1)
        d.join()
        self.assertEqual([m["type"] for m in conn.sent if m["type"] in ("ack", "ready", "nack")], ["ack", "nack"])
        self.assertFalse(self.runners[0].ran)


class HttpUpgradeRefusals(Base):
    def test_non_101_answers_retry_slowly_with_growing_backoff_never_tight(self):
        for status in (302, 401, 403, 404, 429, 500, 503):  # HTTP 401 is NOT close code 4401: it retries, slowly
            code = status
            conns = [Closed(None, http_status=status)] * 3 + [Closed(4401)]
            d, _, tr = self.serve([], conns=conns)
            d.log = lambda m: None
            d.rng = type("R", (), {"random": lambda s: 1.0})()
            d.run_forever()
            self.assertGreaterEqual(min(d.delays), 30.0, code)
            self.assertEqual(d.delays, sorted(d.delays), code)  # exponential, not a tight loop
            self.assertLess(d.delays[0], d.delays[-1], code)

    def test_ordinary_drops_still_retry_fast_and_4408_is_not_slow(self):
        self.assertIsNone(Daemon.slow_params(1006))
        self.assertIsNone(Daemon.slow_params(4408))
        self.assertIsNone(Daemon.slow_params(4401))
        self.assertIsNone(Daemon.slow_params(4302))  # a WebSocket close code, not an HTTP status
        self.assertEqual(Daemon.slow_params(None, 401), (30.0, 300.0))

    def test_websockets_adapter_keeps_http_status_apart_from_close_codes(self):
        src = (Path(__file__).with_name("transport.py")).read_text()
        self.assertIn("http_status=code", src)
        self.assertNotIn("4000 + code", src)


if __name__ == "__main__":
    unittest.main()
