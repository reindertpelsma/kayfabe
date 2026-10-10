# SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later
import os
import tempfile
import unittest

import helpers as H
import mock_guest
import qga


class QgaAgainstMock(unittest.TestCase):
    def setUp(self):
        self.w = mock_guest.MockWorld(H.load_apps(), {}, tempfile.mkdtemp(prefix="kfwa-q-"))
        self.vm = self.w.new_vm()
        self.vm.launch("t")
        self.q = qga.Qga(self.vm.qga_path(), io_timeout=0.5)

    def tearDown(self):
        self.q.close()
        self.vm.stop()

    def test_ping_and_sync(self):
        self.assertTrue(self.q.ping())

    def test_file_roundtrip_small_and_large(self):
        self.q.file_write(r"C:\a.txt", b"hello")
        self.assertEqual(self.q.file_read(r"C:\a.txt"), b"hello")
        big = os.urandom(300_000)
        self.q.file_write(r"C:\big.bin", big)
        self.assertEqual(self.q.file_read(r"C:\big.bin"), big)

    def test_large_file_is_read_as_head_and_tail(self):
        data = b"H" * 2000 + b"-" * 600_000 + b"T" * 2000
        self.q.file_write(r"C:\log.txt", data)
        got = self.q.file_read(r"C:\log.txt", head=1000, tail=1000)
        self.assertTrue(got.startswith(b"H" * 1000))
        self.assertTrue(got.endswith(b"T" * 1000))
        self.assertIn(b"elided", got)

    def test_exec_net_user(self):
        r = self.q.exec(r"C:\Windows\System32\net.exe", ["user", "vast", "pw"], timeout=5)
        self.assertEqual(r.exitcode, 0)
        self.assertFalse(r.timed_out)

    def test_missing_file_is_a_command_error_not_a_transport_error(self):
        self.assertFalse(self.q.file_exists(r"C:\nope"))
        with self.assertRaises(qga.QgaCommandError):
            self.q.file_read(r"C:\nope")

    def test_wedged_agent_times_out(self):
        self.vm.state.wedged = True
        self.assertFalse(self.q.ping(0.3))
        with self.assertRaises(qga.QgaTimeout):
            self.q.call("guest-file-open", {"path": r"C:\x", "mode": "rb"}, retry=False)

    def test_reconnect_after_the_server_drops_us(self):
        self.assertTrue(self.q.ping())
        self.vm.state.down_accepts = 0
        self.q.sock.close()                       # simulate a dead connection: the next call reconnects once
        self.q.f = self.q.sock = None
        self.assertTrue(self.q.ping())

    def test_ppm_to_png(self):
        with tempfile.TemporaryDirectory() as d:
            p = os.path.join(d, "a.ppm")
            open(p, "wb").write(b"P6\n2 2\n255\n" + bytes(range(12)))
            qga.ppm_to_png(p, os.path.join(d, "a.png"))
            png = open(os.path.join(d, "a.png"), "rb").read()
            self.assertTrue(png.startswith(b"\x89PNG\r\n\x1a\n"))
            self.assertIn(b"IHDR", png)


if __name__ == "__main__":
    unittest.main()
