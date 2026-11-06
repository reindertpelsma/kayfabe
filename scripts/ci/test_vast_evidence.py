"""GPU-free tests of the box evidence bundle (vast_evidence.py).

FALSIFIER: a bundle (or an upload) that contains a run token, the broker URL or its secret path,
the Access pair, the runner nonce, the guest password, a harness ssh key, a Vast key, a presigned
URL, a JWT, a bearer header, a private-key block or a GitHub token; that keeps an unscrubbed GPU
UUID, serial number, MAC address or public IP in a text member; that exceeds 8 MiB compressed or
48 MiB unpacked; that drops a higher tier before a lower one; that reads secrets/, tpm/, firmware/,
disk/, a symlink or a path outside its roots; that changes the result file or the exit status; or
that is uploaded after a scan hit.
"""

import gzip
import io
import json
import os
import random
import shutil
import subprocess
import tarfile
import tempfile
import time
import unittest
from pathlib import Path
from unittest import mock

import vast_evidence as ev

RUN = "4242-7"
TOKEN = ".".join(["eyJhbGciOiJIUzI1NiJ9", "eyJydW4iOiI0MjQyIn0", ("Zk9_-Qw" * 7)[:43]])
PREFIX = "pfx-" + "9f3a7c1d" * 5
URL = "https://broker.example/" + PREFIX
RNONCE = "cd" * 32
ACCESS_ID = "0123abcd" * 4 + ".access"
ACCESS_SECRET = "9f8e7d6c" * 8
PASSWORD = "Tq7+Zl0/pass+word=Aa1Bb2Cc3Dd4Ee5Ff6=="
KEY_LINE = "b3BlbnNzaC1rZXktdjEAAAAABG5vbmUAAAAEbm9uZQAAAAAAAAABAAAAMwAAAAtzc2gtZW"
UUID_A = "3f2504e0-4f89-41d3-9a0c-0305e82c3301"
UUID_B = "6ba7b810-9dad-11d1-80b4-00c04fd430c8"
FAKE = "fa4efa4e-fa4e-4fa4-8fa4-fa4efa4e"
NOW = 1_800_000_000.0


def env(**extra):
    base = {
        "KF_PROTOCOL": "7",
        "KF_BROKER_URL": URL,
        "KF_RUN_ID": RUN,
        "KF_RUN_TOKEN": TOKEN,
        "KF_NONCE": "ab" * 16,
        "KF_SHA": "1234abcd" * 5,
        "KF_REPO": "reindertpelsma/kayfabe",
        "KF_RUNNER_NONCE": RNONCE,
        "KF_SOFT_DEADLINE": str(int(time.time()) + 3000),
        "KF_HEARTBEAT_SECONDS": "30",
        "KF_GRACE_FAIL": "600",
        "KF_ACCESS_ID": ACCESS_ID,
        "KF_ACCESS_SECRET": ACCESS_SECRET,
    }
    base.update(extra)
    return base


def no_commands(argv):
    return ("[fake %s]\n" % argv[0]).encode()


def fake_commands(argv):
    name = argv[0]
    if name == "dmesg":
        return b"".join(b"[ Mon Nov  6 10:00:%02d 2026] line %d\n" % (i % 60, i) for i in range(50))
    if name == "nvidia-smi":
        return (
            b"Driver Version : 580.95.05\n"
            + b"    GPU UUID                              : GPU-%s\n" % UUID_A.encode()
            + b"    Serial Number                         : 1320722017777\n"
        )
    if name == "ps":
        return (
            b"  PID  PPID     ELAPSED STAT COMMAND\n"
            b"    1     0       00:10 Ss   /sbin/init\n"
            b"  777     1       00:05 Sl   qemu-system-x86_64 -name win\n"
            b"  778     1       00:05 Sl   swtpm socket --tpmstate dir=/x\n"
        )
    return ("[fake %s]\n" % name).encode()


class Tree:
    """A fake box: /workspace, /root/prov and the Windows VM root, all under one temp dir."""

    def __init__(self, case):
        self.root = Path(tempfile.mkdtemp(prefix="kf-evidence-test-"))
        case.addCleanup(shutil.rmtree, self.root, True)
        self.ws = self.root / "workspace"
        self.prov = self.root / "prov"
        self.winvm = self.ws / "winvm"
        self.home = self.root / "home"
        self.vm = self.winvm / "kfwin"
        for d in (self.ws, self.prov, self.home, self.vm / "logs" / "shots"):
            d.mkdir(parents=True, exist_ok=True)
        for d in ("secrets", "tpm", "firmware", "disk"):
            (self.vm / d).mkdir(exist_ok=True)
        (self.vm / "secrets" / "win_password").write_text(PASSWORD)
        (self.vm / "secrets" / "ssh_ed25519").write_text(
            "-----BEGIN OPENSSH PRIVATE KEY-----\n%s\n-----END OPENSSH PRIVATE KEY-----\n"
            % KEY_LINE
        )
        (self.vm / "tpm" / "tpm2-00.permall").write_text("TPM-STATE-SECRET-NEVER-IN-BUNDLE")
        (self.vm / "firmware" / "OVMF_VARS.fd").write_text("FIRMWARE-NEVER-IN-BUNDLE")
        (self.vm / "disk" / "win.qcow2").write_text("DISK-NEVER-IN-BUNDLE")
        self.log = self.ws / "kf-ci-output.log"
        self.log.write_text("payload started\nKF_PAYLOAD_STAGE_START windows_gate\nFAIL boot1\n")
        (self.ws / "kf-result.json").write_text('{"verdict":"FAIL","suites":[]}')

    def roots(self):
        return ev.Roots(self.ws, self.prov, self.winvm, self.home)

    def tgz(self, path, members):
        with tarfile.open(path, "w:gz") as archive:
            for name, data in members.items():
                data = data.encode() if isinstance(data, str) else data
                info = tarfile.TarInfo(name)
                info.size = len(data)
                archive.addfile(info, io.BytesIO(data))

    def shot(self, name, size=2000, age=0):
        path = self.vm / "logs" / "shots" / name
        path.write_bytes(b"\x89PNG\r\n\x1a\n\0\0\0\rIHDR" + os.urandom(size))
        os.utime(path, (NOW - age, NOW - age))
        return path

    def build(self, cap=ev.BUNDLE_CAP, environment=None, expired=lambda: False, runner=no_commands):
        return ev.build(
            self.roots(),
            self.log,
            environment or env(),
            self.root / "out",
            runner,
            expired,
            NOW,
            cap,
        )


def members(path):
    out = {}
    with tarfile.open(path, "r:gz") as archive:
        for m in archive:
            if m.isreg():
                out[m.name] = archive.extractfile(m).read()
    return out


class Fixture(unittest.TestCase):
    def setUp(self):
        self.t = Tree(self)


