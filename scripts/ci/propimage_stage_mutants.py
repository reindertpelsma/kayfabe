#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later
"""propimage_stage.py mutations (2026-11-19): each is one exact-once text replacement in a private copy of the module, and BOTH named tests of
test_propimage_stage.py must fail on it (the repository's mutant rule: two named killers per mutant). The baseline must pass first."""
import argparse
import re
import shutil
import subprocess
import sys
import tempfile
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
HERE = ROOT / "scripts/ci"
SRC = (HERE / "propimage_stage.py").read_text()
TEST = HERE / "test_propimage_stage.py"
V = "test_propimage_stage."


def case(name, old, new, *tests):
    edits = old if type(old) is list else [(old, new)]
    text = SRC
    for o, n in edits:
        assert text.count(o) == 1, (name, o, text.count(o))
        text = text.replace(o, n)
    assert len(set(tests)) >= 2, name
    return name, text, tests


CASES = [
    # ---- the boot planner
    case("budget-boundary-exclusive", "total + t > budget", "total + t >= budget", "test_a_sum_of_exactly_the_budget_still_fits", "test_the_default_budget_is_the_first_stages"),
    case("budget-ignored", "if cur and total + t > budget:", "if False:", "test_a_row_that_would_exceed_the_budget_starts_the_next_boot", "test_the_default_budget_is_the_first_stages"),
    case("total-not-accumulated", "        cur.append(row)\n        total += t", "        cur.append(row)\n        total += 0", "test_a_row_that_would_exceed_the_budget_starts_the_next_boot",
         "test_the_default_budget_is_the_first_stages"),
    case("default-budget-changed", "BOOT_BUDGET_S = 2700 ", "BOOT_BUDGET_S = 5400 ", "test_the_default_budget_is_the_first_stages", "test_the_budget_constant_is_the_first_stages"),
    case("discovery-row-not-solo", '(lanes and lanes.get(row) == "discovery") or row in solo', "row in solo", "test_a_discovery_row_boots_alone_between_the_others",
         "test_second_image_plan_is_byte_identical_to_the_golden"),
    case("solo-row-not-solo", '(lanes and lanes.get(row) == "discovery") or row in solo', '(lanes and lanes.get(row) == "discovery")', "test_a_named_solo_row_boots_alone_in_its_place",
         "test_first_image_plan_is_byte_identical_to_the_golden"),
    case("solo-row-does-not-flush", "            if cur:\n                boots.append(cur)\n                cur, total = [], 0\n            boots.append([row])",
         "            boots.append([row])", "test_a_discovery_row_boots_alone_between_the_others", "test_a_solo_row_at_the_end_is_last_and_alone"),
    case("solo-row-keeps-the-total", "                boots.append(cur)\n                cur, total = [], 0\n            boots.append([row])", "                boots.append(cur)\n                cur = []\n            boots.append([row])",
         "test_a_discovery_row_resets_the_running_total", "test_the_solo_row_does_not_reset_the_running_total_of_the_boot_after_it"),
    case("last-boot-dropped", "    if cur:\n        boots.append(cur)\n    return boots", "    return boots", "test_rows_share_a_boot_while_the_budget_holds", "test_the_order_of_the_rows_is_kept"),
    # ---- the rows
    case("only-ignored", "    if only is not None:\n        refuse(only and all(r in checks for r in only), \"ROW_UNKNOWN\")\n        order = tuple(r for r in checks if r in only)",
         "    if False:\n        order = ()", "test_only_selects_rows_in_table_order", "test_a_plan_can_be_restricted_to_rows"),
    case("only-order-is-the-callers", "order = tuple(r for r in checks if r in only)", "order = tuple(only)", "test_only_selects_rows_in_table_order", "test_a_plan_can_be_restricted_to_rows"),
    case("only-unknown-allowed", 'refuse(only and all(r in checks for r in only), "ROW_UNKNOWN")', 'refuse(True, "ROW_UNKNOWN")', "test_only_with_an_unknown_row_is_refused", "test_an_empty_only_is_refused"),
    case("entry-rows-unchecked", 'refuse(all(r in checks for r in run["rows"]), "ROW_UNKNOWN")', "pass", "test_entry_rows_the_table_lacks_are_refused", "test_an_entry_row_the_table_lacks_refuses_the_plan"),
    case("entry-rows-ignored", 'elif "rows" in run:', "elif False:", "test_the_entry_rows_win_and_keep_their_order", "test_solo_rows_of_the_entry_split_the_boots"),
    case("disabled-rows-run", 'order = tuple(r for r in checks if not checks[r].get("disabled"))', "order = tuple(checks)", "test_without_rows_the_enabled_rows_of_the_table_in_table_order",
         "test_a_windows_plan_has_rows_lanes_timeouts_boots_and_commands"),
    case("disabled-and-run-rows-reported-as-not-run", 'if c.get("disabled") and r not in order}', 'if c.get("disabled")}', "test_only_may_name_a_disabled_row_and_then_it_is_not_reported_as_not_run",
         "test_an_entry_row_that_is_disabled_is_planned_and_not_reported_as_not_run"),
    case("not-run-reason-untrimmed", 'str(c["disabled"])[:160]', 'str(c["disabled"])', "test_disabled_rows_are_reported_with_their_reason_trimmed_to_160", "test_a_reason_of_161_characters_loses_its_last_one"),
    case("not-run-reason-trimmed-short", 'str(c["disabled"])[:160]', 'str(c["disabled"])[:16]', "test_disabled_rows_are_reported_with_their_reason_trimmed_to_160",
         "test_a_windows_plan_has_rows_lanes_timeouts_boots_and_commands"),
    case("no-rows-allowed", 'refuse(order, "NO_ROWS")', "pass", "test_a_table_with_nothing_enabled_is_refused", "test_a_plan_of_a_table_with_nothing_enabled_is_refused"),
    case("lane-defaults-to-discovery", 'table.CHECKS[r].get("lane") or "gate"', 'table.CHECKS[r].get("lane") or "discovery"', "test_a_windows_plan_has_rows_lanes_timeouts_boots_and_commands",
         "test_first_image_plan_is_byte_identical_to_the_golden"),
    # ---- the harness command
    case("display-word-dropped", '"--gop-efi", a["gop"], "--display", "broker"]', '"--gop-efi", a["gop"]]', "test_the_fixed_words", "test_first_image_plan_is_byte_identical_to_the_golden"),
    case("before-apps-dropped", 'cmd += args.get("before_apps", [])', "pass", "test_before_apps_come_between_the_display_and_the_apps", "test_second_image_plan_is_byte_identical_to_the_golden"),
    case("data-volume-always", 'if run.get("data_iso"):', "if True:", "test_no_data_volume_when_the_entry_has_none", "test_first_image_plan_is_byte_identical_to_the_golden"),
    case("data-volume-never", 'if run.get("data_iso"):', "if False:", "test_the_data_volume_follows_before_apps_and_precedes_the_apps", "test_second_image_plan_is_byte_identical_to_the_golden"),
    case("data-volume-after-the-apps", '    if run.get("data_iso"):\n        cmd += ["--data-iso", a["data"]]\n    cmd += ["--apps", ",".join(a["apps"])]',
         '    cmd += ["--apps", ",".join(a["apps"])]\n    if run.get("data_iso"):\n        cmd += ["--data-iso", a["data"]]', "test_the_data_volume_follows_before_apps_and_precedes_the_apps",
         "test_second_image_plan_is_byte_identical_to_the_golden"),
    case("after-apps-dropped", 'cmd += args.get("after_apps", [])', "pass", "test_after_apps_follow_the_app_list_then_ram_then_audio", "test_second_image_plan_is_byte_identical_to_the_golden"),
    case("ram-ignored", 'if run.get("guest_ram_mb"):', "if False:", "test_after_apps_follow_the_app_list_then_ram_then_audio", "test_second_image_plan_is_byte_identical_to_the_golden"),
    case("zero-ram-emitted", 'if run.get("guest_ram_mb"):', 'if "guest_ram_mb" in run:', "test_zero_ram_and_no_audio_add_nothing", "test_zero_guest_ram_adds_no_ram_words_to_a_plan"),
    case("audio-always", 'if run.get("audio"):', "if True:", "test_zero_ram_and_no_audio_add_nothing", "test_first_image_plan_is_byte_identical_to_the_golden"),
    case("audio-never", 'if run.get("audio"):', "if False:", "test_after_apps_follow_the_app_list_then_ram_then_audio", "test_second_image_plan_is_byte_identical_to_the_golden"),
    case("per-row-words-for-every-row", '    for row in a["apps"]:\n        cmd += args.get("per_row", {}).get(row, [])', '    for row in args.get("per_row", {}):\n        cmd += args["per_row"][row]',
         "test_per_row_words_only_for_rows_on_the_boot_in_boot_order", "test_first_image_plan_is_byte_identical_to_the_golden"),
    case("per-row-words-reversed", '    for row in a["apps"]:\n        cmd += args.get("per_row", {}).get(row, [])', '    for row in reversed(a["apps"]):\n        cmd += args.get("per_row", {}).get(row, [])',
         "test_per_row_words_only_for_rows_on_the_boot_in_boot_order", "test_per_row_words_follow_the_boot_row_order"),
    case("per-row-words-before-after-apps", '    cmd += args.get("after_apps", [])\n', '    for row in a["apps"]:\n        cmd += args.get("per_row", {}).get(row, [])\n    cmd += args.get("after_apps", [])\n',
         "test_per_row_words_come_after_after_apps_and_audio", "test_per_row_words_only_for_rows_on_the_boot_in_boot_order"),
    case("arm-words-always", '    if arm == "vfio":\n        cmd +=', "    if True:\n        cmd +=", "test_the_default_arm_adds_nothing", "test_first_image_plan_is_byte_identical_to_the_golden"),
    case("arm-words-never", '    if arm == "vfio":\n        cmd +=', "    if False:\n        cmd +=", "test_the_vfio_arm_adds_the_platform_and_the_gpu_last", "test_plan_with_the_vfio_arm"),
    case("arm-words-wrong-platform", '["--platform", "vfio", "--vfio-gpu", vfio_gpu]', '["--platform", "kf3", "--vfio-gpu", vfio_gpu]', "test_the_vfio_arm_adds_the_platform_and_the_gpu_last",
         "test_the_vfio_arm_changes_only_the_tail_of_each_committed_command"),
    case("command-skips-arm-check", "    check_arm(arm, vfio_gpu)\n    args = run.get", "    args = run.get", "test_an_unknown_arm_is_refused", "test_vfio_needs_a_pci_address_of_function_zero"),
    # ---- the arm
    case("arm-unchecked", 'refuse(arm in ARMS, "ARM:" + str(arm)[:16])', "pass", "test_an_unknown_arm_is_refused", "test_the_arm_is_checked_before_the_rows_are_read"),
    case("gpu-without-vfio-allowed", 'refuse(arm == "vfio" or vfio_gpu is None, "VFIO_GPU_WITHOUT_VFIO")', "pass", "test_a_gpu_address_without_the_vfio_arm_is_refused",
         "test_a_plan_refuses_a_gpu_address_on_the_default_arm"),
    case("gpu-address-unchecked", 'refuse(arm != "vfio" or (type(vfio_gpu) is str and PCI_RE.fullmatch(vfio_gpu)), "VFIO_GPU")', "pass", "test_vfio_needs_a_pci_address_of_function_zero",
         "test_vfio_without_an_address_exits_2"),
    case("gpu-function-not-zero", 'PCI_RE = re.compile(r"[0-9a-f]{4}:[0-9a-f]{2}:[0-9a-f]{2}\\.0")', 'PCI_RE = re.compile(r"[0-9a-f]{4}:[0-9a-f]{2}:[0-9a-f]{2}\\.[0-7]")',
         "test_vfio_needs_a_pci_address_of_function_zero", "test_a_plan_refuses_function_one_of_the_gpu"),
    case("gpu-address-prefix-match", 'PCI_RE.fullmatch(vfio_gpu)', 'PCI_RE.match(vfio_gpu)', "test_vfio_needs_a_pci_address_of_function_zero", "test_a_plan_refuses_a_gpu_address_with_trailing_text"),
    # ---- the plan
    case("fetch-only-bundle-planned", 'refuse("run" in bundle, "NO_RUN:" + bundle_id)', "pass", "test_a_fetch_only_bundle_has_no_plan", "test_plan_of_a_fetch_only_bundle_exits_2"),
    case("unknown-bundle-unchecked", 'refuse(bundle_id in config["bundles"], "UNKNOWN_BUNDLE:" + str(bundle_id)[:32])', "pass", "test_an_unknown_bundle_is_refused_by_name",
         "test_plan_of_an_unknown_bundle_exits_2"),
    case("config-unvalidated", "    pc.validate(config)\n    bundle = bundle_of", "    bundle = bundle_of", "test_an_invalid_configuration_is_refused_first", "test_a_plan_refuses_a_missing_schema"),
    case("vfio-on-linux-allowed", 'refuse(arm == "kf3" or bundle["os"] == "windows", "ARM_NOT_SUPPORTED:" + bundle["os"])', "pass", "test_the_vfio_arm_is_for_windows_bundles",
         "test_plan_of_a_linux_bundle_with_the_vfio_arm_exits_2"),
    case("isolation-on-windows-allowed", 'refuse(isolation is None or bundle["os"] == "linux", "ISOLATION_ON_WINDOWS")', "pass", "test_isolation_is_not_a_windows_setting",
         "test_plan_of_a_windows_bundle_with_an_isolation_exits_2"),
    case("os-dispatch-swapped", '    if bundle["os"] == "windows":\n        return {**head', '    if bundle["os"] == "linux":\n        return {**head', "test_a_windows_plan_has_rows_lanes_timeouts_boots_and_commands",
         "test_a_linux_plan_has_the_rows_the_timeouts_and_the_boots"),
    case("plan-head-arm-dropped", 'head = {"bundle": bundle_id, "os": bundle["os"], "arm": arm}', 'head = {"bundle": bundle_id, "os": bundle["os"], "arm": "kf3"}',
         "test_the_arm_is_in_the_plan_and_in_every_command", "test_plan_with_the_vfio_arm"),
    case("plan-passes-no-arm-to-commands", "plan_windows(bundle, root, only, arm, vfio_gpu)}", "plan_windows(bundle, root, only)}", "test_the_arm_is_in_the_plan_and_in_every_command",
         "test_the_vfio_arm_changes_only_the_tail_of_each_committed_command"),
    case("commands-share-the-work-directory", 'work="%s/boot%d" % (PATHS["work"], index)', 'work=PATHS["work"]', "test_the_commands_of_a_plan_name_their_own_boot_directories",
         "test_first_image_plan_is_byte_identical_to_the_golden"),
    case("commands-share-the-out-directory", 'out="%s/boot%d" % (PATHS["out"], index)', 'out=PATHS["out"]', "test_the_commands_of_a_plan_name_their_own_boot_directories",
         "test_second_image_plan_is_byte_identical_to_the_golden"),
    case("solo-rows-ignored", 'solo=run.get("solo_rows", ())', "solo=()", "test_solo_rows_of_the_entry_split_the_boots", "test_first_image_plan_is_byte_identical_to_the_golden"),
    case("timeouts-from-the-wrong-key", 'timeouts = {r: table.CHECKS[r]["timeout"] for r in order}', 'timeouts = {r: 1 for r in order}', "test_a_windows_plan_has_rows_lanes_timeouts_boots_and_commands",
         "test_second_image_plan_is_byte_identical_to_the_golden"),
    # ---- linux
    case("linux-only-unchecked", 'refuse(only and all(r in mod.ROWS for r in only), "ROW_UNKNOWN")', "pass", "test_a_linux_row_the_module_lacks_is_refused", "test_an_empty_linux_row_request_is_refused"),
    case("linux-only-keeps-the-preamble", "rows = [r for r in mod.ROW_ORDER if r in only and r != mod.PREAMBLE]", "rows = [r for r in mod.ROW_ORDER if r in only]",
         "test_the_preamble_is_never_a_requested_row", "test_a_request_for_only_the_preamble_has_no_rows"),
    case("linux-only-in-the-callers-order", "rows = [r for r in mod.ROW_ORDER if r in only and r != mod.PREAMBLE]", "rows = [r for r in only if r != mod.PREAMBLE]",
         "test_a_linux_plan_can_be_restricted_to_rows_in_the_modules_order", "test_a_linux_plan_can_be_restricted_to_rows_in_the_modules_order_twice"),
    case("linux-all-rows-keeps-the-preamble", "all_rows = [r for r in mod.ROW_ORDER if r != mod.PREAMBLE]", "all_rows = list(mod.ROW_ORDER)",
         "test_a_linux_plan_has_the_rows_the_timeouts_and_the_boots", "test_matlab_plan_is_byte_identical_to_the_golden_for_both_isolations"),
    case("linux-no-rows-allowed", 'refuse(rows, "NO_ROWS")\n    isolation', "isolation", "test_a_request_for_only_the_preamble_has_no_rows", "test_a_linux_table_with_only_a_preamble_has_no_plan"),
    case("linux-isolation-from-the-wrong-place", 'isolation = isolation or run["isolation"]', 'isolation = isolation or "shared"', "test_a_linux_plan_has_the_rows_the_timeouts_and_the_boots",
         "test_matlab_plan_is_byte_identical_to_the_golden_for_both_isolations"),
    case("linux-isolation-unchecked", 'refuse(isolation in pc.ISOLATIONS, "ISOLATION")', "pass", "test_a_bad_isolation_is_refused", "test_an_isolation_in_the_wrong_case_is_refused"),
    case("linux-timeouts-wrong-column", "{r: mod.ROWS[r][1] for r in rows}", "{r: mod.ROWS[r][0] for r in rows}", "test_a_linux_plan_has_the_rows_the_timeouts_and_the_boots",
         "test_matlab_plan_is_byte_identical_to_the_golden_for_both_isolations"),
    case("linux-boots-ignore-isolation", '"boots": mod.plan_boots(rows, isolation)}', '"boots": mod.plan_boots(rows, "row")}', "test_a_linux_plan_can_share_one_boot",
         "test_matlab_plan_is_byte_identical_to_the_golden_for_both_isolations"),
    # ---- the command line
    case("cli-apps-ignored", "only = [x for x in a.apps.split(\",\") if x] if a.apps is not None else None", "only = None", "test_apps_restricts_the_rows",
         "test_plan_with_the_vfio_arm_and_apps"),
    case("cli-refusal-escapes", "    except (pc.ConfigError, StageError) as exc:", "    except pc.ConfigError as exc:", "test_plan_of_a_fetch_only_bundle_exits_2", "test_vfio_without_an_address_exits_2"),
    case("cli-exit-status-zero-on-refusal", '        print("KF_PROPIMAGE_STAGE_REFUSED " + exc.reason, flush=True)\n        return 2', '        print("KF_PROPIMAGE_STAGE_REFUSED " + exc.reason, flush=True)\n        return 0',
         "test_plan_of_a_fetch_only_bundle_exits_2", "test_a_bad_config_exits_2"),
]


