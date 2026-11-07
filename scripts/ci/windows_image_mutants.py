#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later
"""Image tooling policy mutants; each must fail >=2 independently named tests.

Uses isolated copies and only fixture executables. PowerShell mutations are
source-contract checks, explicitly not proof of Windows behavior. Run with
--json /path/outside/repo.json to retain the kill table for a lane report.
"""
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
PY = (WIN / "image_tools.py").read_text()
PS = (WIN / "image_prepare_guest.ps1").read_text()
LAUNCHER = (WIN / "win_vm.sh").read_text()


def changed(source, old, new, count=1):
    assert source.count(old) == count, (old, source.count(old), count)
    return source.replace(old, new)


def py(name, old, new, *tests, count=1):
    return name, "image_tools.py", changed(PY, old, new, count), ["Images." + x for x in tests]


def ps(name, old, new, *tests, count=1):
    return name, "image_prepare_guest.ps1", changed(PS, old, new, count), ["GuestSourceContracts." + x for x in tests]


CASES = [
    py("std-vga-default-stage", '["--no-kf3"] if args.display_mode == "std-vga"', '[] if args.display_mode == "std-vga"',
       "test_install_app_stage_uses_std_vga", "test_start_reuses_run_overlay_and_fixed_identity"),
    py("broker-attachment", 'f"display-broker={broker},display-broker-uid={os.getuid()}"', '"display=on"',
       "test_gpu_stage_attaches_broker_and_never_live_vnc", "test_gpu_status_keeps_broker_plan_and_qmp_fallback"),
    py("no-live-kf3-vnc", 'state.get("display_mode") != "std-vga"', 'False',
       "test_gpu_stage_attaches_broker_and_never_live_vnc", "test_gpu_status_keeps_broker_plan_and_qmp_fallback"),
    py("broker-needs-display", 'os.environ.get("WINVM_KF3_DISPLAY", "on") == "on"', 'True',
       "test_gpu_display_off_is_refused", "test_gpu_install_display_off_is_refused"),
    py("broker-caller-socket", 'broker.is_socket() and broker.stat().st_uid == os.getuid()', 'True',
       "test_gpu_requires_live_caller_broker_socket", "test_gpu_install_refuses_missing_broker_before_install"),
    py("broker-socket-no-injection", 'broker.is_absolute() and not re.search(r"[,\\s]", str(broker))', 'True',
       "test_gpu_socket_property_injection_refused", "test_gpu_socket_whitespace_injection_refused"),
    py("resume-clean-receipt", 'args.verb == "start" and previous.get("phase") == "clean" and receipt.get("tag") == previous["tag"]', 'True',
       "test_resume_rejects_nonclean_phase_with_receipt", "test_resume_rejects_wrong_receipt_tag"),
    py("resume-unchanged-disks", 'disk_fingerprints(vm, previous) == receipt.get("disks")', 'True',
       "test_resume_rejects_changed_source", "test_resume_rejects_changed_base"),
    ("launcher-network-default", "win_vm.sh", changed(LAUNCHER, '${WINVM_NET_RESTRICT:-on}', '${WINVM_NET_RESTRICT:-off}'),
     ["LauncherNetworkDefaults.test_network_default_is_byte_identical_to_explicit_on",
      "LauncherNetworkDefaults.test_unset_default_retains_exact_old_netdev"]),
    ("launcher-network-validation", "win_vm.sh", changed(LAUNCHER, '         on|off) ;;', '         *) ;;'),
     ["LauncherNetworkDefaults.test_invalid_network_restriction_never_emits_qemu",
      "LauncherNetworkDefaults.test_network_option_injection_is_rejected"]),
    py("session-network-opt-in", '"off" if args.allow_internet else "on"', '"on"',
       "test_online_session_requires_explicit_opt_in", "test_install_forwards_online_opt_in", count=2),
    py("session-network-default", '"off" if args.allow_internet else "on"', '"off"',
       "test_start_reuses_run_overlay_and_fixed_identity", "test_session_network_is_restricted_by_default", count=2),
    py("session-fixed-identity", 'WINVM_VM_IDENTITY="1"', 'WINVM_VM_IDENTITY="0"',
       "test_start_reuses_run_overlay_and_fixed_identity", "test_session_network_is_restricted_by_default", count=2),
    py("account-password-change", 'Set-LocalUser -Name', 'Get-LocalUser -Name',
       "test_password_literal_only_on_stdin", "test_connection_commands_are_loopback_and_exact"),
    py("account-password-stdin", 'data=pw, timeout=30', 'data="", timeout=30',
       "test_password_literal_only_on_stdin", "test_connection_commands_are_loopback_and_exact"),
    py("account-no-password-logging", 'pw = password(args.env_file)', 'pw = password(args.env_file); print(pw)',
       "test_password_literal_only_on_stdin", "test_no_password_logging_when_guest_fails"),
    py("secret-permissions", "s.st_mode & 0o077 == 0", "True",
       "test_world_readable_secret_refused_before_boot", "test_group_readable_password_refused"),
    py("secret-outside-checkout", "not path.resolve().is_relative_to(REPO)", "True",
       "test_secret_inside_repo_refused_before_install", "test_password_in_repo_refused_on_start"),
    py("env-data-format", 'len(lines) == 1 and lines[0].startswith("WIN_IMAGE_PASSWORD=")', "True",
       "test_env_is_data_never_sourced", "test_duplicate_password_line_is_rejected"),
    py("exclusive-vm-lock", "fcntl.flock(f, fcntl.LOCK_EX | fcntl.LOCK_NB)", "pass",
       "test_busy_vm_refused_before_start", "test_busy_vm_refused_before_qemu_img",
       "test_busy_vm_never_creates_output_directory"),
    py("one-active-session", 'not (vm / "image-session.json").exists()', "True",
       "test_second_start_invalidates_no_running_session", "test_active_session_cannot_be_reinstalled"),
    py("fresh-build-overlay", 'not (vm / f"disk/{OVERLAY}.qcow2").exists()', "True",
       "test_existing_overlay_refused_on_start", "test_existing_overlay_refused_on_install"),
    py("guest-preconditions", 'data.get(key) is True', "True",
       "test_failed_inventory_never_stops_or_seals", "test_hibernation_enabled_never_stops"),
    py("guest-zero-completion", 'data.get("zero_fill", {}).get("completed") is True', "True",
       "test_incomplete_zero_fill_never_stops", "test_zero_completion_marker_required"),
    py("inventory-session-binding", 'data.get("schema") == "kayfabe-windows-inventory/1" and data.get("session_id") == tag', "True",
       "test_inventory_for_old_session_refused", "test_unsupported_inventory_schema_refused"),
    py("guest-origin-shutdown", 'shuts and shuts[-1].get("guest") is True and shuts[-1].get("reason") == "guest-shutdown"', "True",
       "test_qmp_quit_not_a_clean_shutdown", "test_missing_guest_origin_refused"),
    py("no-panic-capture", 'not any(x.get("event") == "GUEST_PANICKED" for x in rows)', "True",
       "test_guest_panic_then_shutdown_not_clean", "test_panic_after_shutdown_refused"),
    py("clean-receipt-binding", 'state.get("phase") == "clean" and receipt.get("tag") == state["tag"]', "True",
       "test_wrong_receipt_tag_refused", "test_nonclean_session_phase_refused"),
    py("unchanged-overlay-and-base", 'disk_fingerprints(vm, state) == receipt.get("disks")', "True",
       "test_changed_disk_refused", "test_touched_disk_refused", "test_base_changed_after_shutdown_refused",
       "test_base_touched_after_shutdown_refused"),
    py("unchanged-identity", 'digest(vm / "identity.env") == receipt.get("identity_sha256")', "True",
       "test_changed_identity_refused", "test_identity_receipt_mismatch_refused"),
    py("unchanged-event-evidence", 'clean_shutdown(vm, state) == receipt.get("qmp")', "True",
       "test_event_log_changed_after_receipt_refused", "test_new_event_after_shutdown_refused"),
    py("unchanged-inventory-evidence", 'digest(vm / "image-inventory.json") == receipt.get("inventory_sha256")', "True",
       "test_inventory_file_changed_after_receipt_refused", "test_inventory_whitespace_change_refused"),
    py("qemu-check-before-and-after", 'command(["qemu-img", "check", "-f", "qcow2", disk])', "pass",
       "test_check_failure_does_not_convert", "test_check_failure_publishes_nothing"),
    py("known-backing-chain", 'isinstance(chain, list) and len(chain) == 2 and all(x.get("format") == "qcow2" for x in chain)', "True",
       "test_extra_backing_parent_is_refused", "test_unknown_source_chain_never_converts"),
    py("compressed-qcow2", '["qemu-img", "convert", "-c", "-f", "qcow2", "-O", "qcow2", disk, artifact]',
       '["qemu-img", "convert", "-f", "qcow2", "-O", "qcow2", disk, artifact]',
       "test_convert_is_compressed_flattened_qcow2", "test_manifest_records_artifact_pins_inventory_revision"),
    py("compare-virtual-content", 'command(["qemu-img", "compare", "-f", "qcow2", "-F", "qcow2", disk, artifact])', "pass",
       "test_compare_failure_removes_partial_package", "test_compare_failure_never_publishes_manifest"),
    py("standalone-output", 'info.get("format") == "qcow2" and no_external_data(info)', "True",
       "test_output_backing_file_refused", "test_output_external_dependency_never_published", "test_nested_external_data_file_refused"),
    py("repo-media-provenance", "pins = media_pins()", "pins = []",
       "test_manifest_records_artifact_pins_inventory_revision", "test_exact_repo_media_pins_are_embedded"),
    py("manual-installer-pins", 're.fullmatch(r"[0-9a-f]{64}", row["sha256"]) and row["official_url"].startswith("https://")', "True",
       "test_manual_installer_short_hash_refused", "test_manual_installer_insecure_url_refused"),
    py("new-output-outside-checkout", "not out.is_relative_to(REPO) and not out.exists()", "True",
       "test_existing_output_directory_is_not_replaced", "test_package_inside_checkout_refused"),
    py("local-upload-hash", "\n        matches(path, a)\n", "\n        pass\n",
       "test_upload_corruption_refused_before_network", "test_upload_same_size_corruption_refused"),
    py("download-hash", "\n            matches(path, a)\n", "\n            pass\n",
       "test_verify_hash_mismatch_fails", "test_wrong_download_is_not_retained"),
    py("https-only", 'parts.scheme == "https" and parts.hostname', "parts.hostname",
       "test_signed_url_must_be_https", "test_verify_refuses_plain_http_too"),
    py("url-config-injection", "not any(ord(c) <= 32 or c in '\\\\\"' for c in url)", "True",
       "test_curl_config_injection_refused", "test_quoted_url_cannot_inject_curl_option"),
    py("curl-retries", '"--retry", "3", "--retry-all-errors", "--retry-delay", "2"', '"--retry", "0", "--retry-delay", "2"',
       "test_upload_put_retries_and_hides_url", "test_verify_download_matches_manifest"),
    py("curl-error-redaction", 'f"{Path(argv[0]).name} failed (exit {r.returncode}); no receipt published"', "r.stderr",
       "test_curl_error_does_not_leak_signed_url", "test_failed_get_hides_signed_url_too"),
    py("transfer-http-success", 're.fullmatch(r"2[0-9]{2}", status.strip())', "True",
       "test_redirect_is_not_acknowledged_as_upload", "test_redirect_get_rejected_even_with_matching_body", count=2),
    py("single-put-limit", 'a["bytes"] <= 5 * 1024**3', "True",
       "test_upload_size_limit_before_hashing", "test_multipart_required_for_large_record"),
    py("manifest-artifact-name", 'a.get("name") == "windows.qcow2"', "True",
       "test_parent_artifact_name_refused_on_verify", "test_absolute_artifact_name_refused_on_verify"),
    ps("guest-hibernation-command", "powercfg.exe /h off", "powercfg.exe /h on",
       "test_power_and_time_commands_fail_closed", "test_commands_and_inventory_precede_zeroing_and_commit"),
    ps("guest-stop-time", "Stop-Service -Name W32Time -Force", "# time stop removed",
       "test_power_and_time_commands_fail_closed", "test_commands_and_inventory_precede_zeroing_and_commit"),
    ps("guest-fast-startup-off", "Set-ItemProperty -LiteralPath $power -Name HiberbootEnabled -Type DWord -Value 0",
       "Set-ItemProperty -LiteralPath $power -Name HiberbootEnabled -Type DWord -Value 1",
       "test_power_and_time_commands_fail_closed", "test_commands_and_inventory_precede_zeroing_and_commit"),
    ps("guest-disable-time", "Set-Service -Name W32Time -StartupType Disabled", "Set-Service -Name W32Time -StartupType Automatic",
       "test_power_and_time_commands_fail_closed", "test_commands_and_inventory_precede_zeroing_and_commit"),
    ps("guest-package-inventory", "Get-Package |", "Get-Item |",
       "test_inventory_records_shutdown_preconditions", "test_commands_and_inventory_precede_zeroing_and_commit"),
    ps("guest-registry-inventory", "WOW6432Node", "MISSING_NODE",
       "test_inventory_records_shutdown_preconditions", "test_registry_driver_and_build_inventory_coverage"),
    ps("guest-driver-inventory", "Win32_PnPSignedDriver", "Win32_Service",
       "test_inventory_records_shutdown_preconditions", "test_registry_driver_and_build_inventory_coverage"),
    ps("guest-windows-build", "CurrentBuild", "CurrentVersion",
       "test_inventory_records_shutdown_preconditions", "test_registry_driver_and_build_inventory_coverage"),
    ps("guest-exclusion-report", "exclusions = @", "excluded_unreported = @",
       "test_inventory_is_written_after_zeroing", "test_registry_driver_and_build_inventory_coverage"),
    ps("guest-file-hashes", "Get-FileHash -LiteralPath", "Get-Item -LiteralPath",
       "test_inventory_records_shutdown_preconditions", "test_file_inventory_records_exclusions_and_refuses_reparse_points"),
    ps("guest-bitlocker-check", "Get-BitLockerVolume -MountPoint", "Get-Volume -DriveLetter",
       "test_inventory_records_shutdown_preconditions", "test_commands_and_inventory_precede_zeroing_and_commit"),
    ps("guest-success-marker", "hibernation_disabled = $true", "hibernation_disabled = $false",
       "test_inventory_records_shutdown_preconditions", "test_guest_writes_only_successful_preconditions"),
    ps("guest-stop-on-error", "$ErrorActionPreference = 'Stop'", "$ErrorActionPreference = 'Continue'",
       "test_power_and_time_commands_fail_closed", "test_guest_writes_only_successful_preconditions"),
    ps("guest-sdelete-zero", "& $sdelete '-z'", "& $sdelete '-c'",
       "test_zeroing_uses_only_local_tool_or_zero_stream", "test_zero_stream_is_cleaned_and_success_recorded"),
    ps("guest-real-zero-buffers", "New-Object byte[]", "New-Object int[]",
       "test_zeroing_uses_only_local_tool_or_zero_stream", "test_zero_stream_is_cleaned_and_success_recorded"),
    ps("guest-exclusive-zero-file", "FileMode]::CreateNew", "FileMode]::Create",
       "test_zeroing_uses_only_local_tool_or_zero_stream", "test_zero_stream_is_cleaned_and_success_recorded"),
    ps("guest-flush-zero-file", ".Flush($true)", ".Flush($false)",
       "test_zeroing_uses_only_local_tool_or_zero_stream", "test_zero_stream_is_cleaned_and_success_recorded"),
    ps("guest-remove-zero-file", "Remove-Item -LiteralPath $zeroPath", "Write-Output $zeroPath",
       "test_zeroing_uses_only_local_tool_or_zero_stream", "test_zero_stream_is_cleaned_and_success_recorded"),
    ps("guest-reparse-refusal", "[IO.FileAttributes]::ReparsePoint", "[IO.FileAttributes]::Hidden",
       "test_inventory_is_written_after_zeroing", "test_file_inventory_records_exclusions_and_refuses_reparse_points", count=2),
    ps("guest-no-downloads", "    # ZERO_FILL_START", "    Invoke-WebRequest 'https://example.invalid/fixture'\n    # ZERO_FILL_START",
       "test_zeroing_uses_only_local_tool_or_zero_stream", "test_guest_network_downloads_are_absent"),
]

