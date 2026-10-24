"""Controller 02:30 grant: selection override (30498 default profile, 99805 7vis, then next).

Run: KF_DEF=/workspace/bw3-round-b1g-a1_m30498-run KF_7V=/workspace/bw3-round-b1g-a1_m99805-run \
     python3 -m unittest test_grant_b1g
"""

import hashlib
import json
import os
import re
from pathlib import Path
import shutil
import subprocess
import sys
import tempfile
import unittest

DEF = Path(os.environ.get("KF_DEF", "/workspace/bw3-round-b1g-a1_m30498-run"))
V7 = Path(os.environ.get("KF_7V", "/workspace/bw3-round-b1g-a1_m99805-run"))
TB = Path("/workspace/bw-native-edge-wt/traces/blackwell_ab_20261023/ROUND_B1g_0050")
ROUNDS = {"b1g": "ROUND_B1g_0050", "b1h": "ROUND_B1h_0350", "b1i": "ROUND_B1i_0400"}


def kit(d):
    m = re.search(r"bw3-round-(b1[ghi])-a(\d+_m\d+)-run$", d.name)
    return TB.parent / ROUNDS[m[1]] / ("attempt" + m[2]) / "run_kit"


KDEF = kit(DEF)
K7 = kit(V7)


def offer(oid, machine, gpu, die, cores, price=0.4, geo="Quebec, CA", ram=64000):
    return {"id": oid, "machine_id": machine, "gpu_name": gpu, "candidate_die": die,
            "verification": "verified", "dph_total": price, "reliability": 0.99,
            "inet_down": 1000, "vms_enabled": True, "cpu_cores_effective": cores,
            "cpu_ram": ram, "disk_space": 500, "is_vm_deverified": False, "geolocation": geo}


MARKET = [
    offer(1, 30498, "RTX 5070", "GB205", 32.0, 0.43),
    offer(2, 99805, "RTX 5060 Ti", "GB206", 8.0, 0.28),
    offer(3, 55678, "RTX 5090", "GB202", 32.0, 0.87),
    offer(4, 153177, "RTX 5080", "GB203", 16.0, 0.56, ram=31000),
    offer(5, 12345, "RTX 5070 Ti", "GB203", 16.0, 0.30),
]


def oracle(d, offers, credit=28.0):
    tmp = Path(tempfile.mkdtemp())
    try:
        sd = tmp / "D"
        sd.mkdir()
        media = sd / "media"
        media.mkdir()
        files = []
        for i in range(4):
            p = media / f"m{i}"
            p.write_bytes(bytes([i]) * 10)
            files.append({"file": p.name, "expected_bytes": 10,
                          "expected_sha256": hashlib.sha256(p.read_bytes()).hexdigest()})
        (sd / "media_manifest.json").write_text(json.dumps({"all_verified": True,
                                                            "files": files}))
        for name in ("owner_authority.json", "known_bad_machines.json", "lease_profile.py",
                     "pin_evidence.py", "grant_b1g.py", "grant_pin.json",
                     "dry_loop.py", "static_check.py"):
            if (d / name).exists() or name not in ("dry_loop.py", "static_check.py"):
                shutil.copy2(d / name, sd / name)
        ledger = tmp / "owned_instances.json"
        ledger.write_text("[]\n")
        src = (d / "ops.py").read_text().replace(f'D = Path("{d}")', f'D = Path("{sd}")')
        a = src.index("L = W / ")
        src = src[:a] + f'L = Path("{ledger}")' + src[src.index("\n", a):]
        (sd / "ops.py").write_text(src)
        cache = next(n for n in src.split('"') if re.match(r"single_market_B1[ghi]_", n))
        (sd / cache).write_text(json.dumps({"utc": "2026-10-24T00:00:00+00:00",
                                            "offers": offers}))
        b = tmp / "bin"
        b.mkdir()
        fake = b / "vastai"
        fake.write_text("#!/bin/sh\ncase \"$1 $2\" in\n 'show instances') echo '[]';;\n"
                        f" 'show user') echo '{{\"credit\": {credit}}}';;\n *) exit 9;;\nesac\n")
        fake.chmod(0o755)
        env = {**os.environ, "PATH": f"{b}:{os.environ['PATH']}", "PYTHONDONTWRITEBYTECODE": "1"}
        p = subprocess.run([sys.executable, str(sd / "ops.py"), "preflight"], capture_output=True,
                           text=True, env=env, timeout=60)
        if p.returncode == 75:
            return None
        assert p.returncode == 0, p.stdout + p.stderr
        return json.loads(p.stdout.splitlines()[-1])
    finally:
        shutil.rmtree(tmp)


