#!/usr/bin/env python3
"""Mutation checks of the clock/network assert and its launch paths (2026-11-16): every mutant needs two named killers (stdlib, GPU-free).

Each mutant is one exact-once text replacement in a private copy of clocknet_assert.py, propimage.py, propimage_images.py,
propimage_run.sh or win_vm.sh (the whole of scripts/ci plus the harness files they import are copied); the named tests of
test_clocknet_assert.py run against the copy and the failing ones are the killers. The unmutated copy must be green first.

    python3 scripts/ci/clocknet_assert_mutants.py [--jobs N] [substring-of-mutant-name]
"""
import argparse
import concurrent.futures
import json
from pathlib import Path
import shutil
import subprocess
import sys
import tempfile

HERE = Path(__file__).resolve().parent
ROOT = HERE.parents[1]
FILES = {
    "cna": "scripts/bench/windows/clocknet_assert.py",
    "pi": "scripts/bench/windows/propimage.py",
    "images": "scripts/ci/propimage_images.py",
    "run_sh": "scripts/bench/windows/propimage_run.sh",
    "winvm": "scripts/bench/windows/win_vm.sh",
}
SOURCES = {key: (ROOT / path).read_text() for key, path in FILES.items()}
SUITE = "test_clocknet_assert."
COPY_DIRS = ["scripts/ci", "scripts/kfci"]
COPY_FILES = [
    "scripts/bench/windows/propimage.py",
    "scripts/bench/windows/propimage_check.py",
    "scripts/bench/windows/propimage_plain.py",
    "scripts/bench/windows/propimage_registry.py",
    "scripts/bench/windows/propimage_runspec.py",
    "scripts/bench/windows/propimage_run.sh",
    "scripts/bench/windows/propimage_autounattend.xml.in",
    "scripts/bench/windows/win_vm.sh",
    "scripts/bench/windows/clocknet_assert.py",
    "scripts/bench/windows/app_defs/core_apps.py",
    "scripts/bench/windows/kf3_flags.sample.env",
]
COLLECT = """import io, json, sys, unittest
suite = unittest.defaultTestLoader.loadTestsFromNames(sys.argv[1:])
r = unittest.TextTestRunner(stream=io.StringIO()).run(suite)
print('KF_MUTATION_RESULT=' + json.dumps({'tests': r.testsRun, 'killers': [t.id() for t, _ in r.failures + r.errors]}))
"""
PL, AR, CL, PB, HP, SH, CN = (SUITE + n for n in ("Plan.", "Argv.", "Cli.", "PropimageBuildArgv.", "HarnessProof.", "ShellLaunchers.", "PlainLauncher."))


def M(name, key, old, new, *tests):
    return (name, key, old, new, list(tests))