class Content(Fixture):
    def test_bundle_holds_the_run_the_gate_and_the_host_state(self):
        self.t.tgz(
            self.t.prov / "ci1_windows.tgz",
            {"ci1_windows/hostref_after.txt": "green\n", "ci1_windows/MANIFEST.sha256": "ab  x\n"},
        )
        (self.t.prov / "ci1_windows").mkdir()
        (self.t.prov / "ci1_windows" / "dup.txt").write_text("same content as the tgz")
        (self.t.prov / "ci1_windows.log").write_text("WINDOWS_GATE_RETRY class=PROVIDER\n")
        (self.t.prov / "prov.log").write_text("READY driver=580.95.05\n")
        (self.t.prov / "other_dir").mkdir()
        (self.t.prov / "other_dir" / "note.txt").write_text("a dir with no tgz\n")
        (self.t.ws / "kf-ci-evidence").mkdir()
        (self.t.ws / "kf-ci-evidence" / "windows.run").write_text("run output\n")
        (self.t.vm / "logs" / "install.log").write_text("TIMEOUT after 4200s\n")
        self.t.shot("timeout_101500.png")
        built = self.t.build(runner=fake_commands)
        names = members(built["path"])
        self.assertIn("MANIFEST.json", names)
        for want in (
            "log/output.log",
            "log/kf-result.json",
            "host/free.txt",
            "host/df.txt",
            "host/dmesg_tail.txt",
            "host/ps_qemu_swtpm.txt",
            "host/nvidia-smi_q_head.txt",
            "prov/ci1_windows/ci1_windows/hostref_after.txt",
            "prov/ci1_windows.log",
            "prov/prov.log",
            "prov/other_dir/note.txt",
            "evidence/windows.run",
            "vm/kfwin/install.log",
            "shots/kfwin/timeout_101500.png",
        ):
            self.assertIn(want, names, sorted(names))
        # the directory that the tgz already holds is not packed twice
        self.assertFalse(any("dup.txt" in n for n in names))
        self.assertIn(b"swtpm", names["host/ps_qemu_swtpm.txt"])
        self.assertNotIn(b"/sbin/init", names["host/ps_qemu_swtpm.txt"])
        first = tarfile.open(built["path"], "r:gz").getmembers()[0].name
        self.assertEqual(first, "MANIFEST.json")

    def test_forbidden_directories_and_symlinks_are_never_read(self):
        outside = self.t.root / "outside.txt"
        outside.write_text("OUTSIDE-FILE-CONTENT")
        (self.t.prov / "link.log").symlink_to(outside)
        (self.t.ws / "kf-ci-evidence").mkdir()
        (self.t.ws / "kf-ci-evidence" / "link.txt").symlink_to(outside)
        (self.t.ws / "kf-ci-evidence" / "linkdir").symlink_to(self.t.vm / "secrets")
        (self.t.prov / "linkdir").symlink_to(self.t.vm / "secrets")
        (self.t.vm / "logs" / "id_rsa").write_text("not a key but named like one")
        (self.t.vm / "logs" / "disk.qcow2").write_text("DISK")
        (self.t.vm / "logs" / "secrets").mkdir()
        (self.t.vm / "logs" / "secrets" / "x.log").write_text("SECRETS-SUBDIR")
        built = self.t.build()
        manifest = json.loads(members(built["path"])["MANIFEST.json"])
        for listed in ("prov/link.log", "evidence/link.txt", "evidence/linkdir"):
            self.assertNotIn(listed, manifest["skipped"], "lstat skips a symlink without a trace")
        blob = b"".join(members(built["path"]).values())
        for text in (
            PASSWORD,
            KEY_LINE,
            "TPM-STATE",
            "FIRMWARE-NEVER",
            "DISK",
            "OUTSIDE-FILE",
            "SECRETS-SUBDIR",
            "not a key",
        ):
            self.assertNotIn(text.encode(), blob, text)

    def test_the_log_tail_keeps_the_newest_bytes(self):
        self.t.log.write_bytes(b"OLD\n" * 1_000_000 + b"NEWEST LINE\n")
        built = self.t.build()
        data = members(built["path"])["log/output.log"]
        self.assertTrue(data.endswith(b"NEWEST LINE\n"))
        self.assertLessEqual(len(data), ev.LOG_CAP)
        manifest = json.loads(members(built["path"])["MANIFEST.json"])
        self.assertGreater(manifest["cut"]["log/output.log"], 0)

    def test_gz_members_are_expanded_and_screenshots_are_newest_first(self):
        with gzip.open(self.t.vm / "logs" / "b1_serial.log.gz", "wb") as handle:
            handle.write(b"serial text\n")
        for i in range(9):
            self.t.shot("s%02d.png" % i, age=i * 10)
        names = members(self.t.build()["path"])
        self.assertIn("vm/kfwin/b1_serial.log", names)
        shots = sorted(n for n in names if n.startswith("shots/"))
        self.assertEqual(shots, ["shots/kfwin/s%02d.png" % i for i in range(ev.SHOT_COUNT)])

    def test_a_tar_bomb_member_is_bounded(self):
        self.t.tgz(self.t.prov / "big_windows.tgz", {"big_windows/huge.txt": b"A" * (40 << 20)})
        built = self.t.build()
        data = members(built["path"])["prov/big_windows/big_windows/huge.txt"]
        self.assertLessEqual(len(data), 2 * ev.FILE_CAP)

    def test_manifest_is_small_and_names_what_was_done(self):
        built = self.t.build(runner=fake_commands)
        manifest = json.loads(members(built["path"])["MANIFEST.json"])
        self.assertEqual(manifest["scan"], "clean")
        self.assertEqual(manifest["caps"]["bundle_bytes"], ev.BUNDLE_CAP)
        self.assertEqual({s["name"] for s in manifest["sections"]} >= {"host", "log"}, True)
        self.assertLess(len(json.dumps(manifest)), 8192)
        header = json.loads(built["meta"])
        self.assertLessEqual(len(built["meta"]), ev.META_MAX)
        self.assertEqual(header["scan"], "clean")
        self.assertEqual(set(header["scrub"]), {"uuid", "serial", "mac", "ipv4", "ipv6"})


class Symlinks(Fixture):
    def names(self):
        return set(members(self.t.build()["path"]))

    def test_a_symlinked_file_in_prov_is_skipped(self):
        outside = self.t.root / "o.txt"
        outside.write_text("OUTSIDE")
        (self.t.prov / "x.log").symlink_to(outside)
        built = self.t.build()
        self.assertNotIn("prov/x.log", members(built["path"]))
        manifest = json.loads(members(built["path"])["MANIFEST.json"])
        self.assertNotIn("prov/x.log", manifest["skipped"], "not even listed: lstat skips it")

    def test_a_symlinked_file_or_directory_in_the_payload_evidence_is_skipped(self):
        outside = self.t.root / "o.txt"
        outside.write_text("OUTSIDE")
        evidence = self.t.ws / "kf-ci-evidence"
        evidence.mkdir()
        (evidence / "a.run").symlink_to(outside)
        (evidence / "d").symlink_to(self.t.root)
        built = self.t.build()
        self.assertFalse(any(n.startswith("evidence/") for n in members(built["path"])))
        manifest = json.loads(members(built["path"])["MANIFEST.json"])
        self.assertFalse(any(x.startswith("evidence/") for x in manifest["skipped"]))

    def test_a_symlinked_prov_subdirectory_is_skipped(self):
        (self.t.prov / "d").symlink_to(self.t.root)
        self.assertFalse(any(n.startswith("prov/d") for n in self.names()))

    def test_a_symlinked_vm_logs_directory_is_skipped(self):
        other = self.t.root / "otherlogs"
        other.mkdir()
        (other / "x.log").write_text("OUTSIDE")
        shutil.rmtree(self.t.vm / "logs")
        (self.t.vm / "logs").symlink_to(other)
        self.assertFalse(any(n.startswith("vm/") for n in self.names()))


class Forbidden(Fixture):
    def test_forbidden_directory_names_are_skipped_inside_walked_trees(self):
        evidence = self.t.ws / "kf-ci-evidence"
        (evidence / "secrets").mkdir(parents=True)
        (evidence / "secrets" / "a.run").write_text("SECRETS-DIR-CONTENT")
        (self.t.prov / "d" / "tpm").mkdir(parents=True)
        (self.t.prov / "d" / "tpm" / "b.txt").write_text("TPM-DIR-CONTENT")
        (self.t.prov / "d" / "ok.txt").write_text("fine")
        names = members(self.t.build()["path"])
        self.assertIn("prov/d/ok.txt", names)
        blob = b"".join(names.values())
        self.assertNotIn(b"SECRETS-DIR-CONTENT", blob)
        self.assertNotIn(b"TPM-DIR-CONTENT", blob)

    def test_forbidden_file_names_are_skipped_even_with_a_text_suffix(self):
        for name in ("unattend.log", "swtpm.log", "id_rsa.txt", "win_password"):
            (self.t.prov / name).write_text("NAMED-" + name)
        (self.t.prov / "id_ed25519").write_text("NAMED-id_ed25519")
        (self.t.prov / "vast_api_key").write_text("NAMED-vast")
        (self.t.prov / "prov.log").write_text("fine")
        names = members(self.t.build()["path"])
        self.assertIn("prov/prov.log", names)
        blob = b"".join(names.values())
        for name in ("unattend.log", "swtpm.log", "win_password", "id_ed25519", "vast"):
            self.assertNotIn(("NAMED-" + name).encode(), blob)


class MoreSymlinks(Fixture):
    def test_a_symlinked_payload_evidence_root_is_skipped(self):
        other = self.t.root / "other"
        other.mkdir()
        (other / "a.run").write_text("OUTSIDE")
        (self.t.ws / "kf-ci-evidence").symlink_to(other)
        names = set(members(self.t.build()["path"]))
        self.assertFalse(any(n.startswith("evidence/") for n in names))

    def test_a_symlinked_payload_evidence_root_with_subdirectories_is_skipped(self):
        other = self.t.root / "other2"
        (other / "deep" / "er").mkdir(parents=True)
        (other / "deep" / "er" / "a.run").write_text("OUTSIDE")
        (other / "top.run").write_text("OUTSIDE")
        (self.t.ws / "kf-ci-evidence").symlink_to(other)
        built = self.t.build()
        self.assertNotIn(b"OUTSIDE", b"".join(members(built["path"]).values()))

    def test_a_symlinked_prov_directory_with_files_is_skipped(self):
        other = self.t.root / "other"
        other.mkdir()
        (other / "a.txt").write_text("OUTSIDE")
        (self.t.prov / "d").symlink_to(other)
        names = set(members(self.t.build()["path"]))
        self.assertFalse(any(n.startswith("prov/d") for n in names))

    def test_a_symlinked_vm_logs_directory_with_shots_is_skipped(self):
        other = self.t.root / "otherlogs"
        (other / "shots").mkdir(parents=True)
        (other / "shots" / "a.png").write_bytes(b"\x89PNG\0\0OUTSIDE")
        (other / "x.log").write_text("OUTSIDE")
        shutil.rmtree(self.t.vm / "logs")
        (self.t.vm / "logs").symlink_to(other)
        names = set(members(self.t.build()["path"]))
        self.assertFalse(any(n.startswith(("vm/", "shots/")) for n in names))

    def test_vm_names_and_files_are_sanitised(self):
        (self.t.vm / "logs" / "a b$(x)`y`.log").write_text("ok")
        names = set(members(self.t.build()["path"]))
        self.assertIn("vm/kfwin/a_b__x__y_.log", names)
        self.assertTrue(all(n == ev.safe_name(n) for n in names), names)


