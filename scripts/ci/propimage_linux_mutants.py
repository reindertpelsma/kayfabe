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
SRC = (ROOT / "scripts/bench/linux/propimage_linux.py").read_text()


def case(name, old, new, *tests, count=1):
    assert SRC.count(old) == count, (name, old, SRC.count(old))
    return name, SRC.replace(old, new), tests


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
]


def run(tree):
    proc = subprocess.run([sys.executable, "-I", "-B", str(tree / "scripts/ci/test_propimage_linux.py")], text=True, capture_output=True, timeout=600)
    out = proc.stdout + proc.stderr
    return proc.returncode, sorted(set(re.findall(r"^(?:FAIL|ERROR): (\S+)", out, re.M))), out


def main():
    argparse.ArgumentParser(description=__doc__).parse_args()
    test_src = ROOT / "scripts/ci/test_propimage_linux.py"
    known = {n.name for n in ast.walk(ast.parse(test_src.read_text())) if isinstance(n, ast.FunctionDef)}
    assert not {t for c in CASES for t in c[2]} - known, "unknown test selector: %s" % ({t for c in CASES for t in c[2]} - known)
    weak = 0
    with tempfile.TemporaryDirectory(prefix="propimage-linux-mutants-") as tmp:
        tree = Path(tmp)
        for rel in ("scripts/bench/windows/propimage.py", "scripts/bench/windows/clockrule.py", "scripts/bench/linux/auth_stub_app.py"):
            (tree / rel).parent.mkdir(parents=True, exist_ok=True)
            shutil.copy(ROOT / rel, tree / rel)
        (tree / "scripts/ci").mkdir(parents=True)
        shutil.copy(test_src, tree / "scripts/ci/test_propimage_linux.py")
        target = tree / "scripts/bench/linux/propimage_linux.py"
        target.write_text(SRC)
        code, _, out = run(tree)
        if code:
            print("FAIL unmutated baseline\n" + out[-4000:])
            return 2
        print("PASS baseline: " + re.search(r"Ran (\d+) tests", out).group(1), flush=True)
        for name, mutated, tests in CASES:
            target.write_text(mutated)
            try:
                code, killers, out = run(tree)
            finally:
                target.write_text(SRC)
            names = [k.split(".")[-1] for k in killers]
            ok = code != 0 and len(set(names)) >= 2 and bool(set(tests) & set(names))
            print("%s %s: %d killers %s" % ("PASS" if ok else "WEAK", name, len(set(names)), names[:4]), flush=True)
            weak += not ok
    print("%d/%d mutants killed by at least two tests" % (len(CASES) - weak, len(CASES)))
    return 1 if weak else 0


if __name__ == "__main__":
    sys.exit(main())
