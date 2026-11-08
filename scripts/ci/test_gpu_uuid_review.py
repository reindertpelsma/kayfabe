"""RV2 falsifiers: marked encodings or tracked paths must not pass as clean.

Fixtures are generated identities. Symlink probes must never read their targets.
"""

import base64
import contextlib
import gzip
import io
import json
import os
import struct
import subprocess
import sys
import tempfile
import unittest
from pathlib import Path
from unittest.mock import patch

import no_gpu_uuids as gate
from test_gpu_uuid_archives import UUID, PAYLOAD, tar
import test_gpu_uuid_archives as archives


def wrapped(value, width):
    return b"\n".join(value[i : i + width] for i in range(0, len(value), width))


def skipped_zstd(body):
    return (
        struct.pack("<II", 0x184D2A5F, 5)
        + b"extra"
        + archives.ExtraCompressionTests().zstd(body)
    )


class Base64FramingTests(unittest.TestCase):
    """A neighboring alphanumeric label must not choose the payload's alignment."""

    def cli_case(self, label, width=0, compressed=False):
        encoded = base64.b64encode(PAYLOAD)
        if width:
            encoded = wrapped(encoded, width)
        data = label + b"\n" + encoded
        if compressed:
            data = gzip.compress(data)
        with tempfile.TemporaryDirectory() as d:
            root = Path(d)
            subprocess.run(["git", "init", "-q", d], check=True)
            (root / "capture").write_bytes(data)
            subprocess.run(["git", "-C", d, "add", "capture"], check=True)
            baseline = root / "baseline.json"
            baseline.write_text(json.dumps({"files": {}}))
            before = baseline.read_bytes()
            result = subprocess.run(
                [
                    sys.executable,
                    gate.__file__,
                    "--root",
                    d,
                    "--baseline",
                    str(baseline),
                ],
                capture_output=True,
            )
            self.assertEqual(result.returncode, 1)
            self.assertEqual(baseline.read_bytes(), before)

    def test_payload_label_contiguous_cli(self):
        self.cli_case(b"payload")

    def test_abc_label_contiguous_cli(self):
        self.cli_case(b"abc")

    def test_payload_label_wrapped_cli(self):
        self.cli_case(b"payload", width=8)

    def test_abc_label_wrapped_cli(self):
        self.cli_case(b"abc", width=8)

    def test_labelled_contiguous_gzip_cli(self):
        self.cli_case(b"payload", compressed=True)

    def test_labelled_wrapped_gzip_cli(self):
        self.cli_case(b"abc", width=8, compressed=True)

    def test_every_label_alignment_and_wrapping(self):
        encoded = base64.b64encode(PAYLOAD)
        for length in range(2, 17):
            for width in (4, 8, 20, 100):
                with self.subTest(length=length, width=width):
                    data = b"x" * length + b"\n" + wrapped(encoded, width)
                    self.assertEqual(gate.scan(data), gate.scan(PAYLOAD))

    def test_multiple_label_lines_before_payload(self):
        data = b"header\nabc\npayload\n" + wrapped(base64.b64encode(PAYLOAD), 8)
        self.assertEqual(gate.scan(data), gate.scan(PAYLOAD))

    def test_unpadded_payload_before_label(self):
        data = base64.b64encode(PAYLOAD).rstrip(b"=") + b"\nabc"
        self.assertEqual(gate.scan(data), gate.scan(PAYLOAD))

    def test_unpadded_wrapped_payload_before_label(self):
        data = wrapped(base64.b64encode(PAYLOAD).rstrip(b"="), 8) + b"\npayload"
        self.assertEqual(gate.scan(gzip.compress(data)), gate.scan(PAYLOAD))

    def test_two_payloads_at_different_alignments(self):
        encoded = base64.b64encode(PAYLOAD).rstrip(b"=")
        self.assertEqual(
            gate.scan(encoded + b"\nabc\n" + encoded), gate.scan(PAYLOAD * 2)
        )

    def test_alignment_work_is_bounded_independently_of_label_count(self):
        decode = gate.base64.b64decode
        with patch.object(gate.base64, "b64decode", wraps=decode) as calls:
            self.assertFalse(gate.scan(b"abcd\n" * 2000))
        self.assertLessEqual(calls.call_count, 8)

    def test_many_labels_keep_bounded_work_and_payload_detection(self):
        data = b"abc\npayload\n" * 1000 + wrapped(base64.b64encode(PAYLOAD), 8)
        decode = gate.base64.b64decode
        with patch.object(gate.base64, "b64decode", wraps=decode) as calls:
            self.assertEqual(gate.scan(data), gate.scan(PAYLOAD))
        self.assertLessEqual(calls.call_count, 8)


