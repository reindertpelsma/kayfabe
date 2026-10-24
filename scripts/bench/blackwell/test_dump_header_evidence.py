"""Evidence parsing must retain unsupported headers and continue the collection."""

import contextlib
import io
import json
import struct
import sys
import unittest
from pathlib import Path
from unittest.mock import patch

sys.path.insert(0, str(Path(__file__).parent))
import dump_header


def supported_header():
    raw = bytearray(dump_header.HEADER_BYTES)
    raw[:8] = b"PAGEDU64"
    struct.pack_into("<I", raw, 56, 0x50)
    return bytes(raw)


def collector_line(raw, name="evidence.dmp"):
    return f"HDR C:/Windows/{name} 56963072 {raw.hex()}\n"


class DumpHeaderEvidence(unittest.TestCase):
    def test_zero_signature_retains_raw_header_without_a_bugcheck_claim(self):
        raw = bytes(dump_header.HEADER_BYTES)
        result = dump_header.parse_hexline(collector_line(raw))
        self.assertEqual(result["parse_status"], "unparsed")
        self.assertEqual(result["note"], "unparsed header, raw bytes retained in evidence")
        self.assertEqual(result["raw_header_hex"], raw.hex())
        self.assertEqual(result["file"], "C:/Windows/evidence.dmp")
        self.assertEqual(result["bytes"], 56963072)
        self.assertNotIn("bugcheck", result)

    def test_collection_keeps_supported_headers_on_both_sides_of_unsupported_one(self):
        lines = "".join(
            collector_line(raw, name)
            for raw, name in [
                (supported_header(), "first.dmp"),
                (bytes(dump_header.HEADER_BYTES), "watchdog.dmp"),
                (supported_header(), "last.dmp"),
            ]
        )
        output = io.StringIO()
        with patch.object(sys, "stdin", io.StringIO(lines)), contextlib.redirect_stdout(output):
            dump_header.main(["dump_header.py", "--hexlines"])
        rows = [json.loads(line) for line in output.getvalue().splitlines()]
        self.assertEqual(len(rows), 3)
        self.assertEqual(rows[0]["bugcheck"], "0x50")
        self.assertEqual(rows[1]["parse_status"], "unparsed")
        self.assertEqual(rows[2]["bugcheck"], "0x50")

    def test_short_header_is_retained_as_unparsed(self):
        raw = b"PAGEDU64"
        result = dump_header.parse_hexline(collector_line(raw))
        self.assertEqual(result["parse_status"], "unparsed")
        self.assertEqual(result["raw_header_hex"], raw.hex())
        self.assertNotIn("system_time_utc", result)

    def test_transport_errors_still_refuse(self):
        for line in ["HDR only-two-fields", "HDR x 12 invalidhex", "HDR x invalid 00"]:
            with self.subTest(line=line), self.assertRaises(ValueError):
                dump_header.parse_hexline(line)


if __name__ == "__main__":
    unittest.main()