MUTANTS = [
    # the assert itself: two checks, each pinned by its own tests
    M("fake-clock-check-removed", "cna", "    if clock.faked and not net.off:", "    if False:",
      PL + "test_faked_clock_with_internet_is_refused_by_the_fake_clock_check", AR + "test_a_recorded_base_with_restrict_off_is_refused"),
    M("real-clock-check-removed", "cna", "    if not net.off and not real_utc_now(clock, now):", "    if False:",
      PL + "test_internet_with_a_stale_base_is_refused_by_the_real_clock_check", PL + "test_a_vm_time_clock_is_not_real_with_internet"),
    M("fake-clock-check-inverted", "cna", "    if clock.faked and not net.off:", "    if clock.faked and net.off:",
      PL + "test_faked_clock_with_the_network_off_is_fine", AR + "test_a_recorded_base_with_restrict_on_passes", PB + "test_the_modes_have_the_expected_shapes"),
    M("real-clock-check-inverted", "cna", "    if not net.off and not real_utc_now(clock, now):", "    if not net.off and real_utc_now(clock, now):",
      PL + "test_real_clock_with_the_network_off_or_on_is_fine", AR + "test_internet_with_the_real_clock_passes"),
    M("restrict-defaults-to-on", "cna", 'kv.get("restrict", "off").lower() not in RESTRICT_ON', 'kv.get("restrict", "on").lower() not in RESTRICT_ON',
      AR + "test_a_recorded_base_with_no_restrict_option_is_refused", AR + "test_nic_user_restrict_forms"),
    M("any-restrict-value-is-on", "cna", 'kv.get("restrict", "off").lower() not in RESTRICT_ON', 'False',
      AR + "test_restrict_spellings", AR + "test_a_recorded_base_with_restrict_off_is_refused"),
    M("default-nic-counts-as-no-network", "cna", 'return Net(False, "QEMU adds a default user NIC")', 'return Net(True, "x")',
      AR + "test_the_default_nic_is_internet", CL + "test_a_line_without_any_network_option_is_internet"),
    M("any-backend-type-is-fine", "cna", '        if kind != "user":', "        if False:",
      AR + "test_a_non_user_backend_is_internet_even_with_a_restrict_option", AR + "test_nic_with_another_backend_is_internet"),
    M("clock-vm-ignored", "cna", 'vm = kv.get("clock", "host").lower() not in ("host", "rt")', "vm = False",
      AR + "test_clock_vm_with_internet_is_refused", PB + "test_a_clock_that_does_not_follow_the_host_in_a_networked_mode_is_refused"),
    M("explicit-base-counts-as-real", "cna", "return Clock(faked=True, base=dt.datetime.strptime(kv[\"base\"], fmt)", "return Clock(faked=False, base=dt.datetime.strptime(kv[\"base\"], fmt)",
      AR + "test_a_recorded_base_with_restrict_off_is_refused", AR + "test_a_recorded_date_only_base_is_refused_with_internet"),
    M("tolerance-unbounded", "cna", "REAL_TOLERANCE_S = 300 ", "REAL_TOLERANCE_S = 10 ** 9 ",
      PL + "test_a_base_within_the_tolerance_counts_as_now", PL + "test_internet_with_a_stale_base_is_refused_by_the_real_clock_check"),
    M("net-nic-without-backend-is-fine", "cna", 'return Net(False, "-net nic without a backend")', 'return Net(True, "x")',
      AR + "test_net_nic_alone_is_not_provably_off"),
    M("rtc-option-ignored", "cna", '    rtc = _values(argv, "rtc")\n    if not rtc:', "    rtc = []\n    if not rtc:",
      AR + "test_a_recorded_base_with_restrict_off_is_refused", CL + "test_argv_form_ok_and_refused"),
    M("cli-always-ok", "cna", "        assert_argv(words, a.who)", "        pass",
      CL + "test_argv_form_ok_and_refused", CL + "test_line_form_with_the_cmd_prefix_and_quotes"),
    # propimage.py build_argv (setup, install, offline, ci, plain)
    M("build-argv-assert-removed", "pi", '        cna.assert_argv(a, "propimage build_argv " + mode, now)', "        pass",
      PB + "test_a_recorded_clock_in_a_networked_mode_is_refused_install", PB + "test_a_recorded_clock_in_plain_with_internet_is_refused"),
    M("build-argv-violation-swallowed", "pi", "    except cna.ClockNetViolation as exc:\n        raise PropError(str(exc)) from exc", "    except cna.ClockNetViolation:\n        pass",
      PB + "test_a_recorded_clock_in_a_networked_mode_is_refused_install", PB + "test_a_recorded_clock_in_plain_with_internet_is_refused", CN + "test_a_recorded_clock_cannot_reach_a_plain_launch_with_internet"),
    M("plain-net-flag-ignored", "pi", '(mode == PLAIN and net == "on")', "False",
      PB + "test_plain_with_net_on_is_real_clock_and_internet", CN + "test_launch_builds_the_plain_argv_and_the_overlay_command"),
    M("plain-clock-recorded", "pi", 'if mode in ("setup", "install", PLAIN):\n        return None', 'if mode in ("setup", "install"):\n        return None',
      PB + "test_plain_default_is_real_clock_and_network_off", PB + "test_plain_with_net_on_is_real_clock_and_internet"),
    # the harness proof of the generic runner
    M("proof-ignores-the-requested-network", "images", 'got["net"] == want_net)', "True)",
      HP + "test_plain_with_internet_when_off_was_requested_is_not_proven", HP + "test_plain_shape_needs_the_real_clock_and_the_requested_network"),
    M("proof-skips-the-assert", "images", 'clock, net = cna.assert_argv(words, "propimage_images " + mode)', "clock, net = cna.clock_from_argv(words), cna.net_from_argv(words)",
      HP + "test_a_forbidden_combination_in_the_file_is_not_proven", HP + "test_state_shape_with_restrict_off_names_no_clock"),
    # the shell launchers
    M("run-sh-does-not-check", "run_sh", '"${PYA[@]}" line "$QCMD" --who propimage_run.sh > /dev/null || { echo "clock/network rule refused this start" >&2; exit 3; }', ":",
      SH + "test_propimage_run_sh_checks_the_line_before_it_execs", SH + "test_propimage_run_sh_refuses_a_forbidden_line_before_starting_anything"),
    M("run-sh-check-never-fails", "run_sh", 'refused this start" >&2; exit 3; }', 'refused this start" >&2; :; }',
      SH + "test_propimage_run_sh_checks_the_line_before_it_execs", SH + "test_propimage_run_sh_refuses_a_forbidden_line_before_starting_anything"),
    M("win-vm-does-not-check", "winvm", '  python3 -B -I "$HERE/clocknet_assert.py" argv --who win_vm.sh -- "$q" "${QA[@]}" "$@" > /dev/null || die "clock/network rule refused this start (clocknet_assert.py)"\n', "",
      SH + "test_win_vm_sh_checks_every_start_in_vm_start", SH + "test_win_vm_sh_start_passes_the_words_it_launches"),
    M("win-vm-checks-other-words", "winvm", 'argv --who win_vm.sh -- "$q" "${QA[@]}" "$@" >', 'argv --who win_vm.sh -- "$q" >',
      SH + "test_win_vm_sh_start_passes_the_words_it_launches", SH + "test_win_vm_sh_checks_every_start_in_vm_start"),
]


