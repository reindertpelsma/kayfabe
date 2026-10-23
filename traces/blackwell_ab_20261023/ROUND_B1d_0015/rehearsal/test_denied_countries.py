"""Operator check: the lease's country filter rejects every owner-denied country (00:30 ruling).

Run: KF_BW3_OPS=/path/to/ops.py python3 -m unittest test_denied_countries
The filter is the inline offer comprehension in the operator's rent/preflight branch; this test
re-executes exactly that source expression against fixture geolocations.
"""

import ast
import os
from pathlib import Path
import re
import unittest

OPS = Path(os.environ.get("KF_BW3_OPS", "/workspace/bw3-round-b1d-run/ops.py"))
DENIED = {"CU", "IR", "KP", "SY", "CN", "HK"}


def country_filter():
    source = OPS.read_text()
    tree = ast.parse(source)
    matches = []
    for node in ast.walk(tree):
        if isinstance(node, ast.ListComp) and "re.search" in ast.unparse(node):
            matches.append(node)
    if len(matches) != 1:
        raise AssertionError("expected exactly one geolocation filter, found %d" % len(matches))
    expression = ast.Expression(matches[0])
    code = compile(expression, str(OPS), "eval")
    return lambda offers: eval(code, {"re": re}, {"offers": offers})


def offer(geo):
    return {"geolocation": geo}


class DeniedCountries(unittest.TestCase):
    def test_owner_denied_countries_rejected(self):
        keep = country_filter()
        for code in sorted(DENIED):
            for geo in [", " + code, "Somewhere, " + code, "City,  " + code + " "]:
                self.assertEqual(keep([offer(geo)]), [], geo)

    def test_china_and_hong_kong_rejected(self):
        keep = country_filter()
        rows = [offer(", CN"), offer("Hong Kong, HK"), offer("Shanghai, CN")]
        self.assertEqual(keep(rows), [])

    def test_named_candidates_kept(self):
        keep = country_filter()
        rows = [offer("Quebec, CA"), offer("Czechia, CZ"), offer("Minnesota, US")]
        self.assertEqual(keep(rows), rows)

    def test_missing_country_rejected(self):
        keep = country_filter()
        self.assertEqual(keep([offer(""), offer("Nowhere")]), [])


if __name__ == "__main__":
    unittest.main()