class Scrubbing(Fixture):
    def scrubbed(self, text):
        s = ev.Scrubber()
        return s.text(text.encode()).decode(), s.counts

    def test_uuids_become_distinct_synthetic_ones_and_equal_stay_equal(self):
        out, counts = self.scrubbed("a=GPU-%s b=%s c=%s\n" % (UUID_A, UUID_B, UUID_A.upper()))
        self.assertNotIn(UUID_A, out.lower())
        self.assertNotIn(UUID_B, out)
        first, second, third = [w.split("=")[1] for w in out.split()]
        self.assertTrue(first.replace("GPU-", "").startswith(FAKE))
        self.assertEqual(first.replace("GPU-", ""), third)
        self.assertNotEqual(first.replace("GPU-", ""), second)
        self.assertEqual(counts["uuid"], 3)

    def test_synthetic_and_zero_uuids_are_kept(self):
        text = "x %s0001 and 00000000-0000-0000-0000-000000000000\n" % FAKE
        self.assertEqual(self.scrubbed(text)[0], text)

    def test_serial_numbers(self):
        out, counts = self.scrubbed(
            "    Serial Number                         : 1320722017777\n"
            "Product Serial: PSN-99\ndev serial=KF0123456789 other\nSerial Number : N/A\n"
            "ID_SERIAL_SHORT=ABCD1234\n"
        )
        for secret in ("1320722017777", "PSN-99", "KF0123456789", "ABCD1234"):
            self.assertNotIn(secret, out)
        self.assertIn("Serial Number : N/A", out)
        self.assertIn("other", out)
        self.assertEqual(counts["serial"], 4)

    def test_mac_addresses_keep_qemu_s_own(self):
        out, counts = self.scrubbed(
            "eth0 a4:bb:6d:12:34:56 and 52:54:00:4b:46:57 and AA-BB-CC-DD-EE-FF ff:ff:ff:ff:ff:ff\n"
        )
        self.assertNotIn("a4:bb:6d", out)
        self.assertNotIn("AA-BB-CC", out)
        self.assertIn("52:54:00:4b:46:57", out)
        self.assertIn("ff:ff:ff:ff:ff:ff", out)
        self.assertEqual(counts["mac"], 2)

    def test_public_addresses_go_private_and_versions_stay(self):
        out, counts = self.scrubbed(
            "from 8.8.4.4 and 10.0.0.5 192.168.1.9 127.0.0.1 0.0.0.0 169.254.1.1\n"
            "driver 580.95.05 kernel 6.8.0-91 rustc 1.98.1 pci 0000:01:00.0 at 12:34:56\n"
            "peer 2001:4860:4860::8888 and fe80::1 and ::1 std::vec::Vec core::fmt\n"
            "again 8.8.4.4\n"
        )
        self.assertNotIn("8.8.4.4", out)
        self.assertNotIn("2001:4860", out)
        for kept in (
            "10.0.0.5",
            "192.168.1.9",
            "127.0.0.1",
            "0.0.0.0",
            "169.254.1.1",
            "580.95.05",
            "6.8.0-91",
            "1.98.1",
            "0000:01:00.0",
            "12:34:56",
            "fe80::1",
            "std::vec::Vec",
            "core::fmt",
        ):
            self.assertIn(kept, out)
        self.assertEqual(out.count("<ip4-1>"), 2)
        self.assertEqual(counts["ipv4"], 2)
        self.assertEqual(counts["ipv6"], 1)

    def test_scrubbing_reaches_every_text_member_and_the_counts_are_in_the_manifest(self):
        self.t.log.write_text(
            "board GPU-%s at 8.8.4.4 and 2001:4860:4860::8888 via 10.0.0.5\n" % UUID_A
        )
        self.t.tgz(self.t.prov / "g_windows.tgz", {"g_windows/a.txt": "GPU-%s\n" % UUID_B})
        with gzip.open(self.t.vm / "logs" / "q.log.gz", "wb") as handle:
            handle.write(b"mac a4:bb:6d:12:34:56\n")
        built = self.t.build(runner=fake_commands)
        names = members(built["path"])
        blob = b"\n".join(names.values()).decode("latin-1")
        for secret in (UUID_A, UUID_B, "8.8.4.4", "a4:bb:6d:12:34:56", "1320722017777"):
            self.assertNotIn(secret, blob)
        self.assertNotIn("2001:4860", blob)
        self.assertIn("via 10.0.0.5", names["log/output.log"].decode())
        a = names["log/output.log"].decode().split("GPU-")[1][:36]
        b = names["prov/g_windows/g_windows/a.txt"].decode().split("GPU-")[1][:36]
        self.assertTrue(a.startswith(FAKE) and b.startswith(FAKE))
        self.assertNotEqual(a, b, "two boards must stay two boards")
        manifest = json.loads(names["MANIFEST.json"])
        self.assertEqual(manifest["scrub"]["ipv6"], 1)
        self.assertGreaterEqual(manifest["scrub"]["uuid"], 3)
        self.assertGreaterEqual(manifest["scrub"]["serial"], 1)
        self.assertEqual(manifest["scrub"]["mac"], 1)
        self.assertEqual(manifest["scrub"]["ipv4"], 1)

    def test_png_bytes_are_not_rewritten(self):
        shot = self.t.shot("a.png", size=5000)
        shot.write_bytes(
            shot.read_bytes() + b" 8.8.4.4 GPU-" + UUID_A.encode() + b" a4:bb:6d:12:34:56"
        )
        data = members(self.t.build()["path"])["shots/kfwin/a.png"]
        self.assertEqual(data, shot.read_bytes())

    def test_png_members_of_the_gate_tgz_are_not_rewritten(self):
        raw = b"\x89PNG\r\n\x1a\n\0\0 8.8.4.4 GPU-" + UUID_B.encode()
        self.t.tgz(self.t.prov / "p_windows.tgz", {"p_windows/shots/a.png": raw})
        data = members(self.t.build()["path"])["prov/p_windows/p_windows/shots/a.png"]
        self.assertEqual(data, raw)


