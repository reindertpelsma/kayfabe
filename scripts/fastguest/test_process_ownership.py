"""GPU-free regression: startup must preserve unrelated matching processes."""
import os
from pathlib import Path
import subprocess
import sys
import tempfile
import unittest

RUNNER = Path(__file__).with_name("run_fast_guest.sh")


class ProcessOwnership(unittest.TestCase):
    def check_foreign_survives(self, timeout):
        with tempfile.TemporaryDirectory() as tmp:
            p = Path(tmp)
            (p / "bin").mkdir()
            (p / "fg").mkdir()
            for f in ["vmlinuz", "initrd.cpio.gz"]:
                (p / "fg" / f).touch()
            # Model process discovery safely: this pkill double can signal ONLY
            # the sentinel this test created, never any process on the machine.
            helpers = {
                "pkill": "import os,signal\nos.kill(int(os.environ['SENTINEL_PID']),signal.SIGTERM)\n",
                "nvidia-smi": "print(0)\n",
                "qemu": "import os,sys,time\nfrom pathlib import Path\n"
                "if os.environ['FAKE_TIMEOUT']=='1': time.sleep(30)\n"
                "for a in sys.argv:\n"
                " if a.startswith('file,id=kfcon0,path='):\n"
                "  Path(a.split('path=',1)[1]).write_text('FASTGUEST: client rc=0\\nFASTGUEST: DONE\\n')\n",
            }
            for name, body in helpers.items():
                f = p / "bin" / name
                f.write_text("#!" + sys.executable + "\n" + body)
                f.chmod(0o755)
            child = subprocess.Popen([sys.executable, "-c", "import time; time.sleep(60)", "kf-fastguest"])
            try:
                env = dict(os.environ, PATH=str(p / "bin") + os.pathsep + os.environ["PATH"],
                           BENCH_DIR=str(p), KF_FASTGUEST_DIR=str(p / "fg"),
                           QEMU_BIN=str(p / "bin/qemu"), KF_LOCK=str(p / "lock"),
                           KF3_FB_MB="8192", KF_REQUIRE_FORWARD="0",
                           SENTINEL_PID=str(child.pid), FAKE_TIMEOUT=str(int(timeout)))
                r = subprocess.run(["bash", str(RUNNER), "ownership", "1" if timeout else "5"],
                                   env=env, capture_output=True, text=True, timeout=15)
                self.assertEqual(child.poll(), None, "launcher signalled an unrelated matching process")
                self.assertIn("FAST_VERDICT=CRASH" if timeout else "FAST_VERDICT=PASS", r.stdout)
            finally:
                if child.poll() is None:
                    child.terminate()
                child.wait()

    def test_normal_boot_preserves_foreign_process(self):
        self.check_foreign_survives(False)

    def test_budget_expiry_preserves_foreign_process(self):
        self.check_foreign_survives(True)


if __name__ == "__main__":
    unittest.main()
