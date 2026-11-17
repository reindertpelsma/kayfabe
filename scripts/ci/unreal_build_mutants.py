#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later
"""Rule mutations of scripts/bench/unreal, each requiring two independently named killers (the whole suite runs per mutant).

STATUS: DESIGN-ONLY tooling, 2026-11-17. Run: python3 -B -I scripts/ci/unreal_build_mutants.py
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
FILES = {n: (ROOT / "scripts/bench/unreal" / (n + ".py")).read_text() for n in ("ue_lib", "resolve_unreal", "build_installed")}
SUPPORT = ("scripts/bench/windows/clockrule.py", "scripts/ci/box_downloader.py", "scripts/ci/kf_fetch.py", "ci/vast-broker/propimages.json",
           "scripts/bench/unreal/rows.json")


def case(name, mod, old, new, *tests):
    assert FILES[mod].count(old) == 1, (name, old, FILES[mod].count(old))
    return name, mod, FILES[mod].replace(old, new), tests


CASES = [
    case("key-ignores-commit", "ue_lib", '"source": {"kind": identity["kind"], "id": identity["id"], "dirty_digest"', '"source": {"kind": identity["kind"], "id": "x", "dirty_digest"',
         "test_key_changes_with_the_source_commit", "test_key_inputs_name_the_commit_the_engine_version_and_the_recipe"),
    case("key-ignores-dirty-digest", "ue_lib", '"dirty_digest": identity["dirty_digest"]},\n        "deps', '"dirty_digest": None},\n        "deps',
         "test_dirty_tree_is_refused_unless_allowed", "test_the_dirty_digest_depends_on_the_difference"),
    case("key-ignores-toolchain", "ue_lib", '"toolchain": dict(toolchain),', '"toolchain": {"sdk_version": "x"},',
         "test_key_changes_with_the_toolchain_version", "test_key_inputs_name_the_commit_the_engine_version_and_the_recipe"),
    case("key-ignores-flags", "ue_lib", '"flags": {"graph_target": r["graph_target"], "set": dict(r["set"])},', '"flags": {},',
         "test_key_inputs_hold_the_flags_but_never_the_machine_or_the_time", "test_key_inputs_name_the_commit_the_engine_version_and_the_recipe"),
    case("key-ignores-recipe-rev", "ue_lib", '"recipe_rev": recipe_rev(r),', '"recipe_rev": "r1",',
         "test_key_changes_with_a_recipe_field_that_is_not_a_flag", "test_bumping_the_recipe_version_changes_the_key"),
    case("recipe-rev-constant", "ue_lib", 'sha256_bytes(canonical(r).encode())[:12])', 'sha256_bytes(b"x")[:12])',
         "test_recipe_rev_follows_the_recipe_contents", "test_key_changes_with_a_recipe_field_that_is_not_a_flag"),
    case("key-ignores-target", "ue_lib", '"target": dict(r["target"]),', '"target": {},',
         "test_key_names_the_target_platform", "test_key_inputs_name_the_commit_the_engine_version_and_the_recipe"),
    case("key-ignores-engine-version", "ue_lib", '"engine": {k: version[k] for k in ("major", "minor", "branch", "compatible_changelist", "is_promoted", "patch")},', '"engine": {},',
         "test_key_changes_with_the_engine_version", "test_key_inputs_name_the_commit_the_engine_version_and_the_recipe"),
    case("key-ignores-deps-digest", "ue_lib", '"deps_manifest_sha256": deps_digest,', '"deps_manifest_sha256": None,',
         "test_key_changes_with_the_dependency_manifest", "test_key_inputs_name_the_commit_the_engine_version_and_the_recipe"),
    case("dirty-always-allowed", "ue_lib", "        if not allow_dirty:\n", "        if False:\n",
         "test_dirty_tree_is_refused_unless_allowed", "test_real_git_identity_and_dirty_detection"),
    case("declared-id-unchecked", "ue_lib", 'if not source_id or not re.fullmatch(r"[A-Za-z0-9_.:+-]{8,128}", source_id):', 'if False:',
         "test_a_tree_without_history_needs_a_declared_id", "test_a_declared_id_must_be_plain"),
    case("layout-unchecked", "ue_lib", '        if not os.path.isfile(os.path.join(src, rel)):\n            raise UnrealError(FAIL_SOURCE_LAYOUT, "not an Unreal source root', '        if False:\n            raise UnrealError(FAIL_SOURCE_LAYOUT, "not an Unreal source root',
         "test_layout_files_are_required", "test_a_tree_without_setup_sh_is_not_a_source_root"),
    case("disk-half-counted", "ue_lib", '        if free < g["need"]:', '        if free * 2 < g["need"]:',
         "test_disk_refuses_below_the_budget_and_says_the_numbers", "test_disk_boundary_is_exact"),
    case("disk-groups-by-location", "ue_lib", 'g = groups.setdefault(dev_of(path), {', 'g = groups.setdefault(loc, {',
         "test_two_locations_on_one_filesystem_add_up", "test_buildgraph_sums_the_source_and_the_output_on_one_filesystem"),
    case("disk-reserve-dropped", "ue_lib", '{"need": reserve_gb * GB, "locations"', '{"need": 0, "locations"',
         "test_the_reserve_is_part_of_the_budget", "test_disk_boundary_is_exact"),
    case("ram-unchecked", "ue_lib", "    if total_gb < MIN_RAM_GB:", "    if False:",
         "test_ram_below_the_minimum_is_refused", "test_nothing_runs_when_the_memory_is_short"),
    case("jobs-ignore-ram", "ue_lib", "n = max(1, min(int(cpus), by_ram))", "n = max(1, int(cpus))",
         "test_jobs_are_bounded_by_cpus_and_memory_and_a_request_only_lowers", "test_steps_run_in_order_with_the_recipe_arguments"),
    case("jobs-request-raises", "ue_lib", "n = max(1, min(n, int(requested)))", "n = max(1, max(n, int(requested)))",
         "test_jobs_are_bounded_by_cpus_and_memory_and_a_request_only_lowers", "test_a_jobs_request_lowers_the_parallel_actions_in_the_build_command"),
    case("buildgraph-cleans", "ue_lib", 'argv.append("-set:BuiltDirectory=" + installed_dir)', 'argv += ["-clean", "-set:BuiltDirectory=" + installed_dir]',
         "test_steps_run_in_order_with_the_recipe_arguments", "test_the_build_command_never_cleans_and_switches_off_arm64"),
    case("arm64-left-on", "ue_lib", '"WithLinuxArm64": "false",', '"WithLinuxArm64": "true",',
         "test_the_linux_arm64_target_is_switched_off_explicitly", "test_the_build_command_never_cleans_and_switches_off_arm64"),
    case("state-key-ignored", "ue_lib", '            if d.get("key") != key:', "            if False:",
         "test_a_state_belongs_to_one_key", "test_state_of_another_key_refuses_and_restart_clears_it"),
    case("state-schema-ignored", "ue_lib", 'if d.get("schema") != STATE_SCHEMA or not isinstance(d.get("phases"), dict):', "if False:",
         "test_a_state_file_of_another_schema_refuses", "test_a_state_whose_phases_are_not_a_mapping_refuses"),
    case("failed-phase-marked-done", "ue_lib", '"done": rc == 0,', '"done": True,',
         "test_a_failed_phase_is_not_marked_done", "test_a_second_run_resumes_at_the_failed_phase"),
    case("manifest-key-not-recomputed", "ue_lib", '    if cache_key(doc["key_inputs"]) != doc["key"]:', "    if False:",
         "test_a_manifest_with_a_wrong_key_is_refused", "test_a_manifest_with_edited_key_inputs_is_refused"),
    case("manifest-loads-software-optional", "ue_lib", '    if doc.get("loads_software") is not True:', "    if False:",
         "test_loads_software_must_be_true", "test_manifest_strictness"),
    case("member-size-unchecked", "ue_lib", 'if size != a["size"] or sha != a["sha256"]:', 'if sha != a["sha256"]:',
         "test_member_check_catches_a_changed_size", "test_a_manifest_with_the_wrong_size_refuses_an_upload"),
    case("member-hash-unchecked", "ue_lib", 'if size != a["size"] or sha != a["sha256"]:', 'if size != a["size"]:',
         "test_member_check_catches_a_changed_hash_of_the_same_size", "test_upload_refuses_an_artifact_that_does_not_match_its_manifest"),
    case("pin-not-loaded-software", "ue_lib", '        "loads_software": True,\n        "created_utc": manifest["created_utc"],\n    }', '        "loads_software": False,\n        "created_utc": manifest["created_utc"],\n    }',
         "test_the_pin_entry_is_read_by_the_clock_rule_as_loaded_software", "test_pin_command_prints_a_pin_a_manifest_object_and_a_download_manifest"),
    case("object-name-ignores-key", "ue_lib", '% (prefix, version, key[:12], name)', '% (prefix, version, "0" * 12, name)',
         "test_object_name_is_derived_from_the_key", "test_the_pin_object_name_and_ref_carry_the_key_prefix"),
    case("upload-endpoint-unchecked", "ue_lib", '    if not re.fullmatch(r"https?://[A-Za-z0-9.:\\[\\]-]+(/[A-Za-z0-9._~/-]*)?", endpoint or ""):', "    if False:",
         "test_the_upload_plan_refuses_a_bad_endpoint", "test_upload_refuses_a_bad_endpoint_before_printing_anything"),
    case("tar-unsorted", "ue_lib", '"--sort=name", "--owner=0"', '"--owner=0"',
         "test_the_archive_command_is_sorted_with_a_fixed_owner", "test_package_writes_a_manifest_that_describes_the_archive"),
    case("env-leaks-credentials", "ue_lib", '"LOGNAME", "SHELL")', '"LOGNAME", "SHELL", "GH_TOKEN", "AWS_SECRET_ACCESS_KEY")',
         "test_no_credential_reaches_a_build_step", "test_only_the_clone_step_may_carry_a_token"),
    case("env-prompts-allowed", "ue_lib", '    "GIT_TERMINAL_PROMPT": "0",  # never prompt for credentials\n', "",
         "test_no_credential_reaches_a_build_step", "test_only_the_clone_step_may_carry_a_token"),
    case("env-token-everywhere", "ue_lib", "keep = ENV_ALLOW + (ENV_CLONE_EXTRA if clone else ())", "keep = ENV_ALLOW + ENV_CLONE_EXTRA",
         "test_no_credential_reaches_a_build_step", "test_only_the_clone_step_may_carry_a_token"),
    case("verify-tree-passes-anything", "ue_lib", '    problems = [p for p in RECIPE["required_in_tree"] if not os.path.isfile(os.path.join(tree, p))]', "    problems = []",
         "test_an_incomplete_installed_tree_is_a_failure_and_a_missing_expected_file_a_warning", "test_a_directory_is_used_in_standalone_without_a_key_and_refused_in_strict"),
    case("worker-builds", "resolve_unreal", '    if o.mode == "worker" and (o.source or o.source_clone):', "    if False:",
         "test_a_worker_never_builds_from_a_source_path", "test_a_worker_never_clones_a_source"),
    case("standalone-enforces-pins", "resolve_unreal", '    enforce_pins = o.mode in ("strict", "worker")', "    enforce_pins = True",
         "test_standalone_enforces_no_pin_and_records_the_computed_sha256", "test_a_cache_listing_is_fetched_and_a_standalone_run_does_not_enforce_its_hash"),
    case("strict-does-not-enforce-pins", "resolve_unreal", '    enforce_pins = o.mode in ("strict", "worker")', '    enforce_pins = o.mode == "worker"',
         "test_strict_and_worker_enforce_the_pin", "test_a_pin_for_another_key_is_a_key_mismatch"),
    case("integrity-skipped", "resolve_unreal", "        sha, size = f.member_check(a, man)  # integrity against its OWN manifest: every mode", '        sha, size = ("0" * 64, 0)',
         "test_integrity_against_its_own_manifest_is_enforced_in_every_mode", "test_an_explicit_artifact_is_used_in_standalone"),
    case("artifact-key-unchecked", "resolve_unreal", '        if comp["key"] != man["key"]:', "        if False:",
         "test_artifact_and_source_with_different_keys_are_refused", "test_the_key_mismatch_names_both_keys"),
    case("stale-listing-ignored", "resolve_unreal", '            if comp["key"] != key:', "            if False:",
         "test_a_stale_listing_with_a_source_rebuilds_in_standalone", "test_a_stale_listing_with_a_source_fails_in_strict"),
    case("worker-miss-is-a-skip", "resolve_unreal", "return _fail(L.FAIL_CACHE_MISS,", "return _fail(L.SKIP_NO_SOURCE_OR_CACHE,",
         "test_nothing_given_skips_in_standalone_fails_in_strict_and_misses_in_worker", "test_cli_resolve_in_worker_mode_without_a_cache_is_a_failure"),
    case("strict-skips-quietly", "resolve_unreal", '    if o.mode == "strict":\n        return _fail(L.SKIP_NO_SOURCE_OR_CACHE', '    if False:\n        return _fail(L.SKIP_NO_SOURCE_OR_CACHE',
         "test_nothing_given_skips_in_standalone_fails_in_strict_and_misses_in_worker", "test_cli_resolve_in_strict_mode_without_anything_fails"),
    case("expect-sha-unchecked", "resolve_unreal", "    if o.expect_sha256 is not None and sha != o.expect_sha256:", "    if False:",
         "test_expect_sha256_mismatch_is_refused_and_names_both", "test_cli_verify_checks_an_expected_hash_when_one_is_given"),
    case("directory-in-strict", "resolve_unreal", "        if enforce_pins:\n            return _fail(L.FAIL_MANIFEST, \"a directory has no manifest", "        if False:\n            return _fail(L.FAIL_MANIFEST, \"a directory has no manifest",
         "test_strict_refuses_a_directory_because_it_has_no_key", "test_cli_resolve_refuses_a_directory_in_strict_mode"),
    case("clone-fails-instead-of-skipping", "resolve_unreal", '            if o.mode == "standalone":\n                return Decision("skip"', '            if False:\n                return Decision("skip"',
         "test_clone_needs_a_login_and_repository_access", "test_cli_resolve_skips_when_a_clone_is_not_possible"),
    case("artifact-not-first", "resolve_unreal", "    if o.artifact:\n        return _use_artifact(o, f, enforce_pins)", "    if False and o.artifact:\n        return _use_artifact(o, f, enforce_pins)",
         "test_an_explicit_artifact_is_used_in_standalone", "test_the_order_is_artifact_then_listing_then_source"),
    case("modify-flag-ignored", "build_installed", "        if not allow_modify_source:", "        if False:",
         "test_the_build_needs_the_flag_that_allows_modifying_the_source", "test_cli_build_without_the_modify_flag_is_refused_before_anything_runs"),
    case("phase-disk-check-dropped", "build_installed", "            L.check_disk(phase, self.locations(out), free_fn=self.free_fn, dev_of=self.dev_of)", "            pass",
         "test_nothing_runs_when_the_disk_is_short", "test_the_disk_is_checked_again_before_each_phase"),
    case("resume-skip-dropped", "build_installed", "            if state.done(phase):", "            if False:",
         "test_a_second_run_resumes_at_the_failed_phase", "test_a_finished_build_runs_nothing_again"),
    case("package-disk-check-dropped", "build_installed", '        L.check_disk("package", self.locations(out), free_fn=self.free_fn, dev_of=self.dev_of)', "        pass",
         "test_the_package_phase_checks_the_disk_of_the_output_directory", "test_a_short_disk_at_package_time_leaves_no_artifact_and_no_manifest"),
    case("reclaim-follows-links", "build_installed", '    if os.path.realpath(p) != os.path.join(real_root, rel) or os.path.islink(p):', "    if False:",
         "test_reclaim_refuses_a_symlinked_directory", "test_safe_child_refuses_a_path_that_leaves_the_root"),
    case("reclaim-too-much", "build_installed", 'RECLAIM = ("Engine/Intermediate/Build", "LocalBuilds/InstalledDDC")', 'RECLAIM = ("Engine/Intermediate/Build", "LocalBuilds/InstalledDDC", "Engine/Source")',
         "test_reclaim_removes_only_the_two_generated_directories", "test_the_reclaim_list_is_exactly_the_generated_directories"),
    case("upload-runs-without-execute", "build_installed", "    if not a.execute:", "    if False:",
         "test_upload_prints_commands_and_does_not_run_them_without_execute", "test_upload_without_execute_says_it_only_printed"),
    case("upload-skips-integrity", "build_installed", "    L.check_member(a.artifact, man)  # never upload what does not match its own manifest", "    pass",
         "test_upload_refuses_an_artifact_that_does_not_match_its_manifest", "test_a_manifest_with_the_wrong_size_refuses_an_upload"),
]


def run(tree):
    proc = subprocess.run([sys.executable, "-I", "-B", str(tree / "scripts/ci/test_unreal_build.py")], text=True, capture_output=True, timeout=900)
    out = proc.stdout + proc.stderr
    return proc.returncode, sorted(set(re.findall(r"^(?:FAIL|ERROR): (\S+)", out, re.M))), out


def main():
    argparse.ArgumentParser(description=__doc__).parse_args()
    test_src = ROOT / "scripts/ci/test_unreal_build.py"
    known = {n.name for n in ast.walk(ast.parse(test_src.read_text())) if isinstance(n, ast.FunctionDef)}
    unknown = {t for c in CASES for t in c[3]} - known
    assert not unknown, "unknown test selector: %s" % unknown
    weak = 0
    with tempfile.TemporaryDirectory(prefix="unreal-mutants-") as tmp:
        tree = Path(tmp)
        for rel in SUPPORT:
            (tree / rel).parent.mkdir(parents=True, exist_ok=True)
            shutil.copy(ROOT / rel, tree / rel)
        (tree / "scripts/ci").mkdir(parents=True, exist_ok=True)
        shutil.copy(test_src, tree / "scripts/ci/test_unreal_build.py")
        (tree / "scripts/bench/unreal").mkdir(parents=True, exist_ok=True)

        def put(mod, text):
            (tree / "scripts/bench/unreal" / (mod + ".py")).write_text(text)

        for m, t in FILES.items():
            put(m, t)
        code, _, out = run(tree)
        if code:
            print("FAIL unmutated baseline\n" + out[-4000:])
            return 2
        print("PASS baseline: " + re.search(r"Ran (\d+) tests", out).group(1), flush=True)
        for name, mod, mutated, tests in CASES:
            put(mod, mutated)
            try:
                code, killers, out = run(tree)
            finally:
                put(mod, FILES[mod])
            names = [k.split(".")[-1] for k in killers]
            ok = code != 0 and len(set(names)) >= 2 and bool(set(tests) & set(names))
            print("%s %s: %d killers %s" % ("PASS" if ok else "WEAK", name, len(set(names)), names[:4]), flush=True)
            weak += not ok
    print("%d/%d mutants killed by at least two tests" % (len(CASES) - weak, len(CASES)))
    return 1 if weak else 0


if __name__ == "__main__":
    sys.exit(main())
