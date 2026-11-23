#!/usr/bin/env python3
"""Mutation checks of the propimage_apps stage (2026-11-15): every mutant needs two named killers (stdlib, GPU-free).

Each mutant is one exact-once text replacement in a private copy of propimage_apps.py, vast_report.py, vast_payload.py, propimage_apps_b_stage.py or
propimage_check.py (2026-11-16: the image B guest RAM and sound card mutants; their killers are in test_propimage_apps_b_stage.py and test_propimage_check.py) (the whole of
scripts/ci and scripts/kfci is copied, plus the harness files the stage imports); the named tests of test_propimage_apps.py run against
the copy and the failing ones are the killers. The unmutated copy must be green first.

    python3 scripts/ci/propimage_apps_mutants.py [--jobs N] [substring-of-mutant-name]
"""
import argparse
import concurrent.futures
import json
import os
from pathlib import Path
import shutil
import subprocess
import sys
import tempfile

HERE = Path(__file__).resolve().parent
ROOT = HERE.parents[1]
FILES = {
    "pa": "scripts/ci/propimage_apps.py",
    "report": "scripts/ci/vast_report.py",
    "payload": "scripts/ci/vast_payload.py",
    "pab": "scripts/ci/propimage_apps_b_stage.py",
    "pabm": "scripts/bench/windows/propimage_apps_b.py",
    "uit": "scripts/bench/windows/apps_b/ui_table.py",
    "check": "scripts/bench/windows/propimage_check.py",
}
SOURCES = {key: (ROOT / path).read_text() for key, path in FILES.items()}
SUITE = "test_propimage_apps."
COPY_DIRS = ["scripts/ci", "scripts/kfci", "scripts/bench/windows/apps_b"]
COPY_FILES = [
    "scripts/bench/native_xid_guard.py",
    "scripts/bench/native_fault_names.json",
    "scripts/bench/specviewperf/lanes.json",
    "scripts/bench/windows/propimage.py",
    "scripts/bench/windows/propimage_check.py",
    "scripts/bench/windows/clocknet_assert.py",
    "scripts/bench/windows/propimage_plain.py",
    "scripts/bench/windows/propimage_registry.py",
    "scripts/bench/windows/propimage_apps_b.py",
    "scripts/bench/windows/gpu_assert.py",
    "scripts/bench/windows/kf3_census.py",
    "scripts/bench/windows/media_check.py",
    "scripts/bench/windows/propimage_runspec.py",
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

PF, RS, SH, RP2 = ("test_propimage_apps_b_stage." + n for n in ("PinsFlag.", "RowSelection.", "Shrink.", "Report."))   # image B test branch 2026-11-20
UI = "test_propimage_apps_b_ui."
UPULL, USCR, UGRADE, UTAB, UWORDS, UCAP = (UI + n for n in ("Pull.", "Scrub.", "GradeLaunch.", "LaunchTable.", "Words.", "CaptureScript."))
UT = "test_ui_table.T."
PB, BG, CK = ("test_propimage_apps_b_stage.Plan.", "test_propimage_apps_b_stage.GuestRam.", "test_propimage_check.T.")   # image B's stage and the harness
P, V, B, K, R, RP, W, G, RV = (SUITE + n for n in (
    "Pins.", "Preflight.", "Boots.", "Verdicts.", "Run.", "ReportValidation.", "PayloadWiring.", "KernelLogWindows.", "ReportValidation."))


def M(name, key, old, new, *tests):
    return (name, key, old, new, list(tests))


MUTANTS = [
    # pins and the unpinned refusal
    M("pin-needs-only-one-of-hash-and-size", "pa", '(i["sha256"] is None or i["size"] is None)', '(i["sha256"] is None and i["size"] is None)',
      P + "test_image_with_hash_but_no_size_is_unpinned", P + "test_half_pinned_roles_refuse_the_manifest"),
    M("unpinned-manifest-accepted", "pa", '    if unset:\n        raise Refused("PIN_UNSET:" + unset[0], ",".join(unset))', "    if False:\n        raise Refused()",
      P + "test_unpinned_image_refuses_even_when_everything_else_is_pinned", P + "test_unpinned_swtpm_sources_matter_only_when_a_build_is_needed"),
    M("image-not-a-required-pin", "pa", 'return ("image", "gop") + (("libtpms", "swtpm") if build_swtpm else ())',
      'return ("gop",) + (("libtpms", "swtpm") if build_swtpm else ())',
      P + "test_unpinned_image_refuses_even_when_everything_else_is_pinned", V + "test_the_first_refusal_in_row_order_names_the_reason", R + "test_unpinned_image_refuses_and_nothing_is_fetched"),
    M("gop-not-a-required-pin", "pa", 'return ("image", "gop") + (("libtpms", "swtpm") if build_swtpm else ())',
      'return ("image",) + (("libtpms", "swtpm") if build_swtpm else ())',
      P + "test_unpinned_swtpm_sources_matter_only_when_a_build_is_needed", P + "test_manifest_is_accepted_by_the_real_downloader_and_the_image_is_signed_only"),
    M("pin-dest-traversal-allowed", "pa", 'any(p in ("", ".", "..") or not re.fullmatch(r"[A-Za-z0-9._@+-]{1,128}", p) for p in parts)', "False",
      P + "test_pins_file_hostile_variants_refused", P + "test_pins_dest_must_be_relative_and_clean"),
    M("pins-duplicate-keys-allowed", "pa", '        if k in out:\n            raise ValueError("duplicate key")', "        if False:\n            raise ValueError()",
      P + "test_pins_duplicate_keys_unknown_keys_and_missing_image_refused", K + "test_read_json_refuses_duplicates_and_huge_files"),
    M("image-and-gop-not-required-in-pins-file", "pa", 'if not {"image", "gop"} <= roles:', "if False:",
      P + "test_pins_duplicate_keys_unknown_keys_and_missing_image_refused", P + "test_pins_file_without_the_image_role_is_refused"),
    # 2026-11-17: the tool tarballs carry a private ref AND the public mirror (signed first, mirror as the fallback)
    M("tools-mirror-listed-before-the-signed-ref", "pa",
      '    return ([{"kind": "signed", "ref": it["ref"]}] if "ref" in it else []) + ([{"kind": "mirror", "url": it["mirror"]}] if "mirror" in it else [])',
      '    return ([{"kind": "mirror", "url": it["mirror"]}] if "mirror" in it else []) + ([{"kind": "signed", "ref": it["ref"]}] if "ref" in it else [])',
      P + "test_an_item_with_ref_and_mirror_lists_the_signed_source_first", P + "test_both_committed_pins_files_give_the_tools_signed_then_mirror_with_unchanged_pins",
      P + "test_manifest_is_accepted_by_the_real_downloader_and_the_image_is_signed_only"),
    M("tools-ref-dropped-when-a-mirror-exists", "pa",
      '    return ([{"kind": "signed", "ref": it["ref"]}] if "ref" in it else []) + ([{"kind": "mirror", "url": it["mirror"]}] if "mirror" in it else [])',
      '    return [{"kind": "mirror", "url": it["mirror"]}] if "mirror" in it else [{"kind": "signed", "ref": it["ref"]}]',
      P + "test_an_item_with_ref_and_mirror_lists_the_signed_source_first", P + "test_both_committed_pins_files_give_the_tools_signed_then_mirror_with_unchanged_pins",
      P + "test_a_urls_file_without_the_tool_refs_has_no_private_url_so_the_downloader_falls_through_to_the_mirror"),
    M("tools-mirror-dropped-when-a-ref-exists", "pa",
      '    return ([{"kind": "signed", "ref": it["ref"]}] if "ref" in it else []) + ([{"kind": "mirror", "url": it["mirror"]}] if "mirror" in it else [])',
      '    return [{"kind": "signed", "ref": it["ref"]}] if "ref" in it else [{"kind": "mirror", "url": it["mirror"]}]',
      P + "test_an_item_with_ref_and_mirror_lists_the_signed_source_first", P + "test_both_committed_pins_files_give_the_tools_signed_then_mirror_with_unchanged_pins",
      P + "test_manifest_is_accepted_by_the_real_downloader_and_the_image_is_signed_only"),
    M("pin-with-neither-ref-nor-mirror-accepted", "pa", 'or not ("ref" in it or "mirror" in it):', "or False:",
      P + "test_a_pin_with_neither_ref_nor_mirror_is_refused", P + "test_an_image_pin_with_neither_ref_nor_mirror_is_refused"),
    M("pin-with-both-ref-and-mirror-refused", "pa", 'or not ("ref" in it or "mirror" in it):', 'or ("ref" in it) == ("mirror" in it):',
      P + "test_an_item_with_ref_and_mirror_lists_the_signed_source_first", P + "test_both_committed_pins_files_give_the_tools_signed_then_mirror_with_unchanged_pins"),
    # CPU, GPU and host preflight
    M("any-cpu-vendor-allowed", "pa", 'ALLOWED_CPU_VENDORS = ("AuthenticAMD",)', 'ALLOWED_CPU_VENDORS = ("AuthenticAMD", "GenuineIntel")',
      V + "test_the_vendor_list_is_the_extension_point", R + "test_intel_host_is_not_run_cpu_vendor_and_touches_nothing", V + "test_intel_is_not_run_cpu_vendor_before_any_other_refusal"),
    M("cpu-vendor-refusal-is-not-not-run", "pa", '"cpu_vendor": ("NOT_RUN", "CPU_VENDOR")', '"cpu_vendor": ("REFUSED", "CPU_VENDOR")',
      V + "test_intel_is_not_run_cpu_vendor_before_any_other_refusal", R + "test_intel_host_is_not_run_cpu_vendor_and_touches_nothing", V + "test_unreadable_cpuinfo_is_a_refusal"),
    M("cpu-features-not-refused", "pa", '"cpu_features": ("REFUSED", "CPU_PROFILE_UNSUPPORTED")', '"cpu_features": ("REFUSED", "PREFLIGHT")',
      V + "test_amd_with_missing_cpu_flags_is_cpu_profile_unsupported", R + "test_missing_cpu_flags_are_refused_by_name_in_a_run"),
    M("cpu-vendor-row-not-first", "pa", "    rows += cpu_rows(cpuinfo, pi=pi)\n    vendor =", "    vendor =",
      V + "test_precedence_cpu_vendor_row_comes_first", V + "test_the_first_refusal_in_row_order_names_the_reason", R + "test_precedence_cpu_vendor_row_comes_first"),
    M("datacenter-family-accepted", "pa", '750: "turing", 860: "ampere"', '750: "turing", 800: "ampere", 860: "ampere"',
      V + "test_gpu_families", R + "test_unsupported_gpu_family_is_refused_by_name"),
    M("unknown-family-accepted", "pa", "    if family is None:\n        return row(", "    if False:\n        return row(",
      V + "test_gpu_families", R + "test_unsupported_gpu_family_is_refused_by_name"),
    M("old-swtpm-accepted", "pa", "build = version is None or version < (0, 10, 0)", "build = version is None",
      V + "test_swtpm_version_rows", R + "test_old_swtpm_is_built_from_the_pinned_sources_and_used", R + "test_built_swtpm_that_is_still_old_is_refused"),
    M("swtpm-build-result-unchecked", "pa", "    if version is None or version < (0, 10, 0):\n        raise Refused(\"SWTPM_BUILD\")", "    if False:\n        raise Refused(\"SWTPM_BUILD\")",
      R + "test_built_swtpm_that_is_still_old_is_refused", R + "test_build_that_leaves_no_working_swtpm_is_refused"),
    M("swtpm-build-steps-unchecked", "pa", "        if run_command(ctx, argv, log, timeout, cwd=cwd, env=env) != 0:\n            raise Refused(\"SWTPM_BUILD\")",
      "        run_command(ctx, argv, log, timeout, cwd=cwd, env=env)",
      R + "test_swtpm_build_failure_is_named", R + "test_a_timed_out_swtpm_build_is_named"),
    M("disk-never-short", "pa", '"MATCH" if free_bytes >= need else "MISMATCH"', '"MATCH"',
      V + "test_disk_row_needs_the_archive_twice_plus_headroom", R + "test_low_disk_is_refused_by_name_before_any_download"),
    M("disk-need-counts-the-archive-once", "pa", "need = 2 * size + DISK_HEADROOM", "need = size + DISK_HEADROOM",
      V + "test_disk_row_needs_the_archive_twice_plus_headroom", R + "test_disk_just_below_the_need_is_refused_in_a_run"),
    M("urls-symlink-accepted", "pa", 'ctx.urls.is_file() and not ctx.urls.is_symlink()', "ctx.urls.is_file()",
      V + "test_urls_symlink_is_not_accepted", R + "test_a_symlinked_url_file_is_refused_by_name_in_a_run"),
    M("kf3-qemu-not-checked", "pa", 'row("kf3_qemu", "MATCH" if os.access(ctx.qemu, os.X_OK) else "MISMATCH"', 'row("kf3_qemu", "MATCH"',
      V + "test_missing_kf3_qemu_and_urls_are_refusals", R + "test_missing_kf3_qemu_is_refused_by_name_in_a_run"),
    # boots
    M("wukong-grouped-with-others", "pa", "    boots.append([APPS[-1]])", "    boots[-1].append(APPS[-1])",
      B + "test_real_timeouts_make_three_boots_each_under_the_budget", B + "test_wukong_is_alone_even_with_an_unlimited_budget", B + "test_every_app_is_in_exactly_one_boot_in_order"),
    M("boot-budget-ignored", "pa", "        if cur and total + t > budget:", "        if False:",
      B + "test_real_timeouts_make_three_boots_each_under_the_budget", B + "test_a_tiny_budget_gives_one_app_per_boot"),
    M("superposition-run-flag-dropped", "pa", '        cmd.append("--superposition-run")', "        pass",
      B + "test_boot_command_flags", R + "test_full_pass_record"),
    M("wukong-framebuffer-flag-dropped", "pa", '        cmd += ["--fb-mb", str(a.get("fb_mb", WUKONG_FB_MB))]', "        pass",
      B + "test_boot_command_flags", R + "test_full_pass_record"),
    M("display-off-instead-of-broker", "pa", '"--display", "broker"', '"--display", "off"',
      B + "test_boot_command_flags", R + "test_full_pass_record"),
    M("offline-never-proven", "pa", '    return ("restrict=on" in text and "restrict=off" not in text', '    return True or ("restrict=on" in text and "restrict=off" not in text',
      B + "test_offline_proof", R + "test_online_boot_is_not_a_pass"),
    M("offline-restrict-off-tolerated", "pa", '"restrict=on" in text and "restrict=off" not in text', '"restrict=on" in text',
      B + "test_offline_proof", R + "test_a_boot_with_both_restrict_settings_is_not_a_pass"),
    M("offline-proof-not-applied", "pa", '                if not boot["offline"] and item["verdict"] == "PASS":', "                if False:",
      R + "test_online_boot_is_not_a_pass", R + "test_a_boot_with_both_restrict_settings_is_not_a_pass"),
    # reading what the harness left
    M("pass-without-proof-accepted", "pa", "        if not proven:", "        if False:",
      K + "test_a_pass_row_without_a_verdict_file_is_unproven", K + "test_a_pass_row_with_the_wrong_evidence_kind_is_unproven", K + "test_row_and_verdict_file_must_agree"),
    M("expected-evidence-kind-dropped", "pa", 'verdict_doc.get("evidence") == rec["kind"] == (expected or EXPECTED_KIND.get(app))', 'verdict_doc.get("evidence") == rec["kind"]',
      K + "test_a_pass_row_with_the_wrong_evidence_kind_is_unproven", R + "test_a_pass_with_the_wrong_evidence_kind_in_a_boot_is_not_a_stage_pass"),
    M("verdict-ok-flag-ignored", "pa", 'verdict_doc.get("ok") is True', "True",
      K + "test_row_and_verdict_file_must_agree", K + "test_a_verdict_file_that_says_not_ok_is_not_a_pass"),
    M("verdict-app-name-ignored", "pa", 'verdict_doc.get("app") == app and ', "",
      K + "test_row_and_verdict_file_must_agree", K + "test_a_verdict_file_of_another_app_is_not_a_pass"),
    M("run-dirs-follow-symlinks", "pa", "RUN_ID.fullmatch(p.name) and p.is_dir() and not p.is_symlink()", "RUN_ID.fullmatch(p.name) and p.is_dir()",
      K + "test_hostile_run_directories_and_files_are_ignored", K + "test_hostile_run_directories_are_not_a_pass"),
    M("evidence-follows-symlinks", "pa", "if p.is_symlink() or not p.is_file() or not re.fullmatch", "if not p.is_file() or not re.fullmatch",
      K + "test_evidence_files_skip_symlinks_odd_names_and_cap_the_count", K + "test_evidence_listing_never_reads_through_a_symlink"),
    M("guest-text-not-sanitised", "pa", r'return re.sub(r"[^\x20-\x7e]", "?", str(text))[:limit]', "return str(text)[:limit]",
      K + "test_guest_text_is_sanitised_and_bounded", R + "test_hostile_guest_text_never_reaches_the_record"),
    M("unknown-harness-verdict-trusted", "pa", 'rec["verdict"] = said if said in APP_VERDICTS else "FAIL"', 'rec["verdict"] = said',
      K + "test_unknown_verdict_words_from_the_harness_become_fail", R + "test_an_unknown_harness_verdict_word_is_a_fail"),
    M("wukong-events-not-counted", "pa", 'out["live_kernel_events"], out["codes"] = len(codes), codes[:8]', 'out["live_kernel_events"], out["codes"] = 0, []',
      K + "test_wukong_events_are_counted_by_name", R + "test_wukong_failure_is_named_and_the_record_still_validates"),
    M("tdr-events-not-counted", "pa", r'out["tdr_4101"] = len(re.findall(r"\bid=4101\b", text))', 'out["tdr_4101"] = 0',
      K + "test_wukong_events_are_counted_by_name", K + "test_tdr_events_are_counted_from_system_events_alone"),
    M("not-collected-reported-as-collected", "pa", 'out = {"collected": False,', 'out = {"collected": True,',
      K + "test_zero_guest_events_for_an_app_that_collects_none_say_not_collected", R + "test_apps_without_event_collection_say_not_collected_in_the_record"),
    # the verdict
    M("harness-problem-is-an-app-fail", "pa", 'return "RETRY", "HARNESS:" + failed[0]', 'return "FAIL", "HARNESS:" + failed[0]',
      K + "test_grade_harness_only_problems_are_retry_and_a_missing_app_is_not_a_pass", R + "test_a_harness_that_dies_before_the_apps_is_retry_not_a_guest_fail"),
    M("inconclusive-is-not-a-failure", "pa", 'in ("FAIL", "INCONCLUSIVE", "LAUNCH_ONLY")]', 'in ("FAIL",)]',
      K + "test_wukong_inconclusive_is_a_named_fail", R + "test_wukong_inconclusive_as_the_harness_grades_it_today_is_a_fail_by_name", K + "test_pc_w1_superposition_launch_only_is_not_a_pass"),
    M("first-failed-app-not-named", "pa", '"APP_FAIL:" + app_class[0]', '"APP_FAIL"',
      K + "test_grade_all_pass_and_the_first_failed_app_is_named", R + "test_wukong_failure_is_named_and_the_record_still_validates"),
    M("a-skipped-boot-for-time-still-starts", "pa", "time.time() + need + 900 > ctx.deadline_epoch", "time.time() + need + 900 < ctx.deadline_epoch",
      R + "test_no_time_for_a_boot_is_retry_time_and_the_boot_is_not_started", R + "test_enough_time_runs_all_boots"),
    M("not-started-boots-not-reported", "pa", "            if app not in have:", "            if False:",
      R + "test_no_time_for_a_boot_is_retry_time_and_the_boot_is_not_started", R + "test_every_app_has_a_record_even_when_a_boot_never_started"),
    # images never stay
    M("archive-kept-after-unpack", "pa", '        (dl / "image/image.zip").unlink(missing_ok=True)', "        pass",
      R + "test_the_archive_is_deleted_after_unpack_and_everything_after_the_run", R + "test_full_pass_record"),
    M("no-cleanup", "pa", "        if started:\n            cleanup(ctx)", "        pass",
      R + "test_the_archive_is_deleted_after_unpack_and_everything_after_the_run", R + "test_a_download_failure_is_refused_by_name_and_cleans_up"),
    M("cleanup-even-before-any-work", "pa", "        if started:\n            cleanup(ctx)", "        cleanup(ctx)",
      R + "test_a_cpu_refusal_leaves_a_prefetched_directory_alone", R + "test_a_pin_refusal_leaves_a_prefetched_directory_alone"),
    M("cleanup-misses-the-unpacked-image", "pa", 'for sub in ("dl", "unpacked", "work"):', 'for sub in ("dl", "work"):',
      R + "test_the_archive_is_deleted_after_unpack_and_everything_after_the_run", R + "test_a_download_failure_is_refused_by_name_and_cleans_up"),
    M("prefix-not-checked", "pa", "        check_prefix(ctx.prefix)\n        rows, family", "        rows, family",
      R + "test_refusal_on_a_bad_prefix_creates_and_deletes_nothing", R + "test_a_relative_prefix_is_refused_by_name_in_a_run"),
    M("evidence-copy-takes-pngs", "pa", 'EVIDENCE_COPY = ("verdict.json", "result.json", "system_events.txt", "nvidia_smi_memory.txt")',
      'EVIDENCE_COPY = ("verdict.json", "result.json", "system_events.txt", "nvidia_smi_memory.txt", "screen.png")',
      R + "test_only_text_evidence_is_copied", R + "test_evidence_copy_never_takes_native_files"),
    # the host kernel log
    M("xid-window-off-by-one", "pa", "out[label] = (opened.pop(label) + 1, int(n))", "out[label] = (opened.pop(label), int(n))",
      G + "test_lines_map_to_the_app_window_they_fall_in", R + "test_host_xids_are_attributed_per_app_and_unattributed_ones_are_kept"),
    M("silent-kernel-log-accepted", "pa", "            if self.proc.poll() is None and self.count() > 0:\n                return",
      "            if self.proc.poll() is None:\n                return",
      G + "test_follower_captures_lines_and_unreadable_log_is_an_error", R + "test_a_silent_kernel_log_refuses_the_run_before_any_download"),
    M("xid-pattern-too-narrow", "pa", 'XID_RE = re.compile(r"NVRM: Xid")', 'XID_RE = re.compile(r"NVRM: Xid \\(PCI:0000")',
      G + "test_lines_map_to_the_app_window_they_fall_in", R + "test_host_xids_are_attributed_per_app_and_unattributed_ones_are_kept"),
    M("unattributed-xids-dropped", "pa", "    return per, [(n, t) for n, t in rest if n not in inside]", "    return per, []",
      G + "test_lines_map_to_the_app_window_they_fall_in", R + "test_host_xids_are_attributed_per_app_and_unattributed_ones_are_kept"),
    M("app-end-marks-never-written", "pa", '            self.klog.mark(self.label + ":" + app, "end")\n', "            pass\n",
      G + "test_mark_lines_follow_the_harness_stdout", R + "test_host_xids_are_attributed_per_app_and_unattributed_ones_are_kept"),
    # the report validator
    M("report-kind-unchecked", "report", 'a["verdict"] == "PASS" and a["kind"] == PROPIMAGE_EXPECTED_KIND[a["app"]] for a in apps)', 'a["verdict"] == "PASS" for a in apps)',
      RV + "test_pass_needs_the_expected_evidence_kind", RV + "test_result_file_refuses_a_malformed_record"),
    M("report-offline-unchecked", "report", 'and all(b["offline"] for b in boots) and len(boots) >= 1)', ")",
      RV + "test_pass_needs_every_boot_offline", RV + "test_result_file_refuses_a_malformed_record"),
    M("report-pass-with-reason", "report", 'and not failed and value["reason"] == "" and all(', "and not failed and all(",
      RV + "test_pass_carries_no_reason_or_failed_list", RV + "test_result_file_refuses_a_malformed_record"),
    M("report-pass-with-failed-list", "report", "and not failed and value", "and value",
      RV + "test_pass_carries_no_reason_or_failed_list", RV + "test_result_file_refuses_a_pass_record_with_a_failed_list"),
    M("report-pass-needs-all-seven", "report", "ok = (names == list(PROPIMAGE_APPS) and", "ok = (True and",
      RV + "test_pass_needs_every_app_to_pass_in_order", RV + "test_result_file_refuses_a_malformed_record"),
    M("report-app-order-unchecked", "report", "names == [n for n in PROPIMAGE_APPS if n in names]", "True",
      RV + "test_non_pass_record_keeps_the_app_order_and_unique_apps", RV + "test_a_failed_record_with_apps_out_of_order_is_refused_in_a_result_file"),
    M("report-reason-optional", "report", '        ok = value["reason"] != ""\n    ok = ok and len(json.dumps(value, separators=(",", ":")).encode()) <= 24576', '        ok = True\n    ok = ok and len(json.dumps(value, separators=(",", ":")).encode()) <= 24576',
      RV + "test_non_pass_needs_an_upper_case_reason", RV + "test_a_refused_record_without_a_reason_is_rejected"),
    M("report-extra-keys-allowed", "report", 'ok = type(value) is dict and top <= set(value) <= top | {"error"}\n    ok = ok and value["schema"] == 1', "ok = type(value) is dict and top <= set(value)\n    ok = ok and value[\"schema\"] == 1",
      RV + "test_shape_extra_keys_and_types", RV + "test_result_file_refuses_a_malformed_record"),
    M("report-text-not-printable-checked", "report", '    token = re.compile(r"[A-Z][A-Z0-9_]{0,31}(:[A-Za-z0-9_]{1,32})?")\n    text = re.compile(r"[\\x20-\\x7e]{0,200}")', '    token = re.compile(r"[A-Z][A-Z0-9_]{0,31}(:[A-Za-z0-9_]{1,32})?")\n    text = re.compile(r"(?s).{0,2000}")',
      RV + "test_text_fields_are_printable_and_bounded", RV + "test_xid_lines_are_printable_and_few"),
    M("report-xid-lines-unbounded", "report", 'ok = ok and len(x["lines"]) <= 8 and', "ok = ok and",
      RV + "test_xid_lines_are_printable_and_few", RV + "test_result_file_bounds_the_xid_lines"),
    M("report-red-record-under-pass-payload", "report", 'doc["propimage_apps"]["verdict"] not in ("PASS", "NOT_RUN")', "False",
      RV + "test_payload_pass_with_a_red_record_is_refused_but_a_red_payload_may_carry_it", RV + "test_payload_pass_with_a_refused_record_is_refused"),
    M("report-hash-unchecked", "report", 'and num(e["bytes"]) and re.fullmatch("[0-9a-f]{64}", str(e["sha256"])) for e in a["evidence"])', "for e in a['evidence'])",
      RV + "test_hashes_sizes_and_counts_are_checked", RV + "test_result_file_refuses_a_bad_evidence_hash"),
    # image B's guest RAM override and sound card (2026-11-16): the stage's boot command, its refusal, and the harness that forwards them
    M("b-stage-ram-flag-dropped", "pab", '        cmd += ["--ram-mb", str(ram)]', "        pass",
      PB + "test_boot_command_carries_the_guest_ram_and_the_sound_card_by_default", BG + "test_every_boot_gets_the_ram_and_audio_flags_and_the_stage_passes"),
    M("b-stage-audio-off-by-default", "pab", "AUDIO = True", "AUDIO = False",
      PB + "test_boot_command_carries_the_guest_ram_and_the_sound_card_by_default", BG + "test_cli_defaults_and_switches"),
    M("b-stage-ram-default-8192", "pab", "GUEST_RAM_MB = 16384", "GUEST_RAM_MB = 8192",
      PB + "test_boot_command_carries_the_guest_ram_and_the_sound_card_by_default", BG + "test_cli_defaults_and_switches"),
    M("b-stage-ram-override-field-ignored", "pab", 'ram = a.get("ram_mb", GUEST_RAM_MB)', "ram = GUEST_RAM_MB",
      PB + "test_boot_command_override_fields_change_or_drop_them", BG + "test_run_arguments_switch_both_off"),
    M("b-stage-ram-refusal-skipped", "pab", "        check_guest_ram(pi_check, guest_ram_mb)\n", "",
      BG + "test_an_override_above_75_percent_of_memavailable_is_refused_before_anything_is_fetched", BG + "test_the_boundary_is_exactly_75_percent"),
    M("b-stage-refusal-reason-lost", "pab", 'named.group(1) if named else "RAM_OVERRIDE_INVALID"', '"RAM_OVERRIDE_INVALID"',
      BG + "test_an_override_above_75_percent_of_memavailable_is_refused_before_anything_is_fetched", BG + "test_the_boundary_is_exactly_75_percent"),
    M("b-stage-zero-still-checked", "pab", "    if not guest_ram_mb:\n        return", "    if False:\n        return",
      BG + "test_an_invalid_override_is_refused_by_name_and_zero_skips_the_check", BG + "test_run_arguments_switch_both_off"),
    M("b-check-ram-refusal-dropped", "check", "            PI.check_ram_override(a.ram_mb, PI.mem_available_mb())", "            pass",
      CK + "test_pipeline_refuses_a_ram_override_above_75_percent_of_memavailable_before_any_boot", CK + "test_pipeline_refuses_an_invalid_ram_override"),
    M("b-check-ram-not-forwarded", "check", "ram_mb=a.ram_mb, audio=a.audio,", "ram_mb=None, audio=a.audio,",
      CK + "test_pipeline_passes_the_ram_and_audio_overrides_to_the_argv", CK + "test_pipeline_ram_alone_sets_no_sound_card"),
    M("b-check-audio-not-forwarded", "check", "ram_mb=a.ram_mb, audio=a.audio,", "ram_mb=a.ram_mb, audio=False,",
      CK + "test_pipeline_passes_the_ram_and_audio_overrides_to_the_argv", CK + "test_pipeline_audio_alone_sets_no_ram_override"),
    # image B test branch (2026-11-20): --pins, the discovery launch rows, the UI text capture (host side), ui_table.py. Two named killers each.
    M("b-stage-pins-flag-not-wired", "pab", "        ctx.pins_path = check_pins_file(a.pins) if a.pins else PINS_PATH\n", "        ctx.pins_path = PINS_PATH\n",
      PF + "test_main_hands_the_given_file_to_the_run", PF + "test_main_refuses_a_missing_file_by_name_before_the_run"),
    M("b-stage-run-ignores-the-given-pins", "pab", "ctx.pins_path = check_pins_file(PINS_PATH if ctx.pins_path == pa.PINS_PATH else ctx.pins_path)",
      "ctx.pins_path = check_pins_file(PINS_PATH)",
      PF + "test_the_record_image_block_shows_the_given_file", PF + "test_an_unpinned_data_role_in_the_given_file_still_refuses"),
    M("b-stage-pins-missing-not-named", "pab", '        raise pa.Refused("PINS_MISSING")', '        raise pa.Refused("PINS_MALFORMED")',
      PF + "test_main_refuses_a_missing_file_by_name_before_the_run", PF + "test_the_run_itself_refuses_a_missing_or_malformed_file_before_anything_starts"),
    M("b-stage-pins-malformed-accepted", "pab", "        doc = pa.load_pins(p)", '        doc = {"items": [{"role": "data"}]}',
      PF + "test_main_refuses_a_malformed_file_by_name_before_the_run", PF + "test_the_run_itself_refuses_a_missing_or_malformed_file_before_anything_starts"),
    M("b-stage-pins-size-unbounded", "pab", "        if p.stat().st_size > PINS_MAX_BYTES:\n            raise ValueError(\"size\")", "        if False:\n            raise ValueError(\"size\")",
      PF + "test_main_refuses_a_malformed_file_by_name_before_the_run", PF + "test_the_run_itself_refuses_a_missing_or_malformed_file_before_anything_starts"),
    M("b-stage-pins-without-data-accepted", "pab", '    if DATA_ROLE not in {i["role"] for i in doc["items"]}:', "    if False:",
      PF + "test_a_file_without_the_data_volume_is_refused_by_name", PF + "test_the_run_refuses_a_file_without_the_data_volume_before_anything_starts"),
    M("b-stage-last-shrink-tier-dropped", "pab", "(4, 1, 0), (2, 1, 0), (0, 0, 0)):", "(4, 1, 0), (2, 1, 0)):",
      SH + "test_every_row_count_up_to_the_whole_table_fits_the_bound", RP2 + "test_the_worst_case_record_is_shrunk_by_the_stage_to_fit_and_still_validates"),
    M("b-stage-shrink-keeps-the-evidence", "pab", 'a["evidence"], a["host_xid"]["lines"] = a["evidence"][:evidence], a["host_xid"]["lines"][:lines]',
      'a["evidence"], a["host_xid"]["lines"] = a["evidence"], a["host_xid"]["lines"][:lines]',
      SH + "test_every_row_count_up_to_the_whole_table_fits_the_bound", RP2 + "test_the_worst_case_record_is_shrunk_by_the_stage_to_fit_and_still_validates"),
    M("b-stage-ui-text-not-copied-as-evidence", "pab", '"steps.log", "uitext.txt", "uitext_hits.txt", "inventory.txt")', '"steps.log")',
      "test_propimage_apps_b_stage.Run.test_whole_run_passes_and_copies_text_evidence_only", RS + "test_ui_text_files_land_in_the_evidence_directory_of_their_row"),
    M("b-stage-boot-need-ignores-the-row-timeouts", "pab", '            need = sum(timeouts[a] for a in boot["apps"])', "            need = 100",
      RS + "test_each_boot_is_given_the_sum_of_its_rows_timeouts_plus_the_slack", RS + "test_the_deadline_guard_counts_the_new_rows_timeouts"),
    M("b-stage-discovery-launch-rows-share-a-boot", "pab", '        if lanes and lanes.get(app) == "discovery":', "        if False:",
      RS + "test_discovery_launch_rows_boot_alone_and_gate_rows_share_a_boot_within_the_budget", RS + "test_the_plan_command_lists_the_new_rows_as_boots_of_their_own"),
    M("b-module-ui-files-not-pulled", "pabm", "    return names + (list(UI_FILES) if name in LAUNCH_ONLY else [])", "    return names",
      UPULL + "test_the_ui_files_are_in_the_pull_list_of_launch_rows_only", UPULL + "test_run_check_pulls_the_ui_files_and_scrubs_them_on_the_host"),
    M("b-module-ui-files-pulled-for-every-row", "pabm", "(list(UI_FILES) if name in LAUNCH_ONLY else [])", "list(UI_FILES)",
      UPULL + "test_the_ui_files_are_in_the_pull_list_of_launch_rows_only", UPULL + "test_a_row_that_is_not_a_launch_row_is_never_asked_for_the_ui_files"),
    M("b-module-ui-bound-lost", "pabm", "    return MAX_UI_PULL if fn in UI_FILES else MAX_PULL", "    return MAX_PULL",
      UPULL + "test_the_ui_files_have_their_own_small_bound", UPULL + "test_an_oversized_ui_file_is_not_pulled"),
    M("b-module-pulled-ui-files-not-scrubbed", "pabm", "                if fn in UI_FILES:\n                    scrub_file(d / fn)\n", "                pass\n",
      UPULL + "test_run_check_pulls_the_ui_files_and_scrubs_them_on_the_host", UPULL + "test_every_ui_file_is_scrubbed_including_the_inventory"),
    M("b-module-unscrubbable-file-kept", "pabm", "        p.unlink(missing_ok=True)\n", "        pass\n",
      UPULL + "test_scrub_file_deletes_a_file_it_cannot_scrub", UPULL + "test_a_file_that_grew_after_the_size_check_is_deleted"),
    M("b-module-scrub-email-dropped", "pabm", '    r"[A-Za-z0-9._%+-]+@[A-Za-z0-9.-]+\\.[A-Za-z]{2,}",\n', "",
      USCR + "test_email_guid_hex_key_and_after_keyword", UCAP + "test_the_scrub_patterns_equal_the_python_ones"),
    M("b-module-scrub-key-pattern-dropped", "pabm", '    r"\\b[A-Za-z0-9]{4,6}(?:-[A-Za-z0-9]{4,6}){2,}\\b",\n', "",
      USCR + "test_email_guid_hex_key_and_after_keyword", UCAP + "test_the_scrub_patterns_equal_the_python_ones"),
    M("b-module-scrub-after-keyword-dropped", "pabm", '        out.append(re.sub(SCRUB_AFTER, r"\\1 " + SCRUB, line))', "        out.append(line)",
      USCR + "test_email_guid_hex_key_and_after_keyword", USCR + "test_a_multi_line_text_is_scrubbed_line_by_line_and_keeps_its_newline"),
    M("b-module-hit-word-dropped", "pabm", '"library", "cloud", "account")', '"library", "account")',
      UCAP + "test_the_hit_words_equal_the_python_ones", UWORDS + "test_the_requested_words_are_all_hit_words"),
    M("b-module-ui-hits-not-reported", "pabm", '            row["ui_hits"] = ui_hits(r, directory)       # the UI text capture is reported, never graded\n', "            pass\n",
      UGRADE + "test_ui_hits_are_counted_from_the_pulled_file_and_reported", UGRADE + "test_without_the_file_the_guest_count_is_used_and_without_both_it_is_zero"),
    M("b-module-ui-hits-from-the-guest-only", "pabm", "        if p.is_file() and not p.is_symlink() and p.stat().st_size <= MAX_UI_PULL:", "        if False:",
      UGRADE + "test_ui_hits_are_counted_from_the_pulled_file_and_reported", UGRADE + "test_blank_lines_are_not_counted"),
    M("b-module-new-row-not-launch-only", "pabm", '"dreamweaver", "incopy", "substance_designer",\n               "substance_modeler",', '"substance_designer",\n               "substance_modeler",',
      UTAB + "test_discovery_rows_are_launch_only_discovery_rows_with_a_short_timeout", "test_propimage_apps_b.T.test_every_check_has_a_script_and_grader"),
    M("b-module-discovery-row-timeout-1200", "pabm", '"dreamweaver": {"timeout": 900,', '"dreamweaver": {"timeout": 1200,',
      UTAB + "test_discovery_rows_are_launch_only_discovery_rows_with_a_short_timeout", RS + "test_each_boot_is_given_the_sum_of_its_rows_timeouts_plus_the_slack"),
    M("b-module-discovery-row-is-a-gate-row", "pabm", '"incopy": {"timeout": 900, "script": "launch.ps1", "gpu": (), "lane": "discovery"', '"incopy": {"timeout": 900, "script": "launch.ps1", "gpu": (), "lane": "gate"',
      UTAB + "test_discovery_rows_are_launch_only_discovery_rows_with_a_short_timeout", "test_propimage_apps_b_stage.Report.test_the_row_table_equals_the_application_table"),
    M("ui-table-severity-order-reversed", "uit", 'SEVERITIES = ("high", "medium", "low")', 'SEVERITIES = ("low", "medium", "high")',
      UT + "test_ordering_is_severity_then_app_then_appearance", UT + "test_high_severity_rows_survive_the_bound"),
    M("ui-table-no-dedupe", "uit", "            if not name or key in seen:", "            if not name:",
      UT + "test_dedupe_per_app_ignores_case_and_keeps_other_apps", UT + "test_the_same_text_in_the_out_tree_and_its_evidence_copy_is_one_row"),
    M("ui-table-cells-not-scrubbed", "uit", '    return scrub_text(text).replace("|", "\\\\|").replace("\\n", " ").strip() or "-"', '    return text.replace("|", "\\\\|").replace("\\n", " ").strip() or "-"',
      UT + "test_the_table_text_is_scrubbed_even_when_the_file_was_not", UT + "test_the_app_and_feature_cells_are_scrubbed_too"),
    M("ui-table-unbounded", "uit", "    for _, (app, feature, text, sev) in ordered[:max_rows]:", "    for _, (app, feature, text, sev) in ordered:",
      UT + "test_the_table_is_bounded_and_says_how_many_rows_were_cut", UT + "test_high_severity_rows_survive_the_bound"),
    M("ui-table-heading-line-is-its-own-feature", "uit", '        out.append(heading_at[i - 1] if i else "-")', '        out.append(heading_at[i] if i is not None else "-")',
      UT + "test_no_heading_before_the_text_gives_a_dash_and_a_heading_line_is_not_its_own_feature", UT + "test_without_a_context_file_the_hit_file_is_the_context"),
    # the payload wiring
    M("payload-stage-in-the-default-plan", "payload", "PROPIMAGE_APPS_IN_LARGE = False", "PROPIMAGE_APPS_IN_LARGE = True",
      W + "test_not_in_the_default_plan_and_only_with_only_in_large", W + "test_manual_only_table_and_the_budget_guard"),
    M("payload-stage-in-every-tier", "payload", 'if i.propimages or (i.tier == "large" and (PROPIMAGE_APPS_IN_LARGE or "propimage_apps" in only)):', 'if i.propimages or (PROPIMAGE_APPS_IN_LARGE or "propimage_apps" in only):',
      W + "test_not_in_the_default_plan_and_only_with_only_in_large", W + "test_dry_run_refuses_the_stage_outside_the_large_tier"),
    M("payload-record-sha-unbound", "payload", 'if type(doc) is not dict or doc.get("sha") != sha:\n        raise EvidenceError("propimage_apps")', 'if type(doc) is not dict:\n        raise EvidenceError("propimage_apps")',
      W + "test_rows_bind_the_record_to_the_sha", W + "test_rows_pass_fail_and_not_run"),
    M("payload-not-run-any-reason", "payload", 'doc.get("verdict") == "NOT_RUN" and doc.get("reason") == "CPU_VENDOR" and rc == 0',
      'doc.get("verdict") == "NOT_RUN" and rc == 0',
      W + "test_not_run_with_another_reason_is_a_failure", W + "test_rows_pass_fail_and_not_run"),
    M("payload-exit-code-ignored", "payload", 'if rc or doc.get("verdict") != "PASS":\n        raise EvidenceError("propimage_apps")', 'if doc.get("verdict") != "PASS":\n        raise EvidenceError("propimage_apps")',
      W + "test_a_nonzero_exit_with_a_pass_record_is_a_failure", W + "test_rows_pass_fail_and_not_run"),
    M("payload-record-not-kept", "payload", '    result["propimage_apps"] = doc\n', "    pass\n",
      W + "test_rows_need_exactly_one_parseable_line_and_keep_a_red_record", W + "test_rows_pass_fail_and_not_run"),
    M("payload-stage-after-the-spec-lane", "payload", '    if i.propimages or (i.tier == "large" and (PROPIMAGE_APPS_IN_LARGE or "propimage_apps" in only)):\n        names.append("propimage_apps")\n    if i.tier == "large" and (PROPIMAGE_APPS_B_IN_LARGE or "propimage_apps_b" in only):\n        names.append("propimage_apps_b")\n    if i.tier == "large" and (i.matlab is not None or MATLAB_IN_LARGE or "matlab" in only):\n        names.append("matlab")\n    if "specviewperf" in i.stages:\n        names.append("specviewperf")',
      '    if "specviewperf" in i.stages:\n        names.append("specviewperf")\n    if i.propimages or (i.tier == "large" and (PROPIMAGE_APPS_IN_LARGE or "propimage_apps" in only)):\n        names.append("propimage_apps")',
      W + "test_runs_after_every_windows_stage_and_before_the_spec_lane", W + "test_runs_before_the_spec_lane_without_windows_stages"),
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


def run_tests(root, names):
    done = subprocess.run([sys.executable, "-B", "-c", COLLECT, *names], cwd=root / "scripts/ci", capture_output=True, text=True,
                          timeout=1800, env={"PATH": os.environ["PATH"], "HOME": os.environ.get("HOME", "/tmp"), "PYTHONDONTWRITEBYTECODE": "1"})
    line = next((s for s in done.stdout.splitlines() if s.startswith("KF_MUTATION_RESULT=")), None)
    if line is None:
        raise SystemExit("harness failure: " + done.stderr[-600:] + done.stdout[-300:])
    return json.loads(line.split("=", 1)[1])


def one(parent, mutant):
    name, key, old, new, tests = mutant
    text = SOURCES[key]
    pairs = list(zip(old, new)) if isinstance(old, list) else [(old, new)]
    for anchor, _ in pairs:
        if text.count(anchor) != 1:
            return {"mutant": name, "count": 0, "killers": [], "error": "anchor x%d: %r" % (text.count(anchor), anchor[:60])}
    for anchor, replacement in pairs:
        text = text.replace(anchor, replacement)
    root = tree(parent, "m", {key: text})
    killers = sorted(set(run_tests(root, tests)["killers"]))
    shutil.rmtree(root, ignore_errors=True)
    print("%s: %d named killers" % (name, len(killers)), file=sys.stderr, flush=True)
    return {"mutant": name, "count": len(killers), "killers": killers}


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--jobs", type=int, default=2)
    parser.add_argument("only", nargs="?", default="")
    args = parser.parse_args()
    chosen = [m for m in MUTANTS if args.only in m[0]]
    with tempfile.TemporaryDirectory(prefix="kf-propimage-apps-mutants-") as parent:
        baseline = run_tests(tree(parent, "base", {}), [SUITE.rstrip("."), "test_propimage_apps_b_stage", "test_propimage_check", "test_propimage_apps_b_ui", "test_ui_table"])
        if baseline["killers"]:
            print("BASELINE RED: %s" % baseline["killers"], file=sys.stderr)
            return 1
        print("baseline green: %d tests" % baseline["tests"], file=sys.stderr, flush=True)
        with concurrent.futures.ThreadPoolExecutor(args.jobs) as pool:
            rows = list(pool.map(lambda m: one(parent, m), chosen))
    bad = [r for r in rows if r["count"] < 2]
    print(json.dumps({"cases": rows}, indent=2))
    for row in bad:
        print("TOO FEW KILLERS: %(mutant)s (%(count)d) %(error)s" % dict(row, error=row.get("error", "")), file=sys.stderr)
    return 1 if bad else 0


if __name__ == "__main__":
    sys.exit(main())
