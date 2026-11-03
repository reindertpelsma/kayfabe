#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later
"""A transport failure must not duplicate a guest command or report success."""
import base64
import importlib.util
import io
import json
from pathlib import Path
import socket
import tempfile
import threading
import unittest
from unittest.mock import Mock, patch

spec = importlib.util.spec_from_file_location(
    "windows_qmp", Path(__file__).resolve().parents[1] / "windows" / "qmp.py")
qmp = importlib.util.module_from_spec(spec)
spec.loader.exec_module(qmp)


class GuestExecTests(unittest.TestCase):
    def run_exec(self, replies, opens=None, timeout=5):
        sends = []
        opens = opens or [(Mock(), Mock()), (Mock(), Mock())]
        with patch.object(qmp, "qga_open", side_effect=opens) as connect, \
             patch.object(qmp, "send", side_effect=lambda f, msg: sends.append(msg)), \
             patch.object(qmp, "recv", side_effect=replies), \
             patch.object(qmp.time, "sleep"), \
             patch.object(qmp.sys, "stdout", new_callable=io.StringIO) as out, \
             patch.object(qmp.sys, "stderr", new_callable=io.StringIO):
            result = qmp.qga_exec("fixture.sock", "probe.exe", ["--check"], timeout)
        return result, sends, connect.call_count, out.getvalue()

    def test_cli_deadline_allows_a_seven_minute_probe(self):
        replies = [{"return": {"pid": 73}}, {"return": {"exited": False}},
                   {"return": {"exited": True, "exitcode": 7}}]
        sends = []
        with patch.object(qmp, "qga_open", return_value=(Mock(), Mock())), \
             patch.object(qmp, "send", side_effect=lambda f, m: sends.append(m)), \
             patch.object(qmp, "recv", side_effect=replies), \
             patch.object(qmp.time, "sleep"), \
             patch.object(qmp.time, "monotonic", side_effect=[0, 0, 0, 421]):
            rc = qmp.main(["qmp.py", "fixture.sock", "qga-exec", "--timeout=660",
                           "probe.exe", "--check"])
        self.assertEqual(rc, 7)
        self.assertEqual(sends[0]["arguments"]["path"], "probe.exe")
        self.assertEqual(sends[0]["arguments"]["arg"], ["--check"])
        self.assertEqual(sum(m["execute"] == "guest-exec" for m in sends), 1)

    def test_short_cli_deadline_stops_before_an_unrequested_status_poll(self):
        sends = []
        with patch.object(qmp, "qga_open", return_value=(Mock(), Mock())), \
             patch.object(qmp, "send", side_effect=lambda f, m: sends.append(m)), \
             patch.object(qmp, "recv", side_effect=[
                 {"return": {"pid": 73}},
                 {"return": {"exited": True, "exitcode": 0}},
             ]), \
             patch.object(qmp.time, "monotonic", side_effect=[0, 1]):
            rc = qmp.main(["qmp.py", "fixture.sock", "qga-exec", "--timeout=0.5",
                           "probe.exe", "--check"])
        self.assertEqual(rc, 124)
        self.assertEqual([m["execute"] for m in sends], ["guest-exec"])

    def test_invalid_cli_deadline_never_launches(self):
        for value in ("nan", "inf", "0", "-1", "3601", "bad"):
            with self.subTest(value=value), patch.object(qmp, "qga_exec") as launch, \
                 patch.object(qmp.sys, "stderr", new_callable=io.StringIO):
                rc = qmp.main(["qmp.py", "fixture.sock", "qga-exec",
                               "--timeout=" + value, "probe.exe"])
                self.assertEqual(rc, 2)
                launch.assert_not_called()

    def test_status_timeout_reconnects_to_the_same_process(self):
        encoded = base64.b64encode(b"hardware verdict: FAIL\n").decode()
        rc, sends, opens, output = self.run_exec([
            {"return": {"pid": 73}}, socket.timeout("delayed agent"),
            {"return": {"exited": True, "exitcode": 7, "out-data": encoded}},
        ])
        self.assertEqual(rc, 7)
        self.assertEqual(opens, 2)
        self.assertEqual(output, "hardware verdict: FAIL\n")
        self.assertEqual([m["execute"] for m in sends],
                         ["guest-exec", "guest-exec-status", "guest-exec-status"])
        self.assertEqual([m["arguments"]["pid"] for m in sends[1:]], [73, 73])

    def test_lost_launch_reply_is_never_replayed(self):
        with self.assertRaises(OSError):
            self.run_exec([socket.timeout("unknown launch outcome")])

    def test_agent_error_does_not_turn_into_success_or_poll_forever(self):
        rc, sends, opens, _ = self.run_exec([
            {"return": {"pid": 73}},
            {"error": {"class": "GenericError", "desc": "unknown pid"}},
        ])
        self.assertEqual(rc, 1)
        self.assertEqual(len(sends), 2)
        self.assertEqual(opens, 1)

    def test_deadline_expires_during_reconnection(self):
        with patch.object(qmp.time, "monotonic", side_effect=[0, 0, 0, 0, 6, 6]):
            rc, sends, opens, _ = self.run_exec([
                {"return": {"pid": 73}}, socket.timeout("delayed agent"),
            ], opens=[(Mock(), Mock()), socket.timeout("still unavailable")])
        self.assertEqual(rc, 124)
        self.assertEqual(opens, 2)
        self.assertEqual(sum(m["execute"] == "guest-exec" for m in sends), 1)

    def test_unavailable_reconnect_preserves_the_original_pid(self):
        rc, sends, opens, _ = self.run_exec([
            {"return": {"pid": 73}}, EOFError("disconnected"),
            {"return": {"exited": True, "exitcode": 0}},
        ], opens=[(Mock(), Mock()), OSError("not ready"), (Mock(), Mock())])
        self.assertEqual(rc, 0)
        self.assertEqual(opens, 3)
        self.assertEqual([m["execute"] for m in sends],
                         ["guest-exec", "guest-exec-status", "guest-exec-status"])
        self.assertEqual(sends[-1]["arguments"], {"pid": 73})

    def test_real_socket_disconnect_does_not_duplicate_guest_execution(self):
        requests, errors = [], []
        with tempfile.TemporaryDirectory() as directory:
            path = str(Path(directory) / "qga.sock")
            with socket.socket(socket.AF_UNIX, socket.SOCK_STREAM) as listener:
                listener.bind(path)
                listener.listen(2)
                listener.settimeout(5)

                def agent():
                    try:
                        for attempt in range(2):
                            with listener.accept()[0] as conn:
                                conn.settimeout(5)
                                with conn.makefile("rwb", buffering=0) as f:
                                    sync = json.loads(f.readline())
                                    qmp.send(f, {"return": sync["arguments"]["id"]})
                                    msg = json.loads(f.readline())
                                    requests.append(msg)
                                    if attempt == 0:
                                        qmp.send(f, {"return": {"pid": 73}})
                                        requests.append(json.loads(f.readline()))
                                        # The process still exists. Lose only this status reply.
                                    else:
                                        qmp.send(f, {"return": {"exited": True, "exitcode": 7}})
                    except Exception as error:
                        errors.append(error)

                worker = threading.Thread(target=agent, daemon=True)
                worker.start()
                result = qmp.qga_exec(path, "probe.exe", ["--check"], timeout=5)
                worker.join(timeout=6)
                self.assertFalse(worker.is_alive())
                self.assertEqual(errors, [])
        self.assertEqual(result, 7)
        self.assertEqual([m["execute"] for m in requests],
                         ["guest-exec", "guest-exec-status", "guest-exec-status"])
        self.assertEqual([m["arguments"]["pid"] for m in requests[1:]], [73, 73])


if __name__ == "__main__":
    unittest.main()
