#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later
"""propimage.py rule mutations, each requiring two independently named killers."""
import argparse
import ast
import re
import shutil
import subprocess
import sys
import tempfile
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
SRC = (ROOT / "scripts/bench/windows/propimage.py").read_text()


def case(name, old, new, *tests, count=1):
    assert SRC.count(old) == count, (name, old, SRC.count(old))
    return name, SRC.replace(old, new), tests


CASES = [
    case("seed-in-hash", 'f"{DOMAIN}\\0{seed}\\0{field}"', 'f"{DOMAIN}\\0{field}"',
         "test_different_seed_different_ids", "test_cli_ids_and_verify_exit_codes"),
    case("field-in-hash", '\\0{field}"', '"',
         "test_fields_use_independent_streams", "test_nics_and_disks_are_distinct"),
    case("uuid-version", "b[6] = (b[6] & 0x0F) | 0x50", "b[6] = b[6]",
         "test_formats_valid", "test_validate_ok"),
    case("mac-unicast-prefix", '"52:54:00:" +', '"53:54:00:" +',
         "test_formats_valid", "test_validate_ok"),
    case("disk-serial-length", '.hex()[:12].upper()', '.hex()[:30].upper()',
         "test_formats_valid", "test_validate_ok"),
    case("subnet-not-default", "10.{16 + net_b[0] % 16}", "10.0",
         "test_net_is_fixed_and_not_default", "test_validate_ok"),
    case("clock-gap-positive", "600 + int.from_bytes", "0 + int.from_bytes",
         "test_many_seeds_clock_gap_in_range", "test_golden_ids_for_proposed_seed"),
    case("seed-validation", 'require(isinstance(seed, str) and 4 <= len(seed) <= 128 and seed.isprintable(),', 'require(True,',
         "test_bad_seed_is_rejected", "test_pack_refuses_state_not_derived_from_seed"),
    case("seed-binding", 'return want == state["ids"]', 'return True',
         "test_edited_identity_is_rejected", "test_pack_refuses_state_not_derived_from_seed"),
    case("blob-hash-check", 'hashlib.sha256(data).hexdigest() == rec["sha256"]', 'True',
         "test_validate_rejects_bad_blob_hash", "test_pack_refuses_state_not_derived_from_seed" if False else "test_verify_host_files"),
    case("blob-length-check", 'len(data) == rec["bytes"]', 'True',
         "test_validate_rejects_bad_blob_length", "test_validate_rejects_bad_blob_hash"),
    case("blob-traversal", '".." not in Path(name).parts', 'True',
         "test_validate_rejects_path_traversal", "test_validate_ok"),
    case("mac-multicast-check", 'int(n["mac"][:2], 16) & 1 == 0', 'True',
         "test_validate_rejects_multicast_mac", "test_formats_valid"),
    case("disk-serial-check", '[A-Z0-9]{1,20}', '[A-Z0-9]+',
         "test_validate_rejects_long_disk_serial", "test_formats_valid"),
    case("subnet-check", r'10\.\d+\.\d+\.0/24", ids["net"]["subnet"]', r'.*", ids["net"]["subnet"]',
         "test_validate_rejects_default_slirp_subnet_format", "test_net_is_fixed_and_not_default"),
    case("online-restrict-flag", 'restrict = "off" if mode in ("setup", "install") else "on"', 'restrict = "off"',
         "test_offline_and_ci_are_restricted_with_faked_clock", "test_same_ids_in_every_mode"),
    case("offline-real-clock", 'if mode in ("setup", "install"):\n        return None', 'return None',
         "test_offline_and_ci_are_restricted_with_faked_clock", "test_fake_clock_is_stop_time_plus_gap_never_before"),
    case("online-fake-clock", 'if mode in ("setup", "install"):\n        return None', 'if False:\n        return None',
         "test_install_modes_are_online_and_real_clock", "test_offline_needs_recorded_stop_time" if False else "test_install_modes_are_online_and_real_clock"),
    case("clock-add-gap", "utc(last) + dt.timedelta(seconds=state[\"clock\"][\"gap_s\"])\n\n\n# ----", "utc(last) - dt.timedelta(seconds=state[\"clock\"][\"gap_s\"])\n\n\n# ----",
         "test_fake_clock_is_stop_time_plus_gap_never_before", "test_offline_and_ci_are_restricted_with_faked_clock"),
    case("clock-vm-base", ',clock=vm"', '"',
         "test_offline_and_ci_are_restricted_with_faked_clock", "test_same_ids_in_every_mode" if False else "test_offline_and_ci_are_restricted_with_faked_clock"),
    case("hostfwd-loopback", "hostfwd=tcp:127.0.0.1:%d-:22", "hostfwd=tcp::%d-:22",
         "test_hostfwd_only_on_loopback", "test_same_ids_in_every_mode"),
    case("gpu-uuid-flag", '"gpu-uuid=" + ids["gpu_uuid"]', '"gpu-uuid=auto"',
         "test_kf3_modes_carry_gpu_uuid_and_broker", "test_headless_display_has_no_broker"),
    case("disk-serial-arg", 'serial=" + ids["disks"][0]["serial"]', 'serial=KF"',
         "test_same_ids_in_every_mode", "test_validate_ok" if False else "test_same_ids_in_every_mode"),
    case("smbios-uuid-arg", '"-uuid", sm["uuid"],', '',
         "test_smbios_triplet_present", "test_same_ids_in_every_mode"),
    case("tpm-device", '"-device", "tpm-crb,tpmdev=tpm0",', '',
         "test_same_ids_in_every_mode", "test_same_ids_in_every_mode"),
    case("setup-std-vga", '["-vga", "std", "-display", "gtk"', '["-vga", "none", "-display", "gtk"',
         "test_setup_is_std_vga_without_kf3", "test_setup_is_std_vga_without_kf3"),
    case("broker-needs-args", 'require(broker_sock and gop_efi, "broker display needs broker_sock and gop_efi")', 'pass',
         "test_broker_display_needs_socket_and_gop", "test_broker_display_needs_socket_and_gop"),
    case("mode-check", 'require(mode in MODES, "mode must be one of %s" % (MODES,))', 'pass',
         "test_unknown_mode_and_display_rejected", "test_unknown_mode_and_display_rejected"),
    case("compare-none-is-mismatch", 'want == got and want is not None', 'want == got',
         "test_missing_guest_value_is_mismatch_not_match", "test_missing_guest_value_is_mismatch_not_match"),
    case("clock-lower-bound", 'base <= g <= base +', 'g <= base +',
         "test_clock_before_base_mismatches", "test_clock_before_base_mismatches"),
    case("clock-upper-bound", 'g <= base + dt.timedelta(seconds=max_clock_age_s)', 'True',
         "test_clock_too_far_after_base_mismatches", "test_clock_too_far_after_base_mismatches"),
    case("tpm-present-check", 'MATCH" if guest.get("tpm_present") else', 'MATCH" if True else',
         "test_compare_each_item_mismatches", "test_compare_each_item_mismatches"),
    case("verify-host-hash", 'got == rec["sha256"]', 'True',
         "test_verify_host_files", "test_cli_ids_and_verify_exit_codes"),
    case("verify-host-empty", 'if not state["files"]:', 'if False:',
         "test_verify_host_empty_state_is_mismatch", "test_verify_host_empty_state_is_mismatch"),
    case("ssh-batchmode", '"BatchMode=yes"', '"BatchMode=no"',
         "test_ssh_argv_is_loopback_key_only", "test_ssh_argv_is_loopback_key_only"),
    case("zip-stored", "compression=zipfile.ZIP_STORED, allowZip64=True", "compression=zipfile.ZIP_DEFLATED, allowZip64=True",
         "test_pack_unpack_round_trip", "test_unpack_rejects_compressed_members" if False else "test_pack_unpack_round_trip"),
    case("unpack-member-set", 'require(sorted(z.namelist()) == ["image.qcow2", "manifest.json", "state.json"], "unexpected zip members")', 'pass',
         "test_unpack_rejects_extra_member", "test_unpack_rejects_extra_member"),
    case("unpack-stored-only", 'require(all(i.compress_type == zipfile.ZIP_STORED for i in z.infolist()), "members must be STORED")', 'pass',
         "test_unpack_rejects_compressed_members", "test_unpack_rejects_compressed_members"),
    case("unpack-sha", 'require(sha256_file(dest / name) == want, "sha256 mismatch for " + name)', 'pass',
         "test_unpack_detects_tampered_image", "test_unpack_detects_tampered_image"),
    # CPU profile: blocking checks only (the fingerprint flags and brand are informational)
    case("cpu-preflight-vendor", '"MATCH" if vendor == c["vendor"] else "MISMATCH"', '"MATCH"',
         "test_preflight_refuses_intel_host", "test_argv_cli_refuses_with_named_error"),
    case("cpu-preflight-features", '"MATCH" if not missing and flags else "MISMATCH"', '"MATCH"',
         "test_preflight_names_missing_feature", "test_preflight_refuses_empty_cpuinfo"),
    case("cpu-named-model-not-host", 'return ",".join([c["base_model"], "model-id="', 'return ",".join(["host", "model-id="',
         "test_every_mode_uses_the_named_profile_not_host", "test_cpu_arg_pins_family_model_stepping_explicitly"),
    case("cpu-family-pinned", '"family=%d" % c["family"], ', '',
         "test_every_mode_uses_the_named_profile_not_host", "test_cpu_arg_pins_family_model_stepping_explicitly"),
    case("cpu-removals-required", 'require(not set(FORBIDDEN_GUEST_FLAGS) - set(c["remove"]),', 'require(True,',
         "test_validate_rejects_profile_without_avx512_removal", "test_validate_rejects_rtm_not_removed"),
    case("cpu-vendor-validated", 'require(c["vendor"] == "AuthenticAMD", ', 'require(True, ',
         "test_validate_rejects_vendor_spoof_profile", "test_pack_refuses_spoofed_vendor"),
    case("cpu-brand-validated", 'require(re.fullmatch(r"[A-Za-z0-9 ._()-]{1,48}", c["model_id"]),', 'require(True,',
         "test_validate_rejects_brand_injection", "test_validate_rejects_comma_brand_only"),
    case("cpu-vendor-blocking", '("vendor", "cpu_vendor", True)', '("vendor", "cpu_vendor", False)',
         "test_compare_cpu_blocking_items_mismatch", "test_compare_without_recorded_fingerprint_is_mismatch"),
    case("cpu-fms-blocking", '("description", "cpu_family_model_stepping", True)', '("description", "cpu_family_model_stepping", False)',
         "test_compare_cpu_blocking_items_mismatch", "test_compare_without_recorded_fingerprint_is_mismatch"),
    case("cpu-brand-informational", '("brand", "cpu_brand", False)', '("brand", "cpu_brand", True)',
         "test_compare_cpu_informational_items_only_warn", "test_compare_without_recorded_fingerprint_is_mismatch"),
    case("warn-not-failing", 'bad = [r for r in rows if r[1] == "MISMATCH"]\n    print("RESULT', 'bad = [r for r in rows if r[1] != "MATCH"]\n    print("RESULT',
         "test_warn_does_not_fail_the_result", "test_result_line_says_match_when_only_warnings"),
    # unattended media, state files, install media
    case("unattend-user-validated", 'require(re.fullmatch(r"[A-Za-z][A-Za-z0-9_-]{0,19}", user), "bad guest user name")', 'pass',
         "test_render_rejects_bad_user_and_computer_names", "test_unattend_rejects_xml_injection_in_user"),
    case("setup-key-required", 'require(re.fullmatch(PUBKEY_RE, pub), "state has no valid ssh public key")', 'pass',
         "test_setup_script_rejects_missing_or_odd_key", "test_tools_cd_needs_a_public_key_in_the_state"),
    case("tools-hash-pinned", 'require(got == PINS[name], "sha256 mismatch for %s: %s" % (name, got))', 'pass',
         "test_tools_cd_refuses_unpinned_hash", "test_tools_cd_refuses_swapped_installer_contents"),
    case("tools-name-pinned", 'require(Path(src).name == name, "unexpected file name: %s (want %s)" % (Path(src).name, name))', 'pass',
         "test_tools_cd_refuses_wrong_file_name", "test_tools_cd_refuses_installer_name_with_suffix"),
    case("capture-requires-files", 'require(name.endswith(("savestate", "volatilestate")), "missing required file " + name)', 'pass',
         "test_capture_files_requires_tpm_and_ovmf", "test_capture_files_refuses_symlinks"),
    case("capture-no-symlinks", 'if src.is_symlink() or not src.is_file():', 'if not src.is_file():',
         "test_capture_files_refuses_symlinks", "test_capture_files_refuses_symlinked_ovmf_vars"),
    case("s3-disabled", '"-global", "ICH9-LPC.disable_s3=1", "-global", "ICH9-LPC.disable_s4=1",', '',
         "test_s3_s4_disabled_in_every_mode", "test_s3_s4_flags_are_global_options"),
    case("windows-iso-boots-first", '",bootindex=1" if i == 0 else ""', '""',
         "test_setup_argv_attaches_install_media_windows_iso_first", "test_only_the_first_cdrom_has_a_boot_index"),
    case("cdrom-port-limit", 'require(len(cdroms) <= 6, "the q35 AHCI controller has 6 ports")', 'pass',
         "test_more_than_six_cdroms_refused", "test_cli_refuses_seven_cdroms"),
    case("memfd-backend-bound", ',accel=kvm,smm=on,memory-backend=ram0"', ',accel=kvm,smm=on"',
         "test_guest_ram_is_a_shared_memfd_backend_in_every_mode", "test_machine_option_binds_the_ram_backend"),
    case("memfd-backend-shared", 'size=%dM,share=on"', 'size=%dM,share=off"',
         "test_guest_ram_is_a_shared_memfd_backend_in_every_mode", "test_machine_option_binds_the_ram_backend"),
    case("guest-driver-abi", '"guest-driver=" + q["guest_driver"]]', '"x=1"]',
         "test_kf3_device_carries_guest_driver_abi", "test_guest_driver_in_every_kf3_mode"),
    case("guest-driver-validated", 'require(re.fullmatch(r"\\d+\\.\\d+\\.\\d+", state["qemu"]["guest_driver"]), "bad guest-driver")', 'pass',
         "test_validate_rejects_bad_guest_driver", "test_validate_rejects_nonnumeric_guest_driver"),
    case("password-must-be-derived", 'require(pw is None or pw == derive_password(state["seed"]), "guest.password is not the seed-derived password")', 'pass',
         "test_validate_refuses_a_foreign_password", "test_unpack_refuses_a_foreign_password"),
    case("password-required-on-unpack", 'validate_state(state, require_password=True)', 'validate_state(state)',
         "test_unpack_refuses_a_package_without_a_password", "test_unpack_refuses_a_null_password"),
    case("pack-adds-password", 'if state["guest"].get("password") is None:', 'if False:',
         "test_pack_adds_the_password_to_an_older_state", "test_cli_pack_writes_the_password_into_the_packed_state"),
    case("init-writes-password", '"password": derive_password(seed), ', '', 
         "test_init_state_writes_the_derived_password", "test_cli_init_writes_the_password"),
]


