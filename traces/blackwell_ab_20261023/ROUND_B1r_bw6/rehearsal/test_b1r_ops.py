"""BW6 grant regression oracle; the inherited BW5 operator deliberately fails this suite."""
import ast
import hashlib
import json
import os
from pathlib import Path
import shutil
import subprocess
import sys
import tempfile
import unittest

HERE = Path(__file__).resolve().parent
REPO = HERE.parents[3]
TARGET = REPO / 'traces/blackwell_ab_20261023/ROUND_B1r_bw6/attempt1/run_kit'
BASE = REPO / 'traces/blackwell_ab_20261023/ROUND_B1q_bw5/attempt1/run_kit'
K = Path(os.environ.get('KF_BW3_K', TARGET if TARGET.exists() else BASE))
D = Path(os.environ.get('KF_BW3_D', '/workspace/bw6-round-b1r-a1-run'))
FIX = '007ba1eb03db9e21d351e465e95b46cbc3134a95'


def source():
    return (K / 'operator_ops.py').read_text()


def literal(name, text=None):
    for node in ast.walk(ast.parse(text or source())):
        if isinstance(node, ast.Assign) and getattr(node.targets[0], 'id', None) == name:
            return ast.literal_eval(node.value)
    raise AssertionError(name)


def offer(**change):
    return dict(id=1, machine_id=987654, gpu_name='RTX 5090', candidate_die='GB202',
                verification='verified', dph_total=0.8, reliability=0.99, inet_down=500,
                vms_enabled=True, is_vm_deverified=False, cpu_ram=65536,
                cpu_cores_effective=16, disk_space=200, geolocation='The Netherlands, NL',
                **change)


def oracle(rows, credit=25):
    """Run the real preflight with synthetic media and a fake read-only provider."""
    with tempfile.TemporaryDirectory() as tmp:
        root = Path(tmp)
        sd = root / 'D'
        sd.mkdir()
        for p in D.glob('*.py'):
            shutil.copy2(p, sd / p.name)
        for name in ('owner_authority.json', 'known_bad_machines.json', 'grant_pin.json'):
            shutil.copy2(D / name, sd / name)
        (sd / 'media').mkdir()
        files = []
        for i in range(4):
            p = sd / 'media' / str(i)
            p.write_bytes(bytes([i]))
            files.append(dict(file=p.name, expected_bytes=1,
                              expected_sha256=hashlib.sha256(p.read_bytes()).hexdigest()))
        (sd / 'media_manifest.json').write_text(json.dumps(dict(all_verified=True, files=files)))
        src = source()
        src = src.replace(str(D), str(sd)).replace('/workspace/bw5-wt', str(REPO))
        src = src.replace('/workspace/bw6-round-b1r-a1-run', str(sd))
        begin = src.index('L = W / ')
        src = src[:begin] + 'L = Path(' + repr(str(root / 'owned.json')) + ')' + src[src.index('\n', begin):]
        (root / 'owned.json').write_text('[]\n')
        (sd / 'ops.py').write_text(src)
        cache = next(s for s in src.split('"') if s.startswith('single_market_'))
        (sd / cache).write_text(json.dumps(dict(utc='2026-10-24T18:00:00Z', offers=rows)))
        provider = root / 'vastai'
        provider.write_text('#!/bin/sh\ncase "$1 $2" in\n'
                            "'show instances') echo '[]';;\n"
                            "'show user') echo '{\"credit\":" + str(credit) + "}';;\n"
                            '*) exit 99;;\nesac\n')
        provider.chmod(0o755)
        env = dict(os.environ, PATH=str(root)+':'+os.environ['PATH'],
                   PYTHONDONTWRITEBYTECODE='1')
        r = subprocess.run([sys.executable, str(sd/'ops.py'), 'preflight'],
                           capture_output=True, text=True, env=env, timeout=30)
        if r.returncode == 75:
            return None
        if r.returncode:
            raise AssertionError(r.stderr[-1500:])
        return json.loads(r.stdout.splitlines()[-1])


