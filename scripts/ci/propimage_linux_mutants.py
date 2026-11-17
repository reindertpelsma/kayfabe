#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later
"""propimage_linux.py rule mutations, each requiring two independently named killers (the whole suite runs per mutant)."""
import argparse
import ast
import re
import shutil
import subprocess
import sys
import tempfile
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
MAIN = "scripts/bench/linux/propimage_linux.py"
PKGS = "scripts/bench/linux/propimage_linux_pkgs.py"   # section 13: manifest, harden-offline, inject (imported by MAIN: must be in the copy list)
SRCS = {rel: (ROOT / rel).read_text() for rel in (MAIN, PKGS)}


def case(name, old, new, *tests, count=1, file=MAIN):
    src = SRCS[file]
    assert src.count(old) == count, (name, old, src.count(old))
    return name, file, src.replace(old, new), tests


def pkg(name, old, new, *tests, count=1):
    return case(name, old, new, *tests, count=count, file=PKGS)


CASES = [
    case("machine-id-width", 'h("machine-id")[:16].hex()', 'h("machine-id")[:8].hex()',
         "test_formats_valid", "test_validate_rejects_bad_fields" if False else "test_linux_ids_are_deterministic_and_seed_dependent"),
    case("seed-binding-linux-ids", ' and derive_linux_ids(state["seed"]) == state["linux_ids"])', ')',
         "test_edited_machine_id_is_rejected", "test_pack_refuses_a_state_not_derived_from_its_seed"),
    case("mac-local-admin-check", 'int(n["mac"][:2], 16) & 3 == 2', 'True',
         "test_validate_rejects_universal_mac", "test_validate_rejects_bad_fields"),
    case("restrict-offline", 'restrict = "off" if net == "online" else "on"', 'restrict = "off"',
         "test_run_is_offline_recorded_clock", "test_offline_argv_is_restricted_in_every_variant"),
    case("vnc-bind", '"127.0.0.1:%d" % vnc_display', '"0.0.0.0:%d" % vnc_display',
         "test_setup_is_online_real_clock_vnc_loopback_only", "test_no_public_listener_anywhere_in_argv"),
    case("hostfwd-bind", 'hostfwd=tcp:127.0.0.1:%d-:22', 'hostfwd=tcp::%d-:22',
         "test_hostfwd_only_on_loopback", "test_no_public_listener_anywhere_in_argv"),
    case("rtc-from-the-clock-rule", '"-rtc", clock["rtc"],', '"-rtc", "base=utc",',
         "test_run_is_offline_recorded_clock", "test_rtc_argument_is_exactly_the_clock_rule_value"),
    case("clock-gap-sign", 'dt.timedelta(seconds=state["clock"]["gap_s"] + offset_s)', 'dt.timedelta(seconds=-state["clock"]["gap_s"] - offset_s)',
         "test_base_is_stop_plus_gap", "test_offline_is_recorded"),
    case("clock-offset-dropped", 'dt.timedelta(seconds=state["clock"]["gap_s"] + offset_s)', 'dt.timedelta(seconds=state["clock"]["gap_s"])',
         "test_base_is_stop_plus_gap", "test_advancing_the_clock_can_clear_the_driver_floor_but_never_go_backwards"),
    case("driver-floor-dropped", 'loaded_software=list(loaded_software))', 'loaded_software=[])',
         "test_driver_installer_newer_than_clock_is_refused", "test_driver_installer_older_than_clock_passes"),
    case("online-clock-real", 'return CR.clock_for("online", "propimage-linux", phase="build" if mode == "setup" else "run")',
         'return CR.clock_for("offline", "propimage-linux", base=CR.fmt_utc(recorded_base(state, offset_s)))',
         "test_online_is_real", "test_setup_is_online_real_clock_vnc_loopback_only"),
    case("compare-machine-id", 'add("machine_id", lx["machine_id"], guest.get("machine_id"))', 'add("machine_id", lx["machine_id"], lx["machine_id"])',
         "test_each_identity_field_is_checked", "test_missing_value_is_a_mismatch_not_a_match"),
    case("compare-clock-lower", 'base <= g <= hi', 'g <= hi',
         "test_clock_window", "test_clock_earlier_than_base_is_never_ok_even_by_a_day"),
    case("compare-clock-upper", 'base <= g <= hi', 'base <= g',
         "test_clock_window", "test_clock_far_after_base_plus_elapsed_is_a_mismatch"),
    case("compare-no-network", 'not guest.get("internet") and not guest.get("ping")', 'True',
         "test_internet_reachable_is_a_mismatch_offline", "test_ping_reachable_is_a_mismatch_offline"),
    case("compare-build-tools", 'guest.get("kernel_cc_present") and guest.get("gcc_present")', 'True',
         "test_kernel_compiler_for_the_driver_build", "test_gcc_and_make_for_the_driver_build"),
    case("grep-label-leaks-pattern", 'hashlib.sha256(raw).hexdigest()[:8]', 'pat[:40]',
         "test_labels_never_contain_the_pattern", "test_report_has_counts_and_paths_but_no_matches"),
    case("grep-straddle-double-count", 'if m.start() < main)', 'if True)',
         "test_raw_scan_straddling_chunk_boundary_counts_once_and_skips_holes", "test_raw_scan_two_straddles"),
    case("account-lines-kept", 'ACCOUNT_LINE_RE = r"licen[sc]e|', 'ACCOUNT_LINE_RE = r"',
         "test_account_and_licence_lines_are_dropped", "test_licence_word_alone_is_enough_to_drop_a_line"),
    case("redact-off", '    sp = urllib.parse.urlsplit(url)\n    q = urllib.parse.parse_qsl(sp.query, keep_blank_values=True)\n    red =',
         '    return url\n    sp = urllib.parse.urlsplit(url)\n    q = urllib.parse.parse_qsl(sp.query, keep_blank_values=True)\n    red =',
         "test_redact_hides_codes_and_tokens_not_the_authorization_request", "test_capture_output_never_contains_a_code_from_a_logged_callback"),
    case("mask-off", '    return TOKEN_RE.sub("<masked>", t)', '    return t',
         "test_masking", "test_auth_capture_paste_mode_never_prints_a_code_like_prompt_token"),
    case("paste-logging-stays-on", 'set -e; S=%s; tmux pipe-pane -t $S; tmux load-buffer', 'set -e; S=%s; tmux load-buffer',
         "test_paste_goes_over_stdin_never_argv_or_files", "test_paste_script_turns_logging_off_before_the_code_and_back_on_after"),
    case("forward-target-family", 'target = "[::1]" if listener and listener["addr"] == "::1" else "127.0.0.1"', 'target = "127.0.0.1"',
         "test_forward_plan_for_all_three_hosts", "test_forward_target_follows_the_listener_family"),
    case("forward-bind-host", '"127.0.0.1": "127.0.0.1:", "::1": "[::1]:"', '"127.0.0.1": "", "::1": ""',
         "test_forward_plan_for_all_three_hosts", "test_forward_bind_follows_the_redirect_host"),
    case("listener-tree-filter", '    if tree is not None:\n        cand = [l for l in cand if l["pid"] in tree]', '    if False:\n        cand = []',
         "test_pick_listener_filters", "test_capture_ignores_a_listener_outside_the_command_tree"),
    case("listener-baseline", '(l["addr"], l["port"]) not in base', 'True',
         "test_pick_listener_filters", "test_capture_ignores_a_preexisting_listener"),
    case("driver-flag-no-questions", '"--no-questions", "-m=kernel-open"', '"-m=kernel-open"',
         "test_install_argv_is_the_repo_flag_set", "test_install_driver_success"),
    case("driver-pinned-by-image", '"nic_model": nic_model, "guest_driver": None}', '"nic_model": nic_model, "guest_driver": "580.65.06"}',
         "test_no_nvidia_driver_is_pinned_by_the_image", "test_kf3_needs_a_run_supplied_driver"),
    case("driver-module-check", 'require(mod == ver and cuda == ver,', 'require(True,',
         "test_module_version_mismatch_is_named", "test_libcuda_version_mismatch_is_named"),
    case("unpack-sha", 'require(sha256_file(dest / name) == want, "sha256 mismatch for " + name)', 'pass',
         "test_tampered_member_is_detected", "test_tampered_state_json_is_detected"),
    case("memory-guard", 'require(have >= min_mb,', 'require(True,',
         "test_memory_guard", "test_memory_guard_boundary"),
    case("kf3-driver-required", 'require(driver_version or q["guest_driver"], "GUEST_DRIVER_REQUIRED', 'require(True, "GUEST_DRIVER_REQUIRED',
         "test_kf3_needs_a_run_supplied_driver", "test_cli_argv_refuses_kf3_without_a_driver"),

    # ---- section 13: package manifest, harden-offline, inject (propimage_linux_pkgs.py unless marked main)
    pkg("vendor-secret-key", 'require(not SECRET_KEY_RE.search(k),', 'require(True,',
        "test_a_key_that_names_a_secret_is_refused", "test_cli_seal_refuses_a_secret_vendor_key_before_touching_the_guest"),
    pkg("vendor-credential-url", 'require(not CRED_VALUE_RE.search(v),', 'require(True,',
        "test_a_credential_url_in_a_value_is_refused", "test_cli_seal_refuses_a_credential_url_in_a_vendor_value"),
    pkg("dpkg-status-filter", 'f[3][1:2] != "i"', 'False',
        "test_a_removed_package_is_not_in_the_manifest", "test_dpkg_keeps_installed_packages_with_versions_manual_and_held"),
    pkg("apt-credentials-not-redacted", 'return USERINFO_RE.sub("REDACTED@", text)', 'return text',
        "test_apt_source_credentials_are_redacted", "test_redaction_helper_alone"),
    pkg("large-dir-any-size", 'r["bytes"] > threshold', 'r["bytes"] > 0',
        "test_large_dir_threshold_is_one_gb_and_caches_are_not_applications", "test_seal_warns_naming_large_dirs_without_a_vendor_entry"),
    pkg("large-dir-warns-despite-vendor", 'if m["large_dirs"] and not m["vendor"]:', 'if m["large_dirs"]:',
        "test_a_vendor_entry_silences_the_large_dir_warning", "test_package_manifest_through_a_scripted_guest_and_vendor_entries"),
    pkg("harden-record-ignored", 'elif recorded_apt and recorded_apt.get("sources"):', 'elif False:',
        "test_sources_recorded_by_harden_are_used_when_the_guest_has_none", "test_manifest_uses_the_host_record_after_harden"),
    pkg("hardened-ignores-sources", 'if n_src != "0":', 'if False:',
        "test_hardened_flag_needs_every_part", "test_remaining_apt_sources_make_the_image_not_hardened_and_warn"),
    pkg("hardened-ignores-masks", 'if unmasked:\n        problems.append("not masked: "', 'if False:\n        problems.append("not masked: "',
        "test_hardened_flag_needs_every_part", "test_an_unmasked_unattended_upgrades_service_is_not_hardened"),
    pkg("diff-drops-changed", 'chg = sorted((k, ma[k], mb[k]) for k in ma if k in mb and ma[k] != mb[k])', 'chg = []',
        "test_added_removed_and_changed_packages", "test_vendor_apt_kernel_and_hardening_changes"),
    case("pack-ignores-manifest-hash", 'require(members.get(P.MANIFEST_NAME) == want,', 'require(True,',
         "test_pack_refuses_a_missing_package_manifest", "test_pack_refuses_a_changed_package_manifest"),
    pkg("ruleset-output-accepts", 'type filter hook output priority 0; policy drop;', 'type filter hook output priority 0; policy accept;',
        "test_ruleset_is_default_drop_with_loopback_and_established_only", "test_the_harden_script_embeds_the_default_drop_ruleset"),
    pkg("classify-any-ct-state", 'if states and states <= {"established", "related"}:', 'if states:',
        "test_accepting_new_connections_is_an_exception", "test_a_ct_state_invalid_accept_is_an_exception"),
    pkg("analyze-any-policy", 'c.get("policy") == "drop"', 'c.get("policy") is not None',
        "test_output_policy_accept_is_refused", "test_forward_policy_accept_is_refused"),
    pkg("exceptions-allowed", 'require(not a["exceptions"],', 'require(True,',
        "test_a_port_accept_is_an_exception", "test_a_ruleset_with_an_exception_after_apply_refuses"),
    pkg("outbound-open-allowed", 'require(not open_,', 'require(True,',
        "test_a_tcp_path_still_open_after_the_rules_refuses", "test_a_ping_path_still_open_after_the_rules_refuses"),
    pkg("unattended-upgrades-not-masked", 'MASK_UNITS = ("apt-daily.timer", "apt-daily-upgrade.timer", "apt-daily.service", "apt-daily-upgrade.service", "unattended-upgrades.service")',
        'MASK_UNITS = ("apt-daily.timer", "apt-daily-upgrade.timer", "apt-daily.service", "apt-daily-upgrade.service")',
        "test_mask_units_are_the_named_five", "test_harden_script_masks_removes_and_checks"),
    case("clone-backing-unchecked", 'require(backing and Path(backing).resolve() == sealed_img,', 'require(True,',
         "test_an_image_without_a_backing_file_is_refused", "test_an_overlay_of_some_other_image_is_refused"),
    case("clone-running-unchecked", 'require(pid, "CLONE_NOT_RUNNING', 'require(True, "CLONE_NOT_RUNNING',
         "test_a_stopped_clone_is_refused_by_unharden", "test_a_stopped_clone_is_refused_by_inject"),
    case("preflight-ignores-mismatch", 'require(not bad, "CLONE_CHECK_FAILED', 'require(True, "CLONE_CHECK_FAILED',
         "test_preflight_refuses_another_identity", "test_preflight_refuses_a_guest_with_network_like_the_setup_vm"),
    pkg("dest-prefix-only", 'ok = any(full.startswith(r + "/") for r in roots)', 'ok = any(full.startswith(r) for r in roots)',
        "test_a_sibling_directory_with_the_same_prefix_is_refused", "test_the_root_itself_is_refused"),
    pkg("symlink-absolute-allowed", 'require(not os.path.isabs(t),', 'require(True,',
        "test_an_absolute_symlink_is_refused", "test_an_absolute_symlink_pointing_inside_the_source_is_refused_too"),
    pkg("symlink-escape-allowed", 'require(_inside(real, root if src.is_dir() else os.path.dirname(root)),', 'require(True,',
        "test_a_relative_symlink_leaving_the_source_is_refused", "test_a_symlink_chain_that_escapes_is_refused"),
    pkg("special-files-allowed", 'raise PkgError("INJECT_SPECIAL_FILE: %s is not a regular file, directory or symlink" % p)', 'pass',
        "test_a_fifo_is_refused", "test_a_socket_is_refused"),
    pkg("size-cap-unchecked", 'require(total <= max_bytes, "INJECT_TOO_LARGE: %d bytes of files', 'require(True, "INJECT_TOO_LARGE: %d bytes of files',
        "test_over_the_cap_is_refused_before_anything_is_sent", "test_the_cap_boundary"),
    pkg("setuid-travels", '    ti.mode &= 0o1777   # no setuid/setgid bits travel', '    pass',
        "test_the_tar_stream_drops_the_setuid_bit", "test_the_tar_stream_drops_the_setgid_bit_and_owner"),
    pkg("stream-hash-not-updated", '        self.h.update(b)\n', '        pass\n',
        "test_the_printed_hash_is_the_hash_of_the_bytes_sent", "test_the_hash_follows_the_content_and_is_reproducible"),
    pkg("lock-hashes-optional", 'require(hashes, "LOCK_NOT_HASHED', 'require(True, "LOCK_NOT_HASHED',
        "test_an_unhashed_lock_is_refused_by_name", "test_one_unhashed_line_among_hashed_ones_is_refused"),
    pkg("lock-direct-reference-allowed", 'require(not re.search(r"@|://|^git\\+|^\\.|^/", spec), "LOCK_NEEDS_INDEX', 'require(True, "LOCK_NEEDS_INDEX',
        "test_requirements_that_need_an_index_or_a_url_are_refused_by_name", "test_a_lock_that_needs_an_index_stops_before_the_guest_is_asked"),
    pkg("missing-wheel-skipped", '            raise PkgError("WHEEL_MISSING: %s has no wheel in %s" % (label, wh))', '            continue',
        "test_a_missing_wheel_is_named", "test_a_wheel_of_the_wrong_version_counts_as_missing"),
    pkg("wheel-hash-unchecked", 'require(good, "WHEEL_HASH_MISMATCH', 'require(True, "WHEEL_HASH_MISMATCH',
        "test_a_changed_wheel_is_a_hash_mismatch", "test_a_wrong_hash_in_the_lock_is_a_hash_mismatch"),
    pkg("wheel-tag-unchecked", 'require(fit, "WHEEL_TAG_MISMATCH', 'require(True, "WHEEL_TAG_MISMATCH',
        "test_a_wheel_for_another_python_is_a_tag_mismatch", "test_a_wheel_for_another_platform_is_a_tag_mismatch"),
    pkg("install-uses-an-index", '--no-index --no-input', '--no-input',
        "test_end_to_end_copies_the_wheels_and_the_staged_lock_and_installs_offline_with_hashes", "test_the_install_command_never_reaches_for_an_index_or_apt"),
    pkg("install-without-hashes", '--require-hashes -r "$D/requirements.lock.txt"', '-r "$D/requirements.lock.txt"',
        "test_end_to_end_copies_the_wheels_and_the_staged_lock_and_installs_offline_with_hashes", "test_the_install_command_never_reaches_for_an_index_or_apt"),
    pkg("staged-lock-keeps-index-lines", 'if not LOCK_OPTION_RE.match(l.strip())]', 'if True]',
        "test_parse_lock_requirements_hashes_markers_and_dropped_options", "test_end_to_end_copies_the_wheels_and_the_staged_lock_and_installs_offline_with_hashes"),
]


