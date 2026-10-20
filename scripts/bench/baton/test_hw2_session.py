import importlib.util
import os
from pathlib import Path
import select
import signal
import subprocess
import sys
import unittest


class OwnedAbort(unittest.TestCase):
    def check_abort(self, xid, nested=False):
        spec = importlib.util.spec_from_file_location("hw2_session", Path(__file__).with_name("hw2_session.py"))
        mod = importlib.util.module_from_spec(spec)
        spec.loader.exec_module(mod)
        body = "import time;time.sleep(30)"
        if nested:
            body = "import subprocess,sys,time;c=subprocess.Popen([sys.executable,'-c','import time;time.sleep(30)']);print(c.pid,flush=True);time.sleep(30)"
        own = subprocess.Popen([sys.executable, "-c", body], stdout=subprocess.PIPE)
        descendant = os.pidfd_open(int(own.stdout.readline())) if nested else None
        foreign = subprocess.Popen([sys.executable, "-c", "import time;time.sleep(30)", "kf-fastguest"])
        try:
            reason, _ = mod.monitor_owned(own, 0.02, lambda: int(xid), 0, lambda: None)
            self.assertEqual(reason, "new_xid" if xid else "deadline")
            self.assertIsNotNone(own.poll(), "the owned QEMU must stop")
            self.assertIsNone(foreign.poll(), "a matching foreign process must survive")
            if descendant is not None:
                poll = select.poll()
                poll.register(descendant, select.POLLIN)
                self.assertTrue(poll.poll(1000), "the owned descendant must stop too")
        finally:
            if descendant is not None:
                try:
                    signal.pidfd_send_signal(descendant, signal.SIGTERM)
                except ProcessLookupError:
                    pass
                os.close(descendant)
            for child in (own, foreign):
                if child.poll() is None:
                    child.terminate()
                child.wait()
            own.stdout.close()

    def test_xid_abort_stops_only_owned_child(self):
        self.check_abort(True)

    def test_deadline_stops_only_owned_child(self):
        self.check_abort(False)

    def test_xid_abort_stops_owned_descendant(self):
        self.check_abort(True, True)

    def test_deadline_stops_owned_descendant(self):
        self.check_abort(False, True)


if __name__ == "__main__":
    unittest.main()
