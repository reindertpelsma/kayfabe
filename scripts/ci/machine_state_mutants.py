#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later
"""Offline state rule mutations, each requiring two independently named killers."""
import argparse
import ast
import json
from pathlib import Path
import re
import shutil
import subprocess
import sys
import tempfile

ROOT = Path(__file__).resolve().parents[2]
WIN = ROOT / "scripts/bench/windows"
PY = (WIN / "machine_state.py").read_text()
SH = (WIN / "win_vm.sh").read_text()


def change(source, old, new, count=1):
    assert source.count(old) == count, (old, source.count(old), count)
    return source.replace(old, new)


def case(name, old, new, *tests, shell=False, count=1):
    return name, "win_vm.sh" if shell else "machine_state.py", change(SH if shell else PY, old, new, count), tests


CASES = [
    case("no-state-symlinks", 'not path.is_symlink()', 'True',
         "test_refresh_refuses_directory_symlink", "test_refresh_refuses_ovmf_symlink", count=2),
    case("virtio-serial-length", 'len(state["disk"]["serial"]) <= 20', 'True',
         "test_long_disk_serial_is_rejected", "test_long_disk_serial_with_prefix_is_rejected"),
    case("cpuid-brand-length", 'len(state["cpu_pin"]["brand"]) <= 48', 'True',
         "test_long_cpu_brand_is_rejected", "test_long_cpu_brand_with_spaces_is_rejected"),
    case("unique-nic-macs", 'len(set(macs)) == len(macs)', 'True',
         "test_duplicate_nic_is_rejected", "test_second_duplicate_nic_is_rejected"),
    case("unicast-nic-macs", 'int(mac[:2], 16) & 1 == 0 and mac != "00:00:00:00:00:00"', 'True',
         "test_multicast_mac_is_rejected", "test_zero_mac_is_rejected"),
    case("bounded-integer-fields", 'type(value) is int and low <= value <= high', 'True',
         "test_invalid_core_count_is_rejected", "test_invalid_ram_size_is_rejected"),
    case("versioned-machine-type", 're.fullmatch(r"pc-q35-\\d+\\.\\d+", state["machine"]["type"])', 'True',
         "test_machine_property_injection_is_rejected", "test_unversioned_machine_is_rejected"),
    case("smbios-uuid", 'str(uuid.UUID(sm["uuid"])) == sm["uuid"]', 'True',
         "test_uuid_noncanonical_is_rejected", "test_uuid_malformed_is_rejected"),
    case("strict-base64", 'validate=True', 'validate=False',
         "test_bad_base64_is_rejected", "test_whitespace_base64_is_rejected"),
    case("remove-drifted-optional-files", '        elif path.exists():\n            path.unlink()', '        elif path.exists():\n            pass',
         "test_refresh_removes_absent_tpm_volatile_file", "test_refresh_removes_absent_password_file"),
    case("remove-unlisted-tpm-files", 'if "tpm/" + path.name not in state["files"]:', 'if False:',
         "test_refresh_removes_unlisted_tpm_backup", "test_refresh_removes_tpm_lock_file"),
    case("inherited-lock-binding", 'os.fstat(fd).st_ino == lock.stat().st_ino and\n                    os.fstat(fd).st_dev == lock.stat().st_dev', 'True',
         "test_refresh_rejects_wrong_lock_fd", "test_refresh_rejects_other_vm_lock_fd"),
    case("strict-object-fields", 'obj.keys() <= set(required) | set(optional)', 'True',
         "test_unknown_root_field_is_rejected", "test_unknown_smbios_field_is_rejected"),
    case("schema-version", 'type(state["schema_version"]) is int and state["schema_version"] == 1', 'True',
         "test_unknown_schema_version_is_rejected", "test_boolean_schema_version_is_rejected"),
    case("blob-sha256", 'digest(data) == record["sha256"]', 'True',
         "test_tpm_hash_mismatch_is_rejected", "test_ovmf_hash_mismatch_is_rejected"),
    case("blob-byte-count", 'len(data) == record["bytes"]', 'True',
         "test_blob_length_mismatch_is_rejected", "test_tpm_blob_length_mismatch_is_rejected"),
    case("file-allowlist", 'state["files"].keys() <= ALLOWED', 'True',
         "test_blob_path_traversal_is_rejected", "test_unrequested_credential_file_is_rejected"),
    case("required-state-files", 'REQUIRED <= state["files"].keys()', 'True',
         "test_missing_tpm_is_rejected", "test_missing_signing_key_is_rejected"),
    case("fixed-rtc-window", 'start <= utc(state["rtc_base_utc"]) <= end', 'True',
         "test_rtc_before_capture_window_is_rejected", "test_rtc_after_capture_window_is_rejected"),
    case("time-service-off", 'is True', 'is not None',
         "test_time_service_must_be_off", "test_capture_refuses_enabled_time_service", count=2),
    case("duplicate-json-keys", 'key not in out', 'True',
         "test_duplicate_json_keys_are_rejected", "test_nested_duplicate_json_keys_are_rejected"),
    case("image-sha256", 'image_hash(image) == state["image_sha256"]', 'True',
         "test_apply_validates_image_hash", "test_verify_checks_image_bytes_when_supplied"),
    case("refresh-pristine-state", '            restore_files(state, target)', '            pass',
         "test_refresh_restores_pristine_tpm", "test_refresh_restores_pristine_ovmf"),
    case("vm-exclusive-lock", 'fcntl.flock(fd, fcntl.LOCK_EX | fcntl.LOCK_NB)', 'pass',
         "test_refresh_rejects_busy_vm", "test_capture_rejects_busy_vm"),
    case("fresh-apply-directory", 'not target.exists() and not target.is_symlink()', 'True',
         "test_initial_apply_never_overwrites_existing_vm", "test_initial_apply_refuses_existing_empty_directory"),
    case("managed-state-binding", 'saved.is_file() and saved.read_text().strip() == marker', 'True',
         "test_refresh_rejects_changed_state_marker", "test_refresh_rejects_different_manifest"),
    case("supported-monitor", 'state["monitor"]["edid_id"] == "KFB-0001-00000001"', 'True',
         "test_invalid_monitor_id_is_rejected", "test_different_monitor_serial_is_rejected"),
    case("supported-disk-model", 'state["disk"]["model"] == "virtio-blk-pci"', 'True',
         "test_invalid_disk_model_is_rejected", "test_unimplemented_scsi_disk_is_rejected"),
    case("no-live-state-capture", 'not (pid.isdigit() and Path("/proc", pid).exists())', 'True',
         "test_capture_refuses_live_qemu_pid", "test_capture_refuses_live_tpm_pid"),
    case("qemu-property-injection", 're.fullmatch(pattern, value) is not None', 'True',
         "test_cpu_brand_feature_injection_is_rejected", "test_smbios_property_injection_is_rejected"),
    case("cpu-pin-brand-only", 'fields(state["cpu_pin"], {"brand"})',
         'fields(state["cpu_pin"], {"brand"}, {"features", "flags"})',
         "test_cpu_pin_feature_array_is_rejected", "test_cpu_pin_flags_with_brand_are_rejected"),
    case("cpu-pin-absent-default", 'cpu_observed=cpu, files={}', 'cpu_observed=cpu, cpu_pin={"brand": cpu["brand"]}, files={}',
         "test_cpu_pin_absent_by_default", "test_capture_cpu_observation_never_becomes_override"),
    case("cpu-observation-diagnostic-only", 'state.get("cpu_pin", {}).get("brand", "")', 'state["cpu_observed"]["brand"]',
         "test_capture_cpu_observation_never_becomes_override", "test_default_apply_cpu_is_byte_identical"),
    case("cpu-brand-append-only", 'flags+=",model-id=$MS_CPU_BRAND"', 'flags="$MS_CPU_BRAND"',
         "test_brand_only_override_preserves_cpu_flags", "test_brand_only_override_preserves_hyperv_flags", shell=True),
    case("cpu-no-new-mask", 'MS_CPU_BRAND=$v ;;', 'MS_CPU_BRAND=$v; [ -n "$MS_CPU_BRAND" ] || MS_CPU_BRAND=Fixture,-avx ;;',
         "test_default_apply_cpu_is_byte_identical", "test_default_apply_hyperv_cpu_is_byte_identical", shell=True),
    case("run-fresh-overlay", '[ ! -e "$VM/disk/$overlay.qcow2" ]', 'true',
         "test_run_refuses_reused_overlay", "test_run_refuses_reused_main_overlay", shell=True),
    case("run-explicit-opt-in", '[ ! -e "$VM/machine-state.sha256" ]', 'true',
         "test_applied_vm_refuses_missing_opt_in", "test_applied_vm_never_falls_back_to_legacy_identity", shell=True),
    case("run-offline-network", '[ "${WINVM_NET_RESTRICT-on}" = on ]', 'true',
         "test_offline_run_refuses_internet_opt_in", "test_offline_dry_run_refuses_internet_opt_in", shell=True),
]
# Remove the whole restore invocation; the fixture binary lookup still refuses
# before any prereq/hardware check, so these are runtime tests without a VM.
CASES.append(("run-restore-before-qemu", "win_vm.sh", change(SH,
    '      python3 "$HERE/machine_state.py" apply "$WINVM_MACHINE_STATE" --vm "$VM" \\\n        --image "$VM/disk/base.qcow2" --refresh --lock-fd 7 || die "machine-state restore refused"',
    '      true'), ("test_run_hook_refreshes_tpm_before_qemu_lookup", "test_run_hook_refreshes_ovmf_before_qemu_lookup")))


