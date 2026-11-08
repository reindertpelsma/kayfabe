"""Falsifier: mapping diverges across forms, bytes outside the scrub change, or hashes go stale."""

import base64
import gzip
import hashlib
import io
import sys
import tarfile
import unittest
import zipfile
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parents[1] / "ci"))
from test_gpu_uuid_archives import UUID, PAYLOAD, tar, zip_bytes
import no_gpu_uuids as gate
import scrub_gpu_uuids as scrub

OTHER = UUID.replace(b"a1", b"a2")
KEY = b"unit-test-key-never-used-for-real-evidence"  # fixture only


class MappingTests(unittest.TestCase):
    def mapper(self):
        return scrub.Pseudonymizer(KEY, {UUID, OTHER})

    def test_stable_across_case_and_instances(self):
        a = self.mapper()
        self.assertEqual(a.uuid(UUID), a.uuid(UUID.upper()))
        self.assertEqual(a.uuid(UUID), self.mapper().uuid(UUID))

    def test_shape_and_reserved_prefix(self):
        value = self.mapper().uuid(UUID)
        self.assertEqual(len(value), len(UUID))
        self.assertTrue(gate.synthetic(value))
        self.assertRegex(value.decode(), r"^[a-f0-9]{8}(-[a-f0-9]{4}){3}-[a-f0-9]{12}$")

    def test_different_inputs_remain_different(self):
        mapper = self.mapper()
        self.assertNotEqual(mapper.uuid(UUID), mapper.uuid(OTHER))

    def test_mapping_uses_key_material_for_each_board(self):
        for value in (UUID, OTHER):
            self.assertNotEqual(
                scrub.Pseudonymizer(KEY, {value}).uuid(value),
                scrub.Pseudonymizer(KEY + b"x", {value}).uuid(value),
            )

    def test_key_changes_output(self):
        self.assertNotEqual(
            self.mapper().uuid(UUID),
            scrub.Pseudonymizer(b"other-key", {UUID}).uuid(UUID),
        )

    def test_collisions_refuse(self):
        from unittest.mock import patch

        with patch.object(
            scrub, "pseudonym", return_value=gate.SYNTHETIC_PREFIX.encode() + b"0001"
        ):
            with self.assertRaises(scrub.ScrubError):
                self.mapper()

    def test_empty_key_refuses(self):
        with self.assertRaises(scrub.ScrubError):
            scrub.Pseudonymizer(b"", {UUID})

    def test_plain_and_bare_and_compact_cross_references(self):
        mapper = self.mapper()
        body = PAYLOAD + b" " + UUID + b" " + UUID.replace(b"-", b"")
        result = scrub.rewrite_text(body, mapper)
        self.assertNotIn(UUID, result)
        self.assertEqual(result.count(mapper.uuid(UUID)), 2)
        self.assertIn(mapper.uuid(UUID).replace(b"-", b""), result)

    def test_bare_hex_uuid_cross_reference(self):
        mapper = self.mapper()
        result = scrub.rewrite_text(UUID.hex().encode(), mapper)
        self.assertEqual(bytes.fromhex(result.decode()), mapper.uuid(UUID))

    def test_hex_and_base64_cross_references(self):
        mapper = self.mapper()
        for raw in (PAYLOAD, b"prefix " + PAYLOAD + b" suffix"):
            for encode, decode in (
                (lambda b: b.hex().encode(), lambda b: bytes.fromhex(b.decode())),
                (base64.b64encode, base64.b64decode),
            ):
                with self.subTest(raw=raw[:6], encode=encode):
                    result = decode(scrub.rewrite_text(encode(raw), mapper))
                    self.assertIn(mapper.uuid(UUID), result)
                    self.assertNotIn(UUID, result)

    def test_wrapped_encoding_whitespace_is_preserved(self):
        mapper = self.mapper()
        for encoded, decode in (
            (PAYLOAD.hex().encode(), lambda b: bytes.fromhex(b.decode())),
            (base64.b64encode(PAYLOAD), lambda b: base64.b64decode(b)),
        ):
            width = 20 if encoded.endswith(b"=") else 32
            source = b"\r\n".join(
                encoded[i : i + width] for i in range(0, len(encoded), width)
            )
            result = scrub.rewrite_text(source, mapper)
            self.assertEqual(
                [i for i, b in enumerate(source) if b in (10, 13)],
                [i for i, b in enumerate(result) if b in (10, 13)],
            )
            self.assertEqual(decode(result), b"GPU-" + mapper.uuid(UUID))

    def test_spaced_hex_format_is_preserved(self):
        mapper = self.mapper()
        source = b" ".join(f"{b:02X}".encode() for b in PAYLOAD)
        result = scrub.rewrite_text(source, mapper)
        self.assertEqual(
            [i for i, b in enumerate(source) if b == 32],
            [i for i, b in enumerate(result) if b == 32],
        )
        self.assertEqual(bytes.fromhex(result.decode()), b"GPU-" + mapper.uuid(UUID))

    def test_binary_uuid_inside_a_record(self):
        mapper = self.mapper()
        raw = bytes.fromhex(UUID.decode().replace("-", ""))
        wanted = bytes.fromhex(mapper.uuid(UUID).decode().replace("-", ""))
        self.assertEqual(
            scrub.rewrite_text(b"prefix" + raw + b"suffix", mapper),
            b"prefix" + wanted + b"suffix",
        )

    def test_utf16_and_binary_cross_references(self):
        mapper = self.mapper()
        for encoding in ("utf-16le", "utf-16be"):
            result = scrub.rewrite_text(PAYLOAD.decode().encode(encoding), mapper)
            self.assertIn(mapper.uuid(UUID).decode(), result.decode(encoding))
        binary = bytes.fromhex(UUID.decode().replace("-", ""))
        self.assertEqual(
            scrub.rewrite_text(binary, mapper),
            bytes.fromhex(mapper.uuid(UUID).decode().replace("-", "")),
        )


