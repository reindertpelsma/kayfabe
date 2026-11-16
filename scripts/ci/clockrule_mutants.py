#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later
"""clockrule.py and win_vm.sh clock-rule mutations, each requiring two independently named killers (GPU-free, VM-free).

STATUS: LIVE as tests, 2026-11-16. Each mutant changes one rule of scripts/bench/windows/clockrule.py (or the win_vm.sh wiring);
the whole of scripts/ci/test_clockrule.py runs on the mutated tree and at least two tests must fail, one of them named here.
"""
import argparse
import ast
import re
import shutil
import subprocess
import sys
import tempfile
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
CLOCK = (ROOT / "scripts/bench/windows/clockrule.py").read_text()
WINVM = (ROOT / "scripts/bench/windows/win_vm.sh").read_text()


def case(name, old, new, *tests, target="clock"):
    src = CLOCK if target == "clock" else WINVM
    assert src.count(old) == 1, (name, old, src.count(old))
    return name, target, src.replace(old, new), tests


CASES = [
    case("online-recorded-allowed", 'if net == "online" and kind == "recorded":', 'if False:',
         "test_online_with_a_recorded_clock_is_always_refused", "test_assert_rule_table"),
    case("strict-ignored", 'if net == "offline" and kind == "real" and strict:', 'if False:',
         "test_offline_without_sidecar_strict_is_refused", "test_assert_rule_table"),
    case("strict-refuses-lenient", 'if net == "offline" and kind == "real" and strict:', 'if net == "offline" and kind == "real":',
         "test_offline_without_sidecar_lenient_is_real_with_one_named_warn", "test_assert_rule_table"),
    case("online-uses-sidecar", 'elif net == "offline" and phase == "run":\n        recorded, build_inputs',
         'elif phase == "run":\n        recorded, build_inputs',
         "test_online_with_sidecar_is_real_in_both_strictness", "test_online_boots_the_real_clock_even_with_a_record"),
    case("no-record-silent", 'out["verdict"] = "WARN"\n            out["lines"].append("KF_CLOCK_NO_RECORD image=%s using=real" % image)',
         'out["verdict"] = "PASS"',
         "test_offline_without_sidecar_lenient_is_real_with_one_named_warn", "test_rtc_prints_one_value_and_named_lines_on_stderr"),
    case("build-phase-warns", 'if net == "offline" and phase == "run":  # lenient', 'if net == "offline":  # lenient',
         "test_build_phase_runs_the_real_clock_without_a_warning_even_strict", "test_the_install_boot_runs_the_real_clock_without_a_warning"),
    case("bad-sidecar-as-none", 'raise ClockError("bad_record", "%s: %s" % (p, e))', 'return None, []',
         "test_a_bad_sidecar_is_refused_not_read_as_none", "test_a_corrupt_clock_json_is_refused_not_ignored"),
    case("clock-not-vm", 'out["rtc"] = "base=%s,clock=vm%s"', 'out["rtc"] = "base=%s%s"',
         "test_offline_with_sidecar_is_recorded_in_both_strictness", "test_offline_with_a_record_boots_the_recorded_clock"),
    case("floor-without-margin", 'if i["created"] + delta > best:\n            best, who = i["created"] + delta, i["name"]\n    return best, who\n\n\ndef clock_for',
         'if i["created"] > best:\n            best, who = i["created"], i["name"]\n    return best, who\n\n\ndef clock_for',
         "test_software_newer_than_the_record_raises_the_clock_by_the_margin", "test_software_created_at_the_recorded_instant_still_gets_the_margin"),
    case("floor-takes-first-not-max", 'if i["created"] + delta > best:', 'if best == base:',
         "test_many_inputs_the_newest_sets_the_floor", "test_margin_is_a_parameter"),
    case("floor-ignored", 'start, who = floor_for(recorded, inputs, margin_s, refuse=base is not None)', 'start, who = recorded, None',
         "test_software_newer_than_the_record_raises_the_clock_by_the_margin", "test_loaded_software_raises_the_clock_and_assets_do_not"),
    case("floor-lowers-clock", 'best, who = base, None\n    for i in inputs:\n        if i["created"] + delta > best:',
         'best, who = base, None\n    for i in inputs:\n        if i["created"] + delta != best:',
         "test_software_older_than_the_record_does_not_move_the_clock", "test_software_just_inside_the_margin_raises_just_to_the_margin"),
    case("before-input-not-refused", 'if refuse and base < i["created"]:', 'if False:',
         "test_explicit_base_earlier_than_an_input_is_refused", "test_before_input_is_named_on_the_line"),
    case("sidecar-refused-on-stale", 'floor_for(recorded, inputs, margin_s, refuse=base is not None)', 'floor_for(recorded, inputs, margin_s, refuse=True)',
         "test_a_stale_record_is_raised_not_refused", "test_loaded_software_raises_the_clock_and_assets_do_not"),
    case("record-allows-earlier-guest-time", 'if refuse and base < i["created"]:', 'if refuse and base < i["created"] and False:',
         "test_record_earlier_than_a_loaded_input_is_refused", "test_explicit_base_earlier_than_an_input_is_refused"),
    case("record-drops-floor-input", '"margin_s": margin_s, "floor_input": who,', '"margin_s": margin_s, "floor_input": None,',
         "test_the_sidecar_remembers_which_input_set_the_floor", "test_record_inputs_set_the_floor_input_and_the_next_boot"),
    case("sidecar-inputs-ignored", 'inputs = build_inputs + run_inputs', 'inputs = run_inputs',
         "test_the_sidecar_remembers_which_input_set_the_floor", "test_record_inputs_set_the_floor_input_and_the_next_boot"),
    case("pin-ignores-loads-software", 'if node.get("loads_software") is True and name in names', 'if name in names',
         "test_a_pin_without_loads_software_is_ignored_for_the_floor", "test_pins_option_applies_only_to_software_pins"),
    case("pin-over-arg", 'if stated:\n            created, source = parse_utc(stated), "arg"\n        else:',
         'if False:\n            pass\n        else:',
         "test_an_explicit_created_beats_the_pin", "test_software_newer_than_the_record_raises_the_clock_by_the_margin"),
    case("mtime-fallback-silent", '"mtime", True', '"mtime", False',
         "test_missing_created_utc_falls_back_to_mtime_with_a_warn", "test_mtime_fallback_gives_warn_in_the_json_verdict"),
    case("conflicting-pins-accepted", 'if len(found) > 1:', 'if False:',
         "test_two_pin_values_for_one_software_are_refused", "test_a_pin_with_two_values_exits_2_and_two_pin_files_that_disagree_too"),
    case("base-without-z-rejected", 'a.base += "Z"  # QEMU', 'pass  # QEMU',
         "test_base_without_z_is_utc", "test_offline_explicit_base_is_the_recorded_clock"),
    # the win_vm.sh wiring
    case("winvm-always-real", '-rtc "$RTC" -global', '-rtc "base=utc,driftfix=slew" -global',
         "test_offline_with_a_record_boots_the_recorded_clock", "test_loaded_software_raises_the_clock_and_assets_do_not", target="winvm"),
    case("winvm-strict-dropped", '--strict "${WINVM_CLOCK_STRICT:-0}"', '--strict 0',
         "test_strict_refuses_offline_without_a_record_and_starts_nothing", "test_the_strict_value_must_be_zero_or_one", target="winvm"),
    case("winvm-install-not-build", 'install_boot_qa() {\n  CLOCK_PHASE=build', 'install_boot_qa() {\n  CLOCK_PHASE=run',
         "test_the_install_boot_runs_the_real_clock_without_a_warning", "test_the_install_boot_ignores_the_record", target="winvm"),
    case("winvm-assets-are-software", 'read -r -a soft <<< "${WINVM_LOADED_SOFTWARE:-}"',
         'read -r -a soft <<< "${WINVM_LOADED_SOFTWARE:-} ${WINVM_APPS_ISO:-}"',
         "test_the_app_disk_asset_never_moves_the_clock", "test_an_asset_does_not_even_warn_about_a_missing_created_utc", target="winvm"),
]


