"""GPU-free publication mutants; run from the repository root."""
import ast
import hashlib
import importlib
import io
import json
from pathlib import Path
import sys
import unittest
from unittest.mock import patch

root = Path(__file__).resolve().parents[3]
sys.path.insert(0, str(root / 'scripts/bench/blackwell'))
import phase_xid_guard as G
import test_fatal_receipt as T

source = Path(G.__file__).read_text()
original_sha = hashlib.sha256(source.encode()).hexdigest()
main = next(node for node in ast.parse(source.replace('record_fatal(a.fatal,', 'atomic_json(a.fatal,')).body if isinstance(node, ast.FunctionDef) and node.name == 'main')
namespace = dict(G.__dict__)
exec(compile(ast.Module(body=[main], type_ignores=[]), '<mutant-main-call>', 'exec'), namespace)
mutants = [('replace_prior_cause', 'record_fatal', G.atomic_json), ('omit_new_fatal_receipt', 'record_fatal', lambda *args: None), ('bypass_first_writer_in_watch', 'main', namespace['main'])]
rows = []
for name, target, replacement in mutants:
    suite = unittest.defaultTestLoader.loadTestsFromModule(T)
    with patch.object(G, target, replacement):
        result = unittest.TextTestRunner(stream=io.StringIO()).run(suite)
    killers = sorted({test.id().split('.')[-1] for test, _ in result.failures + result.errors})
    assert len(killers) >= 2, (name, killers)
    rows.append({'mutant': name, 'killers': killers})
assert hashlib.sha256(Path(G.__file__).read_bytes()).hexdigest() == original_sha
out = Path(__file__).with_name('mutations.json')
out.write_text(json.dumps({'mutants': rows, 'restored_source_sha256': original_sha}, indent=2) + '\n')
print(json.dumps(rows, indent=2))