def run(tree, tests=()):
    proc = subprocess.run([sys.executable, "-I", "-B", str(tree / "scripts/ci/test_propimage.py"),
                           *("PropTests." + t for t in tests)], text=True, capture_output=True, timeout=300)
    out = proc.stdout + proc.stderr
    return proc.returncode, sorted(set(re.findall(r"^(?:FAIL|ERROR): (\S+)", out, re.M))), out


def main():
    argparse.ArgumentParser(description=__doc__).parse_args()
    test_src = ROOT / "scripts/ci/test_propimage.py"
    known = {n.name for n in ast.walk(ast.parse(test_src.read_text())) if isinstance(n, ast.FunctionDef)}
    assert not {t for c in CASES for t in c[2]} - known, "unknown test selector"
    weak = 0
    with tempfile.TemporaryDirectory(prefix="propimage-mutants-") as tmp:
        tree = Path(tmp)
        (tree / "scripts/bench/windows").mkdir(parents=True)
        (tree / "scripts/ci").mkdir(parents=True)
        shutil.copy(test_src, tree / "scripts/ci/test_propimage.py")
        shutil.copy(ROOT / "scripts/bench/windows/win_vm.sh", tree / "scripts/bench/windows/win_vm.sh")
        shutil.copy(ROOT / "scripts/bench/windows/propimage_autounattend.xml.in", tree / "scripts/bench/windows/propimage_autounattend.xml.in")
        shutil.copytree(ROOT / "scripts/ci/fixtures/cpuflags", tree / "scripts/ci/fixtures/cpuflags")
        target = tree / "scripts/bench/windows/propimage.py"
        target.write_text(SRC)
        code, _, out = run(tree)
        if code:
            print("FAIL unmutated baseline\n" + out[-4000:])
            return 2
        print("PASS baseline: " + re.search(r"Ran (\d+) tests", out).group(1), flush=True)
        for name, mutated, tests in CASES:
            target.write_text(mutated)
            try:
                # run the whole suite so every killer is counted, not only the named ones
                code, killers, out = run(tree)
            finally:
                target.write_text(SRC)
            ok = code != 0 and len(killers) >= 2 and bool(set(tests) & {k.split(".")[-1] for k in killers})
            print("%s %s: %d killers %s" % ("PASS" if ok else "WEAK", name, len(killers), [k.split(".")[-1] for k in killers][:4]), flush=True)
            weak += not ok
    print("%d/%d mutants killed by at least two tests" % (len(CASES) - weak, len(CASES)))
    return 1 if weak else 0


if __name__ == "__main__":
    sys.exit(main())
