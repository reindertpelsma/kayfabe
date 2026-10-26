"""Gate visibility falsifiers: when the box reporter must drop text, it drops the MIDDLE.

The verdict-deciding lines (the payload's FINAL SUMMARY) are the last bytes of the log. The reporter
used to drop the NEWEST text when its buffer, post, byte or request caps bound, so a flooded log lost
exactly them. Now the oldest unsent text is dropped, the caps leave a reserve for the end of the log,
and a "[kf-report] N bytes omitted" line marks every gap.
"""

import json
import re
import sys
import unittest
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))
import vast_report as vr


class Fake:
    def __init__(self):
        self.docs = []
        self.sizes = []

    def post(self, url, body):
        self.docs.append(json.loads(body))
        self.sizes.append(len(body))
        return 200, {"state": "ACCEPTED"}


class Clock:
    def __init__(self):
        self.t = 0.0

    def __call__(self):
        self.t += 0.01
        return self.t


def lines(n, width=60):
    return "".join("row %05d %s\n" % (i, "x" * width) for i in range(n))


def make(**limits):
    base = dict(min_gap=0.0, poll=0.0, flush=0.0, final_budget=10**6)
    base.update(limits)
    transport = Fake()
    reporter = vr.Reporter(
        "https://b.example", "1-1", "t", 600, vr.Limits(**base), transport, Clock()
    )
    reporter.sleep = lambda _: None
    return reporter, transport


def drive(reporter, text, step=3000):
    data = text.encode()
    for i in range(0, len(data), step):
        reporter.feed(data[i : i + step])
        reporter.pump()


RESULT = {"verdict": "FAIL", "suites": []}


def logs(transport):
    return [d for d in transport.docs if d["kind"] == "log" and d.get("text")]


def text(transport):
    return "".join(d["text"] for d in logs(transport))


class BufferOverflow(unittest.TestCase):
    def test_overflow_drops_the_oldest_unsent_text(self):
        r, t = make(buffer_cap=3000)
        r.feed(lines(40).encode())
        self.assertIn("row 00039", r.text)
        self.assertNotIn("row 00000", r.text)
        self.assertLessEqual(len(r.text.encode()), 3000)

    def test_overflow_is_counted_as_omitted(self):
        r, t = make(buffer_cap=3000)
        r.feed(lines(40).encode())
        self.assertEqual(r.omitted + len(r.text.encode()), len(lines(40).encode()))

    def test_overflow_gap_is_marked_in_the_next_post(self):
        r, t = make(buffer_cap=3000)
        r.feed(lines(40).encode())
        r.finish(RESULT, "done")
        first = logs(t)[0]["text"]
        self.assertRegex(first, r"^\[kf-report\] [1-9][0-9]* bytes omitted\n")
        self.assertIn("row 00039", text(t))

    def test_a_log_within_the_buffer_is_untouched(self):
        r, t = make(buffer_cap=10**6)
        r.feed(lines(40).encode())
        r.finish(RESULT, "done")
        self.assertEqual(text(t), lines(40))
        self.assertEqual(r.omitted, 0)


class CapsKeepTheEnd(unittest.TestCase):
    def check_end(self, t, total):
        got = text(t)
        self.assertIn("row %05d" % (total - 1), got)
        last = [int(x) for x in re.findall(r"row (\d{5})", got)]
        self.assertEqual(last, sorted(last))
        tail = last[last.index(max(last)) :]
        self.assertEqual(tail[-1], total - 1)

    def test_post_cap_keeps_the_last_lines(self):
        r, t = make(chunk=1000, max_posts=6)
        drive(r, lines(900))
        r.finish(RESULT, "done")
        self.assertLessEqual(len(logs(t)), 6)
        self.check_end(t, 900)

    def test_byte_cap_keeps_the_last_lines(self):
        r, t = make(chunk=2000, max_bytes=9000)
        drive(r, lines(900))
        r.finish(RESULT, "done")
        self.assertLessEqual(sum(t.sizes), 9000)
        self.check_end(t, 900)

    def test_request_cap_keeps_the_last_lines(self):
        r, t = make(chunk=1000, max_requests=14, hard_requests=40)
        drive(r, lines(900))
        r.finish(RESULT, "done")
        self.check_end(t, 900)

    def test_the_head_is_still_delivered(self):
        r, t = make(chunk=1000, max_posts=6)
        drive(r, lines(900))
        r.finish(RESULT, "done")
        self.assertIn("row 00000", text(t))

    def test_the_gap_is_marked_before_the_tail(self):
        r, t = make(chunk=1000, max_posts=6)
        drive(r, lines(900))
        r.finish(RESULT, "done")
        got = text(t)
        self.assertRegex(got, r"\[kf-report\] [1-9][0-9]* bytes omitted\n")
        self.assertLess(got.index("row 00000"), got.index("bytes omitted"))
        self.assertLess(got.index("bytes omitted"), got.rindex("row 00899"))

    def test_omitted_bytes_are_still_reported_in_the_result_note(self):
        r, t = make(chunk=1000, max_posts=6)
        drive(r, lines(900))
        r.finish(RESULT, "done")
        note = [d for d in t.docs if d["kind"] == "result"][0]["text"]
        self.assertRegex(note, r"omitted [1-9][0-9]* raw bytes")

    def test_every_post_stays_within_the_chunk(self):
        r, t = make(chunk=1000, max_posts=6)
        drive(r, lines(900))
        r.finish(RESULT, "done")
        for d in logs(t):
            self.assertLessEqual(len(d["text"].encode()), 1000)

    def test_no_cap_no_marker(self):
        r, t = make(chunk=1000)
        drive(r, lines(50))
        r.finish(RESULT, "done")
        self.assertEqual(text(t), lines(50))
        self.assertNotIn("bytes omitted", text(t))

    def test_the_result_is_still_delivered_when_caps_bind(self):
        r, t = make(chunk=1000, max_posts=6)
        drive(r, lines(900))
        ack = r.finish(RESULT, "done")
        self.assertIsNotNone(ack)
        self.assertEqual([d["kind"] for d in t.docs][-1], "result")


if __name__ == "__main__":
    unittest.main()
