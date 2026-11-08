"""Falsifier: mapping diverges across forms, bytes outside the scrub change, or hashes go stale."""

import base64
import contextlib
import subprocess
import tempfile
import gzip
import hashlib
import io
import json
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


class LabelledBase64CliTests(unittest.TestCase):
    def check_cli(self, label, width=0, compressed=False, plain=True):
        encoded = base64.b64encode(PAYLOAD)
        if width:
            encoded = b"\n".join(
                encoded[i : i + width] for i in range(0, len(encoded), width)
            )
        data = label + b"\n" + encoded
        with tempfile.TemporaryDirectory() as outer:
            root = Path(outer) / "repo"
            root.mkdir()
            subprocess.run(["git", "init", "-q", str(root)], check=True)
            target = root / "encoded.txt"
            target.write_bytes(gzip.compress(data, mtime=12345) if compressed else data)
            if plain:
                (root / "plain.txt").write_bytes(PAYLOAD)
            subprocess.run(["git", "-C", str(root), "add", "."], check=True)
            key = Path(outer) / "test-key"
            key.write_bytes(KEY)
            result = subprocess.run(
                [
                    sys.executable,
                    scrub.__file__,
                    "scrub",
                    "--root",
                    str(root),
                    "--key-file",
                    str(key),
                ],
                capture_output=True,
            )
            self.assertEqual(result.returncode, 0)
            actual = (
                gzip.decompress(target.read_bytes())
                if compressed
                else target.read_bytes()
            )
            header, block = actual.split(b"\n", 1)
            self.assertEqual(header, label)
            expected = b"GPU-" + scrub.pseudonym(KEY, UUID)
            # This oracle does not call the scanner or its encoded-view iterator.
            self.assertEqual(base64.b64decode(block, validate=False), expected)
            if plain:
                self.assertEqual((root / "plain.txt").read_bytes(), expected)
            baseline = Path(outer) / "baseline.json"
            baseline.write_text(json.dumps({"files": {}}))
            gate_result = subprocess.run(
                [
                    sys.executable,
                    gate.__file__,
                    "--root",
                    str(root),
                    "--baseline",
                    str(baseline),
                ],
                capture_output=True,
            )
            self.assertEqual(gate_result.returncode, 0)
            self.assertEqual(json.loads(baseline.read_text()), {"files": {}})

    def test_payload_label_contiguous_scrub_cli(self):
        self.check_cli(b"payload")

    def test_abc_label_contiguous_scrub_cli(self):
        self.check_cli(b"abc")

    def test_payload_label_wrapped_scrub_cli(self):
        self.check_cli(b"payload", width=8)

    def test_abc_label_wrapped_scrub_cli(self):
        self.check_cli(b"abc", width=8)

    def test_labelled_contiguous_gzip_scrub_cli(self):
        self.check_cli(b"payload", compressed=True)

    def test_labelled_wrapped_gzip_scrub_cli(self):
        self.check_cli(b"abc", width=8, compressed=True)

    def test_labelled_contiguous_discovery_without_plain_control(self):
        self.check_cli(b"payload", plain=False)

    def test_labelled_gzip_wrapped_discovery_without_plain_control(self):
        self.check_cli(b"abc", width=8, compressed=True, plain=False)

    def test_different_alignment_views_preserve_both_payloads_and_label(self):
        mapper = scrub.Pseudonymizer(KEY, {UUID})
        token = base64.b64encode(PAYLOAD).rstrip(b"=")
        source = token + b"\nabc\n" + token
        result = scrub.rewrite_text(source, mapper)
        first, label, last = result.split(b"\n")
        self.assertEqual(label, b"abc")
        for encoded in (first, last):
            self.assertEqual(
                base64.b64decode(encoded + b"=" * (-len(encoded) % 4)),
                b"GPU-" + mapper.uuid(UUID),
            )

    def test_multiple_alignment_views_inside_gzip(self):
        mapper = scrub.Pseudonymizer(KEY, {UUID})
        token = base64.b64encode(PAYLOAD).rstrip(b"=")
        source = token + b"\npayload\n" + token
        result = gzip.decompress(scrub.rewrite(gzip.compress(source), mapper))
        first, label, last = result.split(b"\n")
        self.assertEqual(label, b"payload")
        for encoded in (first, last):
            self.assertEqual(
                base64.b64decode(encoded + b"=" * (-len(encoded) % 4)),
                b"GPU-" + mapper.uuid(UUID),
            )

    def test_alignment_matrix_changes_only_encoded_identities(self):
        mapper = scrub.Pseudonymizer(KEY, {UUID})
        expected = b"GPU-" + mapper.uuid(UUID)
        for length in range(2, 17):
            for padded in (True, False):
                with self.subTest(length=length, padded=padded):
                    original = base64.b64encode(PAYLOAD)
                    replacement = base64.b64encode(expected)
                    if not padded:
                        original, replacement = original.rstrip(
                            b"="
                        ), replacement.rstrip(b"=")
                    label = b"x" * length
                    source = label + b"\n" + original + b"\nabc\n" + original
                    wanted = label + b"\n" + replacement + b"\nabc\n" + replacement
                    self.assertEqual(scrub.rewrite_text(source, mapper), wanted)

    def test_labelled_unmarked_compact_counter_is_unchanged(self):
        source = b"payload\n" + base64.b64encode(b"counter=" + UUID.replace(b"-", b""))
        self.assertEqual(
            scrub.rewrite_text(source, scrub.Pseudonymizer(KEY, {UUID})), source
        )

    def test_labelled_unmarked_binary_counter_is_unchanged(self):
        raw = bytes.fromhex(UUID.decode().replace("-", ""))
        source = b"abc\n" + base64.b64encode(raw)
        self.assertEqual(
            scrub.rewrite_text(source, scrub.Pseudonymizer(KEY, {UUID})), source
        )

    def noncanonical_cli(self, compressed):
        encoded = bytearray(base64.b64encode(PAYLOAD))
        alphabet = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/"
        encoded[-3] = alphabet[alphabet.index(encoded[-3]) | 1]
        self.assertEqual(base64.b64decode(encoded), PAYLOAD)
        data = b"payload\n" + encoded
        with tempfile.TemporaryDirectory() as outer:
            root = Path(outer) / "repo"
            root.mkdir()
            subprocess.run(["git", "init", "-q", str(root)], check=True)
            capture = gzip.compress(data) if compressed else data
            (root / "encoded").write_bytes(capture)
            (root / "plain").write_bytes(PAYLOAD)
            subprocess.run(["git", "-C", str(root), "add", "."], check=True)
            key = Path(outer) / "key"
            key.write_bytes(KEY)
            result = subprocess.run(
                [
                    sys.executable,
                    scrub.__file__,
                    "scrub",
                    "--root",
                    str(root),
                    "--key-file",
                    str(key),
                ],
                capture_output=True,
            )
            self.assertEqual(result.returncode, 2)
            self.assertEqual((root / "encoded").read_bytes(), capture)
            self.assertEqual((root / "plain").read_bytes(), PAYLOAD)

    def test_noncanonical_labelled_base64_refuses_without_writes(self):
        self.noncanonical_cli(False)

    def test_noncanonical_labelled_gzip_refuses_without_writes(self):
        self.noncanonical_cli(True)

    def inconsistent_views(self, compressed):
        from unittest.mock import patch

        mapper = scrub.Pseudonymizer(KEY, {UUID, OTHER})
        source = UUID.hex().encode()
        # A contradictory decoder result must not silently choose an identity.
        views = [((0, len(source)), "hex", value) for value in (UUID, OTHER)]
        with patch.object(
            gate,
            "encoded_views",
            side_effect=lambda data, **kwargs: iter(views if data == source else ()),
        ):
            with self.assertRaises(scrub.ScrubError):
                scrub.rewrite(gzip.compress(source) if compressed else source, mapper)

    def test_inconsistent_decoder_views_refused(self):
        self.inconsistent_views(False)

    def test_inconsistent_decoder_views_in_gzip_refused(self):
        self.inconsistent_views(True)

    def inconsistent_width_views(self, compressed):
        from unittest.mock import patch

        mapper = scrub.Pseudonymizer(KEY, {UUID, OTHER})
        source = (b"GPU-" + UUID[:8]).hex().encode()
        views = [
            ((0, len(source)), "hex", b"GPU-" + value[:8]) for value in (UUID, OTHER)
        ]
        with patch.object(
            gate,
            "encoded_views",
            side_effect=lambda data, **kwargs: iter(views if data == source else ()),
        ):
            with self.assertRaises(scrub.ScrubError):
                scrub.rewrite(gzip.compress(source) if compressed else source, mapper)

    def test_inconsistent_width_changing_views_refused(self):
        self.inconsistent_width_views(False)

    def test_inconsistent_width_changing_views_in_gzip_refused(self):
        self.inconsistent_width_views(True)

    def overlapping_aliases(self, compressed):
        mapper = scrub.Pseudonymizer(KEY, {UUID})
        token = base64.b64encode(b"GPU-" + UUID[:8])
        source = token + b"\nabc\n" + token
        with self.assertRaises(scrub.ScrubError):
            scrub.rewrite(gzip.compress(source) if compressed else source, mapper)

    def test_overlapping_width_changing_alias_views_refused(self):
        self.overlapping_aliases(False)

    def test_overlapping_alias_views_in_gzip_refused(self):
        self.overlapping_aliases(True)


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

    def test_known_short_alias_becomes_full_synthetic_reference(self):
        mapper = self.mapper()
        self.assertEqual(
            scrub.rewrite_text(b"board GPU-" + UUID[:8], mapper),
            b"board GPU-" + mapper.uuid(UUID),
        )

    def test_alias_and_provenance_cross_reference_keep_same_identity(self):
        mapper = self.mapper()
        source = PAYLOAD + b"; provenance GPU-" + UUID[:8]
        self.assertEqual(
            scrub.rewrite_text(source, mapper),
            b"GPU-" + mapper.uuid(UUID) + b"; provenance GPU-" + mapper.uuid(UUID),
        )

    def test_ambiguous_short_alias_refuses(self):
        other = UUID[:-1] + b"f"
        mapper = scrub.Pseudonymizer(KEY, {UUID, other})
        with self.assertRaises(scrub.ScrubError):
            scrub.rewrite_text(b"GPU-" + UUID[:8], mapper)

    def test_ambiguous_alias_inside_base64_refuses(self):
        mapper = scrub.Pseudonymizer(KEY, {UUID, UUID[:-1] + b"f"})
        source = base64.b64encode(b"GPU-" + UUID[:8])
        with self.assertRaises(scrub.ScrubError):
            scrub.rewrite_text(source, mapper)

    def test_unrelated_short_alias_stays_unchanged(self):
        source = b"GPU-" + b"98" * 4
        self.assertEqual(scrub.rewrite_text(source, self.mapper()), source)

    def test_marked_compact_rewrites_but_unmarked_counter_stays(self):
        mapper = self.mapper()
        compact = UUID.replace(b"-", b"")
        self.assertEqual(
            scrub.rewrite_text(b"GPU-" + compact + b" counter=" + compact, mapper),
            b"GPU-" + mapper.uuid(UUID).replace(b"-", b"") + b" counter=" + compact,
        )

    def test_compact_gpu_assignment_inside_gzip(self):
        mapper = self.mapper()
        source = b"gpu_uuid=" + UUID.replace(b"-", b"")
        result = gzip.decompress(scrub.rewrite(gzip.compress(source), mapper))
        self.assertEqual(result, b"gpu_uuid=" + mapper.uuid(UUID).replace(b"-", b""))

    def test_wrapped_uuid_preserves_whitespace(self):
        mapper = self.mapper()
        source = b"GPU-\n" + UUID[:13] + b"\r\n" + UUID[13:]
        expected = b"GPU-\n" + mapper.uuid(UUID)[:13] + b"\r\n" + mapper.uuid(UUID)[13:]
        self.assertEqual(scrub.rewrite_text(source, mapper), expected)

    def test_wrapped_uuid_inside_tar(self):
        mapper = self.mapper()
        result = scrub.rewrite(tar([("capture", b"GPU-\n" + UUID)]), mapper)
        with tarfile.open(fileobj=io.BytesIO(result)) as archive:
            self.assertEqual(
                archive.extractfile("capture").read(), b"GPU-\n" + mapper.uuid(UUID)
            )

    def test_eight_character_base64_wrapping_roundtrips(self):
        mapper = self.mapper()
        token = base64.b64encode(PAYLOAD)
        source = b"\n".join(token[i : i + 8] for i in range(0, len(token), 8))
        result = scrub.rewrite_text(source, mapper)
        self.assertEqual(base64.b64decode(result), b"GPU-" + mapper.uuid(UUID))

    def test_short_alias_base64_growth_stays_valid(self):
        mapper = self.mapper()
        source = base64.b64encode(b"GPU-" + UUID[:8])
        self.assertEqual(
            base64.b64decode(scrub.rewrite_text(source, mapper)),
            b"GPU-" + mapper.uuid(UUID),
        )

    def test_short_alias_base64_growth_inside_tar_stays_valid(self):
        mapper = self.mapper()
        source = tar([("encoded", base64.b64encode(b"GPU-" + UUID[:8]))])
        result = scrub.rewrite(source, mapper)
        with tarfile.open(fileobj=io.BytesIO(result)) as archive:
            self.assertEqual(
                base64.b64decode(archive.extractfile("encoded").read()),
                b"GPU-" + mapper.uuid(UUID),
            )

    def test_short_alias_hex_growth_stays_valid(self):
        mapper = self.mapper()
        source = (b"GPU-" + UUID[:8]).hex().encode()
        self.assertEqual(
            bytes.fromhex(scrub.rewrite_text(source, mapper).decode()),
            b"GPU-" + mapper.uuid(UUID),
        )

    def test_formatted_uuid_crossrefs_leave_unmarked_hex_unchanged(self):
        mapper = self.mapper()
        body = PAYLOAD + b" " + UUID + b" " + UUID.replace(b"-", b"")
        result = scrub.rewrite_text(body, mapper)
        self.assertNotIn(UUID, result)
        self.assertEqual(result.count(mapper.uuid(UUID)), 2)
        self.assertIn(UUID.replace(b"-", b""), result)

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

    def test_unmarked_binary_value_inside_record_stays_identical(self):
        mapper = self.mapper()
        raw = bytes.fromhex(UUID.decode().replace("-", ""))
        self.assertEqual(
            scrub.rewrite_text(b"prefix" + raw + b"suffix", mapper),
            b"prefix" + raw + b"suffix",
        )

    def test_utf16_uuid_text_scrubs_without_rewriting_unmarked_binary(self):
        mapper = self.mapper()
        for encoding in ("utf-16le", "utf-16be"):
            result = scrub.rewrite_text(PAYLOAD.decode().encode(encoding), mapper)
            self.assertIn(mapper.uuid(UUID).decode(), result.decode(encoding))
        binary = bytes.fromhex(UUID.decode().replace("-", ""))
        self.assertEqual(
            scrub.rewrite_text(binary, mapper),
            binary,
        )

    def test_numeric_hex_counter_is_not_a_uuid(self):
        low = b"-".join([b"0" * 8, b"0" * 4, b"0" * 4, b"0" * 4, b"0" * 11 + b"1"])
        mapper = scrub.Pseudonymizer(KEY, {low})
        source = b"counter=" + low.replace(b"-", b"")
        self.assertEqual(scrub.rewrite_text(source, mapper), source)

    def test_binary_zero_and_one_counters_are_not_uuid_fields(self):
        low = b"-".join([b"0" * 8, b"0" * 4, b"0" * 4, b"0" * 4, b"0" * 11 + b"1"])
        mapper = scrub.Pseudonymizer(KEY, {low})
        source = b"\0" * 64 + b"\x01" + b"\0" * 32
        self.assertEqual(scrub.rewrite_text(source, mapper), source)

    def test_provenance_uuid_edit_preserves_binary_record_counters(self):
        import struct

        low = b"-".join([b"0" * 8, b"0" * 4, b"0" * 4, b"0" * 4, b"0" * 11 + b"1"])
        mapper = scrub.Pseudonymizer(KEY, {low})
        provenance = b"GPU-" + low
        header = struct.pack(
            "<QIIIIQQQQQQ3Q",
            0x4352434552564B4E,
            1,
            96 + len(provenance),
            32,
            0,
            0,
            0,
            1,
            32,
            0,
            0,
            0,
            0,
            0,
        )
        record = b"\0" * 15 + b"\x01" + b"\0" * 16
        after = scrub.rewrite(header + provenance + record, mapper)
        self.assertEqual(after[:96], header)
        self.assertEqual(after[-32:], record)
        self.assertEqual(after[96:-32], b"GPU-" + mapper.uuid(low))


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