class Scanning(Fixture):
    def refused(self, label, plant):
        plant()
        with self.assertRaises(ev.ScanHit) as caught:
            self.t.build()
        self.assertEqual(caught.exception.label, label)
        return caught.exception

    def in_log(self, text):
        return lambda: self.t.log.write_text("before\n" + text + "\nafter\n")

    def in_gate_tgz(self, text):
        return lambda: self.t.tgz(self.t.prov / "g_windows.tgz", {"g_windows/note.txt": text})

    def test_every_secret_class_refuses_in_the_log(self):
        cases = {
            "run_token": TOKEN,
            "broker_url": "posting to " + URL + "/report/1",
            "broker_path": "path " + PREFIX,
            "access_id": "id " + ACCESS_ID,
            "access_secret": "secret " + ACCESS_SECRET,
            "runner_nonce": "nonce " + RNONCE,
            "guest_password": "password is " + PASSWORD,
            "ssh_key": KEY_LINE,
        }
        for label, text in cases.items():
            with self.subTest(label):
                self.t.log.write_text(text)
                with self.assertRaises(ev.ScanHit) as caught:
                    self.t.build()
                self.assertEqual(caught.exception.label, label)

    def test_every_pattern_class_refuses(self):
        cases = {
            "jwt": "eyJhbGciOiJSUzI1NiJ9.eyJzdWIiOiIxMjM0NTY3ODkwIn0.dBjftJeZ4CVPmB92K27uhbUJU1p1r_w",
            "bearer": "Authorization: Bearer abcdefghijklmnopqrstuvwxyz0123",
            "presigned_url": "https://x.r2.cloudflarestorage.com/b/k?X-Amz-Signature=" + "0" * 64,
            "private_key": "-----BEGIN OPENSSH PRIVATE KEY-----",
            "access_header": "CF-Access-Client-Secret: abcdefgh12345678",
            "vast_key": 'VAST_API_KEY="' + "a1b2c3d4" * 4 + '"',
            "github_token": "ghp_" + "A1b2C3d4" * 4,
            "url_credentials": "git clone https://user:hunter2@example.com/repo",
        }
        for label, text in cases.items():
            with self.subTest(label):
                self.t.log.write_text(text)
                with self.assertRaises(ev.ScanHit) as caught:
                    self.t.build()
                self.assertEqual(caught.exception.label, label)

    def test_exact_secrets_are_found_in_vm_logs_and_gate_members_too(self):
        values = {
            "run_token": TOKEN,
            "access_secret": ACCESS_SECRET,
            "runner_nonce": RNONCE,
            "guest_password": PASSWORD,
            "ssh_key": KEY_LINE,
            "broker_path": PREFIX,
        }
        for label, value in values.items():
            for where in ("vm", "tgz"):
                with self.subTest((label, where)):
                    shutil.rmtree(self.t.prov, ignore_errors=True)
                    self.t.prov.mkdir()
                    (self.t.vm / "logs" / "a.log").write_text("ok")
                    if where == "vm":
                        (self.t.vm / "logs" / "a.log").write_text("x " + value + " y")
                    else:
                        self.in_gate_tgz("x " + value + " y")()
                    with self.assertRaises(ev.ScanHit) as caught:
                        self.t.build()
                    self.assertEqual(caught.exception.label, label)

    def test_patterns_are_found_in_vm_logs_and_gate_members_too(self):
        samples = {
            "jwt": "eyJhbGciOiJSUzI1NiJ9.eyJzdWIiOiIxMjM0NTY3ODkwIn0.dBjftJeZ4CVPmB92K27uhbUJU1p1r_w",
            "bearer": "Authorization: Bearer abcdefghijklmnopqrstuvwxyz0123",
            "presigned_url": "?X-Amz-Credential=AKIA%2F20261106",
            "private_key": "-----BEGIN RSA PRIVATE KEY-----",
            "vast_key": "instance_api_key: " + "a1b2c3d4" * 4,
            "github_token": "github_pat_" + "A1b2C3d4" * 4,
        }
        for label, text in samples.items():
            for where in ("vm", "tgz"):
                with self.subTest((label, where)):
                    shutil.rmtree(self.t.prov, ignore_errors=True)
                    self.t.prov.mkdir()
                    (self.t.vm / "logs" / "a.log").write_text("ok")
                    if where == "vm":
                        (self.t.vm / "logs" / "a.log").write_text(text)
                    else:
                        self.in_gate_tgz(text)()
                    with self.assertRaises(ev.ScanHit) as caught:
                        self.t.build()
                    self.assertEqual(caught.exception.label, label)

    def test_a_redacted_bearer_and_ordinary_hex_do_not_refuse(self):
        self.t.log.write_text(
            "Bearer [redacted]\nsha256 " + "ab" * 32 + "\nrunner ok\nhttps://example.com/path\n"
        )
        self.t.build()

    def test_the_gate_tgz_members_and_gz_logs_and_dmesg_are_scanned(self):
        e = self.refused("guest_password", self.in_gate_tgz("typed " + PASSWORD))
        self.assertIn("g_windows", e.member)
        self.t.prov.joinpath("g_windows.tgz").unlink()
        with gzip.open(self.t.vm / "logs" / "q.log.gz", "wb") as handle:
            handle.write(("leak " + TOKEN).encode())
        self.refused("run_token", lambda: None)
        (self.t.vm / "logs" / "q.log.gz").unlink()
        with self.assertRaises(ev.ScanHit):
            self.t.build(runner=lambda argv: ("dmesg " + TOKEN).encode())

    def test_a_secret_in_a_screenshot_file_refuses(self):
        self.t.shot("x.png")
        path = self.t.vm / "logs" / "shots" / "y.png"
        path.write_bytes(b"\x89PNG\0\0" + PASSWORD.encode())
        self.refused("guest_password", lambda: None)

    def test_a_short_value_is_not_scanned_as_a_secret(self):
        self.t.log.write_text("abc\n")
        scanner = ev.Scanner([("guest_password", "abc")])
        scanner.scan("x", b"abcabc")

    def test_refused_run_uploads_nothing_prints_class_and_member_only_and_exits_zero(self):
        self.t.log.write_text("leak " + TOKEN)
        calls = []

        class Transport:
            def post(self, *args, **kwargs):
                calls.append(args)
                return 200, {}

        out = self.run_cli(Transport())
        self.assertEqual(calls, [])
        self.assertIn(
            "KF_EVIDENCE_REFUSED reason=scan_hit class=run_token member=log/output.log", out
        )
        for secret in (TOKEN, PASSWORD, ACCESS_SECRET, PREFIX):
            self.assertNotIn(secret, out)

    def run_cli(self, transport, environment=None, **kw):
        argv = [
            "run",
            "--log",
            str(self.t.log),
            "--out",
            str(self.t.root / "out"),
            "--workspace",
            str(self.t.ws),
            "--prov",
            str(self.t.prov),
        ]
        args = ev.parser().parse_args(argv + kw.pop("extra", []))
        buffer = io.StringIO()
        import contextlib

        with contextlib.redirect_stdout(buffer):
            rc = ev.run(args, environment or env(), runner=no_commands, transport=transport, **kw)
        self.assertEqual(rc, 0)
        return buffer.getvalue()


class Caps(Fixture):
    def noise(self, n):
        return random.Random(7).randbytes(n)

    def test_the_cap_binds_and_the_lowest_tier_goes_first(self):
        self.t.log.write_bytes(b"LOGLINE\n" * 5000)
        (self.t.prov / "ci1_windows.log").write_text("WINDOWS_GATE_FAIL diag=KAYFABE\n")
        (self.t.prov / "misc.out").write_bytes(self.noise(60_000).hex().encode())
        (self.t.ws / "bench").mkdir()
        (self.t.ws / "bench" / "stage.out").write_bytes(self.noise(60_000).hex().encode())
        for i in range(8):
            self.t.shot("s%d.png" % i, size=20_000, age=i)
        self.t.shot("timeout_1.png", size=20_000, age=900)
        built = self.t.build(cap=100_000)
        names = members(built["path"])
        self.assertLessEqual(built["size"], 100_000)
        self.assertIn("log/output.log", names)
        self.assertIn("prov/ci1_windows.log", names)
        self.assertIn("host/free.txt", names)
        manifest = json.loads(names["MANIFEST.json"])
        self.assertTrue(manifest["dropped"], "something had to go")
        # tier 5 (older screenshots) before tier 4 (bench) before tier 3 before tier 2
        tiers = {"shots/kfwin/s7.png": 5, "bench/stage.out": 4, "prov/misc.out": 2}
        kept = {n for n in tiers if n in names}
        for low, tier in tiers.items():
            for high, other in tiers.items():
                if other < tier and low in kept:
                    self.assertIn(high, kept, (low, high))
        self.assertIn("shots/kfwin/timeout_1.png", names, "the timeout screendump is tier 2")

    def test_the_log_tail_is_halved_before_the_bundle_is_refused(self):
        self.t.log.write_bytes(self.noise(2_000_000).hex().encode())
        built = self.t.build(cap=300_000)
        data = members(built["path"])["log/output.log"]
        self.assertLessEqual(built["size"], 300_000)
        self.assertGreater(len(data), 1000)
        self.assertTrue(self.t.log.read_bytes().endswith(data))

    def test_too_large_for_the_cap_is_refused(self):
        self.t.log.write_bytes(self.noise(100_000))
        with self.assertRaises(ev.Refused):
            self.t.build(cap=2_000)

    def test_many_small_compressible_members_hit_the_unpacked_cap_not_the_packed_one(self):
        self.t.tgz(
            self.t.prov / "m_windows.tgz",
            {"m_windows/f%03d.txt" % i: b"0" * (1 << 20) for i in range(20)},
        )
        with mock.patch.object(ev, "RAW_CAP", 6 << 20):
            built = self.t.build()
        raw = sum(len(d) for d in members(built["path"]).values())
        self.assertLessEqual(raw, (6 << 20) + 65536)
        self.assertGreater(raw, 3 << 20)
        manifest = json.loads(members(built["path"])["MANIFEST.json"])
        self.assertTrue(manifest["dropped"])
        self.assertLess(built["size"], 200_000, "far below the packed cap: the raw cap bound")

    def test_the_unpacked_cap_halves_the_log_tail_too(self):
        self.t.log.write_bytes(b"0" * (3 << 20))
        with mock.patch.object(ev, "RAW_CAP", 1 << 20):
            built = self.t.build()
        data = members(built["path"])["log/output.log"]
        self.assertLess(len(data), 1 << 20)
        self.assertGreater(len(data), 100_000)

    def test_the_timeout_screendump_outlives_gate_files_and_older_shots(self):
        self.t.tgz(
            self.t.prov / "g_windows.tgz", {"g_windows/n.txt": self.noise(60_000).hex().encode()}
        )
        self.t.shot("timeout_9.png", size=20_000, age=500)
        self.t.shot("old.png", size=20_000, age=900)
        names = members(self.t.build(cap=45_000)["path"])
        self.assertIn("shots/kfwin/timeout_9.png", names)
        self.assertNotIn("prov/g_windows/g_windows/n.txt", names)
        self.assertNotIn("shots/kfwin/old.png", names)

    def test_at_most_six_screenshots_timeouts_included(self):
        for i in range(10):
            self.t.shot("s%d.png" % i, age=i)
        self.t.shot("timeout_1.png", age=100)
        shots = [n for n in members(self.t.build()["path"]) if n.startswith("shots/")]
        self.assertEqual(len(shots), ev.SHOT_COUNT)
        self.assertIn("shots/kfwin/timeout_1.png", shots)

    def test_a_cut_member_is_recorded_in_the_manifest(self):
        self.t.tgz(self.t.prov / "big_windows.tgz", {"big_windows/h.txt": b"B" * (5 << 20)})
        names = members(self.t.build()["path"])
        manifest = json.loads(names["MANIFEST.json"])
        self.assertGreater(manifest["cut"]["prov/big_windows/big_windows/h.txt"], 2 << 20)

    def test_a_default_run_is_far_below_the_cap(self):
        built = self.t.build(runner=fake_commands)
        self.assertLess(built["size"], 100_000)

    def test_an_expired_time_budget_stops_collecting_the_slow_sections(self):
        (self.t.vm / "logs" / "late.log").write_text("late\n")
        self.t.tgz(self.t.prov / "g_windows.tgz", {"g_windows/a.txt": "x"})
        items, _ = ev.collect(self.t.roots(), self.t.log, no_commands, lambda: True, NOW)
        names = {i.name for i in items}
        self.assertIn("log/output.log", names)
        self.assertIn("host/free.txt", names)
        self.assertNotIn("vm/kfwin/late.log", names)
        self.assertFalse(any(n.startswith("prov/g_windows") for n in names))

    def test_a_budget_that_expires_before_scanning_ends_the_step_without_a_bundle(self):
        with self.assertRaises(ev.Refused) as caught:
            self.t.build(expired=lambda: True)
        self.assertEqual(str(caught.exception), "time")
        self.assertFalse((self.t.root / "out" / "evidence.tgz").exists())