# Both source and output checks are a single policy. Removing just the source
# check is still caught by the output check; the mutant removes both.
for i, case in enumerate(CASES):
    if case[0] == "qemu-check-before-and-after":
        CASES[i] = (*case[:2], changed(case[2], 'command(["qemu-img", "check", "-f", "qcow2", artifact])', "pass"), case[3])
    elif case[0] == "unchanged-overlay-and-base":
        CASES[i] = (*case[:2], changed(case[2], 'disk_fingerprints(vm, state) == receipt["disks"]', "True"), case[3])


def run(tree, tests=None):
    r = subprocess.run([sys.executable, "-I", str(tree / "scripts/ci/test_windows_image.py"), *(tests or [])],
                       capture_output=True, text=True, timeout=900)
    out = r.stdout + r.stderr
    killers = sorted(set(re.findall(r"^(?:FAIL|ERROR): (\S+)", out, re.M)))
    return r.returncode, killers, out


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--json", type=Path)
    args = parser.parse_args()
    test_tree = ast.parse((ROOT / "scripts/ci/test_windows_image.py").read_text())
    known = {f"{node.name}.{method.name}" for node in test_tree.body if isinstance(node, ast.ClassDef)
             for method in node.body if isinstance(method, ast.FunctionDef) and method.name.startswith("test_")}
    unknown = {test for case in CASES for test in case[3]} - known
    if unknown:
        print(f"FAIL unknown test selectors: {sorted(unknown)}")
        return 2
    table = []
    with tempfile.TemporaryDirectory(prefix="y1-image-mutants-") as tmp:
        tree = Path(tmp)
        win = tree / "scripts/bench/windows"
        ci = tree / "scripts/ci"
        win.mkdir(parents=True)
        ci.mkdir(parents=True)
        for name in ("win_vm.sh", "image_tools.py", "image_prepare_guest.ps1", "image_session.sh", "image_package.sh"):
            shutil.copy(WIN / name, win / name)
        shutil.copy(ROOT / "scripts/ci/test_windows_image.py", ci / "test_windows_image.py")
        (ci / "fixtures").mkdir()
        shutil.copy(ROOT / "scripts/ci/fixtures/windows_image_default_argv.json", ci / "fixtures/windows_image_default_argv.json")
        code, _, out = run(tree)
        if code:
            print("FAIL baseline: unmutated tests fail", flush=True)
            print(out[-8000:])
            return 2
        count = re.search(r"Ran (\d+) tests?", out)
        print(f"PASS baseline ({count.group(1) if count else 'unknown count'} image tests)", flush=True)
        for name, filename, source, tests in CASES:
            path = win / filename
            original = path.read_text()
            try:
                path.write_text(source)
                code, killers, _ = run(tree, tests)
            finally:
                path.write_text(original)
            ok = code != 0 and len(killers) >= 2
            table.append(dict(mutant=name, coverage="source contract" if filename.endswith(".ps1") else "mock execution",
                              killed=ok, killers=killers))
            print(f"{'PASS' if ok else 'WEAK'} {name}: {len(killers)} killers {', '.join(killers)}", flush=True)
    if args.json:
        args.json.write_text(json.dumps(table, indent=2) + "\n")
    passed = sum(row["killed"] for row in table)
    print(f"{passed}/{len(table)} mutants killed by at least two tests", flush=True)
    return 0 if passed == len(table) else 1


if __name__ == "__main__":
    sys.exit(main())