class Grant(unittest.TestCase):
    def test_sku_is_only_geforce_5090(self):
        self.assertEqual(literal('skus'), {'RTX_5090': 'GB202'})

    def test_no_gb205_search(self):
        self.assertNotIn('RTX_5070', source())

    def test_budget_plan_uses_three_dollar_stop(self):
        result = oracle([offer()])
        self.assertIsNotNone(result)
        self.assertEqual(result['plan']['stop_usd'], 3.0)
        self.assertEqual(result['plan']['cap_usd'], 3.5)

    def test_qualifying_offer_not_pinned_to_old_machine(self):
        self.assertEqual(oracle([offer()])['machine'], 987654)

    def test_above_one_dollar_refuses(self):
        row = offer(); row['dph_total'] = 1.0001
        self.assertIsNone(oracle([row]))

    def test_price_1_20_refuses(self):
        row = offer(); row['dph_total'] = 1.2
        self.assertIsNone(oracle([row]))

    def test_reserve_untouched(self):
        self.assertIsNone(oracle([offer()], credit=9))

    def test_denied_countries_refuse(self):
        for country in ('CU', 'IR', 'KP', 'SY', 'CN', 'HK', 'MO'):
            row = offer(); row['geolocation'] = ', '+country
            with self.subTest(country=country):
                self.assertIsNone(oracle([row]))

    def test_deverified_refuses(self):
        for key, value in (('verification', 'deverified'), ('is_vm_deverified', True)):
            row = offer(); row[key] = value
            with self.subTest(key=key):
                self.assertIsNone(oracle([row]))

    def test_core_ram_and_network_guards(self):
        for key, value in (('cpu_cores_effective', 15), ('cpu_ram', 32000), ('inet_down', 199)):
            row = offer(); row[key] = value
            with self.subTest(key=key):
                self.assertIsNone(oracle([row]))

    def test_plan_at_most_three_hours(self):
        result = oracle([offer()])
        self.assertIsNotNone(result)
        self.assertLessEqual(result['plan']['seconds'], 10800)
        self.assertLess(result['plan']['reservation_usd'], 3.0)

    def test_readiness_enforces_price_and_lease_stop(self):
        self.assertIn('< BO.STOP_USD', source())
        self.assertIn('row["observed_dph_total"] <= BO.MAX_RATE', source())

    def test_receipt_records_grant(self):
        self.assertIn('"budget_stop": BO.STOP_USD', source())
        self.assertIn('"phase_cap": BO.CAP_USD', source())
        self.assertIn('**BO.fields()', source())

    def test_label_and_paths_are_bw6(self):
        self.assertIn('"codex-bw6-B1r1-20261024"', source())
        self.assertIn('ROUND_B1r_bw6/attempt1', source())

    def test_only_on_fix_scheduled(self):
        arms = literal('ARMS', (K/'b1n_arms.py').read_text())
        self.assertEqual([a['run'] for a in arms], ['on_fix'])
        self.assertEqual(arms[0]['enable_count'], 1)
        self.assertEqual(arms[0]['env'], {})

    def test_controller_runs_one_cell(self):
        self.assertIn('[ "${#plan[@]}" = 1 ]', (K/'controller.sh').read_text())

    def test_watchdog_precedes_create_and_one_attempt(self):
        text = source()
        self.assertLess(text.index('armed["state"] == "ARMED"'),
                        text.index('"create",\n            "instance"'))
        self.assertIn('state["attempts_used"] == 0', text)
        self.assertIn('assert L.read_text().strip() == "[]"', text)

    def test_settled_identity_and_fixed_binary_preserved(self):
        text = source()
        self.assertIn('proof = C.settle_pin(', text)
        self.assertIn('alias + " ssh-ed25519 " + proof["key"]', text)
        self.assertIn(FIX, text)
        self.assertEqual((K/'p19.flags').read_bytes(), (BASE/'p19.flags').read_bytes())


if __name__ == '__main__':
    unittest.main()
