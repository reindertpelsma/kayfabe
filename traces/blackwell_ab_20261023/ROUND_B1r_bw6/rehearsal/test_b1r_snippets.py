"""B1n box-heredoc tests run on the B1p kit; the two plan-specific ones name B1p arms (BW5)."""

import datetime as dt
import json
import os
from pathlib import Path
import subprocess
import sys
import unittest

HERE = Path(__file__).resolve().parent
K = Path(os.environ.get("KF_BW3_K", HERE.parent / "attempt1" / "run_kit"))
os.environ["KF_BW4_K"] = str(K)
sys.path.insert(0, str(HERE.parent.parent / "ROUND_B1n_bw4" / "rehearsal"))

import test_b1n_snippets as S  # noqa: E402


class Snippets(S.Snippets):
    def test_observation_permit_heredoc_refuses_a_die_outside_the_authority(self):
        now = dt.datetime.now(dt.timezone.utc)
        authority = S.b1n_authority.build(
            lease_id=57500002,
            die="GB205",
            start=now - dt.timedelta(minutes=5),
            end=now + dt.timedelta(minutes=140),
            table=S.O.DIE_PAIRS,
        )
        (self.root / "s2" / "observation_authority.json").write_text(json.dumps(authority))
        (self.root / "prov" / "probed_die.json").write_text(json.dumps({"die": "GB202"}))
        (self.root / "prov" / "arms" / "on_fix").mkdir()
        phase = {"arm": "on_fix", **self.phase, "rpc_path": None}
        (self.root / "prov" / "phase.json").write_text(json.dumps(phase))
        r = self.run_code(S.heredoc("arm.sh", "OBSERVATION_PERMIT"), "on", "on_fix")
        self.assertNotEqual(r.returncode, 0)
        self.assertIn("probed die", r.stderr)
        self.assertFalse((self.root / "prov" / "observation_permit.json").exists())

    def test_arm_plan_line_reads_back_in_bash(self):
        script = S.mapped((K / "b1n_arms.py").read_text(), self.root)
        (self.root / "b.py").write_text(script)
        for run, want in (
            ("on_fix", "[enable580_once.ps1][1][-][MEASUREMENT]"),
        ):
            code = f'read -r a b c d < <(python3 "$1" prepare {run}); echo "[$a][$b][$c][$d]"'
            r = subprocess.run(
                ["bash", "-c", code, "x", str(self.root / "b.py")], capture_output=True, text=True
            )
            self.assertEqual(r.stdout.strip(), want, run)
            self.assertTrue((self.root / "prov" / f"arm_cfg_{run}.json").exists())
            self.assertEqual((self.root / "prov" / f"arm_env_{run}.sh").read_text(), "")


if __name__ == "__main__":
    unittest.main()
