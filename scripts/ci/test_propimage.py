#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later
"""propimage.py fixtures: no VM, GPU, TPM service, ssh or real key."""
import base64
import copy
import hashlib
import importlib.util
import json
from pathlib import Path
import re
import tempfile
import unittest
import zipfile

ROOT = Path(__file__).resolve().parents[2]
SOURCE = ROOT / "scripts/bench/windows/propimage.py"
SEED = "kayfabe-prop-v1"


def load():
    spec = importlib.util.spec_from_file_location("fixture_propimage", SOURCE)
    m = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(m)
    return m


def blob(data):
    return {"bytes": len(data), "sha256": hashlib.sha256(data).hexdigest(), "data_base64": base64.b64encode(data).decode()}


class PropTests(unittest.TestCase):
    def setUp(self):
        self.p = load()
        self.tmp_obj = tempfile.TemporaryDirectory(prefix="propimage-test-")
        self.addCleanup(self.tmp_obj.cleanup)
        self.tmp = Path(self.tmp_obj.name)
        s = self.p.init_state(SEED)
        s["clock"]["last_time_at_stop_utc"] = "2026-11-13T10:00:00Z"
        s["files"] = {"tpm/tpm2-00.permall": blob(b"tpm-state"), "firmware/OVMF_VARS.fd": blob(b"ovmf")}
        s["guest"].update(machine_guid="11111111-2222-3333-4444-555555555555", ek_pub_hash="AB" * 32)
        self.state = s
        self.fp = self.p.cpu_fingerprint(self.cpu_guest())
        s["cpu_fingerprint"] = self.fp

    def cpu_guest(self):
        return {"cpu_manufacturer": "AuthenticAMD", "cpu_name": "AMD Virtual CPU", "cpu_family": 107,
                "cpu_description": "AMD64 Family 23 Model 1 Stepping 2", "cpu_processor_id": "178BFBFF00800F12",
                "cpu_cores": 8, "cpu_logical": 8, "cpu_flags": ["avx", "avx2", "sse4_1", "sse4_2", "ssse3", "xsave"]}

    def argv(self, mode, **kw):
        kw.setdefault("broker_sock", "/run/user/1000/kf.sock")
        kw.setdefault("gop_efi", "/w/gop.efi")
        return self.p.build_argv(self.state, mode, "/w/disk.qcow2", "/w/run", **kw)

    def opt(self, argv, flag, prefix):
        return [v for f, v in zip(argv, argv[1:]) if f == flag and v.startswith(prefix)]

    # --- derivation
    def test_same_seed_same_ids(self):
        self.assertEqual(self.p.derive_ids(SEED), self.p.derive_ids(SEED))

    def test_different_seed_different_ids(self):
        a, b = self.p.derive_ids(SEED), self.p.derive_ids("kayfabe-prop-v2")
        self.assertNotEqual(a["smbios"]["uuid"], b["smbios"]["uuid"])
        self.assertNotEqual(a["nics"][0]["mac"], b["nics"][0]["mac"])
        self.assertNotEqual(a["gpu_uuid"], b["gpu_uuid"])
        self.assertNotEqual(a["disks"][0]["serial"], b["disks"][0]["serial"])

    def test_formats_valid(self):
        ids = self.p.derive_ids(SEED, nics=3, disks=2)
        self.assertRegex(ids["smbios"]["uuid"], r"^[0-9a-f]{8}-[0-9a-f]{4}-5[0-9a-f]{3}-[89ab][0-9a-f]{3}-[0-9a-f]{12}$")
        self.assertRegex(ids["gpu_uuid"], r"^GPU-[0-9a-f]{8}(-[0-9a-f]{4}){3}-[0-9a-f]{12}$")
        for n in ids["nics"]:
            self.assertRegex(n["mac"], r"^52:54:00(:[0-9a-f]{2}){3}$")
        for d in ids["disks"]:
            self.assertLessEqual(len(d["serial"]), 20)
        self.assertRegex(ids["smbios"]["serial"], r"^KF[0-9A-F]{10}$")

    def test_nics_and_disks_are_distinct(self):
        ids = self.p.derive_ids(SEED, nics=4, disks=4)
        self.assertEqual(len({n["mac"] for n in ids["nics"]}), 4)
        self.assertEqual(len({d["serial"] for d in ids["disks"]}), 4)

    def test_fields_use_independent_streams(self):
        ids = self.p.derive_ids(SEED)
        sm = ids["smbios"]
        self.assertEqual(len({sm["serial"][2:], sm["baseboard_serial"][3:], sm["chassis_serial"][3:]}), 3)

    def test_net_is_fixed_and_not_default(self):
        n = self.p.derive_ids(SEED)["net"]
        self.assertRegex(n["subnet"], r"^10\.(1[6-9]|2[0-9]|3[01])\.\d+\.0/24$")
        self.assertTrue(20000 <= n["ssh_port"] < 40000)
        self.assertNotEqual(n["subnet"], "10.0.2.0/24")

    def test_clock_gap_range(self):
        for s in ("a-seed", "b-seed", "c-seed", SEED):
            self.assertTrue(600 <= self.p.derive_ids(s)["clock_gap_s"] < 3600)

    def test_bad_seed_is_rejected(self):
        for s in ("", "abc", "x" * 200, "tab\tseed"):
            with self.assertRaises(self.p.PropError):
                self.p.derive_ids(s)

    def test_bad_counts_are_rejected(self):
        with self.assertRaises(self.p.PropError):
            self.p.derive_ids(SEED, nics=0)
        with self.assertRaises(self.p.PropError):
            self.p.derive_ids(SEED, disks=9)

    def test_edited_identity_is_rejected(self):
        self.assertTrue(self.p.seed_matches(self.state))
        s = copy.deepcopy(self.state)
        s["ids"]["gpu_uuid"] = "GPU-" + "1" * 8 + "-1111-1111-1111-" + "1" * 12
        self.assertFalse(self.p.seed_matches(s))

    # --- state validation
    def test_validate_ok(self):
        self.assertTrue(self.p.validate_state(self.state))

    def test_validate_rejects_bad_blob_hash(self):
        s = copy.deepcopy(self.state)
        s["files"]["tpm/tpm2-00.permall"]["sha256"] = "0" * 64
        with self.assertRaises(self.p.PropError):
            self.p.validate_state(s)

    def test_validate_rejects_bad_blob_length(self):
        s = copy.deepcopy(self.state)
        s["files"]["firmware/OVMF_VARS.fd"]["bytes"] += 1
        with self.assertRaises(self.p.PropError):
            self.p.validate_state(s)

    def test_validate_rejects_path_traversal(self):
        s = copy.deepcopy(self.state)
        s["files"]["../evil"] = blob(b"x")
        with self.assertRaises(self.p.PropError):
            self.p.validate_state(s)

    def test_validate_rejects_multicast_mac(self):
        s = copy.deepcopy(self.state)
        s["ids"]["nics"][0]["mac"] = "53:54:00:00:00:01"
        with self.assertRaises(self.p.PropError):
            self.p.validate_state(s)

    def test_validate_rejects_long_disk_serial(self):
        s = copy.deepcopy(self.state)
        s["ids"]["disks"][0]["serial"] = "K" * 21
        with self.assertRaises(self.p.PropError):
            self.p.validate_state(s)

    def test_validate_rejects_default_slirp_subnet_format(self):
        s = copy.deepcopy(self.state)
        s["ids"]["net"]["subnet"] = "192.168.1.0/24"
        with self.assertRaises(self.p.PropError):
            self.p.validate_state(s)

    # --- argv per mode
    def test_install_modes_are_online_and_real_clock(self):
        for mode in ("setup", "install"):
            a = self.argv(mode)
            self.assertIn("restrict=off", self.opt(a, "-netdev", "user")[0])
            self.assertEqual(self.opt(a, "-rtc", "base")[0], "base=utc,driftfix=slew")

    def test_offline_and_ci_are_restricted_with_faked_clock(self):
        for mode in ("offline", "ci"):
            a = self.argv(mode)
            self.assertIn("restrict=on", self.opt(a, "-netdev", "user")[0])
            self.assertNotIn("restrict=off", " ".join(a))
            self.assertTrue(self.opt(a, "-rtc", "base=2026-11-13T")[0].endswith(",clock=vm,driftfix=slew"))

    def test_fake_clock_is_stop_time_plus_gap_never_before(self):
        gap = self.state["clock"]["gap_s"]
        t = self.p.boot_clock(self.state, "ci")
        self.assertEqual((t - self.p.utc("2026-11-13T10:00:00Z")).total_seconds(), gap)
        self.assertGreater(gap, 0)

    def test_offline_needs_recorded_stop_time(self):
        self.state["clock"]["last_time_at_stop_utc"] = None
        with self.assertRaises(self.p.PropError):
            self.argv("offline")

    def test_same_ids_in_every_mode(self):
        ids = self.state["ids"]
        for mode in self.p.MODES:
            j = " ".join(self.argv(mode))
            self.assertIn(ids["smbios"]["uuid"], j)
            self.assertIn(ids["nics"][0]["mac"], j)
            self.assertIn("serial=" + ids["disks"][0]["serial"], j)
            self.assertIn(ids["net"]["subnet"], j)
            self.assertIn("hostfwd=tcp:127.0.0.1:%d-:22" % ids["net"]["ssh_port"], j)
            self.assertIn("tpm-crb", j)

    def test_hostfwd_only_on_loopback(self):
        for mode in self.p.MODES:
            j = " ".join(self.argv(mode))
            self.assertEqual(re.findall(r"hostfwd=tcp:([^:]*):", j), ["127.0.0.1"])
            self.assertEqual(j.count("hostfwd"), 1)

    def test_setup_is_std_vga_without_kf3(self):
        a = self.argv("setup")
        self.assertNotIn("kf3-gpu", " ".join(a))
        self.assertEqual(a[a.index("-vga") + 1], "std")
        self.assertEqual(a[0], "/usr/bin/qemu-system-x86_64")

    def test_kf3_modes_carry_gpu_uuid_and_broker(self):
        for mode in ("install", "offline", "ci"):
            a = self.argv(mode)
            dev = self.opt(a, "-device", "kf3-gpu")[0]
            self.assertIn("gpu-uuid=" + self.state["ids"]["gpu_uuid"], dev)
            self.assertIn("display-broker=/run/user/1000/kf.sock", dev)
            self.assertIn("gop=on", dev)
            self.assertEqual(a[a.index("-vga") + 1], "none")

    def test_headless_display_has_no_broker(self):
        dev = self.opt(self.argv("ci", display="off"), "-device", "kf3-gpu")[0]
        self.assertNotIn("display-broker", dev)
        self.assertNotIn("display=on", dev)
        self.assertIn("gpu-uuid=", dev)

    def test_broker_display_needs_socket_and_gop(self):
        with self.assertRaises(self.p.PropError):
            self.p.build_argv(self.state, "install", "/d", "/w")

    def test_unknown_mode_and_display_rejected(self):
        with self.assertRaises(self.p.PropError):
            self.argv("online")
        with self.assertRaises(self.p.PropError):
            self.argv("ci", display="vnc")

    def test_smbios_triplet_present(self):
        a = self.argv("ci")
        types = [v.split(",")[0] for f, v in zip(a, a[1:]) if f == "-smbios"]
        self.assertEqual(types, ["type=1", "type=2", "type=3"])
        sm = self.state["ids"]["smbios"]
        self.assertIn("-uuid", a)
        self.assertIn(sm["baseboard_serial"], " ".join(a))
        self.assertIn(sm["chassis_serial"], " ".join(a))

    def test_input_devices_present_in_kf3_modes(self):
        j = " ".join(self.argv("install"))
        self.assertIn("qemu-xhci", j)
        self.assertIn("usb-tablet", j)

    def test_swtpm_and_clone_commands(self):
        s = self.p.swtpm_argv("/w/run")
        self.assertIn("--tpm2", s)
        self.assertIn("--terminate", s)
        self.assertEqual(self.p.clone_cmd("/i.qcow2", "/c.qcow2")[:5], ["qemu-img", "create", "-f", "qcow2", "-b"])
        self.assertIn("-F", self.p.clone_cmd("/i", "/c"))

    # --- compare / verify
    def guest(self):
        ids = self.state["ids"]
        base = self.p.boot_clock(self.state, "ci")
        return {"smbios_uuid": ids["smbios"]["uuid"].upper(), "smbios_serial": ids["smbios"]["serial"],
                "baseboard_serial": ids["smbios"]["baseboard_serial"], "chassis_serial": ids["smbios"]["chassis_serial"],
                "macs": [n["mac"] for n in ids["nics"]], "disk_serials": [d["serial"] for d in ids["disks"]],
                "gpu_uuid": ids["gpu_uuid"], "machine_guid": self.state["guest"]["machine_guid"],
                "date_utc": self.p.fmt(base + __import__("datetime").timedelta(seconds=60)),
                "tpm_present": True, "ek_pub_hash": self.state["guest"]["ek_pub_hash"], **self.cpu_guest()}

    def verdicts(self, guest):
        return {i: v for i, v, _ in self.p.compare(self.state, guest)}

    def test_compare_all_match(self):
        v = self.verdicts(self.guest())
        self.assertEqual(set(v.values()), {"MATCH"})
        self.assertIn("clock", v)
        self.assertIn("tpm_ek_pub_hash", v)

    def test_compare_each_item_mismatches(self):
        for key, bad, item in (("smbios_uuid", "x", "smbios_uuid"), ("smbios_serial", "x", "smbios_serial"),
                               ("macs", ["52:54:00:00:00:09"], "mac"), ("disk_serials", ["Z"], "disk_serial"),
                               ("gpu_uuid", "GPU-x", "gpu_uuid"), ("machine_guid", "x", "machine_guid"),
                               ("ek_pub_hash", "00", "tpm_ek_pub_hash"), ("tpm_present", False, "tpm_present"),
                               ("baseboard_serial", "x", "baseboard_serial"), ("chassis_serial", "x", "chassis_serial")):
            g = self.guest()
            g[key] = bad
            self.assertEqual(self.verdicts(g)[item], "MISMATCH", key)

    def test_clock_before_base_mismatches(self):
        g = self.guest()
        g["date_utc"] = "2026-11-13T09:00:00Z"
        self.assertEqual(self.verdicts(g)["clock"], "MISMATCH")

    def test_clock_too_far_after_base_mismatches(self):
        g = self.guest()
        g["date_utc"] = "2026-11-20T00:00:00Z"
        self.assertEqual(self.verdicts(g)["clock"], "MISMATCH")

    def test_missing_guest_value_is_mismatch_not_match(self):
        g = self.guest()
        del g["gpu_uuid"]
        self.assertEqual(self.verdicts(g)["gpu_uuid"], "MISMATCH")
        self.state["guest"]["machine_guid"] = None
        g = self.guest()
        g["machine_guid"] = None
        self.assertEqual(self.verdicts(g)["machine_guid"], "MISMATCH")

    def test_verify_host_files(self):
        for n, rec in self.state["files"].items():
            f = self.tmp / n
            f.parent.mkdir(parents=True, exist_ok=True)
            f.write_bytes(base64.b64decode(rec["data_base64"]))
        self.assertEqual({v for _, v, _ in self.p.verify_host(self.state, self.tmp)}, {"MATCH"})
        (self.tmp / "tpm/tpm2-00.permall").write_bytes(b"drifted")
        v = {i: x for i, x, _ in self.p.verify_host(self.state, self.tmp)}
        self.assertEqual(v["file:tpm/tpm2-00.permall"], "MISMATCH")
        (self.tmp / "firmware/OVMF_VARS.fd").unlink()
        v = {i: x for i, x, _ in self.p.verify_host(self.state, self.tmp)}
        self.assertEqual(v["file:firmware/OVMF_VARS.fd"], "MISMATCH")

    def test_verify_host_empty_state_is_mismatch(self):
        self.state["files"] = {}
        self.assertEqual(self.p.verify_host(self.state, self.tmp)[0][1], "MISMATCH")

    def test_ssh_argv_is_loopback_key_only(self):
        a = self.p.ssh_argv(self.state, "/k")
        self.assertTrue(a[-6].endswith("@127.0.0.1"))
        self.assertIn(str(self.state["ids"]["net"]["ssh_port"]), a)
        self.assertIn("BatchMode=yes", a)
        self.assertEqual(a[-1], "-")

    def test_probe_script_asks_for_every_item(self):
        for needle in ("MachineGuid", "Win32_ComputerSystemProduct", "Win32_BIOS", "Win32_BaseBoard", "Get-NetAdapter",
                       "Win32_DiskDrive", "nvidia-smi", "Get-Tpm", "Get-TpmEndorsementKeyInfo", "PublicKey.RawData", "ToUniversalTime"):
            self.assertIn(needle, self.p.GUEST_PS)

    def test_run_guest_probe_parses_last_json_line(self):
        class R:
            returncode, stderr = 0, ""
            stdout = "noise\n{\"a\": 1}\n"
        seen = {}

        def runner(argv, **kw):
            seen["input"] = kw["input"]
            return R()
        self.assertEqual(self.p.run_guest_probe(self.state, "/k", runner), {"a": 1})
        self.assertEqual(seen["input"], self.p.GUEST_PS)

    def test_run_guest_probe_ssh_failure(self):
        class R:
            returncode, stderr, stdout = 255, "denied", ""
        with self.assertRaises(self.p.PropError):
            self.p.run_guest_probe(self.state, "/k", lambda *a, **k: R())

    # --- package
    def make_pkg(self):
        img = self.tmp / "i.qcow2"
        img.write_bytes(b"QFI\xfb" + b"\0" * 5000)
        sp = self.tmp / "state.json"
        sp.write_text(json.dumps(self.state))
        out = self.tmp / "p.zip"
        self.p.pack(sp, img, out)
        return out, img

    def test_pack_unpack_round_trip(self):
        out, img = self.make_pkg()
        with zipfile.ZipFile(out) as z:
            self.assertEqual(sorted(z.namelist()), ["image.qcow2", "manifest.json", "state.json"])
            self.assertTrue(all(i.compress_type == zipfile.ZIP_STORED for i in z.infolist()))
        st = self.p.unpack(out, self.tmp / "x")
        self.assertEqual(st["seed"], SEED)
        self.assertEqual((self.tmp / "x/image.qcow2").read_bytes(), img.read_bytes())

    def test_unpack_detects_tampered_image(self):
        out, _ = self.make_pkg()
        bad = self.tmp / "bad.zip"
        with zipfile.ZipFile(out) as z, zipfile.ZipFile(bad, "w", zipfile.ZIP_STORED) as w:
            for i in z.infolist():
                data = z.read(i.filename)
                w.writestr(i.filename, data + b"x" if i.filename == "image.qcow2" else data)
        with self.assertRaises(self.p.PropError):
            self.p.unpack(bad, self.tmp / "y")

    def test_unpack_rejects_extra_member(self):
        out, _ = self.make_pkg()
        with zipfile.ZipFile(out, "a", zipfile.ZIP_STORED) as z:
            z.writestr("extra.bin", b"x")
        with self.assertRaises(self.p.PropError):
            self.p.unpack(out, self.tmp / "z")

    def test_unpack_rejects_compressed_members(self):
        out, _ = self.make_pkg()
        bad = self.tmp / "c.zip"
        with zipfile.ZipFile(out) as z, zipfile.ZipFile(bad, "w", zipfile.ZIP_DEFLATED) as w:
            for i in z.infolist():
                w.writestr(i.filename, z.read(i.filename))
        with self.assertRaises(self.p.PropError):
            self.p.unpack(bad, self.tmp / "w")

    def test_pack_refuses_state_not_derived_from_seed(self):
        self.state["ids"]["smbios"]["serial"] = "KF0000000000"
        sp = self.tmp / "s.json"
        sp.write_text(json.dumps(self.state))
        img = self.tmp / "i2"
        img.write_bytes(b"x")
        with self.assertRaises(self.p.PropError):
            self.p.pack(sp, img, self.tmp / "o.zip")

    def test_cli_ids_and_verify_exit_codes(self):
        import contextlib
        import io
        buf = io.StringIO()
        with contextlib.redirect_stdout(buf):
            self.assertEqual(self.p.main(["ids", SEED]), 0)
        self.assertEqual(json.loads(buf.getvalue()), self.p.derive_ids(SEED))
        sp = self.tmp / "s.json"
        sp.write_text(json.dumps(self.state))
        for n, rec in self.state["files"].items():
            f = self.tmp / n
            f.parent.mkdir(parents=True, exist_ok=True)
            f.write_bytes(base64.b64decode(rec["data_base64"]))
        with contextlib.redirect_stdout(io.StringIO()):
            self.assertEqual(self.p.main(["verify", str(sp), "--workdir", str(self.tmp), "--cpuinfo", str(self.cpu_file())]), 0)
            (self.tmp / "firmware/OVMF_VARS.fd").write_bytes(b"x")
            self.assertEqual(self.p.main(["verify", str(sp), "--workdir", str(self.tmp), "--cpuinfo", str(self.cpu_file())]), 1)

    # --- second killers (each rule is pinned by two independent tests)
    def test_golden_ids_for_proposed_seed(self):
        ids = self.p.derive_ids(SEED)
        self.assertEqual(ids["smbios"]["uuid"], "6b3b9dd7-6030-5a6a-8334-3a35c5f22e73")
        self.assertEqual(ids["nics"][0]["mac"], "52:54:00:67:20:b0")
        raw = hashlib.sha256(("kayfabe-propimage/v1\0%s\0gpu-uuid" % SEED).encode()).digest()[:16].hex()
        shaped = "GPU-" + "-".join((raw[:8], raw[8:12], raw[12:16], raw[16:20], raw[20:32]))
        self.assertEqual(ids["gpu_uuid"], shaped)
        # golden pin without writing a board-shaped value into the tree (the GPU-UUID gate)
        self.assertEqual(hashlib.sha256(ids["gpu_uuid"].encode()).hexdigest(), "06732186a775da99ac1760c7701aa960bb5e7633f35f95c2546783f95b1168c6")
        self.assertEqual((ids["net"]["subnet"], ids["net"]["ssh_port"], ids["clock_gap_s"]), ("10.30.170.0/24", 33037, 1968))

    def test_init_state_follows_seed(self):
        a, b = self.p.init_state("seed-aaaa"), self.p.init_state("seed-bbbb")
        self.assertNotEqual(a["ids"], b["ids"])
        self.assertEqual(a["clock"]["gap_s"], a["ids"]["clock_gap_s"])

    def test_uuid_version_and_variant_bits(self):
        for s in ("s-one", "s-two", "s-three", "s-four"):
            u = self.p.derive_ids(s)["smbios"]["uuid"].split("-")
            self.assertEqual(u[2][0], "5")
            self.assertIn(u[3][0], "89ab")

    def test_gpu_uuid_not_all_zero_and_unique_per_seed(self):
        g = {self.p.derive_ids("s%d-xx" % i)["gpu_uuid"] for i in range(50)}
        self.assertEqual(len(g), 50)

    def test_many_seeds_net_in_range(self):
        for i in range(300):
            n = self.p.derive_ids("seed-%d" % i)["net"]
            self.assertRegex(n["subnet"], r"^10\.(1[6-9]|2[0-9]|3[01])\.\d+\.0/24$")
            self.assertTrue(20000 <= n["ssh_port"] < 40000)

    def test_many_seeds_clock_gap_in_range(self):
        for i in range(300):
            self.assertTrue(600 <= self.p.derive_ids("seed-%d" % i)["clock_gap_s"] < 3600)

    def test_many_seeds_disk_serial_length(self):
        for i in range(100):
            self.assertLessEqual(len(self.p.derive_ids("seed-%d" % i)["disks"][0]["serial"]), 20)
            self.assertEqual(len(self.p.derive_ids("seed-%d" % i)["disks"][0]["serial"]), 15)

    def test_many_seeds_validate(self):
        for i in range(100):
            s = self.p.init_state("seed-%d" % i, nics=2)
            self.assertTrue(self.p.validate_state(s))
            self.assertTrue(self.p.seed_matches(s))

    def test_short_seed_rejected_by_init_and_cli(self):
        with self.assertRaises(self.p.PropError):
            self.p.init_state("ab")
        self.assertEqual(self.p.main(["ids", "ab"]), 2)

    def test_state_rejects_blob_with_valid_length_but_wrong_hash(self):
        s = copy.deepcopy(self.state)
        s["files"]["tpm/tpm2-00.permall"] = dict(s["files"]["tpm/tpm2-00.permall"], data_base64=base64.b64encode(b"tpm-STATE").decode())
        with self.assertRaises(self.p.PropError):
            self.p.validate_state(s)

    def test_state_rejects_blob_with_valid_hash_but_wrong_length(self):
        s = copy.deepcopy(self.state)
        s["files"]["tpm/tpm2-00.permall"]["bytes"] = 1
        with self.assertRaises(self.p.PropError):
            self.p.validate_state(s)

    def test_state_rejects_absolute_blob_path(self):
        s = copy.deepcopy(self.state)
        s["files"]["/etc/passwd"] = blob(b"x")
        with self.assertRaises(self.p.PropError):
            self.p.validate_state(s)

    def test_state_rejects_nested_traversal(self):
        s = copy.deepcopy(self.state)
        s["files"]["tpm/../../x"] = blob(b"x")
        with self.assertRaises(self.p.PropError):
            self.p.validate_state(s)

    def test_state_rejects_multicast_second_nic(self):
        s = self.p.init_state(SEED, nics=2)
        s["ids"]["nics"][1]["mac"] = "01:00:5e:00:00:01"
        with self.assertRaises(self.p.PropError):
            self.p.validate_state(s)

    def test_state_rejects_lowercase_disk_serial(self):
        s = copy.deepcopy(self.state)
        s["ids"]["disks"][0]["serial"] = "kfd123"
        with self.assertRaises(self.p.PropError):
            self.p.validate_state(s)

    def test_state_rejects_172_subnet(self):
        s = copy.deepcopy(self.state)
        s["ids"]["net"]["subnet"] = "172.16.0.0/24"
        with self.assertRaises(self.p.PropError):
            self.p.validate_state(s)

    def test_state_rejects_wide_subnet(self):
        s = copy.deepcopy(self.state)
        s["ids"]["net"]["subnet"] = "10.0.0.0/8"
        with self.assertRaises(self.p.PropError):
            self.p.validate_state(s)

    def test_install_has_no_restrict_on_and_offline_no_restrict_off(self):
        self.assertNotIn("restrict=on", " ".join(self.argv("install")))
        self.assertNotIn("restrict=off", " ".join(self.argv("offline")))

    def test_internet_modes_never_use_a_fake_clock(self):
        self.assertIsNone(self.p.boot_clock(self.state, "install"))
        self.assertIsNone(self.p.boot_clock(self.state, "setup"))
        self.assertNotIn("clock=vm", " ".join(self.argv("install")))

    def test_ci_and_offline_boot_the_same_clock(self):
        self.assertEqual(self.p.boot_clock(self.state, "ci"), self.p.boot_clock(self.state, "offline"))
        self.assertIsNotNone(self.p.boot_clock(self.state, "offline"))

    def test_fake_clock_is_after_stop_in_argv(self):
        rtc = self.opt(self.argv("ci"), "-rtc", "base=")[0]
        got = self.p.utc(re.search(r"base=([^,]+)", rtc).group(1) + "Z")
        self.assertGreater(got, self.p.utc("2026-11-13T10:00:00Z"))

    def test_rtc_is_fixed_not_host_for_ci(self):
        self.assertNotIn("base=utc", " ".join(self.argv("ci")))
        self.assertNotIn("base=localtime", " ".join(self.argv("ci")))

    def test_hostfwd_not_on_all_interfaces(self):
        self.assertNotIn("hostfwd=tcp::", " ".join(self.argv("offline")))
        self.assertNotIn("hostfwd=tcp:0.0.0.0", " ".join(self.argv("ci")))

    def test_gpu_uuid_never_auto(self):
        for mode in ("install", "offline", "ci"):
            self.assertNotIn("gpu-uuid=auto", " ".join(self.argv(mode)))
        self.assertNotIn("gpu-uuid=auto", " ".join(self.argv("ci", display="off")))

    def test_disk_serial_on_system_disk_device(self):
        dev = self.opt(self.argv("ci"), "-device", "virtio-blk-pci")[0]
        self.assertTrue(dev.endswith("serial=" + self.state["ids"]["disks"][0]["serial"]))

    def test_uuid_arg_matches_smbios_uuid(self):
        a = self.argv("offline")
        self.assertEqual(a[a.index("-uuid") + 1], self.state["ids"]["smbios"]["uuid"])

    def test_tpm_chardev_and_tpmdev_present(self):
        a = self.argv("ci")
        self.assertIn("tpm-crb,tpmdev=tpm0", a)
        self.assertTrue(self.opt(a, "-tpmdev", "emulator"))

    def test_setup_vga_option_value_is_std_not_none(self):
        a = self.argv("setup")
        self.assertNotEqual(a[a.index("-vga") + 1], "none")
        self.assertEqual(a[a.index("-display") + 1], "gtk")

    def test_broker_display_without_gop_efi_is_refused(self):
        with self.assertRaises(self.p.PropError):
            self.p.build_argv(self.state, "ci", "/d", "/w", broker_sock="/s")
        with self.assertRaises(self.p.PropError):
            self.p.build_argv(self.state, "ci", "/d", "/w", gop_efi="/g")

    def test_bad_mode_names_refused(self):
        for m in ("", "online", "CI", "setup "):
            with self.assertRaises(self.p.PropError):
                self.p.build_argv(self.state, m, "/d", "/w", broker_sock="/s", gop_efi="/g")

    def test_none_guest_values_never_match(self):
        g = {k: None for k in self.guest()}
        v = self.verdicts(g)
        self.assertEqual([k for k, x in v.items() if x == "MATCH"], [])

    def test_state_without_ek_hash_never_matches_tpm(self):
        self.state["guest"]["ek_pub_hash"] = None
        g = self.guest()
        g["ek_pub_hash"] = None
        self.assertEqual(self.verdicts(g)["tpm_ek_pub_hash"], "MISMATCH")

    def test_clock_exactly_at_base_matches_one_second_before_does_not(self):
        base = self.p.boot_clock(self.state, "ci")
        g = self.guest()
        g["date_utc"] = self.p.fmt(base)
        self.assertEqual(self.verdicts(g)["clock"], "MATCH")
        g["date_utc"] = self.p.fmt(base - __import__("datetime").timedelta(seconds=1))
        self.assertEqual(self.verdicts(g)["clock"], "MISMATCH")

    def test_clock_window_upper_edge(self):
        base = self.p.boot_clock(self.state, "ci")
        d = __import__("datetime").timedelta
        g = self.guest()
        g["date_utc"] = self.p.fmt(base + d(seconds=1800))
        self.assertEqual(self.verdicts(g)["clock"], "MATCH")
        g["date_utc"] = self.p.fmt(base + d(seconds=1801))
        self.assertEqual(self.verdicts(g)["clock"], "MISMATCH")

    def test_tpm_absent_is_reported(self):
        g = self.guest()
        g["tpm_present"] = False
        self.assertIn(("tpm_present", "MISMATCH"), [(i, v) for i, v, _ in self.p.compare(self.state, g)])
        g["tpm_present"] = None
        self.assertEqual(self.verdicts(g)["tpm_present"], "MISMATCH")

    def test_verify_host_reports_each_file(self):
        names = {i for i, _, _ in self.p.verify_host(self.state, self.tmp)}
        self.assertEqual(names, {"file:tpm/tpm2-00.permall", "file:firmware/OVMF_VARS.fd"})
        self.assertEqual({v for _, v, _ in self.p.verify_host(self.state, self.tmp)}, {"MISMATCH"})

    def test_verify_host_empty_state_exit_code(self):
        self.state["files"] = {}
        sp = self.tmp / "e.json"
        sp.write_text(json.dumps(self.state))
        self.assertEqual(self.p.main(["verify", str(sp), "--workdir", str(self.tmp), "--cpuinfo", str(self.cpu_file())]), 1)

    def test_ssh_is_batch_and_identity_only(self):
        a = self.p.ssh_argv(self.state, "/k")
        self.assertIn("BatchMode=yes", a)
        self.assertNotIn("BatchMode=no", a)
        self.assertIn("IdentitiesOnly=yes", a)

    def test_zip_members_stored_in_pack_output(self):
        out, _ = self.make_pkg()
        with zipfile.ZipFile(out) as z:
            self.assertEqual({i.compress_type for i in z.infolist()}, {zipfile.ZIP_STORED})
            self.assertEqual(json.loads(z.read("manifest.json"))["members"]["image.qcow2"],
                             hashlib.sha256(z.read("image.qcow2")).hexdigest())

    def test_pack_unpack_manifest_binds_state(self):
        out, _ = self.make_pkg()
        bad = self.tmp / "s.zip"
        with zipfile.ZipFile(out) as z, zipfile.ZipFile(bad, "w", zipfile.ZIP_STORED) as w:
            for i in z.infolist():
                d = z.read(i.filename)
                w.writestr(i.filename, d.replace(b"kayfabe-prop-v1", b"kayfabe-prop-v2") if i.filename == "state.json" else d)
        with self.assertRaises(self.p.PropError):
            self.p.unpack(bad, self.tmp / "q")

    def test_unpack_rejects_missing_member(self):
        out, _ = self.make_pkg()
        bad = self.tmp / "m.zip"
        with zipfile.ZipFile(out) as z, zipfile.ZipFile(bad, "w", zipfile.ZIP_STORED) as w:
            for i in z.infolist():
                if i.filename != "manifest.json":
                    w.writestr(i.filename, z.read(i.filename))
        with self.assertRaises(self.p.PropError):
            self.p.unpack(bad, self.tmp / "r")

    def test_unpack_rejects_tampered_manifest_hash(self):
        out, _ = self.make_pkg()
        bad = self.tmp / "t.zip"
        with zipfile.ZipFile(out) as z, zipfile.ZipFile(bad, "w", zipfile.ZIP_STORED) as w:
            for i in z.infolist():
                d = z.read(i.filename)
                if i.filename == "manifest.json":
                    m = json.loads(d)
                    m["members"]["state.json"] = "0" * 64
                    d = json.dumps(m).encode()
                w.writestr(i.filename, d)
        with self.assertRaises(self.p.PropError):
            self.p.unpack(bad, self.tmp / "u")

    def test_unpack_rejects_extra_member_even_if_hash_listed(self):
        out, _ = self.make_pkg()
        bad = self.tmp / "x.zip"
        with zipfile.ZipFile(out) as z, zipfile.ZipFile(bad, "w", zipfile.ZIP_STORED) as w:
            for i in z.infolist():
                w.writestr(i.filename, z.read(i.filename))
            w.writestr("aux.bin", b"y")
        with self.assertRaises(self.p.PropError):
            self.p.unpack(bad, self.tmp / "v")


    def test_state_rejects_very_long_disk_serial(self):
        s = copy.deepcopy(self.state)
        s["ids"]["disks"][0]["serial"] = "KFD" + "A" * 40
        with self.assertRaises(self.p.PropError):
            self.p.validate_state(s)

    def test_ci_rtc_follows_vm_clock(self):
        self.assertIn(",clock=vm,", self.opt(self.argv("ci"), "-rtc", "base=")[0] + ",")
        self.assertIn("clock=vm", " ".join(self.argv("offline")))

    def test_unpack_rejects_one_deflated_member(self):
        out, _ = self.make_pkg()
        bad = self.tmp / "d1.zip"
        with zipfile.ZipFile(out) as z, zipfile.ZipFile(bad, "w", zipfile.ZIP_STORED) as w:
            for i in z.infolist():
                w.writestr(i.filename, z.read(i.filename),
                           compress_type=zipfile.ZIP_DEFLATED if i.filename == "state.json" else zipfile.ZIP_STORED)
        with self.assertRaises(self.p.PropError):
            self.p.unpack(bad, self.tmp / "d2")


    # --- CPU profile (image-mode-only exception; win_vm.sh default untouched)
    CPUINFO_OK = "processor\t: 0\nvendor_id\t: AuthenticAMD\ncpu family\t: 25\nflags\t\t: %s\n\nprocessor\t: 1\nflags\t: x\n"

    def host_flags(self, drop=()):
        return " ".join(sorted(set(self.state["cpu"]["required_host_flags"]) - set(drop)) + ["svm", "avx512f"])

    def test_every_mode_uses_the_named_profile_not_host(self):
        for mode in self.p.MODES:
            a = self.argv(mode)
            cpu = a[a.index("-cpu") + 1]
            self.assertTrue(cpu.startswith("EPYC-v1,model-id=AMD Virtual CPU,family=23,model=1,stepping=2,"), cpu)
            self.assertNotIn("host", cpu.split(","))
            self.assertEqual(cpu, self.p.cpu_arg(self.state))

    def test_profile_removes_forbidden_features_and_keeps_hyperv_arm_b(self):
        cpu = self.p.cpu_arg(self.state).split(",")
        for f in ("avx512f", "rtm", "hle", "mpx", "sgx", "vmx"):
            self.assertIn("-" + f, cpu)
        for f in ("hv-relaxed", "hv-vapic", "hv-spinlocks=0x1fff", "hv-time", "hv-vpindex", "hv-synic", "hv-stimer", "hv-crash"):
            self.assertIn(f, cpu)
        self.assertNotIn("+avx512f", cpu)

    def test_profile_does_not_enable_avx512_anywhere(self):
        self.assertNotIn("avx512", " ".join(self.argv("ci")).replace("-avx512f", ""))

    def test_smp_is_fixed_by_the_state_not_the_host(self):
        a = self.argv("ci")
        self.assertEqual(a[a.index("-smp") + 1], "8,sockets=1,cores=8,threads=1")
        self.state["qemu"]["smp"] = 4
        a = self.argv("ci")
        self.assertEqual(a[a.index("-smp") + 1], "4,sockets=1,cores=4,threads=1")

    def test_win_vm_default_argv_is_untouched(self):
        sh = (ROOT / "scripts/bench/windows/win_vm.sh").read_text()
        self.assertIn('A) echo "host,-vmx" ;;', sh)
        self.assertIn("host,-vmx,hv-relaxed", sh)
        self.assertNotIn("EPYC", sh)

    def test_preflight_passes_on_amd_host_with_flags(self):
        rows = self.p.cpu_preflight(self.state, self.CPUINFO_OK % self.host_flags())
        self.assertEqual({v for _, v, _ in rows}, {"MATCH"})

    def test_preflight_refuses_intel_host(self):
        txt = (self.CPUINFO_OK % self.host_flags()).replace("AuthenticAMD", "GenuineIntel")
        v = {i: x for i, x, _ in self.p.cpu_preflight(self.state, txt)}
        self.assertEqual(v["cpu_vendor"], "MISMATCH")
        self.assertEqual(v["cpu_features"], "MATCH")

    def test_preflight_names_missing_feature(self):
        rows = self.p.cpu_preflight(self.state, self.CPUINFO_OK % self.host_flags(drop=("avx2", "bmi2")))
        d = {i: (x, y) for i, x, y in rows}
        self.assertEqual(d["cpu_features"][0], "MISMATCH")
        self.assertIn("avx2", d["cpu_features"][1])
        self.assertIn("bmi2", d["cpu_features"][1])

    def test_preflight_refuses_empty_cpuinfo(self):
        v = {i: x for i, x, _ in self.p.cpu_preflight(self.state, "")}
        self.assertEqual(v, {"cpu_vendor": "MISMATCH", "cpu_features": "MISMATCH"})

    def test_preflight_uses_first_processor_only(self):
        txt = "vendor_id\t: AuthenticAMD\nflags\t: aes\n\nvendor_id\t: AuthenticAMD\nflags\t: %s\n" % self.host_flags()
        self.assertEqual(self.p.cpu_preflight(self.state, txt)[1][1], "MISMATCH")

    def test_required_flags_include_the_owner_list(self):
        for f in ("avx2", "bmi1", "bmi2", "fma", "f16c", "movbe", "sse4_2", "aes", "pclmulqdq", "rdrand", "popcnt", "xsave"):
            self.assertIn(f, self.state["cpu"]["required_host_flags"])
        for f in ("avx512f", "rtm", "hle", "mpx", "sgx"):
            self.assertNotIn(f, self.state["cpu"]["required_host_flags"])

    def cpu_file(self):
        f = self.tmp / "cpuinfo.ok"
        f.write_text(self.CPUINFO_OK % self.host_flags())
        return f

    def test_argv_cli_refuses_with_named_error(self):
        import contextlib
        import io
        sp = self.tmp / "s.json"
        sp.write_text(json.dumps(self.state))
        ci = self.tmp / "cpuinfo"
        ci.write_text("vendor_id\t: GenuineIntel\nflags\t: avx2\n")
        err, out = io.StringIO(), io.StringIO()
        with contextlib.redirect_stderr(err), contextlib.redirect_stdout(out):
            rc = self.p.main(["argv", str(sp), "ci", "--disk", "/d", "--workdir", "/w", "--display", "off", "--cpuinfo", str(ci)])
        self.assertEqual(rc, 3)
        self.assertIn("CPU_PROFILE_UNSUPPORTED", err.getvalue())
        self.assertNotIn("qemu-system", out.getvalue())

    def test_argv_cli_prints_on_matching_host_and_on_override(self):
        import contextlib
        import io
        sp = self.tmp / "s.json"
        sp.write_text(json.dumps(self.state))
        ci = self.tmp / "cpuinfo"
        ci.write_text(self.CPUINFO_OK % self.host_flags())
        args = ["argv", str(sp), "ci", "--disk", "/d", "--workdir", "/w", "--display", "off"]
        out = io.StringIO()
        with contextlib.redirect_stdout(out):
            self.assertEqual(self.p.main(args + ["--cpuinfo", str(ci)]), 0)
        self.assertIn("-cpu 'EPYC-v1,model-id=AMD Virtual CPU", out.getvalue())
        ci.write_text("vendor_id\t: GenuineIntel\nflags\t: x\n")
        with contextlib.redirect_stdout(io.StringIO()):
            self.assertEqual(self.p.main(args + ["--cpuinfo", str(ci), "--no-cpu-preflight"]), 0)

    def test_verify_host_includes_cpu_rows(self):
        names = {i for i, _, _ in self.p.verify_host(self.state, self.tmp, self.CPUINFO_OK % self.host_flags())}
        self.assertTrue({"cpu_vendor", "cpu_features"} <= names)
        names = {i for i, _, _ in self.p.verify_host(self.state, self.tmp)}
        self.assertFalse({"cpu_vendor"} & names)

    def test_state_has_profile_and_fingerprint_slot(self):
        s = self.p.init_state(SEED)
        self.assertEqual(s["cpu"]["name"], "kayfabe-amd-v1")
        self.assertIsNone(s["cpu_fingerprint"])
        self.assertEqual(s["cpu"]["vendor"], "AuthenticAMD")

    def test_validate_rejects_vendor_spoof_profile(self):
        s = copy.deepcopy(self.state)
        s["cpu"]["vendor"] = "GenuineIntel"
        with self.assertRaises(self.p.PropError):
            self.p.validate_state(s)

    def test_validate_rejects_profile_without_avx512_removal(self):
        s = copy.deepcopy(self.state)
        s["cpu"]["remove"] = [f for f in s["cpu"]["remove"] if f != "avx512f"]
        with self.assertRaises(self.p.PropError):
            self.p.validate_state(s)

    def test_validate_rejects_rtm_not_removed(self):
        s = copy.deepcopy(self.state)
        s["cpu"]["remove"].remove("rtm")
        with self.assertRaises(self.p.PropError):
            self.p.validate_state(s)

    def test_validate_rejects_brand_injection(self):
        for bad in ("A,B", "a=b", "x" * 49, ""):
            s = copy.deepcopy(self.state)
            s["cpu"]["model_id"] = bad
            with self.assertRaises(self.p.PropError):
                self.p.validate_state(s)

    def test_validate_rejects_bad_hyperv_flag(self):
        s = copy.deepcopy(self.state)
        s["cpu"]["hv_flags"].append("hv-time,+avx512f")
        with self.assertRaises(self.p.PropError):
            self.p.validate_state(s)

    def test_validate_rejects_unknown_profile_and_bad_family(self):
        s = copy.deepcopy(self.state)
        s["cpu"]["name"] = "other"
        with self.assertRaises(self.p.PropError):
            self.p.validate_state(s)
        s = copy.deepcopy(self.state)
        s["cpu"]["family"] = "23"
        with self.assertRaises(self.p.PropError):
            self.p.validate_state(s)

    def test_fingerprint_flags_hash_is_over_sorted_list(self):
        a = self.p.cpu_fingerprint(dict(self.cpu_guest(), cpu_flags=["b", "a"]))
        b = self.p.cpu_fingerprint(dict(self.cpu_guest(), cpu_flags=["a", "b"]))
        self.assertEqual(a["flags_sha256"], b["flags_sha256"])
        self.assertEqual(a["flags_sha256"], hashlib.sha256(b"a\nb").hexdigest())
        self.assertNotEqual(a["flags_sha256"], self.p.cpu_fingerprint(dict(self.cpu_guest(), cpu_flags=["a"]))["flags_sha256"])

    def test_compare_cpu_blocking_items_mismatch(self):
        for key, bad, item in (("cpu_manufacturer", "GenuineIntel", "cpu_vendor"),
                               ("cpu_description", "AMD64 Family 25 Model 97 Stepping 2", "cpu_family_model_stepping")):
            g = self.guest()
            g[key] = bad
            self.assertEqual(self.verdicts(g)[item], "MISMATCH", key)

    def test_compare_cpu_informational_items_only_warn(self):
        for key, bad, item in (("cpu_name", "AMD Ryzen 9 7900", "cpu_brand"), ("cpu_processor_id", "178BFBFF00A60F12", "cpu_processor_id"),
                               ("cpu_cores", 12, "cpu_cores"), ("cpu_logical", 24, "cpu_logical"),
                               ("cpu_flags", ["avx2"], "cpu_flags_sha256")):
            g = self.guest()
            g[key] = bad
            self.assertEqual(self.verdicts(g)[item], "WARN", key)
            self.assertEqual([r[0] for r in self.p.compare(self.state, g) if r[1] == "MISMATCH"], [])

    def test_warn_does_not_fail_the_result(self):
        import contextlib
        import io
        with contextlib.redirect_stdout(io.StringIO()):
            self.assertEqual(self.p._print_rows([("a", "MATCH", ""), ("b", "WARN", "")]), 0)
            self.assertEqual(self.p._print_rows([("a", "MATCH", ""), ("b", "MISMATCH", "")]), 1)

    def test_profile_flags_exist_on_the_two_measured_amd_hosts(self):
        # minimal sanity (not a strict gate): the PC (Zen 4) and a Vast EPYC 7B12 (Zen 2, through a hypervisor)
        fx = ROOT / "scripts/ci/fixtures/cpuflags"
        for name in ("pc_zen4.txt", "vast_epyc7b12_zen2.txt"):
            text = (fx / name).read_text().replace("vendor_id:", "vendor_id\t:").replace("flags:", "flags\t:")
            vendor, flags = self.p.parse_cpuinfo(text)
            self.assertEqual(vendor, "AuthenticAMD")
            self.assertEqual(sorted(set(self.state["cpu"]["required_host_flags"]) - flags), [], name)
        self.assertFalse(set(self.p.FORBIDDEN_GUEST_FLAGS) & set(self.state["cpu"]["required_host_flags"]))

    def test_compare_flags_forbidden_in_guest(self):
        g = self.guest()
        g["cpu_flags"] = g["cpu_flags"] + ["avx512f"]
        self.assertEqual(self.verdicts(g)["cpu_no_forbidden_flags"], "WARN")
        g["cpu_flags"] = None
        self.assertEqual(self.verdicts(g)["cpu_no_forbidden_flags"], "WARN")

    def test_compare_without_recorded_fingerprint_is_mismatch(self):
        self.state["cpu_fingerprint"] = None
        v = self.verdicts(self.guest())
        self.assertEqual(v["cpu_vendor"], "MISMATCH")
        self.assertEqual(v["cpu_family_model_stepping"], "MISMATCH")
        for item in ("cpu_brand", "cpu_processor_id", "cpu_flags_sha256", "cpu_logical"):
            self.assertEqual(v[item], "WARN", item)

    def test_probe_script_reads_cpu_identity(self):
        for needle in ("Win32_Processor", "ProcessorId", "NumberOfLogicalProcessors", "IsProcessorFeaturePresent", "cpu_flags"):
            self.assertIn(needle, self.p.GUEST_PS)

    def test_fill_from_guest_records_non_derivable_facts(self):
        s = self.p.init_state(SEED)
        self.p.fill_from_guest(s, self.guest())
        self.assertEqual(s["guest"]["machine_guid"], self.state["guest"]["machine_guid"])
        self.assertEqual(s["clock"]["last_time_at_stop_utc"], self.guest()["date_utc"])
        self.assertEqual(s["cpu_fingerprint"], self.fp)
        self.assertEqual(s["guest"]["ek_pub_hash"], "AB" * 32)


    def test_result_line_says_match_when_only_warnings(self):
        import contextlib
        import io
        out = io.StringIO()
        with contextlib.redirect_stdout(out):
            rc = self.p._print_rows([("x", "WARN", "d"), ("y", "WARN", "d")])
        self.assertEqual(rc, 0)
        self.assertIn("RESULT MATCH", out.getvalue())

    def test_cpu_arg_pins_family_model_stepping_explicitly(self):
        parts = self.p.cpu_arg(self.state).split(",")
        self.assertEqual(parts[:5], ["EPYC-v1", "model-id=AMD Virtual CPU", "family=23", "model=1", "stepping=2"])

    def test_pack_refuses_spoofed_vendor(self):
        self.state["cpu"]["vendor"] = "GenuineIntel"
        sp = self.tmp / "v.json"
        sp.write_text(json.dumps(self.state))
        img = self.tmp / "iv"
        img.write_bytes(b"x")
        with self.assertRaises(self.p.PropError):
            self.p.pack(sp, img, self.tmp / "v.zip")

    def test_validate_rejects_comma_brand_only(self):
        s = copy.deepcopy(self.state)
        s["cpu"]["model_id"] = "AMD,+avx512f"
        with self.assertRaises(self.p.PropError):
            self.p.validate_state(s)


    # --- unattended media, state files, cdroms
    PUB = "ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAAIFixtureFixtureFixtureFixtureFixtureFixtu test"

    def template(self):
        return (ROOT / "scripts/bench/windows/propimage_autounattend.xml.in").read_text(encoding="utf-8")

    def test_unattend_is_deterministic_and_filled(self):
        a = self.p.render_unattend(self.template(), self.state)
        self.assertEqual(a, self.p.render_unattend(self.template(), self.state))
        self.assertNotIn("@@", a)
        self.assertIn("<Name>kfimage</Name>", a)
        self.assertIn(self.p.derive_password(SEED), a)
        self.assertIn("<Key>/IMAGE/INDEX</Key><Value>6</Value>", a)
        self.assertIn("E:\\viostor\\w11\\amd64", a)
        self.assertNotIn("FirstLogonCommands", a)

    def test_unattend_is_wellformed_xml_and_keeps_secure_boot_and_tpm_checks(self):
        import xml.dom.minidom
        a = self.p.render_unattend(self.template(), self.state)
        xml.dom.minidom.parseString(a.encode())
        self.assertIn("BypassCPUCheck", a)
        self.assertNotIn("BypassTPMCheck", a)
        self.assertNotIn("BypassSecureBootCheck", a)

    def test_password_is_derived_from_the_seed(self):
        self.assertEqual(self.p.derive_password(SEED), self.p.derive_password(SEED))
        self.assertNotEqual(self.p.derive_password(SEED), self.p.derive_password("kayfabe-prop-v2"))
        self.assertRegex(self.p.derive_password(SEED), r"^Kf-[0-9a-f]{16}$")

    def test_render_rejects_bad_user_and_computer_names(self):
        for u in ("a b", "x<y", "", "1abc", "a" * 21):
            s = copy.deepcopy(self.state)
            s["guest"]["user"] = u
            with self.assertRaises(self.p.PropError):
                self.p.render_unattend(self.template(), s)
        with self.assertRaises(self.p.PropError):
            self.p.render_unattend(self.template(), self.state, computer="bad name")

    def test_render_refuses_a_template_without_a_placeholder(self):
        with self.assertRaises(self.p.PropError):
            self.p.render_unattend(self.template().replace("@@USER@@", "root"), self.state)

    def test_capture_and_restore_files_round_trip(self):
        wd = self.tmp / "wd"
        (wd / "tpm").mkdir(parents=True)
        (wd / "firmware").mkdir()
        (wd / "tpm/tpm2-00.permall").write_bytes(b"perm")
        (wd / "firmware/OVMF_VARS.fd").write_bytes(b"vars")
        names = self.p.capture_files(self.state, wd)
        self.assertEqual(names, ["firmware/OVMF_VARS.fd", "tpm/tpm2-00.permall"])
        self.assertTrue(self.p.validate_state(self.state))
        out = self.tmp / "out"
        self.p.restore_files(self.state, out)
        self.assertEqual((out / "tpm/tpm2-00.permall").read_bytes(), b"perm")
        self.assertEqual((out / "firmware/OVMF_VARS.fd").read_bytes(), b"vars")
        self.assertEqual({v for _, v, _ in self.p.verify_host(self.state, out)}, {"MATCH"})

    def test_capture_files_requires_tpm_and_ovmf(self):
        wd = self.tmp / "wd2"
        (wd / "tpm").mkdir(parents=True)
        (wd / "firmware").mkdir()
        (wd / "firmware/OVMF_VARS.fd").write_bytes(b"v")
        with self.assertRaises(self.p.PropError):
            self.p.capture_files(self.state, wd)
        (wd / "tpm/tpm2-00.permall").write_bytes(b"p")
        (wd / "firmware/OVMF_VARS.fd").unlink()
        with self.assertRaises(self.p.PropError):
            self.p.capture_files(self.state, wd)

    def test_capture_files_refuses_symlinks(self):
        wd = self.tmp / "wd3"
        (wd / "tpm").mkdir(parents=True)
        (wd / "firmware").mkdir()
        (self.tmp / "real").write_bytes(b"x")
        (wd / "tpm/tpm2-00.permall").symlink_to(self.tmp / "real")
        (wd / "firmware/OVMF_VARS.fd").write_bytes(b"v")
        with self.assertRaises(self.p.PropError):
            self.p.capture_files(self.state, wd)

    def test_setup_argv_attaches_install_media_windows_iso_first(self):
        a = self.argv("setup", cdroms=["/m/win.iso", "/m/virtio.iso", "/m/unattend.iso"])
        j = " ".join(a)
        self.assertIn("file=/m/win.iso", j)
        self.assertIn("file=/m/virtio.iso", j)
        self.assertEqual(self.opt(a, "-device", "ide-cd,drive=cd0"), ["ide-cd,drive=cd0,bus=ide.0,bootindex=1"])
        self.assertEqual(self.opt(a, "-device", "ide-cd,drive=cd1"), ["ide-cd,drive=cd1,bus=ide.1"])
        self.assertEqual(j.count("media=cdrom"), 3)
        self.assertEqual(j.count("readonly=on,file=/m/"), 3)

    def test_each_cdrom_has_its_own_ahci_port(self):
        a = self.argv("setup", cdroms=["/a", "/b", "/c"])
        self.assertEqual([v.split("bus=")[1].split(",")[0] for v in self.opt(a, "-device", "ide-cd")], ["ide.0", "ide.1", "ide.2"])

    def test_more_than_six_cdroms_refused(self):
        with self.assertRaises(self.p.PropError):
            self.argv("setup", cdroms=["/x"] * 7)

    def test_six_cdroms_are_accepted(self):
        self.assertEqual(self.argv("setup", cdroms=["/x%d" % i for i in range(6)]).count("-drive"), 2 + 1 + 6)

    def test_cli_refuses_seven_cdroms(self):
        sp = self.tmp / "c7.json"
        sp.write_text(json.dumps(self.state))
        args = ["argv", str(sp), "setup", "--disk", "/d", "--workdir", "/w", "--no-cpu-preflight"]
        for i in range(7):
            args += ["--cdrom", "/x%d" % i]
        self.assertEqual(self.p.main(args), 2)

    def test_no_cdroms_by_default(self):
        self.assertNotIn("ide-cd", " ".join(self.argv("ci")))

    def test_s3_s4_disabled_in_every_mode(self):
        for mode in self.p.MODES:
            j = " ".join(self.argv(mode))
            self.assertIn("ICH9-LPC.disable_s3=1", j)
            self.assertIn("ICH9-LPC.disable_s4=1", j)

    def test_unattend_rejects_xml_injection_in_user(self):
        for u in ('a"b', "a</Name><X/>", "a&b"):
            s = copy.deepcopy(self.state)
            s["guest"]["user"] = u
            with self.assertRaises(self.p.PropError):
                self.p.render_unattend(self.template(), s)

    def test_capture_files_refuses_symlinked_ovmf_vars(self):
        wd = self.tmp / "wd4"
        (wd / "tpm").mkdir(parents=True)
        (wd / "firmware").mkdir()
        (wd / "tpm/tpm2-00.permall").write_bytes(b"p")
        (self.tmp / "realv").write_bytes(b"v")
        (wd / "firmware/OVMF_VARS.fd").symlink_to(self.tmp / "realv")
        with self.assertRaises(self.p.PropError):
            self.p.capture_files(self.state, wd)

    def test_s3_s4_flags_are_global_options(self):
        a = self.argv("offline")
        self.assertEqual([a[i] for i, v in enumerate(a) if v.startswith("ICH9-LPC.disable_s")], ["ICH9-LPC.disable_s3=1", "ICH9-LPC.disable_s4=1"])
        for i, v in enumerate(a):
            if v.startswith("ICH9-LPC"):
                self.assertEqual(a[i - 1], "-global")

    def test_only_the_first_cdrom_has_a_boot_index(self):
        a = self.argv("setup", cdroms=["/m/a.iso", "/m/b.iso"])
        self.assertEqual(" ".join(a).count("bootindex"), 1)
        self.assertNotIn("bootindex", " ".join(self.argv("setup")))


    # --- the one install CD
    def tools_files(self):
        d = self.tmp / "src"
        d.mkdir(exist_ok=True)
        return d

    def make_tools(self, pins_ok=True):
        d = self.tools_files()
        msi, nv = d / self.p.OPENSSH_MSI, d / self.p.NVIDIA_EXE
        msi.write_bytes(b"msi")
        nv.write_bytes(b"nvidia")
        if pins_ok:
            self.p.PINS[self.p.OPENSSH_MSI] = hashlib.sha256(b"msi").hexdigest()
            self.p.PINS[self.p.NVIDIA_EXE] = hashlib.sha256(b"nvidia").hexdigest()
        return msi, nv

    def test_tools_cd_holds_exactly_four_files(self):
        self.state["guest"]["ssh_public_key"] = self.PUB
        msi, nv = self.make_tools()
        names = self.p.write_tools_dir(self.state, self.tmp / "cd", msi, nv)
        self.assertEqual(names, sorted(["autounattend.xml", self.p.OPENSSH_MSI, self.p.NVIDIA_EXE, "setup_ssh.ps1"]))

    def test_tools_cd_refuses_unpinned_hash(self):
        self.state["guest"]["ssh_public_key"] = self.PUB
        msi, nv = self.make_tools(pins_ok=False)
        with self.assertRaises(self.p.PropError):
            self.p.write_tools_dir(self.state, self.tmp / "cd1", msi, nv)

    def test_tools_cd_refuses_wrong_file_name(self):
        self.state["guest"]["ssh_public_key"] = self.PUB
        msi, nv = self.make_tools()
        other = self.tmp / "src/evil.exe"
        other.write_bytes(b"nvidia")
        with self.assertRaises(self.p.PropError):
            self.p.write_tools_dir(self.state, self.tmp / "cd2", msi, other)

    def test_setup_script_carries_only_the_public_key(self):
        self.state["guest"]["ssh_public_key"] = self.PUB
        self.state["guest"]["ssh_private_key"] = "-----BEGIN " + "OPENSSH PRIVATE KEY-----\nSECRET\n"
        ps = self.p.render_setup_ssh(self.state)
        self.assertIn(self.PUB, ps)
        self.assertNotIn("PRIVATE", ps)
        self.assertNotIn("SECRET", ps)

    def test_setup_script_is_deterministic_idempotent_and_pinned(self):
        self.state["guest"]["ssh_public_key"] = self.PUB
        ps = self.p.render_setup_ssh(self.state)
        self.assertEqual(ps, self.p.render_setup_ssh(self.state))
        self.assertIn(self.p.PINS[self.p.OPENSSH_MSI] if False else "ddec9c53864280759cf9f74791cefd387100e3946aa849a1c138a4ed1b96b7d9", ps)
        for needle in ("Get-Service sshd", "Get-NetFirewallRule", "DefaultShell", "administrators_authorized_keys", "icacls.exe",
                       "Restart-Service sshd", "SSH_READY", "MACHINE_GUID", "SMBIOS_UUID", "$MyInvocation.MyCommand.Path"):
            self.assertIn(needle, ps)

    def test_setup_script_rejects_missing_or_odd_key(self):
        for k in (None, "", "ssh-rsa AAAA x", "ssh-ed25519 AAAA'; calc; '"):
            s = copy.deepcopy(self.state)
            s["guest"]["ssh_public_key"] = k
            with self.assertRaises(self.p.PropError):
                self.p.render_setup_ssh(s)

    def test_guest_ram_is_a_shared_memfd_backend_in_every_mode(self):
        for mode in self.p.MODES:
            a = self.argv(mode)
            j = " ".join(a)
            self.assertIn("memory-backend-memfd,id=ram0,size=8192M,share=on", j)
            self.assertIn("memory-backend=ram0", j)

    def test_machine_option_binds_the_ram_backend(self):
        a = self.argv("ci", display="off")
        self.assertIn("memory-backend=ram0", a[a.index("-machine") + 1])
        self.assertIn("share=on", a[a.index("-object") + 1])
        self.assertEqual(a[a.index("-m") + 1], "8192")

    def test_kf3_device_carries_guest_driver_abi(self):
        dev = self.opt(self.argv("install"), "-device", "kf3-gpu")[0]
        self.assertIn("guest-driver=580.65.06", dev)
        self.assertIn("guest-driver=580.65.06", self.opt(self.argv("ci", display="off"), "-device", "kf3-gpu")[0])

    def test_guest_driver_in_every_kf3_mode(self):
        for mode in ("install", "offline", "ci"):
            self.assertIn("guest-driver=580.65.06", " ".join(self.argv(mode)))
        self.assertNotIn("guest-driver", " ".join(self.argv("setup")))

    def test_validate_rejects_nonnumeric_guest_driver(self):
        s = copy.deepcopy(self.state)
        s["qemu"]["guest_driver"] = "latest"
        with self.assertRaises(self.p.PropError):
            self.p.validate_state(s)

    def test_validate_rejects_bad_guest_driver(self):
        s = copy.deepcopy(self.state)
        s["qemu"]["guest_driver"] = "580.65.06,x=1"
        with self.assertRaises(self.p.PropError):
            self.p.validate_state(s)

    def test_cli_has_only_the_flow_verbs(self):
        import contextlib
        import io
        err = io.StringIO()
        with contextlib.redirect_stderr(err), self.assertRaises(SystemExit):
            self.p.main(["--help"] if False else ["bogus"])
        for verb in ("ids", "password", "init", "tools", "argv", "verify", "pack", "unpack"):
            self.assertIn(verb, err.getvalue())
        for gone in ("media", "capture-files", "restore-files", "record", "verify-host"):
            self.assertNotIn("'%s'" % gone, err.getvalue())

    def test_pack_and_unpack_carry_the_work_directory_files(self):
        wd = self.tmp / "wdp"
        (wd / "tpm").mkdir(parents=True)
        (wd / "firmware").mkdir()
        (wd / "tpm/tpm2-00.permall").write_bytes(b"perm2")
        (wd / "firmware/OVMF_VARS.fd").write_bytes(b"vars2")
        s = self.p.init_state(SEED)
        sp = self.tmp / "pp.json"
        sp.write_text(json.dumps(s))
        img = self.tmp / "pp.qcow2"
        img.write_bytes(b"QFI")
        z = self.tmp / "pp.zip"
        import contextlib
        import io
        with contextlib.redirect_stdout(io.StringIO()):
            self.assertEqual(self.p.main(["pack", str(sp), str(img), "--out", str(z), "--workdir", str(wd)]), 0)
            out = self.tmp / "ppo"
            self.assertEqual(self.p.main(["unpack", str(z), "--dir", str(out / "x"), "--workdir", str(out / "w")]), 0)
        self.assertEqual((out / "w/tpm/tpm2-00.permall").read_bytes(), b"perm2")
        self.assertEqual((out / "w/firmware/OVMF_VARS.fd").read_bytes(), b"vars2")


    def test_tools_cd_needs_a_public_key_in_the_state(self):
        msi, nv = self.make_tools()
        with self.assertRaises(self.p.PropError):
            self.p.write_tools_dir(self.state, self.tmp / "cd3", msi, nv)

    def test_tools_cd_refuses_swapped_installer_contents(self):
        self.state["guest"]["ssh_public_key"] = self.PUB
        msi, nv = self.make_tools()
        msi.write_bytes(b"nvidia")
        with self.assertRaises(self.p.PropError):
            self.p.write_tools_dir(self.state, self.tmp / "cd4", msi, nv)

    def test_tools_cd_refuses_installer_name_with_suffix(self):
        self.state["guest"]["ssh_public_key"] = self.PUB
        msi, nv = self.make_tools()
        alt = self.tmp / "src" / (self.p.OPENSSH_MSI + ".bak")
        alt.write_bytes(b"msi")
        with self.assertRaises(self.p.PropError):
            self.p.write_tools_dir(self.state, self.tmp / "cd5", alt, nv)


    # --- the login lives in the state
    def test_init_state_writes_the_derived_password(self):
        st = self.p.init_state(SEED)
        self.assertEqual(st["guest"]["user"], "kfimage")
        self.assertEqual(st["guest"]["password"], self.p.derive_password(SEED))

    def test_validate_refuses_a_foreign_password(self):
        s = copy.deepcopy(self.state)
        s["guest"]["password"] = "Kf-0000000000000000"
        with self.assertRaises(self.p.PropError):
            self.p.validate_state(s)

    def test_validate_can_require_a_password(self):
        s = copy.deepcopy(self.state)
        s["guest"].pop("password", None)
        self.assertTrue(self.p.validate_state(s))
        with self.assertRaises(self.p.PropError):
            self.p.validate_state(s, require_password=True)

    def test_verify_cli_refuses_a_foreign_password(self):
        s = copy.deepcopy(self.state)
        s["guest"]["password"] = "Kf-ffffffffffffffff"
        sp = self.tmp / "pw.json"
        sp.write_text(json.dumps(s))
        self.assertEqual(self.p.main(["verify", str(sp), "--cpuinfo", str(self.cpu_file())]), 2)

    def test_pack_adds_the_password_to_an_older_state(self):
        s = copy.deepcopy(self.state)
        s["guest"].pop("password", None)
        sp = self.tmp / "old.json"
        sp.write_text(json.dumps(s))
        img = self.tmp / "old.qcow2"
        img.write_bytes(b"QFI")
        z = self.tmp / "old.zip"
        self.p.pack(sp, img, z)
        with zipfile.ZipFile(z) as zf:
            packed = json.loads(zf.read("state.json"))
        self.assertEqual(packed["guest"]["password"], self.p.derive_password(SEED))
        self.assertEqual(json.loads(sp.read_text())["guest"]["password"], self.p.derive_password(SEED))
        self.assertEqual(self.p.unpack(z, self.tmp / "oldx")["guest"]["password"], self.p.derive_password(SEED))

    def test_unpack_refuses_a_package_without_a_password(self):
        out, _ = self.make_pkg()
        bad = self.tmp / "nopw.zip"
        with zipfile.ZipFile(out) as z, zipfile.ZipFile(bad, "w", zipfile.ZIP_STORED) as w:
            members = {i.filename: z.read(i.filename) for i in z.infolist()}
            st = json.loads(members["state.json"])
            st["guest"].pop("password")
            members["state.json"] = json.dumps(st).encode()
            m = json.loads(members["manifest.json"])
            m["members"]["state.json"] = hashlib.sha256(members["state.json"]).hexdigest()
            members["manifest.json"] = json.dumps(m).encode()
            for k, v in members.items():
                w.writestr(k, v)
        with self.assertRaises(self.p.PropError):
            self.p.unpack(bad, self.tmp / "nopwx")

    def test_unpack_refuses_a_foreign_password(self):
        out, _ = self.make_pkg()
        bad = self.tmp / "fpw.zip"
        with zipfile.ZipFile(out) as z, zipfile.ZipFile(bad, "w", zipfile.ZIP_STORED) as w:
            members = {i.filename: z.read(i.filename) for i in z.infolist()}
            st = json.loads(members["state.json"])
            st["guest"]["password"] = "Kf-1111111111111111"
            members["state.json"] = json.dumps(st).encode()
            m = json.loads(members["manifest.json"])
            m["members"]["state.json"] = hashlib.sha256(members["state.json"]).hexdigest()
            members["manifest.json"] = json.dumps(m).encode()
            for k, v in members.items():
                w.writestr(k, v)
        with self.assertRaises(self.p.PropError):
            self.p.unpack(bad, self.tmp / "fpwx")


    def test_unpack_refuses_a_null_password(self):
        out, _ = self.make_pkg()
        bad = self.tmp / "nullpw.zip"
        with zipfile.ZipFile(out) as z, zipfile.ZipFile(bad, "w", zipfile.ZIP_STORED) as w:
            members = {i.filename: z.read(i.filename) for i in z.infolist()}
            st = json.loads(members["state.json"])
            st["guest"]["password"] = None
            members["state.json"] = json.dumps(st).encode()
            m = json.loads(members["manifest.json"])
            m["members"]["state.json"] = hashlib.sha256(members["state.json"]).hexdigest()
            members["manifest.json"] = json.dumps(m).encode()
            for k, v in members.items():
                w.writestr(k, v)
        with self.assertRaises(self.p.PropError):
            self.p.unpack(bad, self.tmp / "nullpwx")

    def test_cli_pack_writes_the_password_into_the_packed_state(self):
        import contextlib
        import io
        s = copy.deepcopy(self.state)
        s["guest"].pop("password", None)
        sp = self.tmp / "cp.json"
        sp.write_text(json.dumps(s))
        img = self.tmp / "cp.qcow2"
        img.write_bytes(b"QFI")
        with contextlib.redirect_stdout(io.StringIO()):
            self.assertEqual(self.p.main(["pack", str(sp), str(img), "--out", str(self.tmp / "cp.zip")]), 0)
        with zipfile.ZipFile(self.tmp / "cp.zip") as zf:
            self.assertIn("password", json.loads(zf.read("state.json"))["guest"])

    def test_cli_init_writes_the_password(self):
        out = self.tmp / "init.json"
        self.assertEqual(self.p.main(["init", SEED, "--out", str(out)]), 0)
        self.assertEqual(json.loads(out.read_text())["guest"]["password"], self.p.derive_password(SEED))


if __name__ == "__main__":
    unittest.main()
