"""Exercise the real builder using only inert device/mount executable fixtures."""

import json
import os
from pathlib import Path
import signal
import subprocess
import sys
import tempfile
import unittest

HERE = Path(__file__).resolve().parent
BUILDER = HERE.parent / "build_fast_guest.sh"


class BuildCleanup(unittest.TestCase):
    def build(self, **settings):
        with tempfile.TemporaryDirectory() as folder:
            root = Path(folder)
            commands = root / "bin"
            commands.mkdir()
            mock = commands / "mock"
            mock.write_text(f"#!{sys.executable}\n" + (HERE / "build_cleanup_mock.py").read_text())
            mock.chmod(0o755)
            for name in (
                "qemu-nbd",
                "mount",
                "umount",
                "modprobe",
                "partprobe",
                "udevadm",
                "pgrep",
                "uname",
                "cp",
                "cpio",
                "busybox",
                "ldd",
                "sleep",
            ):
                (commands / name).symlink_to(mock)
            image = root / "image.qcow2"
            image.touch()
            client = root / "client"
            client.touch()
            client.chmod(0o755)
            calls = root / "calls.jsonl"
            env = {
                "PATH": f"{commands}:/usr/bin:/bin",
                "CLIENT": str(client),
                "KF_ROOT": str(root),
                "BUILDER_CALLS": str(calls),
                **settings,
            }
            result = subprocess.run(
                ["bash", str(BUILDER), str(image), str(root / "output")],
                env=env,
                capture_output=True,
                text=True,
                timeout=10,
            )
            operations = [json.loads(line) for line in calls.read_text().splitlines()]
            nbd = [row[1:] for row in operations if row[0] == "qemu-nbd"]
            releases = [row for row in nbd if row[0] == "--disconnect"]
            return result, nbd, releases

    def test_host_mode_failure_never_disconnects_another_attachment(self):
        result, operations, releases = self.build(KF_FROM_HOST="1")
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("no /boot/vmlinuz-", result.stderr)
        self.assertEqual(operations, [])
        self.assertEqual(releases, [])

    def test_skip_nbd_pack_failure_never_disconnects(self):
        result, operations, releases = self.build(SKIP_NBD="1", BUILDER_PACK_RC="1")
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("cpio failed", result.stderr)
        self.assertEqual(operations, [])
        self.assertEqual(releases, [])

    def test_unstaged_driver_failure_before_attach_never_disconnects(self):
        result, operations, releases = self.build(KF_GUEST_DRIVER_DIR="/nonexistent-fixture")
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("no STAGED marker", result.stderr)
        self.assertEqual(operations, [])
        self.assertEqual(releases, [])

    def test_failed_attach_does_not_disconnect_an_unacquired_device(self):
        result, operations, releases = self.build(BUILDER_ATTACH_RC="1")
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("could not attach", result.stderr)
        self.assertEqual(len(operations), 1)
        self.assertEqual(releases, [])

    def test_term_before_attach_never_disconnects(self):
        result, _, releases = self.build(
            KF_FROM_HOST="1", BUILDER_SIGNAL_AT="host-copy", BUILDER_SIGNAL=str(signal.SIGTERM)
        )
        self.assertNotEqual(result.returncode, 0)
        self.assertEqual(releases, [])

    def test_interrupted_failed_attach_does_not_claim_ownership(self):
        result, _, releases = self.build(
            BUILDER_ATTACH_RC="1",
            BUILDER_SIGNAL_AT="attach",
            BUILDER_SIGNAL=str(signal.SIGTERM),
        )
        self.assertNotEqual(result.returncode, 0)
        self.assertEqual(releases, [])

    def test_owned_attachment_is_released_after_mount_failure(self):
        result, _, releases = self.build(BUILDER_MOUNT_RC="1")
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("cannot mount the guest root", result.stderr)
        self.assertEqual(releases, [["--disconnect", "/dev/nbd0"]])

    def test_owned_attachment_is_released_after_term(self):
        result, _, releases = self.build(
            BUILDER_SIGNAL_AT="mount", BUILDER_SIGNAL=str(signal.SIGTERM)
        )
        self.assertNotEqual(result.returncode, 0)
        self.assertEqual(releases, [["--disconnect", "/dev/nbd0"]])

    def test_owned_attachment_is_released_after_int(self):
        result, _, releases = self.build(
            BUILDER_SIGNAL_AT="mount", BUILDER_SIGNAL=str(signal.SIGINT)
        )
        self.assertNotEqual(result.returncode, 0)
        self.assertEqual(releases, [["--disconnect", "/dev/nbd0"]])

    def test_successful_attach_interrupted_by_term_is_released(self):
        result, _, releases = self.build(
            BUILDER_SIGNAL_AT="attach", BUILDER_SIGNAL=str(signal.SIGTERM)
        )
        self.assertNotEqual(result.returncode, 0)
        self.assertEqual(releases, [["--disconnect", "/dev/nbd0"]])

    def test_successful_attach_interrupted_by_int_is_released(self):
        result, _, releases = self.build(
            BUILDER_SIGNAL_AT="attach", BUILDER_SIGNAL=str(signal.SIGINT)
        )
        self.assertNotEqual(result.returncode, 0)
        self.assertEqual(releases, [["--disconnect", "/dev/nbd0"]])

    def test_pack_failure_after_release_does_not_disconnect_twice(self):
        result, _, releases = self.build(BUILDER_PACK_RC="1")
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("cpio failed", result.stderr)
        self.assertEqual(releases, [["--disconnect", "/dev/nbd0"]])

    def test_normal_completion_disconnects_exactly_once(self):
        result, _, releases = self.build()
        self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
        self.assertEqual(releases, [["--disconnect", "/dev/nbd0"]])

    def test_successful_release_interrupted_by_term_does_not_disconnect_twice(self):
        result, _, releases = self.build(
            BUILDER_SIGNAL_AT="release", BUILDER_SIGNAL=str(signal.SIGTERM)
        )
        self.assertNotEqual(result.returncode, 0)
        self.assertEqual(releases, [["--disconnect", "/dev/nbd0"]])

    def test_successful_release_interrupted_by_int_does_not_disconnect_twice(self):
        result, _, releases = self.build(
            BUILDER_SIGNAL_AT="release", BUILDER_SIGNAL=str(signal.SIGINT)
        )
        self.assertNotEqual(result.returncode, 0)
        self.assertEqual(releases, [["--disconnect", "/dev/nbd0"]])

    def test_failed_release_is_retried_while_still_owned(self):
        result, _, releases = self.build(BUILDER_RELEASE_RETRY="1")
        self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
        self.assertEqual(releases, [["--disconnect", "/dev/nbd0"]] * 2)

    def test_pack_failure_retries_a_failed_owned_release(self):
        result, _, releases = self.build(BUILDER_PACK_RC="1", BUILDER_RELEASE_RETRY="1")
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("cpio failed", result.stderr)
        self.assertEqual(releases, [["--disconnect", "/dev/nbd0"]] * 2)


if __name__ == "__main__":
    unittest.main()
