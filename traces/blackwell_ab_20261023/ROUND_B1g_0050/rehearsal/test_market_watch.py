"""B1g market watcher: reuses the operator's own offer rules, read-only, bounded cadence.

Run: KF_BW3_D=/workspace/bw3-round-b1g-a1-run python3 -m unittest test_market_watch
"""

import hashlib
import json
import os
from pathlib import Path
import random
import shutil
import subprocess
import sys
import tempfile
import unittest

D = Path(os.environ.get("KF_BW3_D", "/workspace/bw3-round-b1g-a1-run"))
TB = Path("/workspace/bw-native-edge-wt/traces/blackwell_ab_20261023")
SNAPSHOTS = [
    TB / "ROUND_B1b_1815/market.json",
    TB / "ROUND_B1c_2340/market.json",
    TB / "ROUND_B1d_0015/market.json",
    TB / "ROUND_B1e_0055/market.json",
    TB / "ROUND_B1f_0115/market.json",
    TB / "ROUND_B1g_0050/attempt1/market.json",
]
sys.dont_write_bytecode = True
sys.path.insert(0, str(D))
import market_watch as M  # noqa: E402

RULES = M.load_rules(D / "ops.py")


def offers_of(path):
    return json.loads(path.read_text())["offers"]


class OracleOps:
    """Run the unchanged operator (preflight) in a sandbox with a fake read-only vastai."""

    def __init__(self):
        self.tmp = Path(tempfile.mkdtemp())
        self.d = self.tmp / "D"
        self.d.mkdir()
        media = self.d / "media"
        media.mkdir()
        files = []
        for i in range(4):
            p = media / f"m{i}"
            p.write_bytes(bytes([i]) * 10)
            files.append(
                {
                    "file": p.name,
                    "expected_bytes": 10,
                    "expected_sha256": hashlib.sha256(p.read_bytes()).hexdigest(),
                }
            )
        (self.d / "media_manifest.json").write_text(
            json.dumps({"all_verified": True, "files": files})
        )
        for name in (
            "owner_authority.json",
            "known_bad_machines.json",
            "lease_profile.py",
            "pin_evidence.py",
        ):
            shutil.copy2(D / name, self.d / name)
        self.ledger = self.tmp / "trace" / "owned_instances.json"
        self.ledger.parent.mkdir()
        self.ledger.write_text("[]\n")
        src = (D / "ops.py").read_text()
        src = src.replace(f'D = Path("{D}")', f'D = Path("{self.d}")')
        a = src.index("L = W / ")
        b = src.index("\n", a)
        src = src[:a] + f'L = Path("{self.ledger}")' + src[b:]
        (self.d / "ops.py").write_text(src)
        self.bin = self.tmp / "bin"
        self.bin.mkdir()

    def run(self, offers, credit):
        (self.d / "single_market_B1g_1.json").write_text(
            json.dumps({"utc": "2026-10-23T00:00:00+00:00", "offers": offers})
        )
        fake = self.bin / "vastai"
        fake.write_text(
            '#!/bin/sh\ncase "$1 $2" in\n'
            " 'show instances') echo '[]';;\n"
            f" 'show user') echo '{{\"credit\": {credit}}}';;\n"
            " *) exit 9;;\nesac\n"
        )
        fake.chmod(0o755)
        env = {
            **os.environ,
            "PATH": f"{self.bin}:{os.environ['PATH']}",
            "PYTHONDONTWRITEBYTECODE": "1",
        }
        p = subprocess.run(
            [sys.executable, str(self.d / "ops.py"), "preflight"],
            capture_output=True,
            text=True,
            env=env,
            timeout=60,
        )
        if p.returncode == 75:
            return None
        assert p.returncode == 0, p.stdout + p.stderr
        return json.loads(p.stdout.splitlines()[-1])["selected_offer"]