class CutsAndFragments(Fixture):
    def straddle(self, total_cap, offset_in_token=40, fill=b"A"):
        """A file whose `total_cap` tail cut falls `offset_in_token` bytes into the token."""
        token = TOKEN.encode()
        before = 5000
        size = before + offset_in_token + total_cap
        body = fill * before + token + fill * (size - before - len(token))
        assert len(body) == size
        return body

    def test_a_token_crossing_the_log_tail_cut_is_refused(self):
        self.t.log.write_bytes(self.straddle(ev.LOG_CAP))
        with self.assertRaises(ev.ScanHit) as caught:
            self.t.build()
        self.assertEqual(caught.exception.label, "run_token")

    def test_a_token_crossing_the_tgz_member_cut_is_refused(self):
        self.t.tgz(
            self.t.prov / "g_windows.tgz", {"g_windows/a.txt": self.straddle(2 * ev.FILE_CAP)}
        )
        with self.assertRaises(ev.ScanHit) as caught:
            self.t.build()
        self.assertEqual(caught.exception.label, "run_token")

    def test_a_token_crossing_the_gz_log_cut_is_refused(self):
        with gzip.open(self.t.vm / "logs" / "a.log.gz", "wb") as handle:
            handle.write(self.straddle(1024 * 1024))
        with self.assertRaises(ev.ScanHit) as caught:
            self.t.build()
        self.assertEqual(caught.exception.label, "run_token")

    def test_a_token_at_the_size_halving_cut_is_refused(self):
        # incompressible filler, so that the cap halves the log; the token sits on the first cut
        noise = random.Random(3).randbytes(100_000).hex().encode()
        half = len(noise) // 2
        self.t.log.write_bytes(noise[: half - 20] + TOKEN.encode() + noise[half:])
        with self.assertRaises(ev.ScanHit):
            self.t.build(cap=60_000)

    def test_a_prefix_or_a_suffix_of_a_secret_is_a_hit(self):
        for label, secret in (
            ("run_token", TOKEN),
            ("guest_password", PASSWORD),
            ("access_secret", ACCESS_SECRET),
            ("runner_nonce", RNONCE),
            ("broker_path", PREFIX),
        ):
            for name, fragment in (("prefix", secret[:20]), ("suffix", secret[-20:])):
                with self.subTest((label, name)):
                    self.t.log.write_text("cut line " + fragment)
                    with self.assertRaises(ev.ScanHit) as caught:
                        self.t.build()
                    self.assertEqual(caught.exception.label, label + "_fragment")

    def test_a_secret_cut_before_the_file_starts_is_caught_in_every_kind_of_source(self):
        tail = TOKEN[20:]
        self.t.tgz(self.t.prov / "g_windows.tgz", {"g_windows/a.txt": "..." + tail})
        with self.assertRaises(ev.ScanHit) as caught:
            self.t.build()
        self.assertEqual(caught.exception.label, "run_token_fragment")
        (self.t.prov / "g_windows.tgz").unlink()
        with gzip.open(self.t.vm / "logs" / "a.log.gz", "wb") as handle:
            handle.write(("..." + tail).encode())
        with self.assertRaises(ev.ScanHit) as caught:
            self.t.build()
        self.assertEqual(caught.exception.label, "run_token_fragment")
        (self.t.vm / "logs" / "a.log.gz").unlink()
        (self.t.vm / "logs" / "b.log").write_text("..." + tail)
        with self.assertRaises(ev.ScanHit):
            self.t.build()

    def test_a_short_piece_and_ordinary_text_of_the_urls_are_not_fragments(self):
        self.t.log.write_text(
            "https://broker.example/ and " + TOKEN[:10] + " and " + ACCESS_ID[:12] + "\n"
        )
        self.t.build()

    def test_the_final_bytes_are_checked_again_for_exact_secrets(self):
        scanner = ev.Scanner([("run_token", TOKEN)])
        with self.assertRaises(ev.ScanHit):
            scanner.scan_exact("x", b"..." + TOKEN.encode())
        scanner.scan_exact("x", b"nothing here")


class NamesAndManifest(Fixture):
    GHP = "ghp_" + "A1b2C3d4" * 4

    def test_a_secret_in_a_file_name_is_refused_without_printing_the_name(self):
        (self.t.prov / (self.GHP + ".log")).write_text("fine")
        with self.assertRaises(ev.ScanHit) as caught:
            self.t.build()
        self.assertEqual(caught.exception.label, "github_token")
        self.assertRegex(caught.exception.member, r"^name-[0-9a-f]{10}$")
        self.assertNotIn("A1b2C3d4", caught.exception.member)

    def test_the_step_prints_a_hash_for_a_name_hit_and_nothing_of_the_name(self):
        (self.t.prov / (self.GHP + ".log")).write_text("fine")
        out = io.StringIO()
        import contextlib

        with contextlib.redirect_stdout(out):
            Step.run_with(self, Fake())
        self.assertIn(
            "KF_EVIDENCE_REFUSED reason=scan_hit class=github_token member=name-", out.getvalue()
        )
        self.assertNotIn(self.GHP, out.getvalue())
        self.assertNotIn("A1b2C3d4", out.getvalue())

    def test_a_content_hit_prints_the_member_name_only(self):
        (self.t.prov / "notes.log").write_text("x " + TOKEN)
        with self.assertRaises(ev.ScanHit) as caught:
            self.t.build()
        self.assertEqual(caught.exception.member, "prov/notes.log")

    def test_skipped_names_in_the_manifest_are_sanitised(self):
        (self.t.prov / "weird name$(x).log").write_bytes(b"\0binary")
        names = members(self.t.build()["path"])
        manifest = json.loads(names["MANIFEST.json"])
        self.assertIn("prov/weird_name__x_.log", manifest["skipped"])
        self.assertTrue(all(n == ev.safe_name(n) for n in manifest["skipped"]))

    def test_the_manifest_holds_ascii_path_characters_only(self):
        (self.t.prov / "caf\u00e9\x07 x.log").write_bytes(b"\0binary")
        manifest = json.loads(members(self.t.build()["path"])["MANIFEST.json"])
        self.assertTrue(manifest["skipped"])
        for name in manifest["skipped"]:
            self.assertRegex(name, r"^[A-Za-z0-9._/-]+$")

    def test_the_manifest_is_scanned_too(self):
        # a binary file named like a token is skipped, and its (sanitised) name lands in the manifest
        (self.t.prov / (self.GHP + ".log")).write_bytes(b"\0binary")
        with self.assertRaises(ev.ScanHit) as caught:
            self.t.build()
        self.assertEqual(caught.exception.member, "MANIFEST.json")
        self.assertEqual(caught.exception.label, "github_token")

    def test_a_dropped_name_with_a_secret_shape_is_caught_in_the_manifest(self):
        self.t.shot("a.png", size=20_000)
        (self.t.vm / "logs" / "shots" / (self.GHP + ".png")).write_bytes(
            b"\x89PNG\0" + bytes(20_000)
        )
        with self.assertRaises(ev.ScanHit):
            self.t.build(cap=30_000)


