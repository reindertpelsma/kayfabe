import importlib.util
from pathlib import Path
import subprocess
import sys
import unittest


class OwnedAbort(unittest.TestCase):
    def check_abort(self, xid):
        spec = importlib.util.spec_from_file_location("hw2_session", Path(__file__).with_name("hw2_session.py"))
        mod = importlib.util.module_from_spec(spec)
        spec.loader.exec_module(mod)
        own = subprocess.Popen([sys.executable, "-c", "import time;time.sleep(30)"])
        foreign = subprocess.Popen([sys.executable, "-c", "import time;time.sleep(30)", "kf-fastguest"])
        try:
            reason, _ = mod.monitor_owned(own, 0.02, lambda: int(xid), 0, lambda: None)
            self.assertEqual(reason, "new_xid" if xid else "deadline")
            self.assertIsNotNone(own.poll(), "the owned QEMU must stop")
            self.assertIsNone(foreign.poll(), "a matching foreign process must survive")
        finally:
            for child in (own, foreign):
                if child.poll() is None:
                    child.terminate()
                child.wait()

    def test_xid_abort_stops_only_owned_child(self):
        self.check_abort(True)

    def test_deadline_stops_only_owned_child(self):
        self.check_abort(False)


if __name__ == "__main__":
    unittest.main()