class Selection(unittest.TestCase):
    def test_default_pin_30498_selected_with_150_minutes(self):
        r = oracle(DEF, MARKET)
        self.assertEqual(r["machine"], 30498)
        # default ceiling (150 min) minus any prior-attempt spend; never the 3 h profile ceiling
        self.assertLessEqual(r["plan"]["seconds"], 150 * 60)
        self.assertGreaterEqual(r["plan"]["seconds"], 90 * 60)

    def test_default_pin_refuses_when_30498_absent(self):
        self.assertIsNone(oracle(DEF, [m for m in MARKET if m["machine_id"] != 30498]))

    def test_default_profile_rejects_8_core_offer(self):
        self.assertIsNone(oracle(DEF, [MARKET[1]]))

    def test_7vis_pin_99805_selected_even_when_30498_present(self):
        r = oracle(V7, MARKET)
        self.assertEqual(r["machine"], 99805)
        self.assertEqual(r["plan"]["seconds"], 3 * 3600)

    def test_7vis_pin_refuses_when_99805_absent(self):
        self.assertIsNone(oracle(V7, [m for m in MARKET if m["machine_id"] != 99805]))

    def test_unpinned_eligible_offer_does_not_replace_the_pin(self):
        rest = [m for m in MARKET if m["machine_id"] not in (30498, 99805)]
        self.assertIsNone(oracle(DEF, rest))
        self.assertIsNone(oracle(V7, rest))

    def test_ram_31g_offer_never_selected(self):
        self.assertIsNone(oracle(DEF, [MARKET[3]]))


class Module(unittest.TestCase):
    def test_select_pin_and_none(self):
        sys.path.insert(0, str(DEF))
        import grant_b1g as G

        rows = [{"machine_id": 30498}, {"machine_id": 7}]
        self.assertEqual(G.select(rows, pinned=30498), [rows[0]])
        self.assertEqual(G.ORDER, (30498, 99805))

    def test_pin_files(self):
        self.assertEqual(json.loads((DEF / "grant_pin.json").read_text()),
                         {"machine": 30498, "variant": "default"})
        self.assertEqual(json.loads((V7 / "grant_pin.json").read_text()),
                         {"machine": 99805, "variant": "7vis"})


class Wiring(unittest.TestCase):
    def test_default_run_kit_is_standard(self):
        self.assertIn("WINVM_SMP=8", (KDEF / "install.sh").read_text())
        self.assertIn("WINVM_SMP=8", (KDEF / "common.sh").read_text())
        self.assertIn("CARGO_BUILD_JOBS=8", (KDEF / "controller.sh").read_text())
        self.assertIn("CARGO_BUILD_JOBS=16", (KDEF / "bind.sh").read_text())
        self.assertIn("cargo test -j8", (KDEF / "build_b1.sh").read_text())

    def test_default_operator_has_no_deviation(self):
        ops = (DEF / "ops.py").read_text()
        self.assertIn("floor = LP.offer_floor(None)", ops)
        self.assertNotIn("LP.deviation_fields(", ops)
        self.assertNotIn("max_seconds=LP.MAX_SECONDS", ops)
        self.assertIn('"lease_profile": "default"', ops)

    def test_default_offer_filter_uses_default_floor(self):
        ops = (DEF / "ops.py").read_text()
        self.assertIn("LP.offer_ok(x, None)", ops)
        self.assertNotIn("floor = LP.offer_floor(LP.PROFILE)", ops)

    def test_7vis_run_kit_keeps_profile(self):
        self.assertIn("WINVM_SMP=6", (K7 / "install.sh").read_text())
        self.assertIn("CARGO_BUILD_JOBS=7", (K7 / "controller.sh").read_text())
        ops = (V7 / "ops.py").read_text()
        self.assertIn("floor = LP.offer_floor(LP.PROFILE)", ops)
        self.assertIn("GRANT.select(eligible)", ops)

    def test_both_judge_preflight(self):
        for k in (KDEF, K7):
            self.assertIn("python3 /root/sweep2/preflight_profile.py", (k / "controller.sh").read_text())


if __name__ == "__main__":
    unittest.main()
