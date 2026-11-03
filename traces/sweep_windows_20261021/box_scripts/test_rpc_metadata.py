"""Regression: real Other(n) trace records count toward every RPC ordinal."""
import ast
from pathlib import Path
import re
import unittest

HERE = Path(__file__).parent

def pattern(name, variable):
    for node in ast.walk(ast.parse((HERE/name).read_text())):
        if isinstance(node, ast.Assign) and any(isinstance(x, ast.Name) and x.id == variable for x in node.targets):
            return re.compile(ast.literal_eval(node.value.args[0]))
    raise AssertionError('production trace pattern missing')

OTHER = 'kf-rm: rpc-trace fn=4 Other(4) seq=0  result=0x0'
ALLOC = 'kf-rm: rpc-trace fn=103 RmAlloc seq=0 class=0xca7a result=0x0'

class Metadata(unittest.TestCase):
    def test_export_other4_is_a_real_rpc(self):
        m=pattern('export.py','TR').search(OTHER)
        self.assertIsNotNone(m)
        self.assertEqual(m[2], 'Other(4)')

    def test_export_cursor_ordinal_counts_other_records(self):
        p=pattern('export.py','TR')
        self.assertEqual([m[1] for l in [OTHER,OTHER,ALLOC] if (m:=p.search(l))], ['4','4','103'])

    def test_observer_other4_is_a_real_rpc(self):
        m=pattern('rpc_timing.py','pat').search(OTHER)
        self.assertIsNotNone(m)
        self.assertEqual(m[2], 'Other(4)')

    def test_observer_cursor_ordinal_counts_other_records(self):
        p=pattern('rpc_timing.py','pat')
        self.assertEqual([m[1] for l in [OTHER,OTHER,ALLOC] if (m:=p.search(l))], ['4','4','103'])

if __name__ == '__main__':
    unittest.main()
