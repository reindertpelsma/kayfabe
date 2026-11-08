"""Falsifier: an encoded/archive UUID passes, or an incomplete bounded scan passes.

Identifiers are generated fixtures, never captured board identifiers.
"""

import base64
import contextlib
import gzip
import io
import json
import lzma
import subprocess
import tarfile
import tempfile
import unittest
import zipfile
from pathlib import Path
from unittest.mock import patch

import no_gpu_uuids as gate

UUID = (
    "a1" * 4 + "-" + "b2" * 2 + "-" + "c3" * 2 + "-" + "d4" * 2 + "-" + "e5" * 6
).encode()
PAYLOAD = b"GPU-" + UUID


def tar(entries):
    out = io.BytesIO()
    with tarfile.open(fileobj=out, mode="w") as archive:
        for name, data in entries:
            info = tarfile.TarInfo(name)
            info.size, info.mtime, info.mode = len(data), 123456, 0o640
            archive.addfile(info, io.BytesIO(data))
    return out.getvalue()


def zip_bytes(entries):
    out = io.BytesIO()
    with zipfile.ZipFile(out, "w", compression=zipfile.ZIP_DEFLATED) as archive:
        for name, data in entries:
            archive.writestr(name, data)
    return out.getvalue()


class ArchiveTests(unittest.TestCase):
    def assert_hit(self, data):
        self.assertGreater(sum(gate.scan(data).values()), 0)

    def test_gzip_by_magic(self):
        self.assert_hit(gzip.compress(PAYLOAD))

    def test_gzip_concatenated_streams(self):
        self.assert_hit(gzip.compress(b"clean") + gzip.compress(PAYLOAD))

    def test_tar_member(self):
        self.assert_hit(tar([("one", PAYLOAD)]))

    def test_tar_inside_gzip(self):
        self.assert_hit(gzip.compress(tar([("one", PAYLOAD)])))

    def test_zip_member(self):
        self.assert_hit(zip_bytes([("one", PAYLOAD)]))

    def test_zip_with_executable_preamble(self):
        self.assert_hit(b"MZ" + b"preamble" * 20 + zip_bytes([("one", PAYLOAD)]))

    def test_zip_with_arbitrary_preamble_and_nested_gzip(self):
        self.assert_hit(b"prefix" + zip_bytes([("one", gzip.compress(PAYLOAD))]))

    def test_zip_nested_gzip(self):
        self.assert_hit(zip_bytes([("one", gzip.compress(PAYLOAD))]))

    def test_member_order_does_not_hide_later_tar_member(self):
        self.assert_hit(tar([("clean", b"hello"), ("last", PAYLOAD)]))

    def test_duplicate_zip_names_are_scanned_individually(self):
        self.assert_hit(zip_bytes([("same", b"clean"), ("same", PAYLOAD)]))

    def test_hex_encoded(self):
        self.assert_hit(PAYLOAD.hex().encode())

    def test_spaced_hex_encoded(self):
        self.assert_hit(b" ".join(f"{b:02X}".encode() for b in PAYLOAD))

    def test_base64_encoded(self):
        self.assert_hit(base64.b64encode(PAYLOAD))

    def test_base64_unpadded_in_archive(self):
        self.assert_hit(
            gzip.compress(base64.b64encode(b"text " + PAYLOAD).rstrip(b"="))
        )

    def test_wrapped_base64(self):
        encoded = base64.b64encode(PAYLOAD)
        self.assert_hit(
            b"\n".join(encoded[i : i + 20] for i in range(0, len(encoded), 20))
        )

    def test_wrapped_hex(self):
        encoded = PAYLOAD.hex().encode()
        self.assert_hit(
            b"\r\n".join(encoded[i : i + 32] for i in range(0, len(encoded), 32))
        )

    def test_utf16_device_uuid_assignment(self):
        self.assert_hit((b"deviceUUID = " + UUID).decode().encode("utf-16le"))

    def test_utf16_underscore_assignment(self):
        self.assert_hit((b"gpu_uuid = " + UUID).decode().encode("utf-16be"))

    def test_base64_device_uuid_assignment(self):
        self.assert_hit(base64.b64encode(b"deviceUUID = " + UUID))

    def test_utf16le(self):
        self.assert_hit(PAYLOAD.decode().encode("utf-16le"))

    def test_utf16be_in_archive(self):
        self.assert_hit(gzip.compress(PAYLOAD.decode().encode("utf-16be")))

    def test_tar_member_name(self):
        self.assert_hit(tar([(PAYLOAD.decode(), b"clean")]))

    def test_zip_member_name(self):
        self.assert_hit(zip_bytes([(PAYLOAD.decode(), b"clean")]))

    def test_tar_nonzero_trailing_raw_bytes_are_refused(self):
        with self.assertRaises(gate.ScanError):
            gate.scan(tar([("clean", b"clean")]) + PAYLOAD)

    def test_zip_trailing_raw_bytes_keep_previous_gate_coverage(self):
        self.assert_hit(zip_bytes([("clean", b"clean")]) + PAYLOAD)

    def test_zip_trailing_encoded_bytes_keep_previous_gate_coverage(self):
        self.assert_hit(zip_bytes([("clean", b"clean")]) + base64.b64encode(PAYLOAD))

    def test_no_extract_of_traversal_or_links(self):
        data = tar([("../../outside", PAYLOAD)])
        with patch.object(
            tarfile.TarFile, "extractall", side_effect=AssertionError("extracted")
        ):
            self.assert_hit(data)

    def test_synthetic_inside_every_container(self):
        fake = b"GPU-" + gate.SYNTHETIC_PREFIX.encode() + b"1234"
        for wrap in (
            gzip.compress,
            lambda b: tar([("x", b)]),
            lambda b: zip_bytes([("x", b)]),
        ):
            self.assertFalse(gate.scan(wrap(fake)))

    def test_tar_count_does_not_double_count_raw_container(self):
        self.assertEqual(sum(gate.scan(tar([("one", PAYLOAD)])).values()), 1)

    def test_zip_metadata_and_payload_are_both_scanned(self):
        self.assertEqual(
            sum(gate.scan(zip_bytes([(PAYLOAD.decode(), PAYLOAD)])).values()), 2
        )

    def test_metadata_and_payload_are_both_scanned(self):
        self.assertEqual(sum(gate.scan(tar([(PAYLOAD.decode(), PAYLOAD)])).values()), 2)


