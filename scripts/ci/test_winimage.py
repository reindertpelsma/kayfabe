#!/usr/bin/env python3
"""GPU-free, VM-free falsifiers for scripts/bench/windows/winimage/ (the Windows disk-capture kit).

No QEMU, no Windows, no network, no root. The shell scripts are run in their self-check mode or driven
into their refusal paths, and the PowerShell and shell sources are checked for the properties the
capture must keep: both boot paths are swapped and restored, a backup is made first, Secure Boot ON
refuses, the disk goes read-only before it is read, nothing in the kit can write to a Windows partition
or to the BCD/NVRAM. What this does NOT prove: that a UKI boots on a real firmware, that the swap works
on a real ESP, or that a captured disk boots; the real runs are in traces/winimage_skeleton_20261107/
and uki_selftest.sh (needs /dev/kvm).
"""

import os
import re
import subprocess
import tempfile
import unittest
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
D = ROOT / "scripts/bench/windows/winimage"


def run(args, env=None, timeout=60):
    e = dict(os.environ)
    e.update(env or {})
    return subprocess.run(args, capture_output=True, text=True, env=e, timeout=timeout)


class Kit(unittest.TestCase):
    def test_every_shell_script_parses(self):
        for f in sorted(D.glob("*.sh")):
            r = run(["bash", "-n", str(f)])
            self.assertEqual(r.returncode, 0, f"{f.name}: {r.stderr}")

    def test_uki_builder_selfcheck(self):
        r = run(["bash", str(D / "build_alpine_uki.sh")], {"TEST_ONLY": "1"})
        self.assertEqual(r.returncode, 0, r.stderr)
        self.assertIn("SELFCHECK_OK", r.stdout)

    def test_uki_builder_refuses_a_private_key(self):
        with tempfile.TemporaryDirectory() as t:
            k = Path(t) / "k"
            k.write_text("-----BEGIN OPENSSH PRIVATE KEY-----\nAAAA\n-----END OPENSSH PRIVATE KEY-----\n")
            r = run(["bash", str(D / "build_alpine_uki.sh")], {"AUTH_KEY_PUB": str(k), "WORK": t})
            self.assertNotEqual(r.returncode, 0)
            self.assertIn("private key", r.stderr)

    def test_uki_builder_refuses_a_missing_key(self):
        r = run(["bash", str(D / "build_alpine_uki.sh")], {"AUTH_KEY_PUB": "/nonexistent/key.pub"})
        self.assertNotEqual(r.returncode, 0)

    def test_uki_init_keeps_the_way_back_and_the_lock(self):
        s = (D / "build_alpine_uki.sh").read_text()
        self.assertIn("kf.autorestore", s)            # the recovery timer
        self.assertIn("kf-restore-esp", s)
        self.assertIn("PasswordAuthentication no", s)
        self.assertIn("prohibit-password", s)
        self.assertNotRegex(s, r"PermitRootLogin\s+yes")
        # the image gets the PUBLIC key only
        self.assertIn("is not a public key", s)

    def test_restore_covers_both_paths_and_verifies(self):
        s = (D / "build_alpine_uki.sh").read_text()
        self.assertIn("EFI/Microsoft/Boot/bootmgfw.efi", s)
        self.assertIn("EFI/BOOT/BOOTX64.EFI", s)
        self.assertIn("sha256sum", s)
        self.assertIn("BOOTX64.EFI.absent", s)        # a fallback that did not exist must not keep pointing at the UKI

    def test_esp_swap_properties(self):
        s = (D / "esp_swap.ps1").read_text()
        self.assertIn("bootmgfw.efi", s)
        self.assertIn("BOOTX64.EFI", s)
        self.assertIn("kf-backup", s)
        # the backup is written before the replacement
        self.assertLess(s.index("bootmgfw.efi\"", s.index("kf-backup")), s.index("Copy-Item -LiteralPath $Uki -Destination $boot"))
        # Secure Boot ON refuses
        self.assertRegex(s, r"\$sb -eq 'on'.*REFUSED", )
        # it checks the UKI hash and free space
        self.assertIn("ExpectSha256", s)
        self.assertIn("SizeRemaining", s)
        # nothing in the script edits the BCD or NVRAM, formats, partitions or deletes outside the ESP
        for bad in ("bcdedit /set", "bcdboot", "Format-Volume", "Remove-Partition", "Clear-Disk", "Set-Disk", "diskpart", "bcdedit /delete"):
            self.assertNotIn(bad, s)

    def test_capture_is_read_only_and_pull_based(self):
        s = (D / "capture_raw.sh").read_text()
        self.assertLess(s.index("blockdev --setro"), s.index("dd if="))
        self.assertNotRegex(s, r"of=/dev/")           # no dd to a device, ever
        self.assertNotRegex(s, r"mount [^\n]*-o rw")
        self.assertIn("zstd", s)
        self.assertIn("sha256", s)

    def test_capture_modes_use_zero_detection_and_never_write_the_source(self):
        s = (D / "capture_raw.sh").read_text()
        self.assertEqual(s.count("--detect-zeroes=unmap"), 2)   # ntfsclone writes zeros for free clusters: they must not be allocated
        self.assertNotIn("of=$DISK", s)
        self.assertNotIn('of="$DISK"', s)
        for mode in ("dd)", "dd-qcow2)", "ntfsclone-qcow2)", "ntfsclone-probe)"):
            self.assertIn("  " + mode, s)

    def test_uki_init_mounts_the_esp_case_insensitively_and_arms_on_auth_only(self):
        s = (D / "build_alpine_uki.sh").read_text()
        self.assertNotRegex(s, r"mount -t vfat -o (ro|rw) ")   # every vfat mount names iocharset
        self.assertIn("iocharset=iso8859-1", s)
        self.assertIn("[s]shd: root@", s)
        self.assertIn("af_packet", s)                        # udhcpc needs AF_PACKET
        self.assertIn("/dev/fd", s)

    def test_capture_usage_error(self):
        r = run(["bash", str(D / "capture_raw.sh")])
        self.assertNotEqual(r.returncode, 0)

    def test_boot_test_usage_and_refusals(self):
        r = run(["bash", str(D / "qemu_boot_test.sh"), "/nonexistent.qcow2", "/tmp/kf-none"])
        self.assertEqual(r.returncode, 2)
        r = run(["bash", str(D / "qemu_boot_test.sh"), "/nonexistent.qcow2", "/tmp/kf-none", "--sb", "bad"])
        self.assertEqual(r.returncode, 2)

    def test_boot_test_has_the_ci_shape(self):
        s = (D / "qemu_boot_test.sh").read_text()
        for needle in ("pc-q35-10.2", "smm=on", "tpm-crb", "virtio-blk-pci", "virtio-net-pci", "-vga std", "base=utc",
                       "hostfwd", "screendump", "-uuid", "-smbios", "-F qcow2"):
            self.assertIn(needle, s)
        # the image itself is never opened read-write: only an overlay
        self.assertNotRegex(s, r'-drive "file=\$IMG')


if __name__ == "__main__":
    unittest.main()
