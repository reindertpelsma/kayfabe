"""B1m: this lease's market search covers the GeForce RTX 5090 (GB202) only."""

import ast
import os
from pathlib import Path
import unittest

D = Path(os.environ.get("KF_BW3_D", "/workspace/bw3-round-b1m-a1_rtx5090-run"))


class Scope(unittest.TestCase):
    def skus(self):
        tree = ast.parse((D / "ops.py").read_text())
        node = next(n for n in ast.walk(tree) if isinstance(n, ast.Assign)
                    and ast.unparse(n.targets[0]) == "skus")
        return ast.literal_eval(node.value)

    def test_only_rtx5090(self):
        self.assertEqual(self.skus(), {"RTX_5090": "GB202"})

    def test_raised_budget_wired(self):
        ops = (D / "ops.py").read_text()
        self.assertIn("BO.plan(", ops)
        self.assertIn("**BO.fields()", ops)


if __name__ == "__main__":
    unittest.main()
