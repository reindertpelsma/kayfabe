import unittest
import os, sys
sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import fp_table as ft

TXT = ("PC1\tfiles\tpair1\thit=0\tstarved=- 0.0s\tgaps=A:0.01,B:1.57\titers=A:1,B:2\n"
       "PC1\tfiles\tTOTAL\t0/2\n"
       "fp1_C\tfiles\tpair1\thit=1\tstarved=A 15.4s\tgaps=A:15.38,B:2.40\titers=A:1,B:2\n"
       "SC3\tfiles\tpair2\thit=0\tstarved=- 0.0s\tgaps=A:0.01,B:0.25\titers=A:1,B:2\n")


class T(unittest.TestCase):
    def test_arms(self):
        t = {r[0]: r for r in ft.table(ft.parse(TXT))}
        self.assertEqual(t["PC"], ("PC", 2, 2, 1, 1, 15.38))
        self.assertEqual(t["SC"], ("SC", 1, 0, 0, 0, 0.25))


if __name__ == "__main__":
    unittest.main()
