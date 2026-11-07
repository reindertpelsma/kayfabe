#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later
"""Offline machine-state fixtures; no VM, GPU, TPM service or real key is used."""
import copy
import fcntl
import importlib.util
import json
import os
from pathlib import Path
import shlex
import shutil
import subprocess
import tempfile
import unittest

ROOT = Path(__file__).resolve().parents[2]
SOURCE = ROOT / "scripts/bench/windows/machine_state.py"
WIN = ROOT / "scripts/bench/windows/win_vm.sh"


class StateTests(unittest.TestCase):
    def setUp(self):
        spec = importlib.util.spec_from_file_location("fixture_machine_state", SOURCE)
        self.ms = importlib.util.module_from_spec(spec)
        spec.loader.exec_module(self.ms)
        self.tmp_obj = tempfile.TemporaryDirectory(prefix="y1-state-test-")
        self.addCleanup(self.tmp_obj.cleanup)
        self.tmp = Path(self.tmp_obj.name)
        self.vm = self.tmp / "capture/vm"
        for name in ("run", "disk", "tpm", "firmware", "secrets", "secureboot"):
            (self.vm / name).mkdir(parents=True)
        self.file_bytes = {
            "tpm/tpm2-00.permall": b"fixture TPM pristine NV counter=0",
            "tpm/tpm2-00.savestate": b"fixture saved TPM",
            "firmware/OVMF_VARS.fd": b"fixture OVMF pristine variables",
            "secrets/ssh_ed25519": b"fixture guest SSH private material, not a real key",
            "secrets/ssh_ed25519.pub": b"fixture guest SSH public material",
            "secrets/win_password": b"fixture throwaway password",
            "secureboot/db.key": b"fixture guest GOP signing material, not a real key",
            "secureboot/db.crt": b"fixture GOP certificate",
            "secureboot/db.guid": b"00000000-0000-0000-0000-000000000000",
        }
        for name, value in self.file_bytes.items():
            (self.vm / name).write_bytes(value)
        (self.vm / "image-inventory.json").write_text(json.dumps({"time_service_disabled": True}))
        self.image = self.tmp / "windows.qcow2"
        self.image.write_bytes(b"fixture image bytes")
        self.cpu = self.tmp / "cpuinfo"
        self.cpu.write_text("model name : Fixture host CPU\ncpu family : 6\nmodel : 42\nstepping : 7\nflags : sse avx vmx\n")
        self.facts = {
            "smbios": {"uuid": "00000000-0000-0000-0000-000000000000", "serial": "KF0102030405",
                       "manufacturer": "Fixture Manufacturer", "product": "Offline Windows Image"},
            "nics": [{"mac": "52:54:00:01:02:03"}],
            "disk": {"serial": "FIXTUREDISK01", "model": "virtio-blk-pci"},
            "machine": {"type": "pc-q35-10.2", "cores": 4, "ram_mb": 8192},
            "monitor": {"edid_id": "KFB-0001-00000001"},
        }
        self.state = self.ms.capture(self.vm, self.image, self.facts,
                                     "2026-11-07T10:00:00Z", "2026-11-07T11:00:00Z",
                                     "2026-11-07T10:30:00Z", self.cpu)
        self.canonical = self.tmp / "MACHINE_STATE.json"
        self.canonical.write_text(json.dumps(self.state))
        self.target = self.tmp / "runs/kfwin"

    def reject(self, state):
        with self.assertRaises(self.ms.StateError):
            self.ms.verify(state)

    def changed(self, section, field, value):
        data = copy.deepcopy(self.state)
        data[section][field] = value
        return data

    def dry(self, state=True, pin=None, arm="A"):
        if state:
            if pin is not None:
                self.state["cpu_pin"] = pin
                self.canonical.write_text(json.dumps(self.state))
            self.ms.apply(self.state, self.target, self.image)
        env = dict(PATH=os.environ["PATH"], HOME=str(self.tmp), WINVM_ROOT=str(self.tmp / "runs"),
                   WINVM_NAME="kfwin", WINVM_QEMU="/fixture/qemu", WINVM_DRY_RUN="1")
        if state:
            env["WINVM_MACHINE_STATE"] = str(self.canonical)
        r = subprocess.run(["bash", str(WIN), "run", "--dry-run", "--arm", arm, "--no-kf3", "--tag", "fixture"],
                           env=env, text=True, capture_output=True, timeout=30)
        self.assertEqual(r.returncode, 0, r.stderr)
        line = next(x for x in r.stdout.splitlines() if x.startswith("DRYRUN_QEMU "))
        return shlex.split(line)[1:]

    def cpu_arg(self, args):
        return args[args.index("-cpu") + 1]

    def test_cpu_pin_absent_by_default(self):
        self.assertNotIn("cpu_pin", self.state)
        self.assertEqual(self.state["cpu_observed"]["brand"], "Fixture host CPU")
        self.assertEqual(self.state["cpu_observed"]["flags"], ["avx", "sse", "vmx"])

    def test_capture_cpu_observation_never_becomes_override(self):
        text = self.ms.qemu_env(self.state)
        self.assertIn("MS_CPU_BRAND=\n", text)
        self.assertNotIn("Fixture host CPU", text)

    def test_default_apply_cpu_is_byte_identical(self):
        normal = self.cpu_arg(self.dry(state=False))
        applied = self.cpu_arg(self.dry())
        self.assertEqual(normal, applied)
        self.assertEqual(applied, "host,-vmx")

    def test_default_apply_hyperv_cpu_is_byte_identical(self):
        normal = self.cpu_arg(self.dry(state=False, arm="B"))
        applied = self.cpu_arg(self.dry(arm="B"))
        self.assertEqual(normal, applied)
        self.assertEqual(applied, "host,-vmx,hv-relaxed,hv-vapic,hv-spinlocks=0x1fff,hv-time,hv-vpindex,hv-synic,hv-stimer,hv-crash")

    def test_brand_only_override_preserves_cpu_flags(self):
        normal = self.cpu_arg(self.dry(state=False))
        applied = self.cpu_arg(self.dry(pin={"brand": "Fixture Brand Override"}))
        self.assertEqual(applied, normal + ",model-id=Fixture Brand Override")

    def test_brand_only_override_preserves_hyperv_flags(self):
        normal = self.cpu_arg(self.dry(state=False, arm="B"))
        applied = self.cpu_arg(self.dry(pin={"brand": "Fixture Brand Override"}, arm="B"))
        self.assertEqual(applied, normal + ",model-id=Fixture Brand Override")

    def test_cpu_pin_feature_array_is_rejected(self):
        data = copy.deepcopy(self.state)
        data["cpu_pin"] = {"brand": "Fixture", "features": ["-avx"]}
        self.reject(data)

    def test_cpu_pin_model_override_is_rejected(self):
        data = copy.deepcopy(self.state)
        data["cpu_pin"] = {"model": "qemu64", "flags": "-sse"}
        self.reject(data)

    def test_cpu_pin_flags_with_brand_are_rejected(self):
        data = copy.deepcopy(self.state)
        data["cpu_pin"] = {"brand": "Fixture", "flags": "-avx"}
        self.reject(data)

    def test_cpu_brand_feature_injection_is_rejected(self):
        data = copy.deepcopy(self.state)
        data["cpu_pin"] = {"brand": "Fixture,-avx"}
        self.reject(data)

    def test_smbios_property_injection_is_rejected(self):
        self.reject(self.changed("smbios", "product", "Fixture,uuid=other"))

    def test_long_cpu_brand_is_rejected(self):
        data = copy.deepcopy(self.state)
        data["cpu_pin"] = {"brand": "C" * 49}
        self.reject(data)

    def test_long_cpu_brand_with_spaces_is_rejected(self):
        data = copy.deepcopy(self.state)
        data["cpu_pin"] = {"brand": "CPU " * 13}
        self.reject(data)

    def test_long_disk_serial_is_rejected(self):
        self.reject(self.changed("disk", "serial", "S" * 21))

    def test_long_disk_serial_with_prefix_is_rejected(self):
        self.reject(self.changed("disk", "serial", "KAYFABE-" + "0" * 20))

    def test_duplicate_nic_is_rejected(self):
        data = copy.deepcopy(self.state)
        data["nics"] *= 2
        self.reject(data)

    def test_second_duplicate_nic_is_rejected(self):
        data = copy.deepcopy(self.state)
        data["nics"] += [{"mac": "52:54:00:01:02:04"}, dict(data["nics"][0])]
        self.reject(data)

    def test_multicast_mac_is_rejected(self):
        data = copy.deepcopy(self.state)
        data["nics"][0]["mac"] = "53:54:00:01:02:03"
        self.reject(data)

    def test_zero_mac_is_rejected(self):
        data = copy.deepcopy(self.state)
        data["nics"][0]["mac"] = "00:00:00:00:00:00"
        self.reject(data)

    def test_invalid_core_count_is_rejected(self):
        self.reject(self.changed("machine", "cores", 0))

    def test_invalid_ram_size_is_rejected(self):
        self.reject(self.changed("machine", "ram_mb", True))

    def test_machine_property_injection_is_rejected(self):
        self.reject(self.changed("machine", "type", "pc-q35-10.2,accel=tcg"))

    def test_unversioned_machine_is_rejected(self):
        self.reject(self.changed("machine", "type", "q35"))

    def test_uuid_noncanonical_is_rejected(self):
        self.reject(self.changed("smbios", "uuid", "00000000000000000000000000000000"))

    def test_uuid_malformed_is_rejected(self):
        self.reject(self.changed("smbios", "uuid", "invalid"))

    def test_unknown_root_field_is_rejected(self):
        data = copy.deepcopy(self.state)
        data["future_field"] = 1
        self.reject(data)

    def test_unknown_smbios_field_is_rejected(self):
        self.reject(self.changed("smbios", "host_serial", "unrequested"))

    def test_unknown_file_record_field_is_rejected(self):
        data = copy.deepcopy(self.state)
        data["files"]["tpm/tpm2-00.permall"]["extra"] = 1
        self.reject(data)

    def test_unknown_cpu_observation_field_is_rejected(self):
        self.reject(self.changed("cpu_observed", "pin", True))

    def test_unknown_schema_version_is_rejected(self):
        data = copy.deepcopy(self.state)
        data["schema_version"] = 2
        self.reject(data)

    def test_boolean_schema_version_is_rejected(self):
        data = copy.deepcopy(self.state)
        data["schema_version"] = True
        self.reject(data)

    def test_tpm_hash_mismatch_is_rejected(self):
        data = copy.deepcopy(self.state)
        data["files"]["tpm/tpm2-00.permall"]["sha256"] = "0" * 64
        self.reject(data)

    def test_ovmf_hash_mismatch_is_rejected(self):
        data = copy.deepcopy(self.state)
        data["files"]["firmware/OVMF_VARS.fd"]["sha256"] = "0" * 64
        self.reject(data)

    def test_blob_length_mismatch_is_rejected(self):
        data = copy.deepcopy(self.state)
        data["files"]["secureboot/db.crt"]["bytes"] += 1
        self.reject(data)

    def test_tpm_blob_length_mismatch_is_rejected(self):
        data = copy.deepcopy(self.state)
        data["files"]["tpm/tpm2-00.permall"]["bytes"] += 1
        self.reject(data)

    def test_bad_base64_is_rejected(self):
        data = copy.deepcopy(self.state)
        data["files"]["tpm/tpm2-00.permall"]["data_base64"] += "!"
        self.reject(data)

    def test_whitespace_base64_is_rejected(self):
        data = copy.deepcopy(self.state)
        data["files"]["firmware/OVMF_VARS.fd"]["data_base64"] += "\n"
        self.reject(data)

    def test_blob_path_traversal_is_rejected(self):
        data = copy.deepcopy(self.state)
        data["files"]["../escape"] = data["files"]["tpm/tpm2-00.permall"]
        self.reject(data)

    def test_unrequested_credential_file_is_rejected(self):
        data = copy.deepcopy(self.state)
        data["files"]["secrets/personal_account"] = data["files"]["secureboot/db.crt"]
        self.reject(data)

    def test_missing_tpm_is_rejected(self):
        data = copy.deepcopy(self.state)
        del data["files"]["tpm/tpm2-00.permall"]
        self.reject(data)

    def test_missing_signing_key_is_rejected(self):
        data = copy.deepcopy(self.state)
        del data["files"]["secureboot/db.key"]
        self.reject(data)

    def test_rtc_before_capture_window_is_rejected(self):
        data = copy.deepcopy(self.state)
        data["rtc_base_utc"] = "2026-11-07T09:59:59Z"
        self.reject(data)

    def test_rtc_after_capture_window_is_rejected(self):
        data = copy.deepcopy(self.state)
        data["rtc_base_utc"] = "2026-11-07T11:00:01Z"
        self.reject(data)

    def test_time_service_must_be_off(self):
        data = copy.deepcopy(self.state)
        data["time_service_disabled"] = False
        self.reject(data)

    def test_capture_refuses_enabled_time_service(self):
        (self.vm / "image-inventory.json").write_text('{"time_service_disabled":false}')
        with self.assertRaises(self.ms.StateError):
            self.ms.capture(self.vm, self.image, self.facts, "2026-11-07T10:00:00Z", "2026-11-07T11:00:00Z",
                            "2026-11-07T10:30:00Z", self.cpu)

    def test_duplicate_json_keys_are_rejected(self):
        self.canonical.write_text(json.dumps(self.state).replace('"schema_version": 1', '"schema_version": 1,"schema_version": 1'))
        with self.assertRaises(self.ms.StateError):
            self.ms.load_manifest(self.canonical)

    def test_nested_duplicate_json_keys_are_rejected(self):
        self.canonical.write_text(json.dumps(self.state).replace('"product": "Offline Windows Image"',
                                  '"product": "Offline Windows Image","product": "Offline Windows Image"'))
        with self.assertRaises(self.ms.StateError):
            self.ms.load_manifest(self.canonical)

    def test_apply_validates_image_hash(self):
        self.image.write_bytes(b"changed image")
        with self.assertRaises(self.ms.StateError):
            self.ms.apply(self.state, self.target, self.image)
        self.assertFalse(self.target.exists())

    def test_verify_checks_image_bytes_when_supplied(self):
        self.image.write_bytes(b"wrong")
        with self.assertRaises(self.ms.StateError):
            self.ms.verify(self.state, self.image)

    def test_apply_creates_only_private_working_copies(self):
        self.ms.apply(self.state, self.target, self.image)
        for name, value in self.file_bytes.items():
            self.assertEqual((self.target / name).read_bytes(), value)
            self.assertEqual((self.target / name).stat().st_mode & 0o777, 0o600)
        self.assertEqual((self.target / "disk/base.qcow2").read_bytes(), self.image.read_bytes())
        self.assertTrue((self.target / "installed").exists())

    def test_refresh_restores_pristine_tpm(self):
        self.ms.apply(self.state, self.target, self.image)
        before = self.canonical.read_bytes()
        (self.target / "tpm/tpm2-00.permall").write_bytes(b"NV counter advanced")
        self.ms.apply(self.state, self.target, self.target / "disk/base.qcow2", refresh=True)
        self.assertEqual((self.target / "tpm/tpm2-00.permall").read_bytes(), self.file_bytes["tpm/tpm2-00.permall"])
        self.assertEqual(self.canonical.read_bytes(), before)
        self.assertEqual((self.vm / "tpm/tpm2-00.permall").read_bytes(), self.file_bytes["tpm/tpm2-00.permall"])

    def test_refresh_restores_pristine_ovmf(self):
        self.ms.apply(self.state, self.target, self.image)
        (self.target / "firmware/OVMF_VARS.fd").write_bytes(b"mutated Secure Boot NV")
        self.ms.apply(self.state, self.target, self.target / "disk/base.qcow2", refresh=True)
        self.assertEqual((self.target / "firmware/OVMF_VARS.fd").read_bytes(), self.file_bytes["firmware/OVMF_VARS.fd"])

    def test_refresh_removes_absent_tpm_volatile_file(self):
        self.ms.apply(self.state, self.target, self.image)
        extra = self.target / "tpm/tpm2-00.volatilestate"
        extra.write_bytes(b"new NV state")
        self.ms.apply(self.state, self.target, self.image, refresh=True)
        self.assertFalse(extra.exists())

    def test_refresh_removes_absent_password_file(self):
        del self.state["files"]["secrets/win_password"]
        self.ms.apply(self.state, self.target, self.image)
        extra = self.target / "secrets/win_password"
        extra.write_bytes(b"per-run change")
        self.ms.apply(self.state, self.target, self.image, refresh=True)
        self.assertFalse(extra.exists())

    def test_refresh_removes_unlisted_tpm_backup(self):
        self.ms.apply(self.state, self.target, self.image)
        extra = self.target / "tpm/tpm2-00.permall.bak"
        extra.write_bytes(b"drifted backup")
        self.ms.apply(self.state, self.target, self.image, refresh=True)
        self.assertFalse(extra.exists())

    def test_refresh_removes_tpm_lock_file(self):
        self.ms.apply(self.state, self.target, self.image)
        extra = self.target / "tpm/.lock"
        extra.write_bytes(b"stale lock")
        self.ms.apply(self.state, self.target, self.image, refresh=True)
        self.assertFalse(extra.exists())

    def test_refresh_rejects_wrong_lock_fd(self):
        self.ms.apply(self.state, self.target, self.image)
        with self.canonical.open() as wrong:
            with self.assertRaises(self.ms.StateError):
                self.ms.apply(self.state, self.target, self.image, refresh=True, lock_fd=wrong.fileno())

    def test_refresh_rejects_other_vm_lock_fd(self):
        self.ms.apply(self.state, self.target, self.image)
        with (self.vm / "run/vm.lock").open("a+") as wrong:
            with self.assertRaises(self.ms.StateError):
                self.ms.apply(self.state, self.target, self.image, refresh=True, lock_fd=wrong.fileno())

    def test_refresh_rejects_busy_vm(self):
        self.ms.apply(self.state, self.target, self.image)
        with (self.target / "run/vm.lock").open("w") as lock:
            fcntl.flock(lock, fcntl.LOCK_EX)
            with self.assertRaises(self.ms.StateError):
                self.ms.apply(self.state, self.target, self.image, refresh=True)

    def test_capture_rejects_busy_vm(self):
        with (self.vm / "run/vm.lock").open("w") as lock:
            fcntl.flock(lock, fcntl.LOCK_EX)
            with self.assertRaises(self.ms.StateError):
                self.ms.capture(self.vm, self.image, self.facts, "2026-11-07T10:00:00Z", "2026-11-07T11:00:00Z",
                                "2026-11-07T10:30:00Z", self.cpu)

    def test_initial_apply_never_overwrites_existing_vm(self):
        self.target.mkdir(parents=True)
        (self.target / "owner-file").write_text("keep")
        with self.assertRaises(self.ms.StateError):
            self.ms.apply(self.state, self.target, self.image)
        self.assertEqual((self.target / "owner-file").read_text(), "keep")

    def test_refresh_rejects_unmanaged_vm(self):
        self.target.mkdir(parents=True)
        with self.assertRaises(self.ms.StateError):
            self.ms.apply(self.state, self.target, self.image, refresh=True)

    def test_refresh_rejects_different_manifest(self):
        self.ms.apply(self.state, self.target, self.image)
        changed = self.changed("smbios", "serial", "OTHER")
        with self.assertRaises(self.ms.StateError):
            self.ms.apply(changed, self.target, self.image, refresh=True)

    def test_refresh_rejects_changed_state_marker(self):
        self.ms.apply(self.state, self.target, self.image)
        (self.target / "machine-state.sha256").write_text("0" * 64)
        with self.assertRaises(self.ms.StateError):
            self.ms.apply(self.state, self.target, self.image, refresh=True)

    def test_initial_apply_refuses_existing_empty_directory(self):
        self.target.mkdir(parents=True)
        with self.assertRaises(self.ms.StateError):
            self.ms.apply(self.state, self.target, self.image)

    def test_invalid_monitor_id_is_rejected(self):
        self.reject(self.changed("monitor", "edid_id", "unimplemented-monitor"))

    def test_different_monitor_serial_is_rejected(self):
        self.reject(self.changed("monitor", "edid_id", "KFB-0001-00000002"))

    def test_invalid_disk_model_is_rejected(self):
        self.reject(self.changed("disk", "model", "unimplemented-device"))

    def test_unimplemented_scsi_disk_is_rejected(self):
        self.reject(self.changed("disk", "model", "scsi-hd"))

    def test_capture_refuses_live_qemu_pid(self):
        (self.vm / "run/qemu.pid").write_text(str(os.getpid()))
        with self.assertRaises(self.ms.StateError):
            self.ms.capture(self.vm, self.image, self.facts, "2026-11-07T10:00:00Z", "2026-11-07T11:00:00Z",
                            "2026-11-07T10:30:00Z", self.cpu)

    def test_capture_refuses_live_tpm_pid(self):
        (self.vm / "run/swtpm.pid").write_text(str(os.getpid()))
        with self.assertRaises(self.ms.StateError):
            self.ms.capture(self.vm, self.image, self.facts, "2026-11-07T10:00:00Z", "2026-11-07T11:00:00Z",
                            "2026-11-07T10:30:00Z", self.cpu)

    def test_capture_refuses_tpm_symlink(self):
        p = self.vm / "tpm/tpm2-00.permall"
        p.unlink()
        p.symlink_to(self.image)
        with self.assertRaises(self.ms.StateError):
            self.ms.capture(self.vm, self.image, self.facts, "2026-11-07T10:00:00Z", "2026-11-07T11:00:00Z",
                            "2026-11-07T10:30:00Z", self.cpu)

    def test_refresh_refuses_directory_symlink(self):
        self.ms.apply(self.state, self.target, self.image)
        shutil.rmtree(self.target / "tpm")
        (self.target / "tpm").symlink_to(self.vm / "tpm")
        with self.assertRaises(self.ms.StateError):
            self.ms.apply(self.state, self.target, self.image, refresh=True)

    def test_refresh_refuses_ovmf_symlink(self):
        self.ms.apply(self.state, self.target, self.image)
        p = self.target / "firmware/OVMF_VARS.fd"
        p.unlink()
        p.symlink_to(self.vm / "firmware/OVMF_VARS.fd")
        with self.assertRaises(self.ms.StateError):
            self.ms.apply(self.state, self.target, self.image, refresh=True)

    def test_multiple_nics_are_applied_in_order(self):
        self.state["nics"].append({"mac": "52:54:00:01:02:04"})
        self.canonical.write_text(json.dumps(self.state))
        args = self.dry()
        self.assertIn("virtio-net-pci,netdev=n1,mac=52:54:00:01:02:04", args)
        self.assertIn("user,id=n1,restrict=on", args)

    def test_pin_memory_cores_and_disk_device(self):
        args = self.dry()
        self.assertEqual(args[args.index("-m") + 1], "8192")
        self.assertEqual(args[args.index("-smp") + 1], "4,sockets=1,cores=4,threads=1")
        self.assertIn("pc-q35-10.2,accel=kvm,smm=on,memory-backend=ram0", args)
        self.assertIn("virtio-blk-pci,drive=sys,bootindex=0,serial=FIXTUREDISK01", args)

    def launch_without_qemu(self, overlay="new-run", restricted="on", opt_in=True):
        # The nonexistent binary makes qemu_bin refuse BEFORE prereqs/hardware checks.
        env = dict(PATH=os.environ["PATH"], HOME=str(self.tmp), WINVM_ROOT=str(self.tmp / "runs"),
                   WINVM_NAME="kfwin", WINVM_QEMU="/fixture/does-not-exist", WINVM_NET_RESTRICT=restricted)
        if opt_in:
            env["WINVM_MACHINE_STATE"] = str(self.canonical)
        result = subprocess.run(["bash", str(WIN), "run", "--no-kf3", "--overlay", overlay],
                                env=env, text=True, capture_output=True, timeout=30)
        self.assertNotEqual(result.returncode, 0)
        self.assertNotIn("DRYRUN_QEMU", result.stdout)
        return result

    def test_run_hook_refreshes_tpm_before_qemu_lookup(self):
        self.ms.apply(self.state, self.target, self.image)
        (self.target / "tpm/tpm2-00.permall").write_bytes(b"drift")
        result = self.launch_without_qemu()
        self.assertIn("no kf3 QEMU", result.stderr)
        self.assertEqual((self.target / "tpm/tpm2-00.permall").read_bytes(), self.file_bytes["tpm/tpm2-00.permall"])

    def test_run_hook_refreshes_ovmf_before_qemu_lookup(self):
        self.ms.apply(self.state, self.target, self.image)
        (self.target / "firmware/OVMF_VARS.fd").write_bytes(b"drift")
        result = self.launch_without_qemu()
        self.assertIn("no kf3 QEMU", result.stderr)
        self.assertEqual((self.target / "firmware/OVMF_VARS.fd").read_bytes(), self.file_bytes["firmware/OVMF_VARS.fd"])

    def test_run_refuses_reused_overlay(self):
        self.ms.apply(self.state, self.target, self.image)
        (self.target / "disk/new-run.qcow2").write_bytes(b"previous guest disk changes")
        self.assertIn("fresh overlay", self.launch_without_qemu().stderr)

    def test_run_refuses_reused_main_overlay(self):
        self.ms.apply(self.state, self.target, self.image)
        (self.target / "disk/main.qcow2").write_bytes(b"previous guest disk changes")
        self.assertIn("fresh overlay", self.launch_without_qemu(overlay="main").stderr)

    def test_applied_vm_refuses_missing_opt_in(self):
        self.ms.apply(self.state, self.target, self.image)
        self.assertIn("requires WINVM_MACHINE_STATE", self.launch_without_qemu(opt_in=False).stderr)

    def test_applied_vm_never_falls_back_to_legacy_identity(self):
        self.ms.apply(self.state, self.target, self.image)
        (self.target / "identity.env").write_text("VM_UUID=invalid")
        self.assertIn("requires WINVM_MACHINE_STATE", self.launch_without_qemu(opt_in=False).stderr)

    def test_offline_run_refuses_internet_opt_in(self):
        self.ms.apply(self.state, self.target, self.image)
        self.assertIn("restricted networking", self.launch_without_qemu(restricted="off").stderr)

    def test_offline_dry_run_refuses_internet_opt_in(self):
        self.ms.apply(self.state, self.target, self.image)
        env = dict(PATH=os.environ["PATH"], HOME=str(self.tmp), WINVM_ROOT=str(self.tmp / "runs"), WINVM_NAME="kfwin",
                   WINVM_MACHINE_STATE=str(self.canonical), WINVM_NET_RESTRICT="off", WINVM_DRY_RUN="1")
        r = subprocess.run(["bash", str(WIN), "run", "--dry-run", "--no-kf3"], env=env, text=True, capture_output=True)
        self.assertNotEqual(r.returncode, 0)
        self.assertIn("restricted networking", r.stderr)

    def test_qemu_args_pin_machine_rtc_and_identity(self):
        args = self.dry()
        self.assertEqual(args[args.index("-rtc") + 1], "base=2026-11-07T10:30:00,driftfix=slew")
        self.assertEqual(args[args.index("-uuid") + 1], self.facts["smbios"]["uuid"])
        smbios = args[args.index("-smbios") + 1]
        self.assertIn("manufacturer=Fixture Manufacturer", smbios)
        self.assertIn("product=Offline Windows Image", smbios)
        self.assertTrue(any("serial=FIXTUREDISK01" in x for x in args))

    def test_apply_identity_json_overrides_legacy_env(self):
        self.ms.apply(self.state, self.target, self.image)
        (self.target / "identity.env").write_text("VM_UUID=bad\nVM_SERIAL=bad\nVM_MAC=bad\n")
        env = dict(PATH=os.environ["PATH"], HOME=str(self.tmp), WINVM_ROOT=str(self.tmp / "runs"), WINVM_NAME="kfwin",
                   WINVM_QEMU="/fixture/qemu", WINVM_MACHINE_STATE=str(self.canonical), WINVM_DRY_RUN="1")
        r = subprocess.run(["bash", str(WIN), "run", "--dry-run", "--no-kf3"], env=env, capture_output=True, text=True, timeout=30)
        self.assertEqual(r.returncode, 0, r.stderr)
        self.assertIn(self.facts["smbios"]["uuid"], r.stdout)


if __name__ == "__main__":
    unittest.main()