def tree(parent, name, overrides):
    root = Path(tempfile.mkdtemp(prefix=name + "-", dir=parent))
    for rel in COPY_DIRS:
        shutil.copytree(ROOT / rel, root / rel, ignore=shutil.ignore_patterns("__pycache__"))
    for rel in COPY_FILES:
        (root / rel).parent.mkdir(parents=True, exist_ok=True)
        shutil.copyfile(ROOT / rel, root / rel)
    for key, text in overrides.items():
        (root / FILES[key]).write_text(text)
    return root


def run_tests(root):
    done = subprocess.run([sys.executable, "-B", "-c", COLLECT, SUITE.rstrip(".")], cwd=root / "scripts/ci", capture_output=True, text=True, timeout=900)
    line = next((s for s in done.stdout.splitlines() if s.startswith("KF_MUTATION_RESULT=")), None)
    if line is None:
        raise SystemExit("harness failure: " + done.stderr[-600:] + done.stdout[-300:])
    return json.loads(line.split("=", 1)[1])


def one(parent, mutant):
    name, key, old, new, wanted = mutant
    text = SOURCES[key]
    if text.count(old) != 1:
        return name, ["anchor x%d: %r" % (text.count(old), old[:50])], wanted
    root = tree(parent, "m", {key: text.replace(old, new)})
    killers = sorted(set(run_tests(root)["killers"]))
    shutil.rmtree(root, ignore_errors=True)
    return name, killers, wanted


def main():
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument("--jobs", type=int, default=4)
    ap.add_argument("only", nargs="?", default="")
    a = ap.parse_args()
    chosen = [m for m in MUTANTS if a.only in m[0]]
    bad = 0
    with tempfile.TemporaryDirectory(prefix="kf-clocknet-mutants-") as parent:
        base = run_tests(tree(parent, "base", {}))
        if base["killers"]:
            print("BASELINE RED: %s" % base["killers"], file=sys.stderr)
            return 2
        print("baseline green: %d tests" % base["tests"], flush=True)
        with concurrent.futures.ThreadPoolExecutor(a.jobs) as pool:
            for name, killers, wanted in pool.map(lambda m: one(parent, m), chosen):
                short = {k.split(".", 1)[-1] for k in killers}
                named = [w for w in wanted if w.split(".", 1)[-1] in short or w in killers]
                ok = len(killers) >= 2 and len(named) >= 2 and not killers[0].startswith("anchor")
                if len(wanted) == 1:
                    ok = len(killers) >= 1 and bool(named) and not killers[0].startswith("anchor")
                print("%s %s: %d killers (%d named)" % ("PASS" if ok else "WEAK", name, len(killers), len(named)), flush=True)
                if not ok:
                    bad += 1
                    print("   killers: %s" % killers[:6], flush=True)
    print("%d/%d mutants killed" % (len(chosen) - bad, len(chosen)))
    return 1 if bad else 0


if __name__ == "__main__":
    sys.exit(main())
