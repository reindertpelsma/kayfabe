# SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later
import json
import os
from pathlib import Path
import unittest
import metric


@unittest.skipUnless(os.environ.get('N1_TRANSPORT_RECEIPT'), 'real Windows PowerShell receipt required')
class RealPowerShellTransport(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        cls.receipt=json.loads(Path(os.environ['N1_TRANSPORT_RECEIPT']).read_text(encoding='utf-8-sig'))

    def test_production_header_is_plain_string(self):
        self.assertIsInstance(self.receipt['meta']['pm_header'],str)
        self.assertEqual(self.receipt['meta']['pm_header'],'Application,ProcessID,TimeInQPC')

    def test_production_csv_has_header(self):
        # Before the fix this deliberately raises PresentMon header absent.
        rows,skipped=metric.read_csv(self.receipt['csv'].splitlines())
        self.assertEqual(rows,[(1000,'Heaven.exe',7)])
        self.assertEqual(skipped,0)

    def test_other_decorated_values_are_guarded(self):
        self.assertIsNotNone(self.receipt['guarded'])
        self.assertIsInstance(self.receipt['guarded']['label'],str)
        self.assertEqual(self.receipt['guarded']['items'],['Application,ProcessID,TimeInQPC'])

    def test_metadata_is_never_exported(self):
        def visit(v):
            if isinstance(v,dict):
                self.assertFalse(set(v)&{'PSPath','PSParentPath','PSChildName','PSDrive','PSProvider','ReadCount'})
                for x in v.values(): visit(x)
            elif isinstance(v,list):
                for x in v: visit(x)
        visit(self.receipt['payload'])


if __name__=='__main__':
    unittest.main()
