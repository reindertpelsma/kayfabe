"""Adversarial parser checks using synthetic data; no private dump fixtures."""
import importlib.util
from pathlib import Path
import struct
import unittest
import uuid

spec = importlib.util.spec_from_file_location("nvcd", Path(__file__).with_name("decode-nvcd.py"))
nvcd = importlib.util.module_from_spec(spec)
spec.loader.exec_module(nvcd)


def schema():
    # Synthetic descriptor/record ABI, deliberately different from real NVCD sizes.
    return {
        "header_size": 32, "record_size": 6, "protobuf_record_size": 10,
        "root": "Test", "v1_guid": "00112233-4455-6677-8899-aabbccddeeff",
        "constants": {"NVCD_SIGNATURE": 0x12345678, "RmGroup": 3,
                      "NvcdGroup": 0, "EndOfData": 0, "RmProtoBuf_V2": 17,
                      "PRB_IS_PACKED": 2, "PRB_UINT64": 5, "PRB_MESSAGE": 16},
        "offsets": {"NVCD_HEADER.dwSignature": 0, "NVCD_HEADER.gVersion": 4,
                    "NVCD_HEADER.dwSize": 24, "NVCD_HEADER.cCheckSum": 28,
                    "NVCD_RECORD.cRecordGroup": 1, "NVCD_RECORD.cRecordType": 2,
                    "NVCD_RECORD.wRecordSize": 4, "RmProtoBuf_RECORD.dwSize": 6},
        "messages": {"Test": {
            "1": {"name": "number", "type": 5, "flags": 2, "enum": {}},
            "2": {"name": "child", "type": 16, "flags": 0, "message": "Test"}}}}


def frame(payload=b"\x08\x07"):
    s = schema()
    result = bytearray(32)
    struct.pack_into("<I", result, 0, s["constants"]["NVCD_SIGNATURE"])
    result[4:20] = uuid.UUID(s["v1_guid"]).bytes_le
    result.extend(struct.pack("<xBBxHI", 3, 17, 10, len(payload)))
    result.extend(payload)
    result.extend(struct.pack("<xBBxH", 0, 0, 6))
    struct.pack_into("<I", result, 24, len(result))
    return result


class ParserTests(unittest.TestCase):
    def test_compiled_offsets_are_used(self):
        result = nvcd.decode_nvcd(frame(), schema(), 0)
        self.assertEqual(result["decoded"], [{"number": [7]}])
        self.assertTrue(result["nvcd"]["end_marker_seen"])

    def test_truncated_end_preserves_complete_protobuf_without_repair(self):
        result = nvcd.decode_nvcd(frame()[:-1], schema(), 0)
        self.assertEqual(result["decoded"], [{"number": [7]}])
        self.assertEqual(result["nvcd"]["missing_bytes"], 1)
        self.assertNotIn("end_marker_seen", result["nvcd"])
        self.assertIsNone(result["nvcd"]["checksum_valid"])

    def test_partial_protobuf_is_never_decoded(self):
        result = nvcd.decode_nvcd(frame()[:43], schema(), 0)
        self.assertEqual(result["decoded"], [])
        self.assertTrue(result["records"][0]["truncated"])

    def test_varint_truncation_and_overflow(self):
        for value in (b"\x80", b"\xff" * 10, b"\x80" * 10 + b"\x00"):
            with self.subTest(value=value), self.assertRaises(ValueError):
                nvcd.varint(value, 0)

    def test_wrong_wire_and_length_and_field(self):
        for value in (b"\x0d\x00\x00\x00\x00", b"\x12\xff", b"\x00\x01",
                      b"\x0a\x02\x01", b"\x0b"):
            with self.subTest(value=value), self.assertRaises(ValueError):
                nvcd.Decoder(schema()).message(value, "Test")

    def test_packed_elements_spend_budget(self):
        with self.assertRaises(ValueError):
            nvcd.Decoder(schema(), budget=2).message(b"\x0a\x03\x01\x02\x03", "Test")

    def test_nested_message_depth(self):
        value = b"\x08\x01"
        for _ in range(34):
            value = b"\x12" + bytes([len(value)]) + value
        with self.assertRaises(ValueError):
            nvcd.Decoder(schema()).message(value, "Test")

    def test_unknown_bytes_are_hashed(self):
        decoder = nvcd.Decoder(schema())
        result = decoder.message(b"\x1a\x03abc", "Test")
        self.assertEqual(result["unknown_3"], [nvcd.byte_summary(b"abc")])
        self.assertEqual(decoder.unknown["Test.3"], 1)

    def test_nonadvancing_record_and_wrong_guid(self):
        data = frame()
        struct.pack_into("<H", data, 36, 0)
        with self.assertRaises(ValueError):
            nvcd.decode_nvcd(data, schema(), 0)
        data = frame()
        data[4] ^= 1
        with self.assertRaises(ValueError):
            nvcd.decode_nvcd(data, schema(), 0)

    def test_complete_checksum_failure(self):
        data = frame()
        data[28] = 1
        if sum(data) % 256 == 0:
            data[28] = 2
        with self.assertRaises(ValueError):
            nvcd.decode_nvcd(data, schema(), 0)

    def test_enumtag_exact_length_and_unique_tag(self):
        guid = schema()["v1_guid"]
        row = "  " + "01 02 03".ljust(47) + "  ..."
        text = "{" + guid + "} - 0x3 bytes\n" + row
        self.assertEqual(nvcd.extract_enumtag(text, guid), b"\x01\x02\x03")
        with self.assertRaises(ValueError):
            nvcd.extract_enumtag(text + "\n" + text, guid)
        with self.assertRaises(ValueError):
            nvcd.extract_enumtag(text.replace("0x3 bytes", "0x4 bytes"), guid)


if __name__ == "__main__":
    unittest.main()
