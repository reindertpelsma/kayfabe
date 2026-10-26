import importlib.util
import io
import json
from pathlib import Path
import sys
import types
import unittest

SRC = Path(__file__).resolve().parent
sys.path.insert(0, str(SRC))
source = (SRC / 'e5_gate.py').read_text()
mutations = {
    'never_accept_ab': ("ab_verified=not reasons", "ab_verified=False"),
    'ignore_allocation_readback': (
        "if allocation.get('compr', 0) not in (1, 3) or allocation.get('format') not in (8, 9):",
        "if False:"),
    'ignore_coverage': (
        "if (len(coverage) != 1 or coverage[0].get('format') != 1\n"
        "            or not 0 < coverage[0].get('line_min', 0)\n"
        "            <= coverage[0].get('line_max', -1) < 0xffffffff):",
        "if False:"),
    'ignore_matrix_completeness': (
        "if not ptes:", "if not ptes and not maps:"),
    'ignore_pte_kind': (
        "and r.get('page') == page and r.get('kind') == kind",
        "and r.get('page') == page"),
    'ignore_pte_valid_and_page_size': (
        "and r.get('page') == page and r.get('kind') == kind\n"
        "                    and r.get('flags', 0) & 1",
        "and r.get('kind') == kind"),
    'ignore_rm_failures': (
        "if row.get('rc') != 0 or row.get('status') != 0:", "if False:"),
    'ignore_clean_finish': (
        "if len(finishes) != 1 or finishes[0].get('failures') != 0:", "if False:"),
    'ignore_zbc_policy': (
        "zbc_preference_retained=allocation.get('zbc') == 1", "zbc_preference_retained=True"),
    'allow_root_or_missing_uid': (
        "if len(starts) != 1 or starts[0].get('uid', 0) == 0:", "if False:"),
    'certify_task2_without_c_d': ("task2_eligible=False", "task2_eligible=True"),
    'certify_zbc_without_producer': (
        "zbc_suppression_verified=False", "zbc_suppression_verified=True"),
}
results = {}
for name, (old, new) in mutations.items():
    if source.count(old) != 1:
        raise SystemExit(f'{name}: mutation target missing or not unique')
    module = types.ModuleType('e5_gate')
    exec(compile(source.replace(old, new), name, 'exec'), module.__dict__)
    sys.modules['e5_gate'] = module
    spec = importlib.util.spec_from_file_location('test_e5_gate', SRC / 'test_e5_gate.py')
    tests = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(tests)
    suite = unittest.defaultTestLoader.loadTestsFromModule(tests)
    result = unittest.TextTestRunner(stream=io.StringIO()).run(suite)
    killed = [test.id() for test, _ in result.failures + result.errors]
    results[name] = dict(kill_count=len(killed), killing_tests=killed)
    print(f'{name}: {len(killed)} killing tests')
Path('gate-mutations.json').write_text(
    json.dumps(results, indent=2) + '\n')
if any(row['kill_count'] < 2 for row in results.values()):
    raise SystemExit('mutation kill requirement not met')