CASES += [
    case("snapshot-pristine-base", '[ -z "${WINVM_OVERLAY_BACKING:-}" ]', 'true',
         "test_snapshot_refuses_alternate_disk_backing", "test_snapshot_dry_run_refuses_alternate_disk_backing", shell=True),
    case("identity-from-json", 'if [ -n "${WINVM_MACHINE_STATE:-}" ]; then\n    VM_UUID=$MS_UUID VM_SERIAL=$MS_SERIAL VM_MAC=$MS_MAC',
         'if false; then\n    VM_UUID=$MS_UUID VM_SERIAL=$MS_SERIAL VM_MAC=$MS_MAC',
         "test_qemu_args_pin_machine_rtc_and_identity", "test_apply_identity_json_overrides_legacy_env", shell=True),
    case("snapshot-rtc-argument", '"base=${MS_RTC_BASE:-utc},driftfix=slew"', 'base=utc,driftfix=slew',
         "test_qemu_args_pin_machine_rtc_and_identity", "test_snapshot_all_scalars_match_manifest", shell=True),
    case("working-signing-directory", '  SB=$VM/secureboot', '  :',
         "test_signed_gop_uses_restored_guest_signing_directory", "test_signed_gop_never_uses_capture_host_signing_cache", shell=True),
    case("snapshot-disk-serial-argument", '[ -z "${WINVM_MACHINE_STATE:-}" ] || sysdev+=",serial=$MS_DISK_SERIAL"', 'true',
         "test_qemu_args_pin_machine_rtc_and_identity", "test_pin_memory_cores_and_disk_device", shell=True),
    case("snapshot-machine-shape", 'MS_MACHINE_TYPE) MS_MACHINE_TYPE=$v ;; MS_CORES) SMP=$v ;; MS_RAM_MB) RAM_MB=$v ;;',
         'MS_MACHINE_TYPE|MS_CORES|MS_RAM_MB) : ;;',
         "test_pin_memory_cores_and_disk_device", "test_snapshot_all_scalars_match_manifest", shell=True),
    case("no-blob-output", 'print("machine-state: " + args.verb + " complete")',
         'print(json.dumps(state)); print("machine-state: " + args.verb + " complete")',
         "test_cli_capture_verify_round_trip_does_not_print_blobs", "test_cli_apply_round_trip_does_not_print_blobs"),
    case("multiple-nic-identities", 'MS_NIC) MS_NICS+=("$v") ;;', 'MS_NIC) : ;;',
         "test_multiple_nics_are_applied_in_order", "test_snapshot_all_scalars_match_manifest", shell=True),
]