def run(tree):
    proc = subprocess.run([sys.executable, "-I", "-B", str(tree / "scripts/ci/test_clockrule.py")], text=True, capture_output=True, timeout=600)
    out = proc.stdout + proc.stderr
    return proc.returncode, sorted(set(re.findall(r"^(?:FAIL|ERROR): (\S+)", out, re.M))), out


def main():
    argparse.ArgumentParser(description=__doc__).parse_args()
    test_src = ROOT / "scripts/ci/test_clockrule.py"
    known = {n.name for n in ast.walk(ast.parse(test_src.read_text())) if isinstance(n, ast.FunctionDef)}
    missing = {t for c in CASES for t in c[3] if not any(k == t or k.startswith(t) for k in known)}
    assert not missing, "unknown test selector %s" % sorted(missing)
    weak = 0
    with tempfile.TemporaryDirectory(prefix="clockrule-mutants-") as tmp:
        tree = Path(tmp)
        (tree / "scripts/bench/windows").mkdir(parents=True)
        (tree / "scripts/ci").mkdir(parents=True)
        shutil.copytree(ROOT / "scripts/lib", tree / "scripts/lib")
        shutil.copy(test_src, tree / "scripts/ci/test_clockrule.py")
        for f in ("qmp.py", "check.ps1"):
            if (ROOT / "scripts/bench/windows" / f).exists():
                shutil.copy(ROOT / "scripts/bench/windows" / f, tree / "scripts/bench/windows" / f)
        targets = {"clock": tree / "scripts/bench/windows/clockrule.py", "winvm": tree / "scripts/bench/windows/win_vm.sh"}
        targets["clock"].write_text(CLOCK)
        targets["winvm"].write_text(WINVM)
        code, _, out = run(tree)
        if code:
            print("FAIL unmutated baseline\n" + out[-4000:])
            return 2
        print("PASS baseline: " + re.search(r"Ran (\d+) tests", out).group(1), flush=True)
        for name, target, mutated, tests in CASES:
            path = targets[target]
            original = CLOCK if target == "clock" else WINVM
            path.write_text(mutated)
            try:
                code, killers, out = run(tree)
            finally:
                path.write_text(original)
            names = [k.split(".")[-1] for k in killers]
            ok = code != 0 and len(killers) >= 2 and any(t in names or any(n.startswith(t) for n in names) for t in tests)
            print("%s %s: %d killers %s" % ("PASS" if ok else "WEAK", name, len(killers), names[:4]), flush=True)
            weak += not ok
    print("%d/%d mutants killed by at least two tests" % (len(CASES) - weak, len(CASES)))
    return 1 if weak else 0


if __name__ == "__main__":
    sys.exit(main())
