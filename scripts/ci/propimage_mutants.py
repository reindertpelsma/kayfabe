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