class ExtraCompressionTests(unittest.TestCase):
    def zstd(self, data):
        try:
            from compression import zstd
        except ImportError:
            return subprocess.run(
                ["zstd", "-q", "-c"], input=data, capture_output=True, check=True
            ).stdout
        return zstd.compress(data)

    def test_xz_log(self):
        self.assertTrue(gate.scan(lzma.compress(PAYLOAD)))

    def test_xz_tar(self):
        self.assertTrue(gate.scan(lzma.compress(tar([("x", PAYLOAD)]))))

    def test_zstd_log(self):
        self.assertTrue(gate.scan(self.zstd(PAYLOAD)))

    def test_zstd_tar(self):
        self.assertTrue(gate.scan(self.zstd(tar([("x", PAYLOAD)]))))

    def test_xz_corrupt_refused(self):
        with self.assertRaises(gate.ScanError):
            gate.scan(b"\xfd7zXZ\x00corrupt")

    def test_zstd_corrupt_refused(self):
        with self.assertRaises(gate.ScanError):
            gate.scan(b"\x28\xb5\x2f\xfdcorrupt")

    def test_xz_expansion_bound(self):
        with patch.object(gate, "MAX_MEMBER_BYTES", 100):
            with self.assertRaises(gate.ScanError):
                gate.scan(lzma.compress(b"a" * 101))

    def test_zstd_expansion_bound(self):
        data = self.zstd(b"a" * 101)
        with patch.object(gate, "MAX_MEMBER_BYTES", 100):
            with self.assertRaises(gate.ScanError):
                gate.scan(data)

    def test_xz_concatenated(self):
        self.assertTrue(gate.scan(lzma.compress(b"clean") + lzma.compress(PAYLOAD)))

    def test_zstd_concatenated(self):
        self.assertTrue(gate.scan(self.zstd(b"clean") + self.zstd(PAYLOAD)))


