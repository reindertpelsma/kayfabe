"""Run the box-side Python heredocs of the B1n arm scripts against a live owned dummy process.

The heredocs run only on the rental; here each one is extracted verbatim, its fixed /root paths
are mapped into a temporary root, and it runs against a real `sleep` child (pid, starttime and
exe are real), so a wrong name or field fails here and not on a paid lease.
"""

import datetime as dt
import json
import os
from pathlib import Path
import shutil
import subprocess
import sys
import tempfile
import unittest

HERE = Path(__file__).resolve().parent
K = Path(os.environ.get("KF_BW4_K", HERE.parent / "attempt1" / "run_kit"))
BW = Path(os.environ.get("KF_BW4_BW", HERE.parents[3] / "scripts/bench/blackwell"))
sys.path.insert(0, str(BW))
sys.path.insert(0, str(K))

import b1n_arms  # noqa: E402
import b1n_authority  # noqa: E402
import saved_xid32_observation as O  # noqa: E402


def heredoc(script, tag):
    text = (K / script).read_text()
    start = text.index("<<'" + tag + "'\n") + len(tag) + 4
    return text[start : text.index("\n" + tag + "\n", start)]


def mapped(code, root):
    code = code.replace("/root/sweep2/guard/blackwell", str(BW))
    return code.replace("/root/prov", str(root / "prov")).replace("/root/sweep2", str(root / "s2"))


class Snippets(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory()
        self.root = Path(self.tmp.name)
        (self.root / "prov" / "arms").mkdir(parents=True)
        (self.root / "s2").mkdir()
        shutil.copy2(K / "b1n_arms.py", self.root / "s2" / "b1n_arms.py")
        (self.root / "prov" / "pci.txt").write_text("0000:00:07\n")
        (self.root / "prov" / "probed_die.json").write_text(json.dumps({"die": "GB205"}))
        self.child = subprocess.Popen(["sleep", "60"])
        proc = Path("/proc") / str(self.child.pid)
        self.phase = {
            "pid": self.child.pid,
            "starttime": int((proc / "stat").read_text().rpartition(")")[2].split()[19]),
            "exe": os.path.realpath(proc / "exe"),
            "pci": "0000:00:07",
        }

    def tearDown(self):
        self.child.kill()
        self.child.wait()
        self.tmp.cleanup()

    def run_code(self, code, *args):
        path = self.root / "snippet.py"
        path.write_text(mapped(code, self.root))
        return subprocess.run(
            [sys.executable, str(path), *args], capture_output=True, text=True, timeout=30
        )

    def test_observation_permit_heredoc_arms_every_windows_arm(self):
        now = dt.datetime.now(dt.timezone.utc)
        authority = b1n_authority.build(
            lease_id=57500001,
            die="GB205",
            start=now - dt.timedelta(minutes=5),
            end=now + dt.timedelta(minutes=140),
            table=O.DIE_PAIRS,
        )
        (self.root / "s2" / "observation_authority.json").write_text(json.dumps(authority))
        code = heredoc("arm.sh", "OBSERVATION_PERMIT")
        for run in b1n_arms.windows_arms():
            (self.root / "prov" / "arms" / run).mkdir(exist_ok=True)
            phase = {"arm": run, **self.phase, "rpc_path": None}
            (self.root / "prov" / "phase.json").write_text(json.dumps(phase))
            r = self.run_code(code, "on", run)
            self.assertEqual(r.returncode, 0, run + r.stderr[-800:])
            self.assertIn("WINDOWS_ON_OBSERVATION_ARMED", r.stdout)
            permit = json.loads(
                (self.root / "prov" / "arms" / run / "observation_permit.json").read_text()
            )
            want = b1n_arms.find(run)
            self.assertEqual(permit["probed_die"], "GB205")
            self.assertEqual(permit["enable_count"], want["enable_count"])
            self.assertEqual(permit["label"], want["label"])
            self.assertEqual(permit["mode"], O.MODE)

    def test_observation_permit_heredoc_refuses_a_die_outside_the_authority(self):
        now = dt.datetime.now(dt.timezone.utc)
        authority = b1n_authority.build(
            lease_id=57500001,
            die="GB205",
            start=now - dt.timedelta(minutes=5),
            end=now + dt.timedelta(minutes=140),
            table=O.DIE_PAIRS,
        )
        (self.root / "s2" / "observation_authority.json").write_text(json.dumps(authority))
        (self.root / "prov" / "probed_die.json").write_text(json.dumps({"die": "GB202"}))
        (self.root / "prov" / "arms" / "on_urt").mkdir()
        phase = {"arm": "on_urt", **self.phase, "rpc_path": None}
        (self.root / "prov" / "phase.json").write_text(json.dumps(phase))
        r = self.run_code(heredoc("arm.sh", "OBSERVATION_PERMIT"), "on", "on_urt")
        self.assertNotEqual(r.returncode, 0)
        self.assertFalse((self.root / "prov" / "observation_permit.json").exists())

    def test_linux_own_phase_heredoc_registers_the_live_process(self):
        r = self.run_code(
            heredoc("linux_reload.sh", "OWN_PHASE"), str(self.child.pid), "/usr/bin/sleep"
        )
        self.assertEqual(r.returncode, 0, r.stderr[-800:])
        phase = json.loads((self.root / "prov" / "phase.json").read_text())
        self.assertEqual(phase["arm"], "lx_reload")
        self.assertEqual(phase["pid"], self.child.pid)
        self.assertEqual(phase["starttime"], self.phase["starttime"])
        self.assertNotIn("expected_pci", phase)

    def test_linux_own_phase_heredoc_refuses_a_wrong_binary(self):
        r = self.run_code(
            heredoc("linux_reload.sh", "OWN_PHASE"), str(self.child.pid), "/usr/bin/true"
        )
        self.assertNotEqual(r.returncode, 0)
        self.assertFalse((self.root / "prov" / "phase.json").exists())

    def test_arm_plan_line_reads_back_in_bash(self):
        code = 'read -r a b c d < <(python3 "$1" prepare on_tdr60); echo "[$a][$b][$c][$d]"'
        script = mapped((K / "b1n_arms.py").read_text(), self.root)
        (self.root / "b.py").write_text(script)
        r = subprocess.run(
            ["bash", "-c", code, "x", str(self.root / "b.py")], capture_output=True, text=True
        )
        self.assertEqual(r.stdout.strip(), "[enable580_once.ps1][1][prep_tdr60.ps1][DIAGNOSTIC]")
        self.assertTrue((self.root / "prov" / "arm_cfg_on_tdr60.json").exists())


if __name__ == "__main__":
    unittest.main()