def run(tree):
    p = subprocess.run([sys.executable, "-B", "-I", str(tree / "scripts/ci" / TEST.name), "-v"], cwd=tree / "scripts/ci", capture_output=True, text=True, timeout=300)
    out = p.stdout + p.stderr
    killers = re.findall(r"^(?:FAIL|ERROR): (test_\w+) ", out, re.M)
    return p.returncode, killers, out


def main():
    argparse.ArgumentParser(description=__doc__).parse_args()
    weak = 0
    with tempfile.TemporaryDirectory(prefix="propimage-stage-mutants-") as tmp:
        tree = Path(tmp)
        (tree / "scripts/ci").mkdir(parents=True)
        for entry in HERE.iterdir():              # everything the tests read stays the repository's own file
            if entry.name not in ("propimage_stage.py", TEST.name, "__pycache__"):
                (tree / "scripts/ci" / entry.name).symlink_to(entry)
        for top in ("ci", "docs"):
            (tree / top).symlink_to(ROOT / top)
        (tree / "scripts/bench").symlink_to(ROOT / "scripts/bench")
        shutil.copy(TEST, tree / "scripts/ci" / TEST.name)
        target = tree / "scripts/ci/propimage_stage.py"
        target.write_text(SRC)
        code, _, out = run(tree)
        if code:
            print("FAIL unmutated baseline\n" + out[-3000:])
            return 2
        known = set(re.findall(r"^(test_\w+) \(", out, re.M))
        unknown = sorted({t for c in CASES for t in c[2]} - known)
        print("PASS baseline: " + re.search(r"Ran (\d+) tests", out).group(1), flush=True)
        for name, mutated, tests in CASES:
            missing = [t for t in tests if t not in known]
            target.write_text(mutated)
            try:
                code, killers, out = run(tree)
            finally:
                target.write_text(SRC)
            ok = code != 0 and not missing and all(t in killers for t in tests)
            print("%s %s: %d failing, named killers %s%s" % ("PASS" if ok else "WEAK", name, len(killers), [t for t in tests if t in killers],
                                                             (" UNKNOWN TEST " + ",".join(missing)) if missing else ""), flush=True)
            weak += not ok
    print("%d/%d mutants killed by both named tests" % (len(CASES) - weak, len(CASES)))
    return 1 if weak or unknown else 0


if __name__ == "__main__":
    sys.exit(main())