class LimitTests(unittest.TestCase):
    def test_gzip_expansion_bound(self):
        with patch.object(gate, "MAX_MEMBER_BYTES", 100):
            with self.assertRaises(gate.ScanError):
                gate.scan(gzip.compress(b"a" * 101))

    def test_zip_expansion_bound(self):
        with patch.object(gate, "MAX_MEMBER_BYTES", 100):
            with self.assertRaises(gate.ScanError):
                gate.scan(zip_bytes([("x", b"a" * 101)]))

    def test_depth_bound_gzip(self):
        with patch.object(gate, "MAX_DEPTH", 1):
            with self.assertRaises(gate.ScanError):
                gate.scan(gzip.compress(gzip.compress(PAYLOAD)))

    def test_depth_bound_zip(self):
        with patch.object(gate, "MAX_DEPTH", 1):
            with self.assertRaises(gate.ScanError):
                gate.scan(zip_bytes([("nested", gzip.compress(PAYLOAD))]))

    def test_tar_member_count_bound(self):
        with patch.object(gate, "MAX_MEMBERS", 1):
            with self.assertRaises(gate.ScanError):
                gate.scan(tar([("one", b""), ("two", b"")]))

    def test_zip_member_count_bound(self):
        with patch.object(gate, "MAX_MEMBERS", 1):
            with self.assertRaises(gate.ScanError):
                gate.scan(zip_bytes([("one", b""), ("two", b"")]))

    def test_shared_budget_tar(self):
        with patch.object(gate, "MAX_EXPANDED_BYTES", 150):
            with self.assertRaises(gate.ScanError):
                gate.scan(tar([("one", b"a" * 80), ("two", b"b" * 80)]))

    def test_shared_budget_zip(self):
        with patch.object(gate, "MAX_EXPANDED_BYTES", 150):
            with self.assertRaises(gate.ScanError):
                gate.scan(zip_bytes([("one", b"a" * 80), ("two", b"b" * 80)]))

    def test_bad_gzip_refused(self):
        with self.assertRaises(gate.ScanError):
            gate.scan(b"\x1f\x8bcorrupt")

    def test_bad_zip_refused(self):
        with self.assertRaises(gate.ScanError):
            gate.scan(b"PK\x03\x04corrupt")

    def test_truncated_tar_refused(self):
        with self.assertRaises(gate.ScanError):
            gate.scan(tar([("one", b"a" * 800)])[:600])

    def test_gzip_bad_crc_refused(self):
        bad = bytearray(gzip.compress(PAYLOAD))
        bad[-8] ^= 1
        with self.assertRaises(gate.ScanError):
            gate.scan(bytes(bad))

    def run_error(self, data, *args):
        with tempfile.TemporaryDirectory() as d:
            root = Path(d)
            (root / "traces").mkdir()
            (root / "traces/x").write_bytes(data)
            base = root / "baseline.json"
            base.write_text(json.dumps({"files": {}}))
            before = base.read_bytes()
            out = io.StringIO()
            with contextlib.redirect_stdout(out), contextlib.redirect_stderr(out):
                code = gate.main(["--root", d, "--baseline", str(base), *args])
            self.assertEqual(code, 2)
            self.assertEqual(base.read_bytes(), before)
            self.assertNotIn(UUID.decode(), out.getvalue())

    def test_cli_errors_fail_closed(self):
        self.run_error(b"\x1f\x8b" + PAYLOAD)

    def test_shrink_errors_do_not_change_baseline(self):
        self.run_error(b"PK\x03\x04" + PAYLOAD, "--shrink")

    def test_root_file_read_is_bounded(self):
        with patch.object(gate, "MAX_FILE_BYTES", 10):
            self.run_error(b"a" * 11)

    def test_root_file_tail_is_never_silently_skipped(self):
        with patch.object(gate, "MAX_FILE_BYTES", 10):
            self.run_error(b"a" * 11 + PAYLOAD)

    def test_oversize_root_file_does_not_shrink_baseline(self):
        with patch.object(gate, "MAX_FILE_BYTES", 10):
            self.run_error(b"a" * 11, "--shrink")

    def test_root_archive_read_is_bounded(self):
        with patch.object(gate, "MAX_FILE_BYTES", 10):
            self.run_error(gzip.compress(PAYLOAD))


class DecoderMemoryTests(unittest.TestCase):
    @staticmethod
    def large_xz_dictionary():
        import struct
        import zlib

        data = bytearray(lzma.compress(b"clean"))
        # XZ block with one LZMA2 filter, one-byte properties. Change only the
        # advertised dictionary from 8 MiB to 256 MiB and repair header CRC32.
        assert data[13:16] == b"\x00\x21\x01"
        end = 12 + (data[12] + 1) * 4
        data[16] = 32
        data[end - 4 : end] = struct.pack("<I", zlib.crc32(data[12 : end - 4]))
        return bytes(data)

    @staticmethod
    def large_zstd_window():
        # Valid frame: unknown content size, advertised 256 MiB window, one raw
        # final block. Constructing it does not allocate that window.
        body = b"clean"
        return (
            b"\x28\xb5\x2f\xfd\x00\x90"
            + (1 + (len(body) << 3)).to_bytes(3, "little")
            + body
        )

    def test_xz_dictionary_memory_bound(self):
        with self.assertRaises(gate.ScanError):
            gate.scan(self.large_xz_dictionary())

    def test_xz_dictionary_memory_bound_inside_gzip(self):
        with self.assertRaises(gate.ScanError):
            gate.scan(gzip.compress(self.large_xz_dictionary()))

    def test_zstd_window_memory_bound(self):
        with self.assertRaises(gate.ScanError):
            gate.scan(self.large_zstd_window())

    def test_zstd_window_memory_bound_inside_zip(self):
        with self.assertRaises(gate.ScanError):
            gate.scan(zip_bytes([("large-window", self.large_zstd_window())]))

    def test_zstd_cli_window_memory_bound(self):
        import sys

        with patch.dict(sys.modules, {"compression": None}):
            with self.assertRaises(gate.ScanError):
                gate.scan(self.large_zstd_window())

    def test_zstd_cli_nested_window_memory_bound(self):
        import sys

        with patch.dict(sys.modules, {"compression": None}):
            with self.assertRaises(gate.ScanError):
                gate.scan(zip_bytes([("large-window", self.large_zstd_window())]))

    def test_zstd_cli_finds_uuid(self):
        import sys

        data = subprocess.run(
            ["zstd", "-q", "-c"], input=PAYLOAD, capture_output=True, check=True
        ).stdout
        with patch.dict(sys.modules, {"compression": None}):
            self.assertTrue(gate.scan(data))

    def test_zstd_cli_finds_nested_uuid(self):
        import sys

        data = subprocess.run(
            ["zstd", "-q", "-c"],
            input=gzip.compress(PAYLOAD),
            capture_output=True,
            check=True,
        ).stdout
        with patch.dict(sys.modules, {"compression": None}):
            self.assertTrue(gate.scan(data))


if __name__ == "__main__":
    unittest.main()