class MarkedFormatTests(unittest.TestCase):
    def check(self, value):
        identities = set()
        self.assertEqual(gate.scan(value, _uuids=identities), gate.scan(PAYLOAD))
        self.assertEqual(identities, {UUID})

    def test_marked_compact_gpu_prefix(self):
        self.check(b"GPU-" + UUID.replace(b"-", b""))

    def test_marked_compact_assignment_in_gzip(self):
        self.check(gzip.compress(b'gpu_uuid="' + UUID.replace(b"-", b"") + b'"'))

    def test_newline_inside_canonical_uuid(self):
        self.check(b"GPU-" + UUID[:13] + b"\n" + UUID[13:])

    def test_newline_after_gpu_marker_in_tar(self):
        self.check(tar([("x", b"GPU-\n" + UUID)]))

    def test_base64_wrapped_at_eight_characters(self):
        self.check(wrapped(base64.b64encode(PAYLOAD), 8))

    def test_base64_wrapped_at_four_characters_in_gzip(self):
        self.check(gzip.compress(wrapped(base64.b64encode(PAYLOAD), 4)))

    def test_hex_compact_uuid_in_gzip(self):
        self.check(
            gzip.compress((b"gpu_uuid=" + UUID.replace(b"-", b"")).hex().encode())
        )

    def test_hex_compact_device_assignment(self):
        self.check((b"deviceUUID=" + UUID.replace(b"-", b"")).hex().encode())

    def test_utf16_compact_uuid(self):
        self.check((b"GPU-" + UUID.replace(b"-", b"")).decode().encode("utf-16be"))

    def test_synthetic_compact_and_wrapped_are_exempt(self):
        fake = gate.SYNTHETIC_PREFIX.encode() + b"0123"
        for value in (fake.replace(b"-", b""), wrapped(fake, 8)):
            self.assertFalse(gate.scan(b"GPU-\n" + value))

    def test_unmarked_compact_and_binary_are_not_board_fields(self):
        compact = UUID.replace(b"-", b"")
        self.assertFalse(gate.scan(b"counter=" + compact))
        self.assertFalse(gate.scan(bytes.fromhex(compact.decode())))

    def test_long_marker_whitespace_still_detected(self):
        self.check(b"GPU-" + b" " * 65 + UUID)

    def test_long_internal_uuid_whitespace_still_detected(self):
        self.check(b"GPU-" + UUID[:8] + b"\n" * 65 + UUID[8:])

    def test_excessive_marker_wrapping_refused(self):
        with self.assertRaises(gate.ScanError):
            gate.scan(b"GPU-" + b" " * 5000 + UUID)

    def test_excessive_internal_wrapping_refused(self):
        with self.assertRaises(gate.ScanError):
            gate.scan(b"GPU-" + UUID[:8] + b"\n" * 5000 + UUID[8:])

    def test_all_review_formats_fail_complete_cli_in_each_container(self):
        compact = UUID.replace(b"-", b"")
        forms = (
            b"GPU-" + compact,
            b"gpu_uuid=" + compact,
            b"GPU-" + UUID[:13] + b"\n" + UUID[13:],
            b"GPU-\n" + UUID,
            wrapped(base64.b64encode(PAYLOAD), 8),
        )
        wrappers = (
            lambda x: x,
            gzip.compress,
            archives.ExtraCompressionTests().zstd,
            lambda x: tar([("x", x)]),
        )
        for index, value in enumerate(forms):
            for wrap in wrappers:
                with self.subTest(
                    form=index, wrapper=wrap.__name__
                ), tempfile.TemporaryDirectory() as d:
                    root = Path(d)
                    subprocess.run(["git", "init", "-q", d], check=True)
                    (root / "capture").write_bytes(wrap(value))
                    subprocess.run(["git", "-C", d, "add", "capture"], check=True)
                    baseline = root / "baseline.json"
                    baseline.write_text(json.dumps({"files": {}}))
                    out = io.StringIO()
                    with contextlib.redirect_stdout(out), contextlib.redirect_stderr(
                        out
                    ):
                        self.assertEqual(
                            gate.main(["--root", d, "--baseline", str(baseline)]), 1
                        )
                    self.assertNotIn(UUID.decode(), out.getvalue())
                    self.assertEqual(json.loads(baseline.read_text()), {"files": {}})


