"""Exercise the actual fast launcher with inert QEMU and device fixtures."""
import os
from pathlib import Path
import subprocess
import tempfile
import unittest

REPO = Path(__file__).resolve().parents[3]
RUNNER = REPO / "scripts/fastguest/run_fast_guest.sh"


class LaunchSafety(unittest.TestCase):
    def launch(self, name):
        with tempfile.TemporaryDirectory() as folder:
            root = Path(folder)
            binary = root / "bin"
            binary.mkdir()
            guest = root / "guest"
            guest.mkdir()
            for asset in ("vmlinuz", "initrd.cpio.gz"):
                (guest / asset).touch()
            (binary / "pkill").write_text(
                '#!/bin/sh\nprintf "%s\\n" "$*" >> "$KILL_LOG"\n'
            )
            (binary / "nvidia-smi").write_text('#!/bin/sh\necho 0\n')
            qemu = binary / "qemu"
            qemu.write_text(
                '#!/bin/sh\nprintf "%s\\n" "$@" > "$ARGV_LOG"\n'
                'printf "FASTGUEST: client rc=0\\nFASTGUEST: DONE\\n" > "$SERIAL"\n'
            )
            for script in binary.iterdir():
                script.chmod(0o755)
            env = dict(os.environ)
            env.update(
                PATH=f"{binary}:{env['PATH']}",
                BENCH_DIR=str(root),
                KF_FASTGUEST_DIR=str(guest),
                QEMU_BIN=str(qemu),
                KF_LOCK=str(root / "private.lock"),
                KF_VM_NAME=name,
                KF3_FB_MB="4096",
                KILL_LOG=str(root / "kills"),
                ARGV_LOG=str(root / "argv"),
                SERIAL=str(root / "fast_fixture_serial.log"),
            )
            result = subprocess.run(
                ["bash", str(RUNNER), "fixture", "20"],
                env=env, capture_output=True, text=True, check=False,
            )
            self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
            kills = (root / "kills").read_text() if (root / "kills").exists() else ""
            return kills, (root / "argv").read_text().splitlines()

    def test_does_not_kill_a_second_fast_guest(self):
        kills, _ = self.launch("kf-z1-vm-a")
        self.assertEqual(kills, "")

    def test_does_not_kill_an_unrelated_owners_process(self):
        kills, _ = self.launch("kf-z1-rootless")
        self.assertEqual(kills, "")

    def test_vm_a_has_its_own_name(self):
        _, argv = self.launch("kf-z1-vm-a")
        self.assertEqual(argv[argv.index("-name") + 1], "kf-z1-vm-a")

    def test_vm_b_has_its_own_name(self):
        _, argv = self.launch("kf-z1-vm-b")
        self.assertEqual(argv[argv.index("-name") + 1], "kf-z1-vm-b")


if __name__ == "__main__":
    unittest.main()