class FilterParity(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        cls.oracle = OracleOps()

    @classmethod
    def tearDownClass(cls):
        shutil.rmtree(cls.oracle.tmp)

    def test_selected_offer_matches_operator_on_every_snapshot(self):
        for snap in SNAPSHOTS:
            offers = offers_of(snap)
            got = M.decide(RULES, offers, 28.0)["selected"]
            want = self.oracle.run(offers, 28.0)
            self.assertEqual(got["id"] if got else None, want, snap)

    def test_each_offer_accept_reject_matches_operator(self):
        n = 0
        for snap in SNAPSHOTS:
            for offer in offers_of(snap):
                got = M.decide(RULES, [offer], 28.0)["selected"]
                want = self.oracle.run([offer], 28.0)
                self.assertEqual(got["id"] if got else None, want, (snap.name, offer["id"]))
                n += 1
        self.assertGreaterEqual(n, 25)

    def test_low_credit_parity(self):
        offers = offers_of(TB / "ROUND_B1f_0115/market.json")
        got = M.decide(RULES, offers, 9.0)["selected"]
        self.assertEqual(got["id"] if got else None, self.oracle.run(offers, 9.0))

    def test_rejection_counts_add_up(self):
        offers = offers_of(TB / "ROUND_B1g_0050/attempt1/market.json")
        r = M.decide(RULES, offers, 28.0)
        self.assertEqual(sum(r["rejected"].values()) + r["eligible_count"], len(offers))
        self.assertEqual(r["rejected"]["known_bad"], 2)
        self.assertEqual(r["rejected"]["base_filters"], 3)


class Queries(unittest.TestCase):
    def test_queries_are_the_operator_search_strings(self):
        calls = []

        def fake(args):
            calls.append(args)
            if args[:2] == ["show", "user"]:
                return {"credit": 28.0}
            return []

        M.poll_once(RULES, fake)
        searches = [c for c in calls if c[:2] == ["search", "offers"]]
        self.assertEqual(len(searches), 7)
        for c in searches:
            self.assertIn("cpu_cores_effective>=7 ", c[2])
            self.assertIn("vms_enabled=true num_gpus=1", c[2])
        self.assertEqual(sum(c[:2] == ["show", "user"] for c in calls), 1)


class ReadOnly(unittest.TestCase):
    def test_no_rent_or_destroy_names_reachable(self):
        for name in dir(M):
            low = name.lower()
            for bad in ("create", "destroy", "rent", "arm_watch", "save"):
                self.assertNotIn(bad, low, name)
        for bad in ("save", "arm_watch"):
            self.assertNotIn(bad, RULES.namespace)
        self.assertIs(RULES.namespace["api"], M.readonly_api)

    def test_readonly_api_refuses_mutations_before_subprocess(self):
        for args in (
            ["create", "instance", "1"],
            ["destroy", "instance", "1"],
            ["start", "instance", "1"],
            ["show", "instances"],
            ["logs", "1"],
        ):
            with self.assertRaises(M.ReadOnlyViolation):
                M.readonly_api(args, runner=lambda *a, **k: self.fail("subprocess reached"))

    def test_readonly_api_allows_search_and_user(self):
        seen = []

        def runner(argv, **kw):
            seen.append(argv)
            return subprocess.CompletedProcess(argv, 0, "[]", "")

        M.readonly_api(["search", "offers", "q"], runner=runner)
        M.readonly_api(["show", "user"], runner=runner)
        self.assertEqual(seen[0][:3], ["vastai", "search", "offers"])


class Cadence(unittest.TestCase):
    def run_watch(self, results, max_seconds=8 * 3600):
        clock = [0.0]
        sleeps = []
        it = iter(results)

        def poll():
            r = next(it, "none")
            if r == "err":
                raise M.ApiError("boom")
            return {
                "selected": r if r != "none" else None,
                "rejected": {},
                "eligible_count": 0,
                "offers": 0,
            }

        def sleep(s):
            sleeps.append(s)
            clock[0] += s

        with tempfile.TemporaryDirectory() as tmp:
            rc = M.watch(
                Path(tmp),
                poll,
                clock=lambda: clock[0],
                sleep=sleep,
                rng=random.Random(5),
                max_seconds=max_seconds,
            )
            files = sorted(p.name for p in Path(tmp).iterdir())
            log = (Path(tmp) / "market_watch.log").read_text()
        return rc, sleeps, files, log

    def test_interval_bounds(self):
        rc, sleeps, _, _ = self.run_watch(["none"] * 30, max_seconds=4 * 3600)
        self.assertEqual(rc, 3)
        self.assertTrue(sleeps)
        for s in sleeps:
            self.assertGreaterEqual(s, 1200)
            self.assertLessEqual(s, 1320)

    def test_exit_3_after_8h(self):
        rc, sleeps, _, log = self.run_watch(["none"] * 100)
        self.assertEqual(rc, 3)
        self.assertLessEqual(sum(sleeps), 8 * 3600)
        self.assertIn("EXIT 3", log)

    def test_exit_0_on_match_writes_match(self):
        offer = {
            "id": 5,
            "machine_id": 6,
            "dph_total": 0.3,
            "candidate_die": "GB206",
            "geolocation": "Quebec, CA",
        }
        rc, _, files, log = self.run_watch(["none", offer])
        self.assertEqual(rc, 0)
        self.assertIn("match.json", files)
        self.assertIn("MATCH id=5 machine=6", log)

    def test_exit_4_on_three_consecutive_errors(self):
        rc, _, _, log = self.run_watch(["err", "err", "err"])
        self.assertEqual(rc, 4)
        self.assertIn("EXIT 4", log)

    def test_errors_reset_after_success(self):
        rc, _, _, _ = self.run_watch(
            ["err", "err", "none", "err", "err", "none"], max_seconds=3 * 3600
        )
        self.assertEqual(rc, 3)

    def test_only_own_output_dir_written(self):
        _, _, files, _ = self.run_watch(["none"], max_seconds=100)
        self.assertTrue(set(files) <= {"market_watch.log", "match.json", "status.json"})

    def test_one_summary_line_per_poll(self):
        rc, sleeps, _, log = self.run_watch(["none"] * 5, max_seconds=5000)
        polls = [line for line in log.splitlines() if line.split(" ", 1)[1].startswith("POLL ")]
        self.assertEqual(len(polls), len(sleeps) + 1)


if __name__ == "__main__":
    unittest.main()