class Hostile(Fixture):
    def test_scanning_and_scrubbing_are_linear_on_hostile_input(self):
        scanner, scrubber = ev.Scanner([("run_token", TOKEN)]), ev.Scrubber()
        big = 1 << 20
        cases = {
            "jwt-prefix": b"eyJ-" * (big // 4),
            "jwt-dots": b"eyJ." * (big // 4),
            "github-pat": b"github_pat_" * (big // 11),
            "ghp": b"ghp_" * (big // 4),
            "words": b"a " * (big // 2),
            "serial-spaces": b"Serial Number:" + b" " * big + b"x\n",
            "serial-words": b"Product " * (big // 8) + b"\n",
            "zeros": b"0" * big,
            "colons": b":" * big,
            "hex-colons": b"0:" * (big // 2),
            "bearer": b"Bearer " * (big // 7),
            "amz": b"X-Amz-" * (big // 6),
            "dots": b"1." * (big // 2),
            "url-creds": b"http://a:" * (big // 9),
            "newlines": b"\n" * big,
        }
        for name, data in cases.items():
            with self.subTest(name):
                started = time.monotonic()
                try:
                    scanner.scan("x", data)
                except ev.ScanHit:
                    pass
                scrubber.text(data)
                self.assertLess(time.monotonic() - started, 5.0, name)

    def test_the_jwt_matcher_agrees_with_the_old_pattern_on_real_shapes(self):
        good = [
            b"eyJhbGciOiJSUzI1NiJ9.eyJzdWIiOiIxMjM0NTY3ODkwIn0.dBjftJeZ4CVPmB92K27uhbUJU1p1r_w",
            b"tok=eyJabcdefg.abcdef.abcdefghijklmnopqrst&x",
            b"xxeyJabcdefg.abcdef.abcdefghijklmnopqrst",
            b"a.b.c eyJabcdefgh.abcdefg.abcdefghijklmnopqrstuv",
            b"eyJ12345.eyJ1234567.abcdef.abcdefghijklmnopqrstuvwx",
        ]
        bad = [
            b"eyJabc.abcdef.abcdefghijklmnopqrst",
            b"eyJabcdefg.abc.abcdefghijklmnopqrst",
            b"eyJabcdefg.abcdef.abcdefghij",
            b"eyJabcdefg abcdef abcdefghijklmnopqrst",
            b"plain text",
        ]
        for data in good:
            self.assertTrue(ev._jwt(data), data)
        for data in bad:
            self.assertFalse(ev._jwt(data), data)


class NoFollow(Fixture):
    def test_roots_that_are_symlinks_are_not_followed(self):
        other = self.t.root / "elsewhere"
        (other / "logs").mkdir(parents=True)
        (other / "logs" / "x.log").write_text("OUTSIDE")
        (other / "stray.log").write_text("OUTSIDE")
        shutil.rmtree(self.t.prov)
        self.t.prov.symlink_to(other)
        shutil.rmtree(self.t.winvm)
        self.t.winvm.symlink_to(other)
        blob = b"".join(members(self.t.build()["path"]).values())
        self.assertNotIn(b"OUTSIDE", blob)

    def test_a_symlinked_vm_directory_a_symlinked_log_and_result_are_not_followed(self):
        other = self.t.root / "elsewhere"
        (other / "logs").mkdir(parents=True)
        (other / "logs" / "x.log").write_text("OUTSIDE")
        (self.t.winvm / "linked").symlink_to(other)
        outside = self.t.root / "o.txt"
        outside.write_text("OUTSIDE")
        self.t.log.unlink()
        self.t.log.symlink_to(outside)
        (self.t.ws / "kf-result.json").unlink()
        (self.t.ws / "kf-result.json").symlink_to(outside)
        (self.t.ws / "bench").symlink_to(other)
        blob = b"".join(members(self.t.build()["path"]).values())
        self.assertNotIn(b"OUTSIDE", blob)

    def test_open_plain_refuses_symlinks_fifos_and_directories(self):
        target = self.t.root / "t.txt"
        target.write_text("x")
        link = self.t.root / "l.txt"
        link.symlink_to(target)
        fifo = self.t.root / "f"
        os.mkfifo(fifo)
        for path in (link, fifo, self.t.root):
            with self.assertRaises(OSError):
                ev.open_plain(path)
        with ev.open_plain(target) as handle:
            self.assertEqual(handle.read(), b"x")

    def test_read_tail_refuses_a_fifo_and_a_directory(self):
        fifo = self.t.root / "f2"
        os.mkfifo(fifo)
        with self.assertRaises(OSError):
            ev.read_tail(fifo, 10)
        with self.assertRaises(OSError):
            ev.read_tail(self.t.root, 10)
        with self.assertRaises(OSError):
            ev.gz_tail(fifo, 10)

    def test_a_swap_between_check_and_open_is_refused_by_o_nofollow(self):
        victim = self.t.root / "victim.txt"
        victim.write_text("OUTSIDE")
        plain = self.t.prov / "swap.log"
        plain.write_text("inside")
        self.assertTrue(ev.is_plain_file(plain))
        plain.unlink()
        plain.symlink_to(victim)
        with self.assertRaises(OSError):
            ev.read_tail(plain, 100)

    def test_a_fifo_in_a_walked_tree_does_not_block(self):
        evidence = self.t.ws / "kf-ci-evidence"
        evidence.mkdir()
        os.mkfifo(evidence / "pipe.run")
        started = time.monotonic()
        self.t.build()
        self.assertLess(time.monotonic() - started, 10)


class Commands(Fixture):
    def test_a_hung_command_and_its_children_are_killed_within_a_bound(self):
        pids = self.t.root / "pids"
        script = "echo $$ > %s; sleep 60 & echo $! >> %s; wait" % (pids, pids)
        started = time.monotonic()
        out = ev.run_command(["sh", "-c", script], timeout=1.0)
        self.assertLess(time.monotonic() - started, 6)
        self.assertIn(b"timed out, killed", out)
        time.sleep(0.3)
        for pid in pids.read_text().split():
            try:
                state = Path("/proc/%s/stat" % pid).read_text().rsplit(")", 1)[1].split()[0]
            except OSError:
                continue
            self.assertEqual(state, "Z", "process %s still runs" % pid)

    def test_a_command_that_ignores_term_is_killed_too(self):
        started = time.monotonic()
        out = ev.run_command(["sh", "-c", "trap '' TERM HUP INT; sleep 60"], timeout=1.0)
        self.assertLess(time.monotonic() - started, 6)
        self.assertIn(b"timed out, killed", out)

    def test_a_missing_command_and_big_output_are_bounded(self):
        self.assertIn(b"command failed", ev.run_command(["/nonexistent/kf-evidence-cmd"]))
        out = ev.run_command(["sh", "-c", "yes x | head -c 5000000"], limit=1000)
        self.assertLessEqual(len(out), 1000)

    def test_normal_output_and_stderr_are_returned(self):
        self.assertEqual(ev.run_command(["sh", "-c", "echo out; echo err >&2"]), b"out\nerr\n")

    def test_the_files_are_collected_before_any_command_runs(self):
        order = []

        def runner(argv):
            if not order:
                self.t.log.unlink()  # a hung GPU would block here: the log must already be read
            order.append(argv[0])
            return b"[%s]\n" % argv[0].encode()

        built = self.t.build(runner=runner)
        names = members(built["path"])
        self.assertIn("log/output.log", names)

    def test_the_host_items_come_after_every_file_item(self):
        items, _ = ev.collect(self.t.roots(), self.t.log, no_commands, lambda: False, NOW)
        files = [i for i, x in enumerate(items) if not x.name.startswith("host/")]
        hosts = [i for i, x in enumerate(items) if x.name.startswith("host/")]
        self.assertTrue(files and hosts and max(files) < min(hosts))

    def test_the_commands_have_a_phase_budget_and_the_files_still_ship(self):
        calls = []
        with mock.patch.object(ev, "COMMAND_PHASE", -1):
            built = self.t.build(runner=lambda argv: calls.append(argv) or b"x")
        names = members(built["path"])
        self.assertEqual(calls, [])
        self.assertIn("log/output.log", names)
        self.assertIn(b"skipped, out of time", names["host/free.txt"])
        manifest = json.loads(names["MANIFEST.json"])
        self.assertTrue(any("(time)" in x for x in manifest["skipped"]))

    def test_every_command_hung_still_yields_a_bundle_in_bounded_time(self):
        def hung(argv):
            return ev.run_command(["sleep", "60"], timeout=2.0)

        with mock.patch.object(ev, "COMMAND_PHASE", 3):
            started = time.monotonic()
            built = self.t.build(runner=hung)
        self.assertLess(time.monotonic() - started, 12)
        self.assertIn("log/output.log", members(built["path"]))


class Fake:
    """Transport double: records requests, answers from a script."""

    def __init__(self, *answers):
        self.answers, self.requests = list(answers), []

    def post(self, url, body, extra, timeout=None):
        self.requests.append((url, bytes(body), dict(extra), timeout))
        answer = self.answers.pop(0) if self.answers else (200, {"state": "STORED"})
        if isinstance(answer, Exception):
            raise answer
        return answer


class Upload(Fixture):
    def built(self):
        return self.t.build()

    def go(self, transport, deadline=None, clock=None):
        built = self.built()
        sleeps = []
        deadline = time.monotonic() + 100 if deadline is None else deadline
        result = ev.upload(
            env(),
            built,
            transport,
            deadline,
            sleep=sleeps.append,
            **({"clock": clock} if clock else {}),
        )
        return built, result, sleeps

    def test_a_stored_upload_posts_the_exact_bytes_to_the_evidence_route(self):
        fake = Fake()
        built, result, _ = self.go(fake)
        self.assertEqual(result, (True, "stored"))
        url, body, extra, _ = fake.requests[0]
        self.assertEqual(url, URL + "/evidence/" + RUN)
        self.assertEqual(body, built["path"].read_bytes())
        with tarfile.open(built["path"], "r:gz") as archive:
            self.assertEqual(archive.getmembers()[0].name, "MANIFEST.json")
        self.assertEqual(extra["X-KF-Evidence-SHA256"], built["sha256"])
        self.assertEqual(extra["X-KF-Evidence-Meta"], built["meta"])
        self.assertEqual(set(extra), {"X-KF-Evidence-SHA256", "X-KF-Evidence-Meta"})

    def test_real_transport_sends_token_and_access_only_as_headers(self):
        seen = {}

        class Opener:
            def open(self, request, timeout):
                seen.update(
                    url=request.full_url,
                    headers={k.lower(): v for k, v in request.header_items()},
                    method=request.get_method(),
                    timeout=timeout,
                )
                raise OSError("no network in tests")

        t = ev.Transport(TOKEN, access=env())
        t.opener = Opener()
        self.assertEqual(t.post(URL + "/evidence/" + RUN, b"x", {"X-Test": "1"}), (None, None))
        self.assertEqual(seen["method"], "POST")
        self.assertEqual(seen["headers"]["authorization"], "Bearer " + TOKEN)
        self.assertEqual(seen["headers"]["cf-access-client-id"], ACCESS_ID)
        self.assertEqual(seen["headers"]["content-type"], "application/gzip")
        self.assertTrue(seen["headers"]["user-agent"].startswith("kayfabe-vast-box/"))
        self.assertNotIn(TOKEN, seen["url"])
        half = ev.Transport(TOKEN, access={"KF_ACCESS_ID": ACCESS_ID})
        half.opener = Opener()
        half.post(URL + "/evidence/" + RUN, b"x", {})
        self.assertNotIn("cf-access-client-id", seen["headers"])

    def test_the_transport_never_follows_redirects_or_uses_a_proxy(self):
        t = ev.Transport(TOKEN)
        kinds = {type(h).__name__ for h in t.opener.handlers}
        self.assertIn("_NoRedirect", kinds)
        self.assertNotIn("ProxyHandler", {k for k in kinds if k != "ProxyHandler"} | set())
        self.assertIsNone(
            ev._NoRedirect().redirect_request(None, None, 302, "Found", {}, "https://evil/")
        )

    def serve(self, status, headers=None):
        import http.server
        import threading

        seen = []

        class Handler(http.server.BaseHTTPRequestHandler):
            def log_message(self, *args):
                pass

            def do_POST(self):
                self.rfile.read(int(self.headers.get("Content-Length") or 0))
                seen.append({k.lower(): v for k, v in self.headers.items()})
                self.send_response(status)
                for k, v in (headers() if callable(headers) else headers or {}).items():
                    self.send_header(k, v)
                self.send_header("Content-Length", "2")
                self.end_headers()
                self.wfile.write(b"{}")

        server = http.server.ThreadingHTTPServer(("127.0.0.1", 0), Handler)
        threading.Thread(target=server.serve_forever, daemon=True).start()
        self.addCleanup(server.server_close)
        self.addCleanup(server.shutdown)
        return "http://127.0.0.1:%d" % server.server_address[1], seen

    def test_a_redirect_is_not_followed_and_the_token_goes_nowhere_else(self):
        target, elsewhere = self.serve(200)
        base, first = self.serve(302, lambda: {"Location": target + "/stolen"})
        status, _ = ev.Transport(TOKEN, access=env()).post(base + "/evidence/x", b"abc", {})
        self.assertEqual(status, 302)
        self.assertEqual(len(first), 1)
        self.assertEqual(elsewhere, [])

    def test_the_access_pair_is_sent_only_whole_over_a_real_connection(self):
        base, seen = self.serve(200)
        ev.Transport(TOKEN, access={"KF_ACCESS_ID": ACCESS_ID}).post(base + "/e", b"x", {})
        ev.Transport(TOKEN, access=env()).post(base + "/e", b"x", {})
        self.assertNotIn("cf-access-client-id", seen[0])
        self.assertEqual(seen[1]["cf-access-client-id"], ACCESS_ID)
        self.assertEqual(seen[1]["cf-access-client-secret"], ACCESS_SECRET)
        self.assertEqual(seen[1]["authorization"], "Bearer " + TOKEN)
        self.assertEqual(seen[1]["content-type"], "application/gzip")

    def test_time_runs_out_between_retries(self):
        ticks = iter([0.0, 0.0, 99.0, 99.0, 99.0, 99.0])
        fake = Fake((503, None), (200, {}))
        built = self.built()
        result = ev.upload(
            env(), built, fake, deadline=100.0, sleep=lambda s: None, clock=lambda: next(ticks)
        )
        self.assertEqual(result, (False, "time"))
        self.assertEqual(len(fake.requests), 1)

    def test_a_busy_409_is_retried_and_only_evidence_exists_counts_as_stored(self):
        fake = Fake((409, {"state": "BUSY"}), (409, None), (200, {"state": "STORED"}))
        _, result, sleeps = self.go(fake)
        self.assertEqual(result, (True, "stored"))
        self.assertEqual(len(fake.requests), 3)
        self.assertEqual(len(sleeps), 2)
        fake = Fake((409, {"state": "BUSY"}), (409, {"state": "BUSY"}), (409, {"state": "BUSY"}))
        _, result, _ = self.go(fake)
        self.assertEqual(result, (False, "upload_http_409"))
        self.assertEqual(len(fake.requests), 3)
        fake = Fake((409, {"state": "EVIDENCE_EXISTS"}))
        self.assertEqual(self.go(fake)[1], (True, "already_stored"))
        self.assertEqual(len(fake.requests), 1)

    def test_409_counts_as_stored(self):
        _, result, _ = self.go(Fake((409, {"state": "EVIDENCE_EXISTS"})))
        self.assertEqual(result, (True, "already_stored"))

    def test_409_pending_is_retried_never_stored(self):
        # Review of #274, R1 (2026-11-06): the Worker answers a fresh PENDING flag with its own
        # state; the box retries it and never reports it as stored.
        pending = (409, {"state": "EVIDENCE_PENDING"})
        fake = Fake(pending, pending, (200, {}))
        self.assertEqual(self.go(fake)[1], (True, "stored"))
        self.assertEqual(len(fake.requests), 3)
        fake = Fake(pending, pending, pending)
        self.assertEqual(self.go(fake)[1], (False, "upload_http_409"))
        self.assertEqual(len(fake.requests), 3)

    def test_client_errors_are_not_retried(self):
        for status in (400, 401, 403, 404, 411, 413, 422):
            fake = Fake((status, None))
            _, result, sleeps = self.go(fake)
            self.assertEqual(result, (False, "upload_http_%d" % status))
            self.assertEqual(len(fake.requests), 1, status)
            self.assertEqual(sleeps, [])

    def test_transient_errors_retry_three_times_then_stop(self):
        fake = Fake((503, None), (None, None), (429, None), (200, {}))
        _, result, sleeps = self.go(fake)
        self.assertEqual(result, (False, "upload_http_429"))
        self.assertEqual(len(fake.requests), 3)
        self.assertEqual(len(sleeps), 3)
        again = Fake((502, None), (200, {"state": "STORED"}))
        self.assertEqual(self.go(again)[1], (True, "stored"))

    def test_no_time_left_means_no_upload(self):
        fake = Fake()
        _, result, _ = self.go(fake, deadline=time.monotonic() + 1)
        self.assertEqual(result, (False, "time"))
        self.assertEqual(fake.requests, [])


class Step(Fixture):
    """The whole step as vast_onstart.sh calls it."""

    @staticmethod
    def run_with(case, transport):
        args = ev.parser().parse_args(
            [
                "run",
                "--log",
                str(case.t.log),
                "--out",
                str(case.t.root / "out"),
                "--workspace",
                str(case.t.ws),
                "--prov",
                str(case.t.prov),
            ]
        )
        return ev.run(args, env(), runner=no_commands, transport=transport)

    def run_step(self, transport, environment=None, extra=(), **kw):
        args = ev.parser().parse_args(
            [
                "run",
                "--log",
                str(self.t.log),
                "--out",
                str(self.t.root / "out"),
                "--workspace",
                str(self.t.ws),
                "--prov",
                str(self.t.prov),
                *extra,
            ]
        )
        import contextlib

        buffer = io.StringIO()
        with contextlib.redirect_stdout(buffer):
            rc = ev.run(
                args,
                env() if environment is None else environment,
                runner=kw.pop("runner", no_commands),
                transport=transport,
                **kw,
            )
        self.assertEqual(rc, 0, "the evidence step never changes an exit status")
        return buffer.getvalue()

    def test_success_prints_the_single_key_line(self):
        out = self.run_step(Fake())
        lines = [x for x in out.splitlines() if x.startswith("evidence uploaded:")]
        self.assertEqual(len(lines), 1, out)
        self.assertRegex(lines[0], r"^evidence uploaded: 4242/7 [0-9]+ sha256 [0-9a-f]{12}$")
        self.assertNotIn(TOKEN, out)
        self.assertNotIn(PREFIX, out)

    def test_failure_modes_print_one_skipped_line_and_exit_zero(self):
        for answer, reason in (
            ((404, {"state": "EVIDENCE_DISABLED"}), "upload_http_404"),
            ((401, None), "upload_http_401"),
            (OSError("boom"), "upload_error:OSError"),
        ):
            out = self.run_step(Fake(answer))
            self.assertIn("KF_EVIDENCE_SKIPPED reason=" + reason, out, out)
            self.assertNotIn("evidence uploaded", out)
            for secret in (TOKEN, ACCESS_SECRET, RNONCE, PREFIX):
                self.assertNotIn(secret, out)

    def test_a_broken_environment_or_collector_never_raises(self):
        out = self.run_step(Fake(), environment={"KF_RUN_ID": "x"})
        self.assertIn("KF_EVIDENCE_SKIPPED reason=environment_contract", out)

        def broken(argv):
            raise RuntimeError("collector bug")

        out = self.run_step(Fake(), runner=broken)
        self.assertIn("KF_EVIDENCE_SKIPPED reason=build_error:RuntimeError", out)

    def test_a_scan_hit_through_the_step_exits_zero_and_uploads_nothing(self):
        self.t.log.write_text("leak " + TOKEN)
        fake = Fake()
        out = self.run_step(fake)
        self.assertEqual(fake.requests, [])
        self.assertIn("KF_EVIDENCE_REFUSED reason=scan_hit class=run_token", out)

    def test_a_busy_ledger_is_retried_and_not_reported_as_already_stored(self):
        with mock.patch.object(time, "sleep", lambda s: None):
            out = self.run_step(Fake((409, {"state": "BUSY"}), (200, {"state": "STORED"})))
        self.assertRegex(out, r"evidence uploaded: 4242/7 [0-9]+ sha256 [0-9a-f]{12}$")
        self.assertNotIn("already_stored", out)

    def test_an_already_stored_bundle_is_still_reported_as_uploaded(self):
        out = self.run_step(Fake((409, {"state": "EVIDENCE_EXISTS"})))
        self.assertRegex(
            out, r"evidence uploaded: 4242/7 [0-9]+ sha256 [0-9a-f]{12} \(already_stored\)"
        )

    def test_the_line_carries_the_hash_of_the_bundle_that_was_sent(self):
        fake = Fake()
        out = self.run_step(fake)
        import hashlib

        sent = hashlib.sha256(fake.requests[0][1]).hexdigest()
        self.assertIn(" sha256 " + sent[:12], out)
        self.assertIn(" %d sha256" % len(fake.requests[0][1]), out)

    def test_main_never_raises_on_a_broken_environment_or_a_past_deadline(self):
        import contextlib

        base = ["run", "--log", str(self.t.log), "--workspace", str(self.t.ws)]
        base += ["--out", str(self.t.root / "out"), "--prov", str(self.t.prov)]
        for argv, environment, want in (
            (base, {}, "KF_EVIDENCE_SKIPPED reason=environment_contract"),
            (base + ["--until", str(time.time() - 5)], env(), "KF_EVIDENCE_SKIPPED reason=time"),
        ):
            buffer = io.StringIO()
            with contextlib.redirect_stdout(buffer):
                self.assertEqual(ev.main(argv, environment), 0)
            self.assertIn(want, buffer.getvalue())

    def test_it_never_writes_the_result_or_the_log(self):
        result = (self.t.ws / "kf-result.json").read_bytes()
        log = self.t.log.read_bytes()
        self.run_step(Fake())
        self.assertEqual((self.t.ws / "kf-result.json").read_bytes(), result)
        self.assertEqual(self.t.log.read_bytes(), log)

    def test_the_time_budget_is_enforced(self):
        out = self.run_step(Fake(), extra=["--until", str(time.time() + 3)])
        self.assertIn("KF_EVIDENCE_SKIPPED reason=time", out)
        out = self.run_step(Fake(), extra=["--budget", "5"])
        self.assertIn("KF_EVIDENCE_SKIPPED reason=time", out)

    def test_no_upload_builds_only(self):
        fake = Fake()
        out = self.run_step(fake, extra=["--no-upload"])
        self.assertEqual(fake.requests, [])
        self.assertIn("KF_EVIDENCE_BUNDLE files=", out)
        self.assertTrue((self.t.root / "out" / "evidence.tgz").is_file())

    def test_the_meta_header_has_the_shape_the_worker_accepts(self):
        fake = Fake()
        self.run_step(fake)
        meta = fake.requests[0][2]["X-KF-Evidence-Meta"]
        node = shutil.which("node")
        if node is None:
            self.skipTest("node not installed")
        source = Path(__file__).resolve().parents[2] / "ci/vast-broker/src/evidence.js"
        script = (
            "import(process.argv[1]).then(m=>{"
            "const v=m.parseEvidenceMeta(process.argv[2]);"
            "console.log(JSON.stringify(v.scan))})"
        )
        done = subprocess.run(
            [node, "-e", script, source.as_uri(), meta], capture_output=True, text=True
        )
        if "ERR_MODULE_NOT_FOUND" in done.stderr:
            self.skipTest("ci/vast-broker dependencies not installed (npm ci)")
        self.assertEqual(done.returncode, 0, done.stderr[-500:])
        self.assertEqual(done.stdout.strip(), '"clean"')


class Hygiene(unittest.TestCase):
    def test_no_secret_values_are_ever_printed_or_logged_by_the_module(self):
        source = Path(ev.__file__).read_text()
        for name in ("KF_RUN_TOKEN", "KF_ACCESS_SECRET", "KF_RUNNER_NONCE"):
            for line in source.splitlines():
                if "print(" in line or "say(" in line:
                    self.assertNotIn(name, line)

    def test_names_are_sanitised(self):
        self.assertEqual(ev.safe_name("../../etc/passwd"), "etc/passwd")
        self.assertEqual(ev.safe_name("/abs/ x y/ok.txt"), "abs/_x_y/ok.txt")
        self.assertEqual(ev.safe_name(""), "unnamed")
        self.assertFalse(ev.allowed_path("kfwin/secrets/win_password"))
        self.assertFalse(ev.allowed_path("a/tpm/x"))
        self.assertFalse(ev.allowed_path("a/id_ed25519"))
        self.assertFalse(ev.allowed_path("a/disk.qcow2"))
        self.assertTrue(ev.allowed_path("kfwin/logs/install.log"))

    def test_module_can_run_as_a_script_in_isolated_mode(self):
        done = subprocess.run(
            ["python3", "-I", ev.__file__, "run", "--help"], capture_output=True, text=True
        )
        self.assertEqual(done.returncode, 0, done.stderr)


if __name__ == "__main__":
    unittest.main()