def run(tree):
    proc = subprocess.run([sys.executable, "-I", "-B", str(tree / "scripts/ci/test_propimage_linux.py")], text=True, capture_output=True, timeout=600)
    out = proc.stdout + proc.stderr
    return proc.returncode, sorted(set(re.findall(r"^(?:FAIL|ERROR): (\S+)", out, re.M))), out


def main():
    argparse.ArgumentParser(description=__doc__).parse_args()
    test_src = ROOT / "scripts/ci/test_propimage_linux.py"
    known = {n.name for n in ast.walk(ast.parse(test_src.read_text())) if isinstance(n, ast.FunctionDef)}
    assert not {t for c in CASES for t in c[3]} - known, "unknown test selector: %s" % ({t for c in CASES for t in c[3]} - known)
    weak = 0
    with tempfile.TemporaryDirectory(prefix="propimage-linux-mutants-") as tmp:
        tree = Path(tmp)
        for rel in ("scripts/bench/windows/propimage.py", "scripts/bench/windows/clockrule.py", "scripts/bench/windows/clocknet_assert.py", "scripts/bench/linux/auth_stub_app.py",
                    PKGS):   # every file propimage_linux.py imports or loads: a missing one fails the baseline, never a mutant
            (tree / rel).parent.mkdir(parents=True, exist_ok=True)
            shutil.copy(ROOT / rel, tree / rel)
        (tree / "scripts/ci").mkdir(parents=True)
        shutil.copy(test_src, tree / "scripts/ci/test_propimage_linux.py")
        (tree / MAIN).write_text(SRCS[MAIN])
        code, _, out = run(tree)
        if code:
            print("FAIL unmutated baseline\n" + out[-4000:])
            return 2
        print("PASS baseline: " + re.search(r"Ran (\d+) tests", out).group(1), flush=True)
        for name, file, mutated, tests in CASES:
            target = tree / file
            target.write_text(mutated)
            try:
                code, killers, out = run(tree)
            finally:
                target.write_text(SRCS[file])
            names = [k.split(".")[-1] for k in killers]
            ok = code != 0 and len(set(names)) >= 2 and bool(set(tests) & set(names))
            print("%s %s: %d killers %s" % ("PASS" if ok else "WEAK", name, len(set(names)), names[:4]), flush=True)
            weak += not ok
    print("%d/%d mutants killed by at least two tests" % (len(CASES) - weak, len(CASES)))
    return 1 if weak else 0


if __name__ == "__main__":
    sys.exit(main())
