#!/usr/bin/env python3
"""Mutation checks of the generic proprietary-image runner (2026-11-16): every mutant needs two named killers (stdlib, GPU-free).

Each mutant is one exact-once text replacement in a private copy of propimage_runspec.py, propimage_registry.py, propimage_images.py,
propimage_apps.py, vast_payload.py or vast_report.py (the whole of scripts/ci and scripts/kfci is copied, plus the harness files the
runner imports); the named tests of test_propimage_runspec.py / test_propimage_images.py run against the copy and the failing ones are
the killers. The unmutated copy must be green first.

    python3 scripts/ci/propimage_runner_mutants.py [--jobs N] [substring-of-mutant-name]
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
    "spec": "scripts/bench/windows/propimage_runspec.py",
    "reg": "scripts/bench/windows/propimage_registry.py",
    "images": "scripts/ci/propimage_images.py",
    "pa": "scripts/ci/propimage_apps.py",
    "payload": "scripts/ci/vast_payload.py",
    "report": "scripts/ci/vast_report.py",
}
SOURCES = {key: (ROOT / path).read_text() for key, path in FILES.items()}
SUITES = ["test_propimage_runspec", "test_propimage_images", "test_payload_instruction.Propimages"]
COPY_DIRS = ["scripts/ci", "scripts/kfci"]
COPY_FILES = [
    "scripts/bench/native_xid_guard.py",
    "scripts/bench/native_fault_names.json",
    "scripts/bench/specviewperf/lanes.json",
    "scripts/bench/windows/propimage.py",
    "scripts/bench/windows/propimage_check.py",
    "scripts/bench/windows/propimage_plain.py",
    "scripts/bench/windows/propimage_registry.py",
    "scripts/bench/windows/propimage_runspec.py",
    "scripts/bench/windows/clocknet_assert.py",
    "scripts/bench/windows/app_defs/core_apps.py",
    "scripts/bench/windows/propimage_autounattend.xml.in",
    "scripts/bench/windows/kf3_flags.sample.env",
    "ci/vast-broker/planner/schemas/instruction.schema.json",
    "ci/vast-broker/release-matrix.json",
    "ci/vast-broker/defaults.json",
]
COLLECT = """import io, json, sys, unittest
suite = unittest.defaultTestLoader.loadTestsFromNames(sys.argv[1:])
r = unittest.TextTestRunner(stream=io.StringIO()).run(suite)
print('KF_MUTATION_RESULT=' + json.dumps({'tests': r.testsRun, 'killers': [t.id() for t, _ in r.failures + r.errors]}))
"""
RS, V, F, FR, PL, RG = ("test_propimage_runspec." + n for n in ("Registry.", "Validation.", "Flags.", "FlagRefusals.", "Plan.", "Registry."))
TI, SE, CN, SO, MR, GR, RV, PR, CL, PF = ("test_propimage_images." + n for n in (
    "TwoImages.", "Selection.", "ClockAndNetwork.", "Sources.", "MoreRefusals.", "Grading.", "ReportValidation.", "PayloadRows.", "CommandLine.", "Preflight."))


PO = "test_propimage_images.Policies."
LN = "test_propimage_images.Lanes."
RR = "test_propimage_images.ResultRows."


def M(name, key, old, new, *tests):
    return (name, key, old, new, list(tests))


MUTANTS = [
    # the run spec
    M("install-on-prebuilt-allowed", "spec", '            if kind != "built":\n                raise SpecError("INSTALL_ON_PREBUILT"', '            if False:\n                raise SpecError("INSTALL_ON_PREBUILT"',
      V + "test_the_named_refusals", FR + "test_install_on_a_prebuilt_image_from_flags"),
    M("net-on-with-state-file-allowed", "spec", 'if out["net"] == "on" and (out["state_file"] is not None):', "if False:",
      V + "test_the_named_refusals", FR + "test_net_on_with_an_explicit_state_file_from_flags"),
    M("duplicate-labels-allowed", "spec", "    if len(set(labels)) != len(labels):", "    if False:",
      V + "test_the_named_refusals", FR + "test_two_images_may_not_share_a_label_from_flags"),
    M("source-sha256-unchecked", "spec", "    if sha is not None and not (type(sha) is str and HEX64.fullmatch(sha)):", "    if False:",
      V + "test_the_named_refusals", FR + "test_a_sha256_in_a_json_source_must_be_64_hex"),
    M("http-urls-allowed", "spec", '    if src["from"] == "url" and not re.fullmatch(r"https://[^\\s]{1,1000}", ref):', "    if False:",
      V + "test_the_named_refusals", FR + "test_a_plain_http_url_is_refused"),
    M("flag-before-image-allowed", "spec", "                if not images:\n                    raise SpecError(\"FLAG_BEFORE_IMAGE\", flag)", "                if False:\n                    pass",
      F + "test_a_per_image_flag_before_any_image_is_refused", CL + "test_a_flag_before_any_image_is_a_usage_error"),
    M("image-limit-removed", "spec", "    if type(images) is not list or not 1 <= len(images) <= MAX_IMAGES:", "    if type(images) is not list or not 1 <= len(images):",
      V + "test_more_than_eight_images_and_duplicate_json_keys_are_refused", FR + "test_nine_images_from_flags_are_refused"),
    M("skip-does-not-win", "spec", '        if n in skipped:', "        if False:",
      PL + "test_skip_wins_over_the_image_declaration", SE + "test_skip_is_honoured"),
    M("only-ignored", "spec", "        elif only and n not in only:\n            continue", "        elif False:\n            continue",
      PL + "test_only_restricts_and_names_what_is_not_on_the_image", SE + "test_apps_flag_restricts_the_state_declaration"),
    M("unknown-app-runs", "spec", '"action": "not_run", "reason": "NO_DRIVER"', '"action": "run", "reason": "NO_DRIVER"',
      PL + "test_declared_present_apps_run_and_unknown_ones_are_not_run_never_an_error", SE + "test_an_app_without_a_driver_is_not_run_never_an_error"),
    M("spec-declaration-ignored", "spec", "    if present:\n        for n in present:", "    if False:\n        for n in present:",
      PL + "test_declared_present_apps_run_and_unknown_ones_are_not_run_never_an_error", SE + "test_an_app_without_a_driver_is_not_run_never_an_error"),
    # the registry
    M("duplicate-identity-allowed", "reg", "                if name in names:", "                if False:",
      RS + "test_a_definition_set_is_one_more_file_and_a_duplicate_is_refused_by_name", RS + "test_an_alias_cannot_shadow_another_identity"),
    M("program-names-not-matched", "reg", '        if any(re.search(rx, str(text), re.I) for rx in d["match"]):', "        if False:",
      RS + "test_names_are_matched_to_identities", SE + "test_apps_flag_restricts_the_state_declaration"),
    M("unmet-needs-ignored", "reg", '    return [n for n in definition["needs"] if n not in provided]', "    return []",
      RS + "test_unmet_needs", PL + "test_an_app_with_an_unmet_need_is_not_run_by_name"),
    # the runner
    M("built-not-refused-up-front", "images", '        if any(i["kind"] == "built" for i in spec["images"]):\n            raise pa.Refused("BUILT_NOT_IMPLEMENTED")', "        if False:\n            pass",
      SO + "test_built_is_refused_by_name_before_anything_else", CL + "test_a_built_image_is_refused_by_name_on_any_machine_and_prints_one_record"),
    M("net-on-with-state-allowed", "images", '            if img["net"] == "on":\n                raise pa.Refused("NET_ON_WITH_STATE")', "            if False:\n                pass",
      CN + "test_net_on_with_a_state_json_is_refused_by_name_and_nothing_boots", MR + "test_net_on_with_a_state_zip_is_refused_too"),
    M("login-not-required", "images", '        elif img["ssh"] is None:\n            raise pa.Refused("SSH_REQUIRED")', "        elif False:\n            pass",
      SO + "test_a_plain_image_needs_a_login", MR + "test_a_plain_image_without_a_login_boots_nothing"),
    M("state-without-declaration-boots", "images", 'if decided == "NEEDS_PROBE" and loc.kind == "state":', "if False:",
      SE + "test_a_state_image_with_nothing_declared_is_refused_by_name", MR + "test_a_state_image_with_nothing_declared_boots_nothing"),
    M("clock-network-proof-not-enforced", "images", '                if not got["ok"] and item["verdict"] == "PASS":', "                if False:",
      CN + "test_a_state_image_with_internet_in_its_argv_is_not_a_pass", CN + "test_a_plain_image_that_asked_for_the_network_off_but_got_internet_is_not_a_pass"),
    M("source-sha256-not-compared", "images", '        if loc.sha256 != src["sha256"]:', "        if False:",
      SO + "test_a_file_with_the_wrong_sha256_is_refused_and_the_right_one_runs", MR + "test_a_bare_disk_with_the_wrong_sha256_is_refused"),
    M("unpacked-image-left-behind", "images", "        loc.created.append(out)", "        pass",
      SO + "test_a_file_with_the_wrong_sha256_is_refused_and_the_right_one_runs", SO + "test_a_pin_is_fetched_through_the_downloader_and_unpacked"),
    M("failed-images-do-not-fail-the-run", "images", 'for want in ("FAIL", "RETRY", "REFUSED"):', 'for want in ("RETRY", "REFUSED"):',
      GR + "test_the_run_verdict_order", TI + "test_one_failing_image_does_not_stop_the_other_and_fails_the_run_by_name"),
    M("failed-apps-do-not-fail-the-image", "images", "    if hard:\n        return \"FAIL\", \"APP_FAIL:\" + hard[0][\"app\"]", "    if False:\n        pass",
      GR + "test_a_harness_row_is_retry_and_an_app_fail_is_fail", TI + "test_one_failing_image_does_not_stop_the_other_and_fails_the_run_by_name"),
    M("solo-apps-share-boots", "images", 'if reg[x]["group"] == group and not reg[x]["solo"]]:', 'if reg[x]["group"] == group]:',
      SE + "test_solo_apps_boot_alone_after_the_others_and_groups_never_share_a_boot", MR + "test_every_planned_app_is_in_exactly_one_boot"),
    M("groups-share-boots", "images", 'for group in sorted({reg[a]["group"] for a in run}):', 'for group in ["core"]:',
      SE + "test_solo_apps_boot_alone_after_the_others_and_groups_never_share_a_boot", SE + "test_groups_never_share_a_boot_even_when_short"),
    M("intel-state-image-boots", "images", '            bad = pa.refusal_of([r for r in cpu_rows if r["name"] in ("cpu_vendor", "cpu_features")])\n            if bad:', "            bad = None\n            if bad:",
      PF + "test_an_intel_host_refuses_the_state_image_only", PF + "test_a_cpu_profile_failure_refuses_the_state_image_only"),
    M("deadline-ignored", "images", "            if ctx.deadline_epoch and time.time() + need + 900 > ctx.deadline_epoch:", "            if False:",
      PF + "test_no_time_is_retry_time_and_nothing_starts", PF + "test_no_time_for_a_plain_image_either"),
    # the two policies: standalone (skips are not failures) and strict (any skip is a failure)
    M("strict-ignores-skips", "images", "    if strict:\n        skips = skipped_items(images)", "    if False:\n        skips = skipped_items(images)",
      PO + "test_a_strict_run_with_a_skipped_image_fails_by_name_with_its_class", PO + "test_a_strict_run_with_an_app_that_needs_something_missing_fails_with_the_class"),
    M("permissive-fails-on-skips", "images", "    if strict:\n        skips = skipped_items(images)", "    if True:\n        skips = skipped_items(images)",
      PO + "test_the_same_skip_is_not_a_failure_when_permissive", PO + "test_a_permissive_run_with_a_skip_passes_and_says_plainly_what_ran_and_what_was_skipped"),
    M("fail-swallowed-in-the-run-verdict", "images", '    for want in ("FAIL", "RETRY", "REFUSED"):\n        for i in images:', '    for want in ("RETRY", "REFUSED"):\n        for i in images:',
      PO + "test_a_real_failure_is_non_zero_in_both_policies_and_is_not_a_strict_skip", PO + "test_a_failure_and_a_skip_together_fail_as_a_failure"),
    M("fail-exits-zero", "images", '    if verdict == "FAIL":\n        return 4 if reason.startswith("STRICT_") else 1', '    if verdict == "FAIL":\n        return 0',
      PO + "test_the_exit_codes_are_distinct", PO + "test_a_strict_run_with_a_skipped_image_fails_by_name_with_its_class"),
    M("strict-skip-exits-like-a-failure", "images", 'return 4 if reason.startswith("STRICT_") else 1', "return 1",
      PO + "test_the_exit_codes_are_distinct", PO + "test_a_strict_run_with_an_app_that_needs_something_missing_fails_with_the_class"),
    M("strict-unknown-app-not-checked", "images", "            S.strict_unknown_app(spec, reg)", "            pass",
      PO + "test_a_strict_run_with_one_unknown_app_fails_by_name_before_any_boot", PO + "test_a_strict_unknown_app_never_boots_anything"),
    M("skips-not-counted-by-class", "images", '        classes[k["class"]] = classes.get(k["class"], 0) + 1', "        pass",
      PO + "test_a_permissive_run_with_a_skip_passes_and_says_plainly_what_ran_and_what_was_skipped", PO + "test_the_same_needs_are_only_a_skip_when_permissive"),
    M("needs-is-not-missing-input", "spec", 'if str(reason).startswith("NEEDS:"):\n        return "MISSING_INPUT"', 'if False:\n        return "MISSING_INPUT"',
      "test_propimage_runspec.Strict.test_skip_classes_of_reasons", PO + "test_a_strict_run_with_an_app_that_needs_something_missing_fails_with_the_class"),
    M("strict-spec-allows-only-and-skip", "spec", "    if strict:\n        for img in norm:", "    if False:\n        for img in norm:",
      "test_propimage_runspec.Strict.test_a_strict_spec_names_exactly_what_runs", "test_payload_instruction.Propimages.test_a_strict_instruction_names_an_unknown_app_or_a_skip_entry_before_anything_runs"),
    M("report-strict-pass-with-skips", "report", 'ok = sm["skipped"] == 0 and all(i["verdict"] == "PASS" and not i["not_run"] for i in images)', "pass",
      "test_propimage_images.ReportPolicy.test_a_strict_pass_with_a_skip_anywhere_is_refused", "test_propimage_images.ReportPolicy.test_a_strict_pass_with_a_skipped_image_is_refused"),
    # the two lanes
    M("discovery-failures-are-fatal", "images", '    apps = [a for a in rec["apps"] if a["lane"] == "gate"]', '    apps = rec["apps"]',
      LN + "test_a_discovery_failure_is_not_fatal_and_is_listed_with_its_class_and_reason", LN + "test_a_discovery_failure_is_not_fatal_in_a_strict_run_either"),
    M("gate-rows-are-excused", "images", '                item["lane"] = lane_of.get(app, "gate")', '                item["lane"] = "discovery"',
      LN + "test_a_gate_failure_is_fatal_in_both_policies_whatever_the_discovery_rows_do", LN + "test_a_crashed_gate_row_is_not_excused"),
    M("discovery-rows-share-boots", "images", '    run = [a for a in run if a not in discovery]', "    pass",
      LN + "test_every_discovery_row_boots_alone_after_the_gate_rows_with_a_hard_time_box_of_its_own", LN + "test_the_plan_puts_discovery_rows_in_boots_of_their_own"),
    M("a-crashed-boot-stops-the-run", "images", '            boot["secs"], boot["ended_utc"] = int(time.monotonic() - t0), utc_now()\n', '            boot["secs"], boot["ended_utc"] = int(time.monotonic() - t0), utc_now()\n            if boot["rc"] not in (0, None):\n                break\n',
      LN + "test_a_crashed_discovery_row_fails_only_its_own_row_and_the_next_row_still_runs", LN + "test_a_hung_discovery_row_is_a_hang_and_the_next_row_still_runs"),
    M("breakages-not-listed", "images", '                    rec["breakages"] += [broken] if broken else []', "                    pass",
      LN + "test_a_discovery_failure_is_not_fatal_and_is_listed_with_its_class_and_reason", LN + "test_the_verdict_line_prints_both_lanes_and_one_line_per_breakage"),
    M("hang-not-recognised", "images", '    if boot_rc == 124 and app["verdict"] in ("MISSING", "HARNESS"):', '    if False:',
      LN + "test_a_hung_discovery_row_is_a_hang_and_the_next_row_still_runs", LN + "test_failure_classes"),
    M("no-hard-time-box", "images", 'need + pa.BOOT_SLACK_S,', 'need + 10 ** 7,',
      LN + "test_every_discovery_row_boots_alone_after_the_gate_rows_with_a_hard_time_box_of_its_own", "test_propimage_images.Selection.test_every_boot_has_a_hard_time_box"),
    M("lane-summary-missing", "images", '    lanes = "gate: ran %d, passed %d | discovery: ran %d, passed %d, known breakages %d" % (g["ran"], g["passed"], d["ran"], d["passed"], len(rec["breakages"]))', '    lanes = ""',
      LN + "test_the_verdict_line_prints_both_lanes_and_one_line_per_breakage", "test_propimage_images.Policies.test_a_permissive_run_with_a_skip_passes_and_says_plainly_what_ran_and_what_was_skipped"),
    M("unknown-lane-accepted", "spec", '        if entry.get("lane", "gate") not in LANES:', "        if False:",
      "test_propimage_runspec.LaneSpec.test_lane_is_optional_gate_by_default_and_closed", "test_propimage_runspec.Validation.test_the_named_refusals"),
    # the per-row result records
    M("failing-rows-say-pass", "images", 'result="PASS" if ok else "FAIL"', 'result="PASS"',
      RR + "test_pass_fail_and_skipped_rows_with_class_and_reason", RR + "test_a_failing_gate_row_carries_a_class_too"),
    M("rows-without-failure-class", "images", "failure_class=None if ok else failure_class(a, boot.get(\"rc\"))", "failure_class=None",
      RR + "test_pass_fail_and_skipped_rows_with_class_and_reason", RR + "test_a_failing_gate_row_carries_a_class_too"),
    M("unknown-die-is-invented", "images", '(pa.clean((ctx.hardware or {}).get("gpu_name"), 64) if (ctx.hardware or {}).get("gpu_name") not in (None, "unknown") else None)', '(pa.clean((ctx.hardware or {}).get("gpu_name"), 64) if (ctx.hardware or {}).get("gpu_name") else "unknown")',
      RR + "test_an_unknown_die_is_null", RR + "test_a_die_that_was_never_read_is_null"),
    M("host-driver-text-accepted-unchecked", "images", 'return line if pa.re.fullmatch(r"\\d+\\.\\d+(\\.\\d+)?", line) else None', "return line or None",
      RR + "test_text_that_is_not_a_version_is_null", RR + "test_an_nvidia_smi_error_is_null"),
    M("rows-lose-the-image-driver", "images", 'guest_driver=img.get("guest_driver")', "guest_driver=None",
      RR + "test_the_matrix_facts_come_from_the_host_and_the_image", RR + "test_every_row_of_an_image_carries_its_guest_driver_even_the_skipped_ones"),
    M("skipped-rows-dropped", "images", '        for n in img["not_run"]:\n            out.append(', '        for n in []:\n            out.append(',
      RR + "test_pass_fail_and_skipped_rows_with_class_and_reason", RR + "test_every_row_has_exactly_the_stable_keys" if False else RR + "test_pass_fail_and_skipped_rows_with_class_and_reason"),
    M("report-row-shape-unchecked", "report", 'type(r) is dict and set(r) == rkeys and', 'type(r) is dict and', RR + "test_the_report_refuses_a_row_with_a_wrong_shape", RR + "test_the_report_refuses_a_row_with_an_extra_or_a_missing_key"),
    # the report and the payload
    M("report-pass-needs-no-proven-boot", "report", 'all(b["proven"] for b in im["boots"] if b["rc"] != 125)', "True",
      RV + "test_a_pass_image_with_an_unproven_boot_is_refused", TI + "test_the_record_validates_for_the_report_and_a_pass_needs_proven_boots"),
    M("report-extra-keys-allowed", "report", 'keys <= set(im) <= keys | {"error"} and im["verdict"] in verdicts', 'keys <= set(im) and im["verdict"] in verdicts',
      RV + "test_shape_extra_keys_and_a_pass_without_apps_are_refused", RV + "test_an_image_with_an_extra_key_is_refused"),
    M("payload-accepts-not-run", "payload", 'or rc or doc.get("verdict") != "PASS":\n        raise EvidenceError("propimage_apps")\n    return [row("propimage_apps", 1)]\n\n\ndef propimages_refusal', 'or rc:\n        raise EvidenceError("propimage_apps")\n    return [row("propimage_apps", 1)]\n\n\ndef propimages_refusal',
      PR + "test_only_a_pass_with_rc_zero_for_this_sha_is_a_pass", PR + "test_a_refused_run_is_the_stage_failing"),
    M("images-file-and-flags-allowed", "pa", '            if spec:\n                raise spec_mod.SpecError("IMAGES_FILE_AND_FLAGS")', "            if False:\n                pass",
      CL + "test_images_file_and_flags_together_are_refused_and_a_file_is_read", CL + "test_images_file_and_a_propimage_flag_together_are_refused_too"),
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
    done = subprocess.run([sys.executable, "-B", "-c", COLLECT, *SUITES], cwd=root / "scripts/ci", capture_output=True, text=True, timeout=1800)
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
    with tempfile.TemporaryDirectory(prefix="kf-runner-mutants-") as parent:
        base = run_tests(tree(parent, "base", {}))
        if base["killers"]:
            print("BASELINE RED: %s" % base["killers"], file=sys.stderr)
            return 2
        print("baseline green: %d tests" % base["tests"], flush=True)
        with concurrent.futures.ThreadPoolExecutor(a.jobs) as pool:
            for name, killers, wanted in pool.map(lambda m: one(parent, m), chosen):
                named = [w for w in set(wanted) if w in killers]
                ok = len(killers) >= 2 and len(named) >= 1 and not killers[0].startswith("anchor")
                print("%s %s: %d killers (%d named)" % ("PASS" if ok else "WEAK", name, len(killers), len(named)), flush=True)
                if not ok:
                    bad += 1
                    print("   killers: %s" % killers[:6], flush=True)
    print("%d/%d mutants killed" % (len(chosen) - bad, len(chosen)))
    return 1 if bad else 0


if __name__ == "__main__":
    sys.exit(main())