class RewriteTests(unittest.TestCase):
    def setUp(self):
        self.mapper = scrub.Pseudonymizer(KEY, {UUID})

    def test_untouched_archive_is_byte_identical(self):
        for source in (
            gzip.compress(b"clean", mtime=123),
            tar([("x", b"clean")]),
            zip_bytes([("x", b"clean")]),
        ):
            self.assertEqual(scrub.rewrite(source, self.mapper), source)

    def test_gzip_header_mtime_preserved(self):
        source = gzip.compress(PAYLOAD, mtime=123456)
        result = scrub.rewrite(source, self.mapper)
        self.assertEqual(result[:10], source[:10])
        self.assertEqual(gzip.decompress(result), b"GPU-" + self.mapper.uuid(UUID))

    def test_concatenated_gzip_order_preserved(self):
        first, last = gzip.compress(b"first", mtime=3), gzip.compress(b"last", mtime=5)
        source = first + gzip.compress(PAYLOAD, mtime=4) + last
        result = scrub.rewrite(source, self.mapper)
        self.assertTrue(result.startswith(first))
        self.assertTrue(result.endswith(last))
        self.assertEqual(
            gzip.decompress(result), b"firstGPU-" + self.mapper.uuid(UUID) + b"last"
        )

    def test_tar_member_order_names_mtimes_modes_preserved(self):
        source = tar([("second", PAYLOAD), ("first", b"clean")])
        result = scrub.rewrite(source, self.mapper)
        with tarfile.open(fileobj=io.BytesIO(source)) as before, tarfile.open(
            fileobj=io.BytesIO(result)
        ) as after:
            self.assertEqual(
                [(x.name, x.mtime, x.mode) for x in before],
                [(x.name, x.mtime, x.mode) for x in after],
            )
            self.assertEqual(after.extractfile("first").read(), b"clean")
            self.assertEqual(
                after.extractfile("second").read(), b"GPU-" + self.mapper.uuid(UUID)
            )

    def test_zip_metadata_and_order_preserved(self):
        source = zip_bytes([("second", PAYLOAD), ("first", b"clean")])
        result = scrub.rewrite(source, self.mapper)
        with zipfile.ZipFile(io.BytesIO(source)) as before, zipfile.ZipFile(
            io.BytesIO(result)
        ) as after:
            fields = lambda z: [
                (x.filename, x.date_time, x.external_attr, x.compress_type)
                for x in z.infolist()
            ]
            self.assertEqual(fields(before), fields(after))
            self.assertEqual(after.read("first"), b"clean")
            self.assertEqual(after.read("second"), b"GPU-" + self.mapper.uuid(UUID))

    def test_nested_archive(self):
        source = gzip.compress(tar([("zip", zip_bytes([("log", PAYLOAD)]))]))
        result = scrub.rewrite(source, self.mapper)
        self.assertFalse(gate.scan(result))
        self.assertNotEqual(result, source)

    def test_tar_manifest_hashes_recomputed(self):
        manifest = hashlib.sha256(PAYLOAD).hexdigest().encode() + b"  log\n"
        source = tar([("log", PAYLOAD), ("SHA256SUMS", manifest)])
        result = scrub.rewrite(source, self.mapper)
        with tarfile.open(fileobj=io.BytesIO(result)) as after:
            body = after.extractfile("log").read()
            self.assertEqual(
                after.extractfile("SHA256SUMS").read(),
                hashlib.sha256(body).hexdigest().encode() + b"  log\n",
            )

    def test_zip_manifest_hashes_recomputed(self):
        manifest = hashlib.md5(PAYLOAD).hexdigest().encode() + b"  log\n"
        source = zip_bytes([("MD5SUMS", manifest), ("log", PAYLOAD)])
        result = scrub.rewrite(source, self.mapper)
        with zipfile.ZipFile(io.BytesIO(result)) as after:
            self.assertEqual(
                after.read("MD5SUMS"),
                hashlib.md5(after.read("log")).hexdigest().encode() + b"  log\n",
            )

    def test_tar_md5_references_recomputed(self):
        manifest = hashlib.md5(PAYLOAD).hexdigest().encode() + b"  log\n"
        result = scrub.rewrite(
            tar([("MD5SUMS", manifest), ("log", PAYLOAD)]), self.mapper
        )
        with tarfile.open(fileobj=io.BytesIO(result)) as after:
            self.assertEqual(
                after.extractfile("MD5SUMS").read().split()[0],
                hashlib.md5(after.extractfile("log").read()).hexdigest().encode(),
            )

    def test_manifest_tsv_preserves_crlf_and_updates_growth(self):
        before, after = b"a", b"longer"
        digest = hashlib.sha256(before).hexdigest().encode()
        manifest = b"path\tbytes\tsha256\r\ntraces/y\t1\t" + digest + b"\r\n"
        result = scrub.repair_manifest(manifest, {"traces/y": (before, after)})
        self.assertEqual(
            result,
            b"path\tbytes\tsha256\r\ntraces/y\t6\t"
            + hashlib.sha256(after).hexdigest().encode()
            + b"\r\n",
        )

    def test_zip_manifest_order_survives_rewrite(self):
        source = zip_bytes([("z", PAYLOAD), ("a", PAYLOAD), ("m", b"clean")])
        with zipfile.ZipFile(io.BytesIO(scrub.rewrite(source, self.mapper))) as after:
            self.assertEqual(after.namelist(), ["z", "a", "m"])

    def test_tar_directories_and_empty_files_keep_order(self):
        source = tar([("z", PAYLOAD), ("a", b""), ("m", b"clean")])
        with tarfile.open(
            fileobj=io.BytesIO(scrub.rewrite(source, self.mapper))
        ) as after:
            self.assertEqual(after.getnames(), ["z", "a", "m"])

    def test_manifest_tsv_updates_size_and_digest(self):
        before, after = b"long original", b"short"
        manifest = (
            b"path\tbytes\tsha256\ntraces/x\t13\t"
            + hashlib.sha256(before).hexdigest().encode()
            + b"\n"
        )
        result = scrub.repair_manifest(manifest, {"traces/x": (before, after)})
        self.assertEqual(
            result,
            b"path\tbytes\tsha256\ntraces/x\t5\t"
            + hashlib.sha256(after).hexdigest().encode()
            + b"\n",
        )

    def test_checksum_chain_reaches_fixed_point(self):
        body = PAYLOAD
        a = hashlib.sha256(body).hexdigest().encode() + b"  log\n"
        b = hashlib.sha256(a).hexdigest().encode() + b"  SHA256SUMS\n"
        source = tar([("parent/SHA256SUMS", b), ("SHA256SUMS", a), ("log", body)])
        result = scrub.rewrite(source, self.mapper)
        with tarfile.open(fileobj=io.BytesIO(result)) as after:
            self.assertEqual(
                after.extractfile("parent/SHA256SUMS").read().split()[0],
                hashlib.sha256(after.extractfile("SHA256SUMS").read())
                .hexdigest()
                .encode(),
            )

    def test_discovery_includes_encoded_forms(self):
        source = gzip.compress(tar([("log", base64.b64encode(PAYLOAD))]))
        self.assertEqual(scrub.discover(source), {UUID})

    def test_discovery_plain_case_insensitive(self):
        self.assertEqual(scrub.discover(PAYLOAD.upper()), {UUID.lower()})


if __name__ == "__main__":
    unittest.main()
