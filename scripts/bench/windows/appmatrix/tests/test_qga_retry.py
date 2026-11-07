# SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later
"""A reply lost after a stateful read must not turn the next read at EOF into the file's contents.

No socket or guest: the fake executes each read, then simulates a dropped response. This separates
the protocol bug from scheduler-dependent socket timeouts.
"""
import base64
import unittest

import helpers as H  # noqa: F401
import qga


class LostReadReply(qga.Qga):
    def __init__(self, data, lose_at=0, error=qga.QgaError):
        self.data, self.offset = data, 0
        self.lose_at, self.error, self.lost = lose_at, error, False
        self.reads = 0

    def call(self, cmd, args=None, retry=True):
        if cmd == "guest-file-open":
            self.offset = 0
            return 1
        if cmd == "guest-file-close":
            return {}
        if cmd == "guest-file-seek":
            self.offset = (len(self.data) if args["whence"] == 2 else 0) + args["offset"]
            return {"position": self.offset}
        assert cmd == "guest-file-read", cmd
        self.reads += 1
        start = self.offset
        data = self.data[start:start + args["count"]]
        self.offset += len(data)
        if start == self.lose_at and not self.lost:
            self.lost = True
            if retry and self.error is qga.QgaError:
                # Qga.call reconnects and replays the RPC; the old read already advanced the handle.
                return self.call(cmd, args, retry=False)
            raise self.error("reply lost after the read was applied")
        return {"count": len(data), "buf-b64": base64.b64encode(data).decode(), "eof": self.offset == len(self.data)}


class ReadRetry(unittest.TestCase):
    def test_a_lost_head_reply_does_not_return_an_empty_file(self):
        data = b"abcdef" * 50000
        self.assertEqual(LostReadReply(data).file_read(r"C:\result.json"), data)

    def test_a_lost_tail_reply_retries_from_the_same_absolute_offset(self):
        data = b"H" * 32 + b"-" * (2 << 20) + b"T" * 64
        got = LostReadReply(data, len(data) - 64).file_read(r"C:\app.log", head=32, tail=64)
        self.assertTrue(got.startswith(b"H" * 32))
        self.assertTrue(got.endswith(b"T" * 64))

    def test_timeouts_are_not_replayed_or_given_a_larger_budget(self):
        client = LostReadReply(b"data", error=qga.QgaTimeout)
        with self.assertRaises(qga.QgaTimeout):
            client.file_read(r"C:\result.json")
        self.assertEqual(client.reads, 1)

    def test_an_understood_refusal_is_not_replayed(self):
        client = LostReadReply(b"data", error=qga.QgaCommandError)
        with self.assertRaises(qga.QgaCommandError):
            client.file_read(r"C:\result.json")
        self.assertEqual(client.reads, 1)


if __name__ == "__main__":
    unittest.main()