class SkippableZstdTests(unittest.TestCase):
    def test_leading_skippable_zstd_frame(self):
        self.assertEqual(
            gate.scan(skipped_zstd(PAYLOAD * 100)), gate.scan(PAYLOAD * 100)
        )

    def test_skippable_zstd_nested_in_gzip(self):
        self.assertEqual(
            gate.scan(gzip.compress(skipped_zstd(PAYLOAD * 100))),
            gate.scan(PAYLOAD * 100),
        )

    def test_skippable_zstd_cli_decoder_fallback(self):
        import sys

        with patch.dict(sys.modules, {"compression": None}):
            self.assertTrue(gate.scan(skipped_zstd(PAYLOAD)))

    def test_skippable_zstd_preserves_expansion_limit(self):
        with patch.object(gate, "MAX_MEMBER_BYTES", 100), self.assertRaises(
            gate.ScanError
        ):
            gate.scan(skipped_zstd(b"x" * 101))

    def test_skippable_zstd_truncated_frame_refused(self):
        with self.assertRaises(gate.ScanError):
            gate.scan(struct.pack("<II", 0x184D2A50, 100) + b"short")

    def test_every_skippable_magic_is_recognized(self):
        frame = archives.ExtraCompressionTests().zstd(PAYLOAD)
        for magic in range(0x184D2A50, 0x184D2A60):
            with self.subTest(magic=magic):
                self.assertTrue(gate.scan(struct.pack("<II", magic, 0) + frame))

    def test_every_skippable_magic_nested_in_gzip(self):
        frame = archives.ExtraCompressionTests().zstd(PAYLOAD)
        for magic in range(0x184D2A50, 0x184D2A60):
            with self.subTest(magic=magic):
                self.assertTrue(
                    gate.scan(gzip.compress(struct.pack("<II", magic, 0) + frame))
                )

    def test_skippable_frame_cannot_hide_decoder_window_limit(self):
        frame = archives.DecoderMemoryTests.large_zstd_window()
        with self.assertRaises(gate.ScanError):
            gate.scan(struct.pack("<II", 0x184D2A50, 0) + frame)

    def test_skippable_frame_nested_window_limit(self):
        frame = archives.DecoderMemoryTests.large_zstd_window()
        with self.assertRaises(gate.ScanError):
            gate.scan(gzip.compress(struct.pack("<II", 0x184D2A50, 0) + frame))

    def cli_rejects(self, data):
        with tempfile.TemporaryDirectory() as d:
            root = Path(d)
            subprocess.run(["git", "init", "-q", d], check=True)
            (root / "capture").write_bytes(data)
            subprocess.run(["git", "-C", d, "add", "capture"], check=True)
            baseline = root / "baseline.json"
            baseline.write_text(json.dumps({"files": {}}))
            output = io.StringIO()
            with contextlib.redirect_stdout(output), contextlib.redirect_stderr(output):
                self.assertEqual(
                    gate.main(["--root", d, "--baseline", str(baseline)]), 1
                )

    def test_skippable_frame_full_cli(self):
        self.cli_rejects(skipped_zstd(PAYLOAD * 100))

    def test_skippable_frame_nested_full_cli(self):
        self.cli_rejects(gzip.compress(skipped_zstd(PAYLOAD * 100)))