class CompressedManifestCliTests(unittest.TestCase):
    def check_manifest(self, algorithm, kind, manifest):
        with tempfile.TemporaryDirectory() as directory:
            parent = Path(directory)
            root = parent / "repo"
            traces = root / "traces"
            traces.mkdir(parents=True)
            key = parent / "private-fixture-key"
            key.write_bytes(KEY)
            key.chmod(0o600)
            subprocess.run(
                ["git", "init", "-q", str(root)], check=True, capture_output=True
            )
            if kind == "gzip":
                packed = gzip.compress(PAYLOAD)
                target = traces / "log.gz"
            else:
                packed = subprocess.run(
                    ["zstd", "-q", "-c"], input=PAYLOAD, capture_output=True, check=True
                ).stdout
                target = traces / "log.zst"
            target.write_bytes(packed)
            sums = traces / manifest
            sums.write_text(algorithm(PAYLOAD).hexdigest() + "  log\n")
            subprocess.run(
                ["git", "-C", str(root), "add", "traces"],
                check=True,
                capture_output=True,
            )
            with contextlib.redirect_stdout(io.StringIO()), contextlib.redirect_stderr(
                io.StringIO()
            ):
                result = scrub.main(
                    ["scrub", "--root", str(root), "--key-file", str(key)]
                )
            self.assertEqual(result, 0)
            decoded = scrub.unpack_stream(target.read_bytes(), kind)
            self.assertFalse(gate.scan(decoded))
            self.assertEqual(
                sums.read_text(), algorithm(decoded).hexdigest() + "  log\n"
            )

    def test_cli_repairs_md5_for_decompressed_gzip(self):
        self.check_manifest(hashlib.md5, "gzip", "MD5SUMS")

    def test_cli_repairs_sha256_for_decompressed_zstd(self):
        self.check_manifest(hashlib.sha256, "zstd", "SHA256SUMS")


if __name__ == "__main__":
    unittest.main()