CASES += [case("literal-smbios-commas", 'sm[key].replace(",", ",,")', 'sm[key]',
               "test_smbios_product_literal_comma_is_escaped", "test_smbios_manufacturer_literal_comma_is_escaped")]


# Optional TPM cleanup is also covered by the complete-directory cleanup. Remove
# both arms of that one rule so a redundant guard cannot hide a missing killer.
for i, entry in enumerate(CASES):
    if entry[0] == "remove-drifted-optional-files":
        CASES[i] = (*entry[:2], change(entry[2],
                    'if "tpm/" + path.name not in state["files"]:', 'if False:'), entry[3])


def run(tree, tests=()):
    # Fresh imported source for every mutant, including same-size edits within one second.
    proc = subprocess.run([sys.executable, "-I", "-B", str(tree / "scripts/ci/test_machine_state.py"),
                           *("StateTests." + test for test in tests)], text=True, capture_output=True, timeout=300)
    output = proc.stdout + proc.stderr
    return proc.returncode, sorted(set(re.findall(r"^(?:FAIL|ERROR): (\S+)", output, re.M))), output


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--json", type=Path)
    args = parser.parse_args()
    source = ROOT / "scripts/ci/test_machine_state.py"
    known = {node.name for node in ast.walk(ast.parse(source.read_text())) if isinstance(node, ast.FunctionDef)}
    assert not {test for case_ in CASES for test in case_[3]} - known, "unknown test selector"
    table = []
    with tempfile.TemporaryDirectory(prefix="y1-state-mutants-") as tmp:
        tree = Path(tmp)
        win = tree / "scripts/bench/windows"
        ci = tree / "scripts/ci"
        win.mkdir(parents=True)
        ci.mkdir(parents=True)
        for name in ("win_vm.sh", "machine_state.py"):
            shutil.copy(WIN / name, win / name)
        shutil.copy(source, ci / source.name)
        code, _, output = run(tree)
        if code:
            print("FAIL unmutated baseline\n" + output[-8000:])
            return 2
        print("PASS baseline: " + re.search(r"Ran (\d+) tests", output).group(1), flush=True)
        for name, filename, mutated, tests in CASES:
            path = win / filename
            original = path.read_text()
            try:
                path.write_text(mutated)
                code, killers, output = run(tree, tests)
            finally:
                path.write_text(original)
            ok = code != 0 and len(killers) >= 2
            table.append(dict(mutant=name, killed=ok, killers=killers, coverage="offline fixture execution"))
            print(f"{'PASS' if ok else 'WEAK'} {name}: {len(killers)} killers", flush=True)
            if not ok:
                print(output[-1500:])
    if args.json:
        args.json.write_text(json.dumps(table, indent=2) + "\n")
    count = sum(row["killed"] for row in table)
    print(f"{count}/{len(table)} mutants killed by at least two tests")
    return 0 if count == len(table) else 1


if __name__ == "__main__":
    sys.exit(main())