class TrackedCoverageTests(unittest.TestCase):
    def check_path(self, rel, git=True):
        with tempfile.TemporaryDirectory() as d:
            root = Path(d)
            path = root / rel
            path.parent.mkdir(parents=True, exist_ok=True)
            path.write_bytes(PAYLOAD)
            if git:
                subprocess.run(["git", "init", "-q", d], check=True)
                subprocess.run(["git", "-C", d, "add", rel], check=True)
            baseline = root / "baseline.json"
            baseline.write_text(json.dumps({"files": {}}))
            output = io.StringIO()
            with contextlib.redirect_stdout(output):
                self.assertEqual(
                    gate.main(["--root", d, "--baseline", str(baseline)]), 1
                )
            self.assertIn(rel, gate.tracked(root))

    def test_root_tracked_file(self):
        self.check_path("root-capture.txt")

    def test_firmware_tracked_file(self):
        self.check_path("firmware/capture.txt")

    def test_root_without_git(self):
        self.check_path("root-capture.txt", git=False)

    def test_firmware_without_git(self):
        self.check_path("firmware/capture.txt", git=False)

    def test_gitlink_excluded_but_regular_neighbor_included(self):
        with tempfile.TemporaryDirectory() as d:
            root = Path(d)
            subprocess.run(["git", "init", "-q", d], check=True)
            subprocess.run(
                [
                    "git",
                    "-C",
                    d,
                    "update-index",
                    "--add",
                    "--cacheinfo",
                    "160000," + "1" * 40 + ",submodule",
                ],
                check=True,
            )
            (root / "capture").write_bytes(PAYLOAD)
            subprocess.run(["git", "-C", d, "add", "capture"], check=True)
            self.assertEqual(gate.tracked(root), ["capture"])

    def test_missing_gitlink_is_explicitly_outside_this_repository(self):
        with tempfile.TemporaryDirectory() as d:
            root = Path(d)
            subprocess.run(["git", "init", "-q", d], check=True)
            subprocess.run(
                [
                    "git",
                    "-C",
                    d,
                    "update-index",
                    "--add",
                    "--cacheinfo",
                    "160000," + "1" * 40 + ",submodule",
                ],
                check=True,
            )
            self.assertFalse(gate.tracked(root))

    def test_symlink_target_text_is_scanned(self):
        with tempfile.TemporaryDirectory() as d:
            root = Path(d)
            (root / "link").symlink_to(PAYLOAD.decode())
            self.assertEqual(
                gate.scan_tree(root, ["link"]), {"link": gate.scan(PAYLOAD)}
            )

    def test_dangling_directory_link_text_is_scanned_without_git(self):
        with tempfile.TemporaryDirectory() as d:
            root = Path(d)
            (root / "link").symlink_to(PAYLOAD.decode(), target_is_directory=True)
            self.assertIn("link", gate.tracked(root))
            self.assertTrue(gate.scan_tree(root, ["link"]))

    def test_outside_symlink_target_is_not_read(self):
        with tempfile.TemporaryDirectory() as d, tempfile.TemporaryDirectory() as outside:
            root = Path(d)
            target = Path(outside) / "capture"
            target.write_bytes(PAYLOAD)
            (root / "link").symlink_to(target)
            self.assertFalse(gate.scan_tree(root, ["link"]))

    def test_symlink_parent_cannot_escape_tree(self):
        with tempfile.TemporaryDirectory() as d, tempfile.TemporaryDirectory() as outside:
            root = Path(d)
            (Path(outside) / "capture").write_bytes(PAYLOAD)
            (root / "parent").symlink_to(outside, target_is_directory=True)
            with self.assertRaises((OSError, gate.ScanError)):
                gate.scan_tree(root, ["parent/capture"])

    def test_symlink_parent_is_refused_even_for_clean_target(self):
        with tempfile.TemporaryDirectory() as d, tempfile.TemporaryDirectory() as outside:
            root = Path(d)
            (Path(outside) / "capture").write_bytes(b"clean")
            (root / "parent").symlink_to(outside, target_is_directory=True)
            with self.assertRaises((OSError, gate.ScanError)):
                gate.scan_tree(root, ["parent/capture"])

    def test_nonregular_file_refused_without_opening(self):
        with tempfile.TemporaryDirectory() as d:
            root = Path(d)
            os.mkfifo(root / "pipe")
            with self.assertRaises(gate.ScanError):
                gate.scan_tree(root, ["pipe"])

    def test_nonregular_file_cli_refuses_without_baseline_write(self):
        with tempfile.TemporaryDirectory() as d:
            root = Path(d)
            os.mkfifo(root / "pipe")
            baseline = root / "baseline.json"
            baseline.write_text(json.dumps({"files": {}}))
            before = baseline.read_bytes()
            output = io.StringIO()
            with contextlib.redirect_stderr(output), contextlib.redirect_stdout(output):
                self.assertEqual(
                    gate.main(["--root", d, "--baseline", str(baseline), "--shrink"]), 2
                )
            self.assertEqual(baseline.read_bytes(), before)
